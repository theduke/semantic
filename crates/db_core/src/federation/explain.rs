use semantic_data::query::Expr;
use semantic_data::value::{IntoValue, SemanticType, Value};
use semantic_data::vdb::FilterSupport;

use crate::{LogicalPlan, PhysicalPlan};

#[derive(Debug, Clone)]
pub struct FederatedExplain {
    pub logical: LogicalPlan,
    pub physical: PhysicalPlan,
    pub leaves: Vec<LeafExplain>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LeafExplain {
    pub collection: String,
    pub source_tag: String,
    pub filters: Vec<(Expr, FilterSupport)>,
    pub ordered_prefix: u64,
    pub limit_applied: bool,
    pub offset_applied: bool,
    /// The source's estimate, rather than a fabricated host cost estimate.
    pub estimated_rows: Option<u64>,
    pub residual: Option<Expr>,
}

#[derive(facet::Facet, SemanticType, IntoValue)]
struct FilterValue {
    expression: Expr,
    support: FilterSupport,
}

#[derive(facet::Facet, SemanticType, IntoValue)]
struct LeafValue {
    collection: String,
    source_tag: String,
    filters: Vec<FilterValue>,
    ordered_prefix: u64,
    limit_applied: bool,
    offset_applied: bool,
    estimated_rows: Option<u64>,
    residual: Option<Expr>,
}

impl From<LeafExplain> for LeafValue {
    fn from(leaf: LeafExplain) -> Self {
        Self {
            collection: leaf.collection,
            source_tag: leaf.source_tag,
            filters: leaf
                .filters
                .into_iter()
                .map(|(expression, support)| FilterValue {
                    expression,
                    support,
                })
                .collect(),
            ordered_prefix: leaf.ordered_prefix,
            limit_applied: leaf.limit_applied,
            offset_applied: leaf.offset_applied,
            estimated_rows: leaf.estimated_rows,
            residual: leaf.residual,
        }
    }
}

impl SemanticType for LeafExplain {
    fn semantic_type() -> semantic_data::schema::Type {
        LeafValue::semantic_type()
    }
}
impl IntoValue for LeafExplain {
    fn into_value(self) -> Value {
        LeafValue::from(self).into_value()
    }
}

#[derive(facet::Facet, SemanticType, IntoValue)]
struct ExplainValue {
    logical: String,
    physical: String,
    leaves: Vec<LeafValue>,
}

impl SemanticType for FederatedExplain {
    fn semantic_type() -> semantic_data::schema::Type {
        ExplainValue::semantic_type()
    }
}
impl IntoValue for FederatedExplain {
    fn into_value(self) -> Value {
        ExplainValue {
            logical: format!("{:?}", self.logical),
            physical: self.physical.tree(),
            leaves: self.leaves.into_iter().map(Into::into).collect(),
        }
        .into_value()
    }
}

impl std::fmt::Display for FederatedExplain {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.physical.tree())
    }
}
