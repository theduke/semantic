//! Small constructors shared by the programmatic UI queries.

use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    query::{
        BinaryOp, Expr, FieldFormat, Operand, OrderBy, PatternMatchKind, Query, QueryField,
        SelectQuery, SortDirection,
    },
    value::{FieldPath, IntoValue, Object, Value},
};

pub(crate) fn field(parts: &[&str]) -> Expr {
    Expr::Operand(Operand::Field(FieldPath::from_fields(
        parts.iter().copied(),
    )))
}

pub(crate) fn string(value: &str) -> Expr {
    Expr::Operand(Operand::Literal(Value::String(value.into())))
}

pub(crate) fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

pub(crate) fn all(predicates: impl IntoIterator<Item = Expr>) -> Expr {
    predicates
        .into_iter()
        .reduce(|a, b| binary(BinaryOp::And, a, b))
        .unwrap_or(Expr::Operand(Operand::Literal(Value::Bool(true))))
}

pub(crate) fn any(predicates: impl IntoIterator<Item = Expr>) -> Expr {
    predicates
        .into_iter()
        .reduce(|a, b| binary(BinaryOp::Or, a, b))
        .unwrap_or(Expr::Operand(Operand::Literal(Value::Bool(false))))
}

pub(crate) fn ilike(expr: Expr, pattern: &str) -> Expr {
    Expr::PatternMatch {
        kind: PatternMatchKind::Like,
        expr: Box::new(expr),
        pattern: Box::new(string(pattern)),
        case_insensitive: true,
        negated: false,
    }
}

pub(crate) fn projection(expr: Expr, alias: &str) -> QueryField {
    QueryField {
        expr: Box::new(expr),
        alias: Some(alias.into()),
        wildcard: None,
    }
}

pub(crate) fn wildcard(alias: &str) -> QueryField {
    QueryField {
        expr: Box::new(Expr::Operand(Operand::Literal(Value::Null))),
        alias: None,
        wildcard: Some(FieldPath::from_fields([alias])),
    }
}

pub(crate) fn order(parts: &[&str], direction: SortDirection) -> OrderBy {
    OrderBy {
        expr: field(parts),
        direction,
    }
}

pub(crate) fn select(alias: &str) -> SelectQuery {
    SelectQuery::new()
        .with_collection(DEFAULT_COLLECTION)
        .with_source_alias(alias)
        .with_field_format(FieldFormat::Qualified)
}

/// Encode an AST query request with its optional scope and separate parameter values.
pub fn query_payload(
    query: impl Into<Query>,
    scope_id: Option<&str>,
    params: Option<Object>,
) -> Value {
    let mut payload = Object::new();
    payload.insert("query", query.into().into_value());
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id.into()));
    }
    if let Some(params) = params {
        payload.insert("params", Value::Object(params));
    }
    Value::Object(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::value::FromValue;

    #[test]
    fn ast_requests_preserve_parameters_and_scope_without_a_text_format() {
        let query = SelectQuery::new().with_predicate(binary(
            BinaryOp::Eq,
            field(&["id"]),
            Expr::parameter("id"),
        ));
        let mut params = Object::new();
        params.insert("id", Value::U8(7));
        let Value::Object(payload) =
            query_payload(query.clone(), Some("scope"), Some(params.clone()))
        else {
            panic!("object payload")
        };
        assert!(payload.get("format").is_none());
        assert_eq!(
            payload.get("scope_id").and_then(Value::as_str),
            Some("scope")
        );
        assert_eq!(payload.get("params"), Some(&Value::Object(params)));
        assert_eq!(
            Query::from_value(payload.get("query").unwrap().clone()).unwrap(),
            Query::from(query)
        );
        let Value::Object(payload) = query_payload(SelectQuery::new(), None, None) else {
            panic!("object payload")
        };
        assert!(payload.get("scope_id").is_none());
        assert!(payload.get("params").is_none());
    }
}
