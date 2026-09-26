//! Full-text index access for text match predicates.
//!
//! A top-level [`Expr::TextMatch`] conjunct with a literal query is served
//! by a full-text index when the index covers exactly the searched fields
//! and tokenizes with an equivalent analyzer: every query token becomes one
//! equality probe of the index, whose id sets are intersected (`All`) or
//! united (`Any`). The index holds exactly the tokens of each row, so the
//! served conjunct (and the predicate of a partial index it is implied by)
//! is dropped from the residual predicate.

use std::collections::BTreeSet;

use semantic_data::query::TextMatchMode;
use semantic_data::value::{FieldPath, PathSegment, Value};

use super::index_access::{AccessRequest, MAX_PROBES};
use super::optimizer::combine_conjuncts;
use crate::QueryContext;
use crate::catalog::IndexSchema;
use crate::plan::{PhysicalTextSearch, SourceRef, StatsProvider};
use crate::query::{Expr, Operand};

/// Fraction of the rows assumed to contain one query token.
const TOKEN_SELECTIVITY: f64 = 0.05;

/// A text match conjunct a full-text index can serve.
struct TextCandidate<'i> {
    index: &'i IndexSchema,
    tokens: BTreeSet<String>,
    mode: TextMatchMode,
    /// Conjuncts every produced row satisfies.
    exact: BTreeSet<usize>,
    cost: f64,
}

/// Canonical top-level fields searched by a text match, or `None` when an
/// expression is not a plain field.
fn searched_fields(exprs: &[Expr]) -> Option<BTreeSet<&str>> {
    exprs
        .iter()
        .map(|expr| match expr {
            Expr::Operand(Operand::Field(path)) => match path.segments() {
                [PathSegment::Field(field)] => Some(field.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

impl AccessRequest<'_> {
    fn text_candidate<'i>(
        &self,
        index: &'i IndexSchema,
        position: usize,
    ) -> Option<TextCandidate<'i>> {
        let Expr::TextMatch {
            exprs,
            query,
            mode,
            analyzer,
        } = &self.conjuncts[position]
        else {
            return None;
        };
        let Expr::Operand(Operand::Literal(Value::String(query))) = query.as_ref() else {
            return None;
        };
        if !analyzer.equivalent(&index.schema.analyzer)
            || searched_fields(exprs)? != index.columns().collect::<BTreeSet<_>>()
        {
            return None;
        }
        let tokens = analyzer.query_tokens(query);
        if tokens.is_empty() || tokens.len() > MAX_PROBES {
            return None;
        }
        let mut exact = self.implied_conjuncts(index)?;
        exact.insert(position);
        let matched = match mode {
            TextMatchMode::All => TOKEN_SELECTIVITY.powi(tokens.len() as i32),
            TextMatchMode::Any => (TOKEN_SELECTIVITY * tokens.len() as f64).min(1.0),
        };
        let probes = tokens.len() as f64;
        let cost = self.rows * matched + probes * (self.entries(index) + 1.0).log2();
        Some(TextCandidate {
            index,
            tokens,
            mode: *mode,
            exact,
            cost,
        })
    }
}

/// Plan full-text index probes answering `predicate` over `source`.
///
/// Whenever a full-text index serves a text match conjunct with at least one
/// token it is used, unless another index access with cost `baseline_cost`
/// (on the scale of [`super::index_access`] estimates) is cheaper.
pub(crate) fn plan_text_search(
    source: &SourceRef,
    predicate: &Expr,
    stats: Option<&dyn StatsProvider>,
    context: &QueryContext,
    baseline_cost: Option<f64>,
) -> Option<PhysicalTextSearch> {
    if !contains_text_match(predicate) {
        return None;
    }
    let request = AccessRequest::new(source, Some(predicate), stats, context)?;
    let catalog: &crate::catalog::Catalog = request.context.catalog();
    let best = catalog
        .indexes_for_collection(request.collection.lid)
        .filter(|index| index.schema.kind.is_full_text())
        .flat_map(|index| {
            (0..request.conjuncts.len())
                .filter_map(|position| request.text_candidate(index, position))
                .collect::<Vec<_>>()
        })
        .min_by(|a, b| a.cost.total_cmp(&b.cost))?;
    if baseline_cost.is_some_and(|baseline| baseline < best.cost) {
        return None;
    }
    Some(PhysicalTextSearch {
        source: request.source.clone(),
        index: best.index.lid,
        index_name: best.index.schema.name.clone(),
        columns: best
            .index
            .columns()
            .map(|column| FieldPath::from_fields([column]))
            .collect(),
        tokens: best.tokens.into_iter().collect(),
        limit_hint: None,
        mode: best.mode,
        predicate: combine_conjuncts(request.conjuncts.iter().cloned()),
        residual_predicate: combine_conjuncts(
            request
                .conjuncts
                .iter()
                .enumerate()
                .filter(|(position, _)| !best.exact.contains(position))
                .map(|(_, conjunct)| conjunct.clone()),
        ),
    })
}

/// Cheap pre-check: whether `predicate` has a top-level text match.
fn contains_text_match(predicate: &Expr) -> bool {
    match predicate {
        Expr::TextMatch { .. } => true,
        Expr::Binary {
            op: semantic_data::query::BinaryOp::And,
            left,
            right,
        } => contains_text_match(left) || contains_text_match(right),
        _ => false,
    }
}
