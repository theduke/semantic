use semantic_data::{
    query::{JoinType, SortDirection},
    value::{FieldPath, Value},
};

use crate::catalog::{LocalAttrId, LocalCollectionId, LocalFieldId};
use crate::query::Expr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceRef {
    pub source_name: Option<String>,
    pub collection_id: Option<LocalCollectionId>,
    pub binding: Option<String>,
    pub backend_tag: Option<String>,
}

impl SourceRef {
    pub fn unnamed() -> Self {
        Self {
            source_name: None,
            collection_id: None,
            binding: None,
            backend_tag: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldRef {
    AttrId(LocalAttrId),
    FieldId(LocalFieldId),
    CanonicalName(String),
    Path(FieldPath),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalProjectionField {
    pub expr: crate::query::Expr,
    pub field: Option<FieldRef>,
    pub source_path: Option<FieldPath>,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalOrderField {
    pub expr: crate::query::Expr,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalJoinKey {
    pub field: FieldRef,
    pub source_path: FieldPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalJoinAlgorithm {
    Hash,
    NestedLoop,
    Merge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhysicalJoinCondition {
    True,
    Predicate(Expr),
    Eq {
        left: PhysicalJoinKey,
        right: PhysicalJoinKey,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhysicalSource {
    Scan {
        source: SourceRef,
    },
    FilteredScan {
        source: SourceRef,
        predicate: Expr,
    },
    IndexLookup {
        source: SourceRef,
        field: FieldRef,
        value: Value,
        residual_predicate: Option<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalJoinPlan {
    pub left: Box<PhysicalPlan>,
    pub right: Box<PhysicalPlan>,
    pub join_type: JoinType,
    pub algorithm: PhysicalJoinAlgorithm,
    pub condition: PhysicalJoinCondition,
    pub left_binding: String,
    pub right_binding: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhysicalPlan {
    Source(PhysicalSource),
    Values {
        values: Vec<semantic_data::value::Object>,
    },
    Filter {
        input: Box<PhysicalPlan>,
        predicate: Expr,
    },
    Sort {
        input: Box<PhysicalPlan>,
        order_by: Vec<PhysicalOrderField>,
    },
    Project {
        input: Box<PhysicalPlan>,
        projection: Vec<PhysicalProjectionField>,
    },
    Aggregate {
        input: Box<PhysicalPlan>,
        group_by: Vec<crate::query::Expr>,
        projection: Vec<PhysicalProjectionField>,
        having: Option<Expr>,
    },
    Limit {
        input: Box<PhysicalPlan>,
        offset: crate::query::Expr,
        limit: Option<crate::query::Expr>,
    },
    Distinct {
        input: Box<PhysicalPlan>,
    },
    Union {
        inputs: Vec<PhysicalPlan>,
        all: bool,
    },
    Join(PhysicalJoinPlan),
    ApplyExists {
        input: Box<PhysicalPlan>,
        subquery: Box<PhysicalPlan>,
        negated: bool,
    },
    ApplyInSubquery {
        input: Box<PhysicalPlan>,
        left: crate::query::Expr,
        subquery: Box<PhysicalPlan>,
        negated: bool,
    },
    Exchange {
        input: Box<PhysicalPlan>,
        partition_count: usize,
    },
    RepartitionHash {
        input: Box<PhysicalPlan>,
        partition_count: usize,
        partition_keys: Vec<PhysicalJoinKey>,
    },
    Materialize {
        input: Box<PhysicalPlan>,
    },
}
