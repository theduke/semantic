//! Binding values to reusable public query ASTs.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    DdlOperation, Expr, FunctionArg, InsertSource, JoinCondition, Operand, Query, QueryField,
    SelectQuery,
};
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryParameterError {
    pub reason: String,
    pub name: Option<String>,
}

impl std::fmt::Display for QueryParameterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "query parameter error: {} ({:?})",
            self.reason, self.name
        )
    }
}

impl std::error::Error for QueryParameterError {}

pub(crate) fn valid_parameter_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn error(reason: &str, name: &str) -> QueryParameterError {
    QueryParameterError {
        reason: reason.to_string(),
        name: Some(name.to_string()),
    }
}

impl Expr {
    /// Reference a named value supplied separately when executing this AST.
    pub fn parameter(name: impl Into<String>) -> Self {
        Self::Operand(Operand::Parameter(name.into()))
    }
}

impl Query {
    /// Clone this query and replace all parameter references with their supplied values.
    ///
    /// The original AST remains reusable. Projection references are resolved and
    /// query semantics are validated by the execution layer after binding.
    pub fn bind_parameters(
        &self,
        params: &BTreeMap<String, Value>,
    ) -> Result<Self, QueryParameterError> {
        self.clone().into_bound(params)
    }

    /// Bind an owned AST without cloning the tree.
    pub fn into_bound(self, params: &BTreeMap<String, Value>) -> Result<Self, QueryParameterError> {
        for name in params.keys() {
            if !valid_parameter_name(name) {
                return Err(error("invalid_name", name));
            }
        }
        let mut query = self;
        let mut used = BTreeSet::new();
        query.visit_expressions_mut(&mut |expr| {
            if let Expr::Operand(Operand::Parameter(name)) = expr {
                if !valid_parameter_name(name) {
                    return Err(error("invalid_name", name));
                }
                let value = params
                    .get(name)
                    .ok_or_else(|| error("missing", name))?
                    .clone();
                used.insert(name.clone());
                *expr = Expr::Operand(Operand::Literal(value));
            }
            Ok(())
        })?;
        if let Some(name) = params.keys().find(|name| !used.contains(*name)) {
            return Err(error("unused", name));
        }
        Ok(query)
    }

    /// Visit every expression recursively, including joins and nested SELECTs.
    /// Stored schema expressions in DDL are not executable query expressions.
    pub fn visit_expressions_mut<E>(
        &mut self,
        visit: &mut impl FnMut(&mut Expr) -> Result<(), E>,
    ) -> Result<(), E> {
        match self {
            Self::Select(query) => query.visit_expressions_mut(visit),
            Self::Insert(query) => {
                match &mut query.source {
                    InsertSource::Objects(_) => {}
                    InsertSource::Values(rows) => {
                        for row in rows {
                            for expr in row {
                                expr.visit_mut(visit)?;
                            }
                        }
                    }
                    InsertSource::Select(query) => query.visit_expressions_mut(visit)?,
                }
                visit_projection(&mut query.returning, visit)
            }
            Self::Update(query) => {
                visit_option(&mut query.predicate, visit)?;
                for assignment in &mut query.assignments {
                    assignment.value.visit_mut(visit)?;
                }
                visit_option(&mut query.limit, visit)?;
                visit_projection(&mut query.returning, visit)
            }
            Self::Delete(query) => {
                visit_option(&mut query.predicate, visit)?;
                visit_option(&mut query.limit, visit)?;
                visit_projection(&mut query.returning, visit)
            }
            Self::Ddl(query) => {
                for operation in &mut query.batch.operations {
                    if let DdlOperation::UpsertIndex {
                        predicate: Some(predicate),
                        ..
                    } = operation
                    {
                        predicate.visit_mut(visit)?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl SelectQuery {
    pub fn visit_expressions_mut<E>(
        &mut self,
        visit: &mut impl FnMut(&mut Expr) -> Result<(), E>,
    ) -> Result<(), E> {
        for join in &mut self.joins {
            if let JoinCondition::OnExpr(expr) = &mut join.condition {
                expr.visit_mut(visit)?;
            }
            visit_option(&mut join.predicate, visit)?;
        }
        visit_projection(&mut self.projection, visit)?;
        visit_option(&mut self.predicate, visit)?;
        for expr in &mut self.group_by {
            expr.visit_mut(visit)?;
        }
        visit_option(&mut self.having, visit)?;
        for order in &mut self.order_by {
            order.expr.visit_mut(visit)?;
        }
        visit_option(&mut self.limit, visit)?;
        self.offset.visit_mut(visit)
    }
}

fn visit_option<E>(
    expr: &mut Option<Expr>,
    visit: &mut impl FnMut(&mut Expr) -> Result<(), E>,
) -> Result<(), E> {
    if let Some(expr) = expr {
        expr.visit_mut(visit)?;
    }
    Ok(())
}

fn visit_projection<E>(
    projection: &mut [QueryField],
    visit: &mut impl FnMut(&mut Expr) -> Result<(), E>,
) -> Result<(), E> {
    for field in projection {
        field.expr.visit_mut(visit)?;
    }
    Ok(())
}

impl Expr {
    /// Visit child expressions first, then this expression.
    pub fn visit_mut<E>(
        &mut self,
        visit: &mut impl FnMut(&mut Expr) -> Result<(), E>,
    ) -> Result<(), E> {
        match self {
            Self::Operand(_) => {}
            Self::Unary { expr, .. } | Self::IsNull { expr, .. } | Self::ProjectionRef(expr) => {
                expr.visit_mut(visit)?
            }
            Self::Binary { left, right, .. } => {
                left.visit_mut(visit)?;
                right.visit_mut(visit)?;
            }
            Self::IfElse {
                cond,
                then_expr,
                else_expr,
            } => {
                cond.visit_mut(visit)?;
                then_expr.visit_mut(visit)?;
                else_expr.visit_mut(visit)?;
            }
            Self::Coalesce(items) => {
                for expr in items {
                    expr.visit_mut(visit)?;
                }
            }
            Self::Function { args, .. } => {
                for arg in args {
                    if let FunctionArg::Expr(expr) = arg {
                        expr.visit_mut(visit)?;
                    }
                }
            }
            Self::Aggregate { arg, .. } => {
                if let FunctionArg::Expr(expr) = arg.as_mut() {
                    expr.visit_mut(visit)?;
                }
            }
            Self::InList { expr, list, .. } => {
                expr.visit_mut(visit)?;
                for item in list {
                    item.visit_mut(visit)?;
                }
            }
            Self::Subquery(query) | Self::Exists { query, .. } => {
                query.visit_expressions_mut(visit)?
            }
            Self::Between {
                expr, low, high, ..
            } => {
                expr.visit_mut(visit)?;
                low.visit_mut(visit)?;
                high.visit_mut(visit)?;
            }
            Self::PatternMatch { expr, pattern, .. } | Self::RegexMatch { expr, pattern, .. } => {
                expr.visit_mut(visit)?;
                pattern.visit_mut(visit)?;
            }
            Self::TextMatch { exprs, query, .. } => {
                for expr in exprs {
                    expr.visit_mut(visit)?;
                }
                query.visit_mut(visit)?;
            }
            Self::RelationExists {
                relation,
                source,
                target,
                max_depth,
                ..
            } => {
                relation.visit_mut(visit)?;
                source.visit_mut(visit)?;
                target.visit_mut(visit)?;
                if let Some(expr) = max_depth {
                    expr.visit_mut(visit)?;
                }
            }
        }
        visit(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{
        Assignment, InsertQuery, JoinQuery, JoinSource, JoinType, OrderBy, SortDirection,
        UpdateQuery,
    };
    use crate::value::FieldPath;

    #[test]
    fn wildcard_extension_preserves_existing_facet_query_field_encoding() {
        let field = QueryField {
            expr: Box::new(Expr::Operand(Operand::Literal(Value::String(
                "value".into(),
            )))),
            alias: Some("name".into()),
            wildcard: None,
        };
        let encoded = facet_json::to_string(&field).unwrap();
        assert!(!encoded.contains("wildcard"));
        assert_eq!(facet_json::from_str::<QueryField>(&encoded).unwrap(), field);
        let wildcard = QueryField {
            wildcard: Some(FieldPath::new()),
            ..field
        };
        let encoded = facet_json::to_string(&wildcard).unwrap();
        assert!(encoded.contains("wildcard"));
        assert_eq!(
            facet_json::from_str::<QueryField>(&encoded).unwrap(),
            wildcard
        );
    }

    #[test]
    fn bindings_visit_nested_queries_joins_and_all_select_clauses_without_mutating_ast() {
        let parameter = || Expr::parameter("value");
        let nested = SelectQuery::new()
            .with_collection("nested")
            .with_predicate(parameter());
        let query: Query = SelectQuery::new()
            .with_collection("items")
            .with_joins(vec![JoinQuery {
                source: JoinSource {
                    collection: Some("other".into()),
                    class: None,
                },
                alias: None,
                join_type: JoinType::Inner,
                condition: JoinCondition::OnExpr(parameter()),
                predicate: Some(parameter()),
            }])
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Subquery(Box::new(nested))),
                alias: None,
                wildcard: None,
            }])
            .with_predicate(parameter())
            .with_group_by(vec![parameter()])
            .with_having(parameter())
            .with_order_by(vec![OrderBy {
                expr: parameter(),
                direction: SortDirection::Asc,
            }])
            .with_limit(parameter())
            .with_offset(parameter())
            .into();
        for value in [Value::I64(1), Value::String("another".into())] {
            let mut bound = query
                .bind_parameters(&BTreeMap::from([("value".into(), value.clone())]))
                .unwrap();
            let mut count = 0;
            let result: Result<(), std::convert::Infallible> =
                bound.visit_expressions_mut(&mut |expr| {
                    if let Expr::Operand(operand) = expr {
                        if operand != &Operand::Literal(Value::U64(0)) {
                            assert_eq!(operand, &Operand::Literal(value.clone()));
                            count += 1;
                        }
                    }
                    Ok(())
                });
            result.unwrap();
            assert_eq!(count, 9);
        }
        assert_eq!(
            query.bind_parameters(&BTreeMap::new()).unwrap_err().reason,
            "missing"
        );
    }

    #[test]
    fn bindings_cover_insert_values_insert_select_and_update_assignments() {
        for query in [
            Query::Insert(
                InsertQuery::new()
                    .with_source(InsertSource::Values(vec![vec![Expr::parameter("value")]])),
            ),
            Query::Insert(InsertQuery::new().with_source(InsertSource::Select(
                SelectQuery::new().with_predicate(Expr::parameter("value")),
            ))),
            Query::Update(UpdateQuery {
                assignments: vec![Assignment {
                    path: FieldPath::from_fields(["value"]),
                    value: Expr::parameter("value"),
                }],
                ..UpdateQuery::new()
            }),
        ] {
            query
                .bind_parameters(&BTreeMap::from([("value".into(), Value::Null)]))
                .unwrap();
            assert_eq!(
                query.bind_parameters(&BTreeMap::new()).unwrap_err().reason,
                "missing"
            );
        }
    }

    #[test]
    fn invalid_missing_and_unused_bindings_are_reported() {
        let ast: Query = SelectQuery::new()
            .with_predicate(Expr::parameter("Name"))
            .into();
        for (params, reason) in [
            (BTreeMap::new(), "missing"),
            (BTreeMap::from([("name".into(), Value::Null)]), "missing"),
            (
                BTreeMap::from([(":Name".into(), Value::Null)]),
                "invalid_name",
            ),
            (
                BTreeMap::from([("Name".into(), Value::Null), ("extra".into(), Value::Null)]),
                "unused",
            ),
        ] {
            assert_eq!(ast.bind_parameters(&params).unwrap_err().reason, reason);
        }
        let invalid: Query = SelectQuery::new()
            .with_predicate(Expr::parameter("bad-name"))
            .into();
        assert_eq!(
            invalid
                .bind_parameters(&BTreeMap::new())
                .unwrap_err()
                .reason,
            "invalid_name"
        );
    }
}
