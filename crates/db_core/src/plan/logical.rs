use semantic_data::query::JoinType;
use semantic_data::value::{FieldPath, Object};

use crate::catalog::LocalCollectionId;
use crate::plan::SourceRef;
use crate::query::{
    Expr, JoinCondition, Operand, OrderBy, QueryField, SelectQuery, evaluate_usize_expr,
};

#[derive(Debug, Clone, PartialEq)]
pub enum LogicalJoinCondition {
    True,
    Predicate(Expr),
    UsingFields { left: FieldPath, right: FieldPath },
}

impl From<JoinCondition> for LogicalJoinCondition {
    fn from(value: JoinCondition) -> Self {
        match value {
            JoinCondition::OnExpr(predicate) => Self::Predicate(predicate),
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
        pushed_predicate: Option<Expr>,
    },
    Values {
        values: Vec<Object>,
    },
    Filter {
        input: Box<LogicalPlan>,
        predicate: Expr,
    },
    Sort {
        input: Box<LogicalPlan>,
        order_by: Vec<OrderBy>,
    },
    Project {
        input: Box<LogicalPlan>,
        projection: Vec<QueryField>,
    },
    Aggregate {
        input: Box<LogicalPlan>,
        group_by: Vec<Expr>,
        projection: Vec<QueryField>,
        having: Option<Expr>,
    },
    Limit {
        input: Box<LogicalPlan>,
        offset: Expr,
        limit: Option<Expr>,
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
        left: Expr,
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
        let right_binding = join
            .alias
            .clone()
            .unwrap_or_else(|| join.source.default_binding());
        let right_source_name = join
            .source
            .collection
            .clone()
            .or_else(|| source.source_name.clone());
        let right_predicate = join_source_predicate(join);
        let right_source = LogicalPlan::Source {
            source: SourceRef {
                source_name: right_source_name,
                collection_id: None,
                binding: Some(right_binding.clone()),
                backend_tag: None,
            },
            pushed_predicate: right_predicate,
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
        let (applies, residual) = extract_subquery_apply(predicate.clone());
        if let Some(predicate) = residual {
            plan = LogicalPlan::Filter {
                input: Box::new(plan),
                predicate,
            };
        }
        for apply in applies {
            plan = match apply {
                SubqueryApply::Exists { subquery, negated } => {
                    let source = source_ref_for_collection(
                        subquery.collection.clone(),
                        subquery.source_alias.clone(),
                        None,
                    );
                    LogicalPlan::ApplyExists {
                        input: Box::new(plan),
                        subquery: Box::new(build_logical_plan(&subquery, source)),
                        negated,
                    }
                }
                SubqueryApply::InSubquery {
                    left,
                    subquery,
                    negated,
                } => {
                    let source = source_ref_for_collection(
                        subquery.collection.clone(),
                        subquery.source_alias.clone(),
                        None,
                    );
                    LogicalPlan::ApplyInSubquery {
                        input: Box::new(plan),
                        left,
                        subquery: Box::new(build_logical_plan(&subquery, source)),
                        negated,
                    }
                }
            };
        }
    }

    let aggregate_required =
        !query.group_by.is_empty() || query.having.is_some() || projection_has_aggregate(query);
    if aggregate_required {
        plan = LogicalPlan::Aggregate {
            input: Box::new(plan),
            group_by: query.group_by.clone(),
            projection: query.projection.clone(),
            having: query.having.clone(),
        };
        if query.distinct {
            plan = LogicalPlan::Distinct {
                input: Box::new(plan),
            };
        }
        if !query.order_by.is_empty() {
            plan = LogicalPlan::Sort {
                input: Box::new(plan),
                order_by: query.order_by.clone(),
            };
        }
    } else {
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
        if query.distinct {
            plan = LogicalPlan::Distinct {
                input: Box::new(plan),
            };
        }
    }

    if query.limit.is_some()
        || evaluate_usize_expr(&query.offset)
            .map(|offset| offset > 0)
            .unwrap_or(true)
    {
        plan = LogicalPlan::Limit {
            input: Box::new(plan),
            offset: query.offset.clone(),
            limit: query.limit.clone(),
        };
    }

    plan
}

fn join_source_predicate(join: &crate::JoinQuery) -> Option<Expr> {
    let mut predicates = Vec::new();
    if let Some(class_name) = &join.source.class {
        predicates.push(Expr::Binary {
            op: semantic_data::query::BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                "type",
            ])))),
            right: Box::new(Expr::Operand(Operand::Literal(
                semantic_data::value::Value::String(class_name.clone()),
            ))),
        });
    }
    if let Some(predicate) = &join.predicate {
        predicates.push(predicate.clone());
    }
    match predicates.len() {
        0 => None,
        1 => predicates.into_iter().next(),
        _ => {
            let mut iter = predicates.into_iter();
            let first = iter.next().expect("checked non-empty");
            Some(iter.fold(first, |left, right| Expr::Binary {
                op: semantic_data::query::BinaryOp::And,
                left: Box::new(left),
                right: Box::new(right),
            }))
        }
    }
}

fn projection_has_aggregate(query: &SelectQuery) -> bool {
    query
        .projection
        .iter()
        .any(|field| expr_has_aggregate(&field.expr))
}

fn expr_has_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Aggregate { .. } => true,
        Expr::Unary { expr, .. } => expr_has_aggregate(expr),
        Expr::Binary { left, right, .. } => expr_has_aggregate(left) || expr_has_aggregate(right),
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_has_aggregate(cond)
                || expr_has_aggregate(then_expr)
                || expr_has_aggregate(else_expr)
        }
        Expr::Coalesce(items) => items.iter().any(expr_has_aggregate),
        Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            crate::FunctionArg::Expr(expr) => expr_has_aggregate(expr),
            crate::FunctionArg::Wildcard => false,
        }),
        Expr::InList { expr, list, .. } => {
            expr_has_aggregate(expr) || list.iter().any(expr_has_aggregate)
        }
        Expr::Between {
            expr, low, high, ..
        } => expr_has_aggregate(expr) || expr_has_aggregate(low) || expr_has_aggregate(high),
        Expr::PatternMatch { expr, pattern, .. } | Expr::RegexMatch { expr, pattern, .. } => {
            expr_has_aggregate(expr) || expr_has_aggregate(pattern)
        }
        Expr::IsNull { expr, .. } => expr_has_aggregate(expr),
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            expr_has_aggregate(relation)
                || expr_has_aggregate(source)
                || expr_has_aggregate(target)
                || max_depth
                    .as_ref()
                    .is_some_and(|depth| expr_has_aggregate(depth))
        }
        Expr::InSubquery { .. } | Expr::Exists { .. } | Expr::Operand(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum SubqueryApply {
    Exists {
        subquery: SelectQuery,
        negated: bool,
    },
    InSubquery {
        left: Expr,
        subquery: SelectQuery,
        negated: bool,
    },
}

fn extract_subquery_apply(predicate: Expr) -> (Vec<SubqueryApply>, Option<Expr>) {
    let mut applies = Vec::new();
    let mut residual = Vec::new();
    for item in split_conjuncts(predicate) {
        if let Some(apply) = expr_to_apply(&item) {
            applies.push(apply);
        } else {
            residual.push(item);
        }
    }
    (applies, combine_conjuncts(residual))
}

fn expr_to_apply(predicate: &Expr) -> Option<SubqueryApply> {
    match predicate {
        Expr::Exists { query, negated } => Some(SubqueryApply::Exists {
            subquery: query.as_ref().clone(),
            negated: *negated,
        }),
        Expr::InSubquery {
            expr,
            query,
            negated,
        } => Some(SubqueryApply::InSubquery {
            left: expr.as_ref().clone(),
            subquery: query.as_ref().clone(),
            negated: *negated,
        }),
        _ => None,
    }
}

fn split_conjuncts(expr: Expr) -> Vec<Expr> {
    match expr {
        Expr::Binary {
            op: semantic_data::query::BinaryOp::And,
            left,
            right,
        } => {
            let mut out = split_conjuncts(*left);
            out.extend(split_conjuncts(*right));
            out
        }
        other => vec![other],
    }
}

fn combine_conjuncts(mut items: Vec<Expr>) -> Option<Expr> {
    if items.is_empty() {
        return None;
    }
    let first = items.remove(0);
    Some(items.into_iter().fold(first, |left, right| Expr::Binary {
        op: semantic_data::query::BinaryOp::And,
        left: Box::new(left),
        right: Box::new(right),
    }))
}
