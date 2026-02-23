use semantic_data::value::{FieldPath, Value};

use crate::catalog::{LocalAttrId, LocalCollectionId, LocalFieldId};
use crate::query::{Predicate, SortDirection};

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
    pub field: FieldRef,
    pub source_path: FieldPath,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalOrderField {
    pub field: FieldRef,
    pub source_path: FieldPath,
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
    Predicate(Predicate),
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
        predicate: Predicate,
    },
    IndexLookup {
        source: SourceRef,
        field: FieldRef,
        value: Value,
        residual_predicate: Option<Predicate>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalJoinPlan {
    pub left: Box<PhysicalPlan>,
    pub right: Box<PhysicalPlan>,
    pub join_type: crate::query::JoinType,
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
        predicate: Predicate,
    },
    Sort {
        input: Box<PhysicalPlan>,
        order_by: Vec<PhysicalOrderField>,
    },
    Project {
        input: Box<PhysicalPlan>,
        projection: Vec<PhysicalProjectionField>,
    },
    Limit {
        input: Box<PhysicalPlan>,
        offset: usize,
        limit: Option<usize>,
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
        left: PhysicalJoinKey,
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
