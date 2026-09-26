//! Index access paths over equality and range indexes.
//!
//! Plans [`PhysicalIndexScan`]s for predicates and orderings a plain
//! [`PhysicalSource::IndexLookup`](crate::PhysicalSource::IndexLookup)
//! cannot serve:
//!
//! - comparisons (`<`, `<=`, `>`, `>=`, `BETWEEN`) and string prefixes
//!   (`LIKE 'abc%'`, `~ '^abc'`) on range indexes,
//! - `IN` lists (one probe per value) on equality and range indexes,
//! - composite indexes: equality on leading columns, optionally followed by
//!   one range column,
//! - partial indexes, when the query's conjuncts contain every conjunct of
//!   the index predicate (syntactically, after canonicalization),
//! - `ORDER BY` of range index columns (ascending or descending), replacing
//!   the sort,
//! - index-only reads when a projection only needs key columns and ids.
//!
//! Key ranges compare values like filters do (`Value` order, see
//! [`crate::IndexScanRange`]), so every consumed comparison, `IN` and
//! `BETWEEN` conjunct is served exactly and dropped from the residual
//! predicate; string patterns stay residual.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use semantic_data::query::{BinaryOp, PatternMatchKind, SortDirection};
use semantic_data::schema::IndexKind;
use semantic_data::value::{FieldPath, PathSegment, Value};

use super::optimizer::{
    collect_binary_terms, combine_conjuncts, flatten_boolean_expr, resolve_collection_schema,
};
use crate::catalog::{CollectionSchema, IndexSchema, PRIMARY_ID_FIELD};
use crate::plan::{PhysicalIndexScan, PhysicalPlan, PhysicalSource, SourceRef, StatsProvider};
use crate::query::{Expr, Operand, OrderBy, QueryField};
use crate::{IndexColumnRange, IndexScanRange, QueryContext};

/// Upper bound on the probes an `IN` expansion may produce.
const MAX_PROBES: usize = 256;
/// Row count assumed without statistics.
const DEFAULT_ROWS: f64 = 10_000.0;
/// Selectivity of one equality without field statistics.
const EQ_SELECTIVITY: f64 = 0.1;
/// Selectivity of a one-sided comparison.
const OPEN_RANGE_SELECTIVITY: f64 = 0.3;
/// Selectivity of a two-sided comparison or a string prefix.
const CLOSED_RANGE_SELECTIVITY: f64 = 0.1;
/// Selectivity assumed per residual conjunct.
const RESIDUAL_SELECTIVITY: f64 = 0.3;

/// A conjunct an index can serve, on one top-level field.
#[derive(Debug, Clone)]
enum Term {
    Eq(Value),
    In(Vec<Value>),
    Bounds {
        lower: Bound<Value>,
        upper: Bound<Value>,
    },
    Prefix(String),
}

/// A sargable conjunct.
struct Sarg {
    field: String,
    term: Term,
    /// Whether key ranges match exactly the rows the conjunct matches.
    exact: bool,
}

/// The sargable conjuncts on one field, merged.
#[derive(Default)]
struct FieldTerms {
    eq: Option<(Value, usize, bool)>,
    in_list: Option<(Vec<Value>, usize, bool)>,
    lower: Option<Bound<Value>>,
    upper: Option<Bound<Value>>,
    bound_conjuncts: Vec<usize>,
    prefix: Option<(String, usize, bool)>,
}

impl FieldTerms {
    fn add(&mut self, conjunct: usize, term: Term, exact: bool) {
        match term {
            Term::Eq(value) => {
                self.eq.get_or_insert((value, conjunct, exact));
            }
            Term::In(values) => {
                self.in_list.get_or_insert((values, conjunct, exact));
            }
            Term::Bounds { lower, upper } => {
                self.lower = Some(tighter_lower(self.lower.take(), lower));
                self.upper = Some(tighter_upper(self.upper.take(), upper));
                self.bound_conjuncts.push(conjunct);
            }
            Term::Prefix(prefix) => {
                self.prefix.get_or_insert((prefix, conjunct, exact));
            }
        }
    }

    fn has_bounds(&self) -> bool {
        !self.bound_conjuncts.is_empty()
    }
}

fn tighter_lower(current: Option<Bound<Value>>, next: Bound<Value>) -> Bound<Value> {
    let Some(current) = current else {
        return next;
    };
    match (&current, &next) {
        (Bound::Unbounded, _) => next,
        (_, Bound::Unbounded) => current,
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => {
            match a.cmp(b) {
                std::cmp::Ordering::Less => next,
                std::cmp::Ordering::Greater => current,
                std::cmp::Ordering::Equal if matches!(next, Bound::Excluded(_)) => next,
                std::cmp::Ordering::Equal => current,
            }
        }
    }
}

fn tighter_upper(current: Option<Bound<Value>>, next: Bound<Value>) -> Bound<Value> {
    let Some(current) = current else {
        return next;
    };
    match (&current, &next) {
        (Bound::Unbounded, _) => next,
        (_, Bound::Unbounded) => current,
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => {
            match a.cmp(b) {
                std::cmp::Ordering::Greater => next,
                std::cmp::Ordering::Less => current,
                std::cmp::Ordering::Equal if matches!(next, Bound::Excluded(_)) => next,
                std::cmp::Ordering::Equal => current,
            }
        }
    }
}

/// The top-level field `expr` reads, if it is a plain field operand.
fn top_level_field(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Operand(Operand::Field(path)) => match path.segments() {
            [PathSegment::Field(field)] => Some(field.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn literal(expr: &Expr) -> Option<&Value> {
    match expr {
        Expr::Operand(Operand::Literal(value)) => Some(value),
        _ => None,
    }
}

/// Explicit `Void` values read like missing columns in composite keys, so
/// terms comparing with `Void` are kept as residual predicates.
fn is_exact_value(value: &Value) -> bool {
    !matches!(value, Value::Void)
}

fn comparison_term(op: BinaryOp, value: Value) -> Option<Term> {
    Some(match op {
        BinaryOp::Eq => Term::Eq(value),
        BinaryOp::Lt => Term::Bounds {
            lower: Bound::Unbounded,
            upper: Bound::Excluded(value),
        },
        BinaryOp::Lte => Term::Bounds {
            lower: Bound::Unbounded,
            upper: Bound::Included(value),
        },
        BinaryOp::Gt => Term::Bounds {
            lower: Bound::Excluded(value),
            upper: Bound::Unbounded,
        },
        BinaryOp::Gte => Term::Bounds {
            lower: Bound::Included(value),
            upper: Bound::Unbounded,
        },
        BinaryOp::In => match value {
            Value::List(values) => in_term(values),
            _ => return None,
        },
        _ => return None,
    })
}

fn flip(op: BinaryOp) -> Option<BinaryOp> {
    Some(match op {
        BinaryOp::Eq => BinaryOp::Eq,
        BinaryOp::Lt => BinaryOp::Gt,
        BinaryOp::Lte => BinaryOp::Gte,
        BinaryOp::Gt => BinaryOp::Lt,
        BinaryOp::Gte => BinaryOp::Lte,
        _ => return None,
    })
}

fn in_term(mut values: Vec<Value>) -> Term {
    values.sort();
    values.dedup();
    if values.len() == 1 {
        Term::Eq(values.pop().expect("one value"))
    } else {
        Term::In(values)
    }
}

/// Literal prefix every string matching the `LIKE` pattern starts with,
/// and whether the pattern is exactly that prefix followed by `%`.
fn like_prefix(pattern: &str) -> Option<(String, bool)> {
    let end = pattern.find(['%', '_']).unwrap_or(pattern.len());
    let prefix = &pattern[..end];
    (!prefix.is_empty()).then(|| {
        (
            prefix.to_string(),
            pattern.len() == end + 1 && pattern.ends_with('%'),
        )
    })
}

/// Literal prefix every string matching an anchored regex starts with.
///
/// Only a leading run of plain characters after `^` counts; the character
/// before a quantifier is excluded and alternations disable the prefix.
fn regex_prefix(pattern: &str) -> Option<String> {
    let rest = pattern.strip_prefix('^')?;
    if pattern.contains('|') {
        return None;
    }
    let mut prefix = String::new();
    for c in rest.chars() {
        if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | ':' | '/' | '@' | ',' | '=') {
            prefix.push(c);
            continue;
        }
        if matches!(c, '*' | '?' | '{') {
            prefix.pop();
        }
        break;
    }
    (!prefix.is_empty()).then_some(prefix)
}

fn sargable(conjunct: &Expr) -> Option<Sarg> {
    match conjunct {
        Expr::Binary { op, left, right } => {
            let (field, op, value) = match (top_level_field(left), literal(right)) {
                (Some(field), Some(value)) => (field, *op, value),
                _ => (top_level_field(right)?, flip(*op)?, literal(left)?),
            };
            let exact = match value {
                Value::List(values) => values.iter().all(is_exact_value),
                value => is_exact_value(value),
            };
            Some(Sarg {
                field: field.to_string(),
                term: comparison_term(op, value.clone())?,
                exact,
            })
        }
        Expr::InList {
            expr,
            list,
            negated: false,
        } => {
            let field = top_level_field(expr)?;
            let values = list.iter().map(literal).collect::<Option<Vec<_>>>()?;
            Some(Sarg {
                field: field.to_string(),
                exact: values.iter().all(|value| is_exact_value(value)),
                term: in_term(values.into_iter().cloned().collect()),
            })
        }
        Expr::Between {
            expr,
            low,
            high,
            negated: false,
        } => {
            let field = top_level_field(expr)?;
            let (low, high) = (literal(low)?, literal(high)?);
            Some(Sarg {
                field: field.to_string(),
                exact: is_exact_value(low) && is_exact_value(high),
                term: Term::Bounds {
                    lower: Bound::Included(low.clone()),
                    upper: Bound::Included(high.clone()),
                },
            })
        }
        Expr::PatternMatch {
            kind: PatternMatchKind::Like,
            expr,
            pattern,
            case_insensitive: false,
            negated: false,
        } => {
            let field = top_level_field(expr)?;
            let Value::String(pattern) = literal(pattern)? else {
                return None;
            };
            let (prefix, exact) = like_prefix(pattern)?;
            Some(Sarg {
                field: field.to_string(),
                term: Term::Prefix(prefix),
                exact,
            })
        }
        Expr::RegexMatch {
            expr,
            pattern,
            case_insensitive: false,
            negated: false,
        } => {
            let field = top_level_field(expr)?;
            let Value::String(pattern) = literal(pattern)? else {
                return None;
            };
            Some(Sarg {
                field: field.to_string(),
                term: Term::Prefix(regex_prefix(pattern)?),
                exact: false,
            })
        }
        _ => None,
    }
}

fn contains_unsupported(expr: &Expr) -> bool {
    match expr {
        Expr::Subquery(_) | Expr::Exists { .. } | Expr::RelationExists { .. } => true,
        Expr::Aggregate { .. } => true,
        Expr::Operand(_) => false,
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => contains_unsupported(expr),
        Expr::Binary { left, right, .. } => {
            contains_unsupported(left) || contains_unsupported(right)
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            contains_unsupported(cond)
                || contains_unsupported(then_expr)
                || contains_unsupported(else_expr)
        }
        Expr::Coalesce(items) => items.iter().any(contains_unsupported),
        Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            crate::FunctionArg::Expr(expr) => contains_unsupported(expr),
            crate::FunctionArg::Wildcard => false,
        }),
        Expr::InList { expr, list, .. } => {
            contains_unsupported(expr) || list.iter().any(contains_unsupported)
        }
        Expr::Between {
            expr, low, high, ..
        } => contains_unsupported(expr) || contains_unsupported(low) || contains_unsupported(high),
        Expr::PatternMatch { expr, pattern, .. } | Expr::RegexMatch { expr, pattern, .. } => {
            contains_unsupported(expr) || contains_unsupported(pattern)
        }
    }
}

/// Top-level conjuncts of `predicate`, flattened like the logical passes do.
fn conjuncts(predicate: Expr) -> Vec<Expr> {
    let mut out = Vec::new();
    collect_binary_terms(flatten_boolean_expr(predicate), BinaryOp::And, &mut out);
    out
}

/// Planning input shared by all candidate indexes of one source.
struct AccessRequest<'a> {
    source: &'a SourceRef,
    collection: &'a CollectionSchema,
    conjuncts: Vec<Expr>,
    terms: BTreeMap<String, FieldTerms>,
    stats: Option<&'a dyn StatsProvider>,
    context: &'a QueryContext,
    rows: f64,
}

impl<'a> AccessRequest<'a> {
    fn new(
        source: &'a SourceRef,
        predicate: Option<&Expr>,
        stats: Option<&'a dyn StatsProvider>,
        context: &'a QueryContext,
    ) -> Option<Self> {
        if source
            .source_name
            .as_deref()
            .is_some_and(crate::is_all_collection_alias)
        {
            return None;
        }
        let collection = resolve_collection_schema(context, source)?;
        if predicate.is_some_and(contains_unsupported) {
            return None;
        }
        let conjuncts = predicate.cloned().map(conjuncts).unwrap_or_default();
        let mut terms = BTreeMap::<String, FieldTerms>::new();
        for (position, conjunct) in conjuncts.iter().enumerate() {
            if let Some(sarg) = sargable(conjunct) {
                terms
                    .entry(sarg.field)
                    .or_default()
                    .add(position, sarg.term, sarg.exact);
            }
        }
        let rows = stats
            .and_then(|stats| stats.relation_stats(source))
            .map_or(DEFAULT_ROWS, |stats| stats.row_count)
            .max(0.0);
        Some(Self {
            source,
            collection,
            conjuncts,
            terms,
            stats,
            context,
            rows,
        })
    }

    fn entries(&self, index: &IndexSchema) -> f64 {
        self.stats
            .and_then(|stats| stats.index_entry_count(self.source, index.lid))
            .unwrap_or(self.rows)
            .max(0.0)
    }

    fn eq_selectivity(&self, field: &str) -> f64 {
        let Some(stats) = self.stats else {
            return EQ_SELECTIVITY;
        };
        let field_ref = super::optimizer::resolve_field_ref_for_schema(
            self.collection,
            &FieldPath::from_fields([field]),
        );
        stats
            .field_stats(self.source, &field_ref)
            .and_then(|stats| {
                let distinct = stats.distinct_count.filter(|distinct| *distinct > 0.0)?;
                let nulls = stats.null_fraction.unwrap_or(0.0).clamp(0.0, 1.0);
                Some(((1.0 - nulls) / distinct).clamp(0.0001, 1.0))
            })
            .unwrap_or(EQ_SELECTIVITY)
    }

    /// Positions of the conjuncts implied by `index`'s partial predicate, or
    /// `None` when the query does not imply it.
    fn implied_conjuncts(&self, index: &IndexSchema) -> Option<BTreeSet<usize>> {
        let Some(predicate) = index.predicate_expr() else {
            return Some(BTreeSet::new());
        };
        let predicate = crate::canonical::canonicalize_filter_expr(
            &predicate,
            self.context.catalog(),
            self.collection,
        )
        .ok()?;
        conjuncts(predicate)
            .iter()
            .map(|required| {
                self.conjuncts
                    .iter()
                    .position(|conjunct| conjunct == required)
            })
            .collect()
    }

    /// The best use of `index` for the predicate, whether or not it
    /// constrains any key column.
    fn candidate<'i>(&self, index: &'i IndexSchema) -> Option<Candidate<'i>> {
        if !index.schema.kind.is_value_index() {
            return None;
        }
        let implied = self.implied_conjuncts(index)?;
        let ranges_allowed = index.schema.kind == IndexKind::Range;
        let entries = self.entries(index);
        let mut candidate = Candidate {
            index,
            prefixes: vec![Vec::new()],
            fixed: 0,
            single_eq_columns: 0,
            range: None,
            consumed: BTreeSet::new(),
            exact: implied,
            selectivity: 1.0,
            entries,
        };
        let mut single_eq = true;
        for column in index.columns() {
            let Some(terms) = self.terms.get(column) else {
                break;
            };
            if let Some((value, conjunct, exact)) = &terms.eq {
                for prefix in &mut candidate.prefixes {
                    prefix.push(value.clone());
                }
                candidate.consume(*conjunct, *exact);
                candidate.selectivity *= self.eq_selectivity(column);
            } else if let Some((values, conjunct, exact)) = &terms.in_list {
                if candidate.prefixes.len() * values.len().max(1) > MAX_PROBES {
                    break;
                }
                candidate.prefixes = candidate
                    .prefixes
                    .iter()
                    .flat_map(|prefix| {
                        values.iter().map(move |value| {
                            let mut prefix = prefix.clone();
                            prefix.push(value.clone());
                            prefix
                        })
                    })
                    .collect();
                candidate.consume(*conjunct, *exact);
                candidate.selectivity *=
                    (values.len() as f64 * self.eq_selectivity(column)).min(1.0);
                single_eq = false;
            } else if ranges_allowed && terms.has_bounds() {
                let lower = terms.lower.clone().unwrap_or(Bound::Unbounded);
                let upper = terms.upper.clone().unwrap_or(Bound::Unbounded);
                candidate.selectivity *=
                    if matches!(lower, Bound::Unbounded) || matches!(upper, Bound::Unbounded) {
                        OPEN_RANGE_SELECTIVITY
                    } else {
                        CLOSED_RANGE_SELECTIVITY
                    };
                let exact = terms.bound_conjuncts.iter().all(|position| {
                    sargable(&self.conjuncts[*position]).is_some_and(|sarg| sarg.exact)
                });
                for position in &terms.bound_conjuncts {
                    candidate.consume(*position, exact);
                }
                candidate.range = Some(IndexColumnRange::Bounds { lower, upper });
                break;
            } else if ranges_allowed && let Some((prefix, conjunct, exact)) = &terms.prefix {
                candidate.selectivity *= CLOSED_RANGE_SELECTIVITY;
                candidate.consume(*conjunct, *exact);
                candidate.range = Some(IndexColumnRange::StringPrefix(prefix.clone()));
                break;
            } else {
                break;
            }
            candidate.fixed += 1;
            if single_eq {
                candidate.single_eq_columns += 1;
            }
        }
        if index.schema.unique
            && candidate.fixed == index.column_count()
            && candidate.prefixes.len() == 1
            && entries > 0.0
        {
            candidate.selectivity = candidate.selectivity.min(1.0 / entries);
        }
        Some(candidate)
    }

    fn candidates(&self) -> Vec<Candidate<'a>> {
        let catalog: &'a crate::catalog::Catalog = self.context.catalog();
        catalog
            .indexes_for_collection(self.collection.lid)
            .filter_map(|index| self.candidate(index))
            .collect()
    }

    fn residual(&self, candidate: &Candidate<'_>) -> Option<Expr> {
        combine_conjuncts(
            self.conjuncts
                .iter()
                .enumerate()
                .filter(|(position, _)| !candidate.exact.contains(position))
                .map(|(_, conjunct)| conjunct.clone()),
        )
    }

    fn residual_count(&self, candidate: &Candidate<'_>) -> usize {
        self.conjuncts.len().saturating_sub(candidate.exact.len())
    }

    fn build(
        &self,
        candidate: &Candidate<'_>,
        direction: SortDirection,
        ordered: bool,
        limit_hint: Option<usize>,
    ) -> PhysicalIndexScan {
        let predicate = combine_conjuncts(self.conjuncts.iter().cloned());
        PhysicalIndexScan {
            source: self.source.clone(),
            index: candidate.index.lid,
            index_name: candidate.index.schema.name.clone(),
            columns: candidate
                .index
                .columns()
                .map(|column| FieldPath::from_fields([column]))
                .collect(),
            ranges: candidate.ranges(),
            direction,
            ordered,
            limit_hint,
            index_only: false,
            predicate,
            residual_predicate: self.residual(candidate),
        }
    }
}

/// One way to read the matching rows through one index.
struct Candidate<'i> {
    index: &'i IndexSchema,
    /// Values of the leading `fixed` columns, one tuple per probe.
    prefixes: Vec<Vec<Value>>,
    /// Leading columns fixed by equality or `IN`.
    fixed: usize,
    /// Leading columns fixed by a single value each.
    single_eq_columns: usize,
    /// Range of column `fixed`.
    range: Option<IndexColumnRange>,
    /// Conjuncts the key ranges use.
    consumed: BTreeSet<usize>,
    /// Conjuncts every produced row satisfies without re-checking.
    exact: BTreeSet<usize>,
    selectivity: f64,
    entries: f64,
}

impl Candidate<'_> {
    fn consume(&mut self, conjunct: usize, exact: bool) {
        self.consumed.insert(conjunct);
        if exact {
            self.exact.insert(conjunct);
        }
    }

    /// Whether the key ranges constrain the first key column, which also
    /// excludes rows without it (they have no entry).
    fn constrains_first_column(&self) -> bool {
        self.fixed > 0 || self.range.is_some()
    }

    /// Estimated cost of reading the matching entries and their rows.
    fn cost(&self) -> f64 {
        self.entries * self.selectivity + (self.entries + 1.0).log2()
    }

    fn ranges(&self) -> Vec<IndexScanRange> {
        let mut prefixes = self.prefixes.clone();
        prefixes.sort();
        prefixes.dedup();
        let composite = self.index.is_composite();
        prefixes
            .into_iter()
            .map(|prefix| {
                if !composite {
                    return IndexScanRange::single(match prefix.into_iter().next() {
                        Some(value) => IndexColumnRange::eq(value),
                        None => self.range.clone().unwrap_or_else(IndexColumnRange::all),
                    });
                }
                let column = match self.range.clone() {
                    // Missing non-leading columns are keyed as `Void`, below
                    // every present value; comparisons never match them.
                    Some(IndexColumnRange::Bounds {
                        lower: Bound::Unbounded,
                        upper,
                    }) if self.fixed > 0 => IndexColumnRange::Bounds {
                        lower: Bound::Excluded(Value::Void),
                        upper,
                    },
                    Some(range) => range,
                    None => IndexColumnRange::all(),
                };
                IndexScanRange {
                    composite: true,
                    prefix,
                    column,
                }
            })
            .collect()
    }

    /// Whether scanning in key order yields rows ordered by `order_by`, and
    /// in which direction.
    fn order_direction(&self, order_by: &[OrderBy]) -> Option<SortDirection> {
        if self.index.schema.kind != IndexKind::Range || order_by.is_empty() {
            return None;
        }
        let direction = order_by[0].direction;
        if order_by.iter().any(|order| order.direction != direction) {
            return None;
        }
        let fields = order_by
            .iter()
            .map(|order| top_level_field(&order.expr))
            .collect::<Option<Vec<_>>>()?;
        let columns = self.index.columns().collect::<Vec<_>>();
        // Columns fixed to one value may be skipped or listed; after them
        // the order must follow the key columns.
        (0..=self.single_eq_columns.min(columns.len()))
            .any(|start| {
                columns[start..].len() >= fields.len()
                    && columns[start..start + fields.len()] == fields[..]
            })
            .then_some(direction)
    }
}

/// Choose an index scan answering `predicate` over `source` without regard
/// to ordering, when one is cheaper than `baseline_cost`.
///
/// Simple single-equality lookups are left to
/// [`PhysicalSource::IndexLookup`] (the caller's baseline).
pub(crate) fn plan_index_scan(
    source: &SourceRef,
    predicate: &Expr,
    stats: Option<&dyn StatsProvider>,
    context: &QueryContext,
    baseline_cost: Option<f64>,
) -> Option<PhysicalIndexScan> {
    let request = AccessRequest::new(source, Some(predicate), stats, context)?;
    let best = best_unordered(&request)?;
    let baseline = baseline_cost.unwrap_or(request.rows);
    (best.cost() < baseline).then(|| request.build(&best, SortDirection::Asc, false, None))
}

fn best_unordered<'a>(request: &AccessRequest<'a>) -> Option<Candidate<'a>> {
    request
        .candidates()
        .into_iter()
        .filter(|candidate| candidate.constrains_first_column())
        .filter(|candidate| !is_plain_lookup(candidate))
        .min_by(|a, b| {
            a.cost()
                .total_cmp(&b.cost())
                .then(b.exact.len().cmp(&a.exact.len()))
                .then(b.consumed.len().cmp(&a.consumed.len()))
        })
}

/// A single equality on a simple equality index, served by `IndexLookup`.
fn is_plain_lookup(candidate: &Candidate<'_>) -> bool {
    candidate.index.schema.kind == IndexKind::Equality
        && candidate.index.is_simple()
        && candidate.prefixes.len() == 1
        && candidate.range.is_none()
}

/// Choose an index scan producing the rows of `source` matching `predicate`
/// ordered by `order_by`, replacing the sort.
///
/// With a `limit_hint` (`LIMIT` + `OFFSET`), an ordered scan is chosen when
/// its estimated cost (entries read until the limit is reached) does not
/// exceed the cost of the best unordered access plus the sort. Without a
/// limit it is only chosen when the best unordered access already scans
/// that index, which makes the ordering free.
///
/// The scan must return every matching row: either its key ranges
/// constrain the first key column (rows without it cannot match), or the
/// index is not partial and has one entry per row (maintained counts).
pub(crate) fn plan_ordered_index_scan(
    source: &SourceRef,
    predicate: Option<&Expr>,
    order_by: &[OrderBy],
    limit_hint: Option<usize>,
    stats: Option<&dyn StatsProvider>,
    context: &QueryContext,
) -> Option<PhysicalIndexScan> {
    let request = AccessRequest::new(source, predicate, stats, context)?;
    let complete = |candidate: &Candidate<'_>| {
        candidate.constrains_first_column()
            || (!candidate.index.is_partial()
                && stats.is_some_and(|stats| {
                    let entries = stats.index_entry_count(source, candidate.index.lid);
                    let rows = stats.relation_stats(source).map(|stats| stats.row_count);
                    entries.is_some() && entries == rows
                }))
    };
    let unordered = best_unordered(&request);
    let Some(limit) = limit_hint else {
        let candidate = unordered?;
        let direction = candidate.order_direction(order_by)?;
        return complete(&candidate).then(|| request.build(&candidate, direction, true, None));
    };
    let unordered_matches = unordered.as_ref().map_or(request.rows, |candidate| {
        candidate.entries * candidate.selectivity
    });
    let unordered_cost =
        unordered.as_ref().map_or(request.rows, Candidate::cost) + unordered_matches * 0.1;
    request
        .candidates()
        .into_iter()
        .filter(complete)
        .filter_map(|candidate| {
            let direction = candidate.order_direction(order_by)?;
            let residual = request.residual_count(&candidate) as i32;
            let hit_rate = RESIDUAL_SELECTIVITY.powi(residual).max(0.001);
            let read = (limit as f64 / hit_rate).min(candidate.entries * candidate.selectivity);
            let cost = read + (candidate.entries + 1.0).log2();
            (cost <= unordered_cost).then_some((candidate, direction, cost))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(candidate, direction, _)| request.build(&candidate, direction, true, Some(limit)))
}

/// Serve the projection of an index scan from the index keys when every
/// field the plan evaluates above the scan is a key column or the id.
///
/// `plan` is the input of the projection; `Sort`, `TopN` and `Limit`
/// between the projection and the scan are allowed.
pub(crate) fn mark_index_only(
    plan: &mut PhysicalPlan,
    projection: &[QueryField],
    context: &QueryContext,
) {
    let mut needed = Vec::new();
    for field in projection {
        if field.wildcard.is_some() || contains_unsupported(&field.expr) {
            return;
        }
        needed.push(field.expr.as_ref());
    }
    let mut node = plan;
    loop {
        match node {
            PhysicalPlan::Limit { input, .. } => node = input,
            PhysicalPlan::Sort { input, order_by }
            | PhysicalPlan::TopN {
                input, order_by, ..
            } => {
                needed.extend(order_by.iter().map(|order| &order.expr));
                node = input;
            }
            PhysicalPlan::Source(PhysicalSource::IndexRange(scan)) => {
                if covers(scan, &needed, context) {
                    scan.index_only = true;
                }
                return;
            }
            _ => return,
        }
    }
}

fn covers(scan: &PhysicalIndexScan, needed: &[&Expr], context: &QueryContext) -> bool {
    let Some(collection) = resolve_collection_schema(context, &scan.source) else {
        return false;
    };
    let id_field = collection.canonical_field_name(PRIMARY_ID_FIELD);
    let columns = scan
        .columns
        .iter()
        .filter_map(|path| match path.segments() {
            [PathSegment::Field(field)] => Some(field.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    // Columns whose values do not decode losslessly (floats, timestamps)
    // are served from stored rows.
    if columns.iter().any(|column| {
        collection
            .field_type(column)
            .is_some_and(|ty| !is_lossless_key_type(&ty.kind))
    }) {
        return false;
    }
    let mut paths = BTreeSet::new();
    for expr in needed {
        collect_field_paths(expr, &mut paths);
    }
    if let Some(residual) = &scan.residual_predicate {
        collect_field_paths(residual, &mut paths);
    }
    paths.iter().all(|path| match path.segments() {
        [PathSegment::Field(field)] => field == id_field || columns.contains(&field.as_str()),
        _ => false,
    })
}

fn collect_field_paths(expr: &Expr, out: &mut BTreeSet<FieldPath>) {
    match expr {
        Expr::Operand(Operand::Field(path)) => {
            out.insert(path.clone());
        }
        Expr::Operand(Operand::Literal(_)) => {}
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => collect_field_paths(expr, out),
        Expr::Binary { left, right, .. } => {
            collect_field_paths(left, out);
            collect_field_paths(right, out);
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            collect_field_paths(cond, out);
            collect_field_paths(then_expr, out);
            collect_field_paths(else_expr, out);
        }
        Expr::Coalesce(items) => items.iter().for_each(|item| collect_field_paths(item, out)),
        Expr::Function { args, .. } => {
            for arg in args {
                if let crate::FunctionArg::Expr(expr) = arg {
                    collect_field_paths(expr, out);
                }
            }
        }
        Expr::Aggregate { arg, .. } => {
            if let crate::FunctionArg::Expr(expr) = arg.as_ref() {
                collect_field_paths(expr, out);
            }
        }
        Expr::InList { expr, list, .. } => {
            collect_field_paths(expr, out);
            list.iter().for_each(|item| collect_field_paths(item, out));
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            collect_field_paths(expr, out);
            collect_field_paths(low, out);
            collect_field_paths(high, out);
        }
        Expr::PatternMatch { expr, pattern, .. } | Expr::RegexMatch { expr, pattern, .. } => {
            collect_field_paths(expr, out);
            collect_field_paths(pattern, out);
        }
        // Rejected before coverage is checked.
        Expr::Subquery(_) | Expr::Exists { .. } | Expr::RelationExists { .. } => {}
    }
}

fn is_lossless_key_type(kind: &semantic_data::schema::core::type_kind::TypeKind) -> bool {
    use semantic_data::schema::core::type_kind::TypeKind;
    use semantic_data::schema::primitives::number_type::NumberType;

    match kind {
        TypeKind::Temporal(_) => false,
        TypeKind::Number(number) => !matches!(number, NumberType::Float(_)),
        TypeKind::Optional(optional) => is_lossless_key_type(&optional.inner.kind),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_prefixes_stop_at_wildcards() {
        assert_eq!(like_prefix("abc%"), Some(("abc".to_string(), true)));
        assert_eq!(like_prefix("ab_c%"), Some(("ab".to_string(), false)));
        assert_eq!(like_prefix("abc"), Some(("abc".to_string(), false)));
        assert_eq!(like_prefix("abc%d"), Some(("abc".to_string(), false)));
        assert_eq!(like_prefix("%abc"), None);
    }

    #[test]
    fn regex_prefixes_require_an_anchor_and_plain_characters() {
        assert_eq!(regex_prefix("^abc"), Some("abc".to_string()));
        assert_eq!(regex_prefix("^abc.*"), Some("abc".to_string()));
        assert_eq!(regex_prefix("^abc*"), Some("ab".to_string()));
        assert_eq!(regex_prefix("^a?"), None);
        assert_eq!(regex_prefix("abc"), None);
        assert_eq!(regex_prefix("^ab|cd"), None);
        assert_eq!(regex_prefix("^(?i)ab"), None);
    }

    #[test]
    fn bounds_intersect_to_the_tighter_side() {
        let lower = tighter_lower(
            Some(Bound::Included(Value::I64(1))),
            Bound::Excluded(Value::I64(1)),
        );
        assert_eq!(lower, Bound::Excluded(Value::I64(1)));
        let lower = tighter_lower(
            Some(Bound::Excluded(Value::I64(3))),
            Bound::Included(Value::I64(2)),
        );
        assert_eq!(lower, Bound::Excluded(Value::I64(3)));
        let upper = tighter_upper(
            Some(Bound::Included(Value::I64(9))),
            Bound::Excluded(Value::I64(5)),
        );
        assert_eq!(upper, Bound::Excluded(Value::I64(5)));
        let upper = tighter_upper(Some(Bound::Unbounded), Bound::Included(Value::I64(5)));
        assert_eq!(upper, Bound::Included(Value::I64(5)));
    }
}
