//! Compact rendering of physical plans for explain output and logs.

use std::ops::Bound;

use semantic_data::query::SortDirection;
use semantic_data::value::{FieldPath, PathSegment, Value};

use crate::plan::{
    FieldRef, PhysicalJoinAlgorithm, PhysicalJoinCondition, PhysicalOrderField, PhysicalPlan,
    PhysicalSource, SourceRef,
};
use crate::query::{Expr, Operand};
use crate::{AccessPathKind, IndexColumnRange, IndexScanRange};

impl PhysicalPlan {
    /// The inputs of this operator, in execution order: input `i` is the
    /// operator at path `<path>.i` of [`OperatorStats`](crate::OperatorStats).
    pub fn inputs(&self) -> Vec<&PhysicalPlan> {
        match self {
            Self::Source(_) | Self::Values { .. } => Vec::new(),
            Self::Filter { input, .. }
            | Self::Sort { input, .. }
            | Self::TopN { input, .. }
            | Self::Project { input, .. }
            | Self::Aggregate { input, .. }
            | Self::Limit { input, .. }
            | Self::Distinct { input }
            | Self::Exchange { input, .. }
            | Self::RepartitionHash { input, .. }
            | Self::Materialize { input } => vec![input.as_ref()],
            Self::Union { inputs, .. } => inputs.iter().collect(),
            Self::Join(join) => vec![join.left.as_ref(), join.right.as_ref()],
            Self::ApplyExists {
                input, subquery, ..
            }
            | Self::ApplyInSubquery {
                input, subquery, ..
            } => vec![input.as_ref(), subquery.as_ref()],
        }
    }

    /// Compact one-line label of this operator, without its inputs, e.g.
    /// `TopN(n asc, limit=10)` or `IndexRange(items.by_n [5, 20))`.
    pub fn node_label(&self) -> String {
        match self {
            Self::Source(source) => source.label(),
            Self::Values { values } => format!("Values(rows={})", values.len()),
            Self::Filter { .. } => "Filter".to_string(),
            Self::Sort { order_by, .. } => format!("Sort({})", order_label(order_by)),
            Self::TopN {
                order_by,
                offset,
                limit,
                ..
            } => {
                let mut label = format!(
                    "TopN({}, limit={}",
                    order_label(order_by),
                    expr_label(limit)
                );
                push_offset(&mut label, offset);
                label.push(')');
                label
            }
            Self::Project { .. } => "Project".to_string(),
            Self::Aggregate { group_by, .. } if group_by.is_empty() => "Aggregate".to_string(),
            Self::Aggregate { group_by, .. } => format!(
                "Aggregate(group_by={})",
                group_by
                    .iter()
                    .map(expr_label)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Limit { offset, limit, .. } => {
                let mut label = format!(
                    "Limit(limit={}",
                    limit.as_ref().map_or_else(|| "all".to_string(), expr_label)
                );
                push_offset(&mut label, offset);
                label.push(')');
                label
            }
            Self::Distinct { .. } => "Distinct".to_string(),
            Self::Union { all: true, .. } => "Union(all)".to_string(),
            Self::Union { all: false, .. } => "Union(distinct)".to_string(),
            Self::Join(join) => {
                let algorithm = match join.algorithm {
                    PhysicalJoinAlgorithm::Hash => "HashJoin",
                    PhysicalJoinAlgorithm::IndexNestedLoop => "IndexNestedLoopJoin",
                    PhysicalJoinAlgorithm::NestedLoop => "NestedLoopJoin",
                    PhysicalJoinAlgorithm::Merge => "MergeJoin",
                };
                let join_type = format!("{:?}", join.join_type).to_lowercase();
                match &join.condition {
                    PhysicalJoinCondition::Eq { left, right, .. } => format!(
                        "{algorithm}({join_type}, {}.{} = {}.{})",
                        join.left_binding,
                        path_label(&left.source_path),
                        join.right_binding,
                        path_label(&right.source_path)
                    ),
                    PhysicalJoinCondition::True | PhysicalJoinCondition::Predicate(_) => {
                        format!("{algorithm}({join_type})")
                    }
                }
            }
            Self::ApplyExists { negated: false, .. } => "ApplyExists".to_string(),
            Self::ApplyExists { negated: true, .. } => "ApplyNotExists".to_string(),
            Self::ApplyInSubquery { negated, left, .. } => format!(
                "Apply{}In({})",
                if *negated { "Not" } else { "" },
                expr_label(left)
            ),
            Self::Exchange {
                partition_count, ..
            } => format!("Exchange(partitions={partition_count})"),
            Self::RepartitionHash {
                partition_count, ..
            } => format!("RepartitionHash(partitions={partition_count})"),
            Self::Materialize { .. } => "Materialize".to_string(),
        }
    }

    /// One-line summary of the plan: operators from the root down joined by
    /// `->`, several inputs in brackets separated by `|`, e.g.
    /// `Project -> TopN(n asc, limit=10) -> IndexRange(items.by_n [5, 20))`.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        self.write_summary(&mut out);
        out
    }

    fn write_summary(&self, out: &mut String) {
        out.push_str(&self.node_label());
        match self.inputs().as_slice() {
            [] => {}
            [input] => {
                out.push_str(" -> ");
                input.write_summary(out);
            }
            inputs => {
                out.push_str(" [");
                for (index, input) in inputs.iter().enumerate() {
                    if index > 0 {
                        out.push_str(" | ");
                    }
                    input.write_summary(out);
                }
                out.push(']');
            }
        }
    }

    /// Visit the operators of the plan in pre-order with their paths (input
    /// indexes from the root; see [`Self::inputs`]).
    pub fn walk(&self, visit: &mut impl FnMut(&[u32], &PhysicalPlan)) {
        fn go(
            plan: &PhysicalPlan,
            path: &mut Vec<u32>,
            visit: &mut impl FnMut(&[u32], &PhysicalPlan),
        ) {
            visit(path, plan);
            for (index, input) in plan.inputs().into_iter().enumerate() {
                path.push(index as u32);
                go(input, path, visit);
                path.pop();
            }
        }
        go(self, &mut Vec::new(), visit);
    }

    /// Multi-line rendering of the plan: one operator label per line,
    /// indented by depth.
    pub fn tree(&self) -> String {
        let mut out = String::new();
        self.walk(&mut |path, plan| {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&"  ".repeat(path.len()));
            out.push_str(&plan.node_label());
        });
        out
    }
}

impl std::fmt::Display for PhysicalPlan {
    /// The one-line [`PhysicalPlan::summary`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.summary())
    }
}

impl PhysicalSource {
    /// The source read by this access.
    pub fn source(&self) -> &SourceRef {
        match self {
            Self::Scan { source }
            | Self::FilteredScan { source, .. }
            | Self::IndexLookup { source, .. } => source,
            Self::IndexRange(scan) => &scan.source,
            Self::TextSearch(search) => &search.source,
        }
    }

    /// The access path kind of this source.
    pub fn access_kind(&self) -> AccessPathKind {
        match self {
            Self::Scan { .. } => AccessPathKind::FullScan,
            Self::FilteredScan { .. } => AccessPathKind::FilteredScan,
            Self::IndexLookup { .. } => AccessPathKind::IndexLookup,
            Self::IndexRange(_) => AccessPathKind::IndexRange,
            Self::TextSearch(_) => AccessPathKind::TextSearch,
        }
    }

    /// Name of the index read, when the plan names it.
    pub fn index_name(&self) -> Option<&str> {
        match self {
            Self::IndexRange(scan) => Some(&scan.index_name),
            Self::TextSearch(search) => Some(&search.index_name),
            Self::Scan { .. } | Self::FilteredScan { .. } | Self::IndexLookup { .. } => None,
        }
    }

    fn label(&self) -> String {
        let source = self.source().label();
        match self {
            Self::Scan { .. } => format!("Scan({source})"),
            Self::FilteredScan { .. } => format!("FilteredScan({source})"),
            Self::IndexLookup { field, value, .. } => format!(
                "IndexLookup({source}.{} = {})",
                field_ref_label(field),
                value_label(value)
            ),
            Self::IndexRange(scan) => {
                let ranges = scan
                    .ranges
                    .iter()
                    .map(index_range_label)
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut label = format!("IndexRange({source}.{} {ranges}", scan.index_name);
                if scan.direction == SortDirection::Desc {
                    label.push_str(" desc");
                }
                if scan.index_only {
                    label.push_str(" index_only");
                }
                label.push(')');
                label
            }
            Self::TextSearch(search) => format!(
                "TextSearch({source}.{} {}: {})",
                search.index_name,
                format!("{:?}", search.mode).to_lowercase(),
                search.tokens.join(" ")
            ),
        }
    }
}

impl SourceRef {
    /// Display name of the source: its collection (and binding when it
    /// differs).
    pub fn label(&self) -> String {
        match (&self.source_name, &self.binding) {
            (Some(name), Some(binding)) if name != binding => format!("{name} as {binding}"),
            (Some(name), _) => name.clone(),
            (None, Some(binding)) => binding.clone(),
            (None, None) => match self.collection_id {
                Some(id) => format!("#{}", id.0),
                None => "?".to_string(),
            },
        }
    }
}

fn push_offset(label: &mut String, offset: &Expr) {
    if !matches!(offset, Expr::Operand(Operand::Literal(value)) if is_zero(value)) {
        label.push_str(", offset=");
        label.push_str(&expr_label(offset));
    }
}

fn is_zero(value: &Value) -> bool {
    matches!(
        value,
        Value::I8(0)
            | Value::I16(0)
            | Value::I32(0)
            | Value::I64(0)
            | Value::I128(0)
            | Value::U8(0)
            | Value::U16(0)
            | Value::U32(0)
            | Value::U64(0)
            | Value::U128(0)
    )
}

fn order_label(order_by: &[PhysicalOrderField]) -> String {
    order_by
        .iter()
        .map(|field| {
            let direction = match field.direction {
                SortDirection::Asc => "asc",
                SortDirection::Desc => "desc",
            };
            format!("{} {direction}", expr_label(&field.expr))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn expr_label(expr: &Expr) -> String {
    match expr {
        Expr::Operand(Operand::Field(path)) => path_label(path),
        Expr::Operand(Operand::Literal(value)) => value_label(value),
        _ => "expr".to_string(),
    }
}

fn field_ref_label(field: &FieldRef) -> String {
    match field {
        FieldRef::AttrId(id) => format!("attr#{}", id.0),
        FieldRef::FieldId(id) => format!("field#{}", id.0),
        FieldRef::CanonicalName(name) => name.clone(),
        FieldRef::Path(path) => path_label(path),
    }
}

fn path_label(path: &FieldPath) -> String {
    let mut out = String::new();
    for segment in path.segments() {
        match segment {
            PathSegment::Field(name) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(name);
            }
            PathSegment::Index(index) => out.push_str(&format!("[{index}]")),
        }
    }
    out
}

fn value_label(value: &Value) -> String {
    match value {
        Value::Void => "void".to_string(),
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::I8(value) => value.to_string(),
        Value::I16(value) => value.to_string(),
        Value::I32(value) => value.to_string(),
        Value::I64(value) => value.to_string(),
        Value::I128(value) => value.to_string(),
        Value::U8(value) => value.to_string(),
        Value::U16(value) => value.to_string(),
        Value::U32(value) => value.to_string(),
        Value::U64(value) => value.to_string(),
        Value::U128(value) => value.to_string(),
        Value::String(value) => format!("'{value}'"),
        Value::List(values) => format!(
            "({})",
            values
                .iter()
                .map(value_label)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => format!("{other:?}"),
    }
}

fn index_range_label(range: &IndexScanRange) -> String {
    let column = match &range.column {
        IndexColumnRange::StringPrefix(prefix) => format!("prefix '{prefix}'"),
        IndexColumnRange::Bounds {
            lower: Bound::Included(lower),
            upper: Bound::Included(upper),
        } if lower == upper => format!("= {}", value_label(lower)),
        IndexColumnRange::Bounds { lower, upper } => {
            let lower = match lower {
                Bound::Included(value) => format!("[{}", value_label(value)),
                Bound::Excluded(value) => format!("({}", value_label(value)),
                Bound::Unbounded => "(-inf".to_string(),
            };
            let upper = match upper {
                Bound::Included(value) => format!("{}]", value_label(value)),
                Bound::Excluded(value) => format!("{})", value_label(value)),
                Bound::Unbounded => "+inf)".to_string(),
            };
            format!("{lower}, {upper}")
        }
    };
    if range.prefix.is_empty() {
        column
    } else {
        let prefix = range
            .prefix
            .iter()
            .map(value_label)
            .collect::<Vec<_>>()
            .join(", ");
        format!("({prefix}) {column}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> SourceRef {
        SourceRef {
            source_name: Some("items".into()),
            ..SourceRef::unnamed()
        }
    }

    #[test]
    fn summary_renders_operator_chain_and_index_ranges() {
        let scan = PhysicalPlan::Source(PhysicalSource::IndexRange(crate::PhysicalIndexScan {
            source: items(),
            index: crate::catalog::LocalIndexId(1),
            index_name: "by_n".into(),
            columns: vec![FieldPath::from_fields(["n"])],
            ranges: vec![IndexScanRange::single(IndexColumnRange::Bounds {
                lower: Bound::Included(Value::I64(5)),
                upper: Bound::Excluded(Value::I64(20)),
            })],
            direction: SortDirection::Asc,
            ordered: false,
            limit_hint: None,
            index_only: false,
            predicate: None,
            residual_predicate: None,
        }));
        let plan = PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::TopN {
                input: Box::new(scan),
                order_by: vec![PhysicalOrderField {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["n"]))),
                    direction: SortDirection::Desc,
                }],
                offset: Expr::from(0usize),
                limit: Expr::from(10usize),
            }),
            projection: Vec::new(),
        };
        assert_eq!(
            plan.summary(),
            "Project -> TopN(n desc, limit=10) -> IndexRange(items.by_n [5, 20))"
        );
        assert_eq!(plan.to_string(), plan.summary());
        assert_eq!(
            plan.tree(),
            "Project\n  TopN(n desc, limit=10)\n    IndexRange(items.by_n [5, 20))"
        );
    }

    #[test]
    fn summary_brackets_multiple_inputs() {
        let scan = || PhysicalPlan::Source(PhysicalSource::Scan { source: items() });
        let plan = PhysicalPlan::Limit {
            input: Box::new(PhysicalPlan::Union {
                inputs: vec![scan(), scan()],
                all: true,
            }),
            offset: Expr::from(5usize),
            limit: Some(Expr::from(3usize)),
        };
        assert_eq!(
            plan.summary(),
            "Limit(limit=3, offset=5) -> Union(all) [Scan(items) | Scan(items)]"
        );
        let mut paths = Vec::new();
        plan.walk(&mut |path, node| paths.push((path.to_vec(), node.node_label())));
        assert_eq!(
            paths,
            vec![
                (vec![], "Limit(limit=3, offset=5)".to_string()),
                (vec![0], "Union(all)".to_string()),
                (vec![0, 0], "Scan(items)".to_string()),
                (vec![0, 1], "Scan(items)".to_string()),
            ]
        );
    }
}
