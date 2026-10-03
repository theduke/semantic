//! Runtime binding before query planning or execution.

use std::collections::BTreeMap;

use super::{
    Assignment, Expr, FunctionArg, InsertSource, JoinCondition, Operand, Query, QueryField,
    SelectQuery,
};
use crate::{DbError, DdlOperation};
use semantic_data::value::Value;

impl Query {
    /// Bind reusable AST parameters and resolve explicit projection references.
    pub fn bind_parameters(&self, params: &BTreeMap<String, Value>) -> Result<Self, DbError> {
        self.clone().into_bound(params)
    }

    /// Bind an owned AST without cloning its tree. Queries requiring no binding
    /// pass straight through, preserving the normal programmatic query path.
    pub fn into_bound(self, params: &BTreeMap<String, Value>) -> Result<Self, DbError> {
        if params.is_empty() && !self.requires_binding() {
            return Ok(self);
        }
        let public: semantic_data::query::Query = self.into();
        let bound = public
            .into_bound(params)
            .map_err(|error| DbError::QueryParameter {
                reason: error.reason,
                name: error.name,
            })?;
        super::semantics::resolve_projection_references(bound).map_err(DbError::from)
    }

    fn requires_binding(&self) -> bool {
        match self {
            Self::Select(query) => select_requires_binding(query),
            Self::Insert(query) => {
                projection_requires_binding(&query.returning)
                    || match &query.source {
                        InsertSource::Objects(_) => false,
                        InsertSource::Values(rows) => {
                            rows.iter().flatten().any(expr_requires_binding)
                        }
                        InsertSource::Select(query) => select_requires_binding(query),
                    }
            }
            Self::Update(query) => {
                query.predicate.as_ref().is_some_and(expr_requires_binding)
                    || query
                        .assignments
                        .iter()
                        .any(|Assignment { value, .. }| expr_requires_binding(value))
                    || query.limit.as_ref().is_some_and(expr_requires_binding)
                    || projection_requires_binding(&query.returning)
            }
            Self::Delete(query) => {
                query.predicate.as_ref().is_some_and(expr_requires_binding)
                    || query.limit.as_ref().is_some_and(expr_requires_binding)
                    || projection_requires_binding(&query.returning)
            }
            Self::Ddl(query) => query.batch.operations.iter().any(|operation| {
                if let DdlOperation::UpsertIndex {
                    predicate: Some(predicate),
                    ..
                } = operation
                {
                    let internal: Expr = predicate.clone().into();
                    expr_requires_binding(&internal)
                } else {
                    false
                }
            }),
        }
    }
}

fn projection_requires_binding(projection: &[QueryField]) -> bool {
    projection
        .iter()
        .any(|field| expr_requires_binding(&field.expr))
}

fn select_requires_binding(query: &SelectQuery) -> bool {
    projection_requires_binding(&query.projection)
        || query.predicate.as_ref().is_some_and(expr_requires_binding)
        || query.joins.iter().any(|join| {
            matches!(&join.condition, JoinCondition::OnExpr(expr) if expr_requires_binding(expr))
                || join.predicate.as_ref().is_some_and(expr_requires_binding)
        })
        || query.group_by.iter().any(expr_requires_binding)
        || query.having.as_ref().is_some_and(expr_requires_binding)
        || query
            .order_by
            .iter()
            .any(|order| expr_requires_binding(&order.expr))
        || query.limit.as_ref().is_some_and(expr_requires_binding)
        || expr_requires_binding(&query.offset)
}

fn expr_requires_binding(expr: &Expr) -> bool {
    match expr {
        Expr::Operand(Operand::Parameter(_)) | Expr::ProjectionRef(_) => true,
        Expr::Operand(_) => false,
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => expr_requires_binding(expr),
        Expr::Binary { left, right, .. }
        | Expr::PatternMatch {
            expr: left,
            pattern: right,
            ..
        }
        | Expr::RegexMatch {
            expr: left,
            pattern: right,
            ..
        } => expr_requires_binding(left) || expr_requires_binding(right),
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_requires_binding(cond)
                || expr_requires_binding(then_expr)
                || expr_requires_binding(else_expr)
        }
        Expr::Coalesce(items) => items.iter().any(expr_requires_binding),
        Expr::Function { args, .. } => args.iter().any(arg_requires_binding),
        Expr::Aggregate { arg, .. } => arg_requires_binding(arg),
        Expr::InList { expr, list, .. } => {
            expr_requires_binding(expr) || list.iter().any(expr_requires_binding)
        }
        Expr::Subquery(query) | Expr::Exists { query, .. } => select_requires_binding(query),
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_requires_binding(expr) || expr_requires_binding(low) || expr_requires_binding(high)
        }
        Expr::TextMatch { exprs, query, .. } => {
            exprs.iter().any(expr_requires_binding) || expr_requires_binding(query)
        }
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            expr_requires_binding(relation)
                || expr_requires_binding(source)
                || expr_requires_binding(target)
                || max_depth.as_deref().is_some_and(expr_requires_binding)
        }
    }
}

fn arg_requires_binding(arg: &FunctionArg) -> bool {
    matches!(arg, FunctionArg::Expr(expr) if expr_requires_binding(expr))
}
