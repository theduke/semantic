use semantic_data::value::{FieldPath, Object};

use crate::catalog::LocalCollectionId;
use crate::plan::SourceRef;
use crate::query::{JoinCondition, JoinType, OrderBy, Predicate, QueryField, SelectQuery};

#[derive(Debug, Clone, PartialEq)]
pub enum LogicalJoinCondition {
    True,
    Predicate(Predicate),
    UsingFields { left: FieldPath, right: FieldPath },
}

impl From<JoinCondition> for LogicalJoinCondition {
    fn from(value: JoinCondition) -> Self {
        match value {
            JoinCondition::OnPredicate(predicate) => Self::Predicate(predicate),
            JoinCondition::UsingFields { left, right } => Self::UsingFields { left, right },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogicalJoinPlan {
    pub left: Box<LogicalPlan>,
    pub right: Box<LogicalPlan>,
    pub join_type: JoinType,
    pub condition: LogicalJoinCondition,
    pub left_binding: String,
    pub right_binding: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogicalPlan {
    Source {
        source: SourceRef,
        pushed_predicate: Option<Predicate>,
    },
    Values {
        values: Vec<Object>,
    },
    Filter {
        input: Box<LogicalPlan>,
        predicate: Predicate,
    },
    Sort {
        input: Box<LogicalPlan>,
        order_by: Vec<OrderBy>,
    },
    Project {
        input: Box<LogicalPlan>,
        projection: Vec<QueryField>,
    },
    Limit {
        input: Box<LogicalPlan>,
        offset: usize,
        limit: Option<usize>,
    },
    Distinct {
        input: Box<LogicalPlan>,
    },
    Union {
        inputs: Vec<LogicalPlan>,
        all: bool,
    },
    Join(LogicalJoinPlan),
    ApplyExists {
        input: Box<LogicalPlan>,
        subquery: Box<LogicalPlan>,
        negated: bool,
    },
    ApplyInSubquery {
        input: Box<LogicalPlan>,
        left: FieldPath,
        subquery: Box<LogicalPlan>,
        negated: bool,
    },
    Exchange {
        input: Box<LogicalPlan>,
        partition_count: usize,
    },
    RepartitionHash {
        input: Box<LogicalPlan>,
        partition_count: usize,
        partition_keys: Vec<FieldPath>,
    },
}

pub fn source_ref_for_collection(
    source_name: Option<String>,
    source_alias: Option<String>,
    collection_id: Option<LocalCollectionId>,
) -> SourceRef {
    SourceRef {
        source_name,
        collection_id,
        binding: source_alias,
        backend_tag: None,
    }
}

pub fn build_logical_plan(query: &SelectQuery, source: SourceRef) -> LogicalPlan {
    let mut plan = LogicalPlan::Source {
        source: source.clone(),
        pushed_predicate: None,
    };

    let base_binding = source
        .binding
        .clone()
        .or_else(|| source.source_name.clone())
        .unwrap_or_else(|| "left".to_string());

    for join in &query.joins {
        let right_binding = join.alias.clone().unwrap_or_else(|| join.source.clone());
        let right_source = LogicalPlan::Source {
            source: SourceRef {
                source_name: Some(join.source.clone()),
                collection_id: None,
                binding: Some(right_binding.clone()),
                backend_tag: None,
            },
            pushed_predicate: None,
        };
        plan = LogicalPlan::Join(LogicalJoinPlan {
            left: Box::new(plan),
            right: Box::new(right_source),
            join_type: join.join_type,
            condition: join.condition.clone().into(),
            left_binding: base_binding.clone(),
            right_binding,
        });
    }

    if let Some(predicate) = &query.predicate {
        plan = LogicalPlan::Filter {
            input: Box::new(plan),
            predicate: predicate.clone(),
        };
    }

    if !query.order_by.is_empty() {
        plan = LogicalPlan::Sort {
            input: Box::new(plan),
            order_by: query.order_by.clone(),
        };
    }

    if !query.projection.is_empty() {
        plan = LogicalPlan::Project {
            input: Box::new(plan),
            projection: query.projection.clone(),
        };
    }

    if query.offset != 0 || query.limit.is_some() {
        plan = LogicalPlan::Limit {
            input: Box::new(plan),
            offset: query.offset,
            limit: query.limit,
        };
    }

    plan
}
