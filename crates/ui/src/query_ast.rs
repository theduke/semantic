//! Constructors for queries built by the UI. Text is reserved for the SQL editor.

use semantic_data::{
    query::{BinaryOp, Expr, Operand, OrderBy, Query, SelectQuery, SortDirection},
    value::{FieldPath, Object, Value},
};

pub(crate) fn field(alias: Option<&str>, name: &str) -> Expr {
    Expr::Operand(Operand::Field(FieldPath::from_fields(
        alias.into_iter().chain([name]),
    )))
}

pub(crate) fn literal(value: impl Into<Value>) -> Expr {
    Expr::Operand(Operand::Literal(value.into()))
}

pub(crate) fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

pub(crate) fn combine(op: BinaryOp, expressions: impl IntoIterator<Item = Expr>) -> Option<Expr> {
    expressions
        .into_iter()
        .reduce(|left, right| binary(op, left, right))
}

pub(crate) fn order(alias: Option<&str>, name: &str, direction: SortDirection) -> OrderBy {
    OrderBy {
        expr: field(alias, name),
        direction,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum QueryRequest {
    Ast(SelectQuery),
    Sql(String),
}

impl From<SelectQuery> for QueryRequest {
    fn from(query: SelectQuery) -> Self {
        Self::Ast(query)
    }
}

impl QueryRequest {
    pub(crate) fn payload(self, scope_id: Option<&str>) -> Value {
        match self {
            Self::Ast(query) => {
                semantic_ui_core::query_ast::query_payload(Query::Select(query), scope_id, None)
            }
            Self::Sql(query) => {
                let mut payload = Object::new();
                payload.insert("query", Value::String(query));
                payload.insert("format", Value::String("sql".into()));
                if let Some(scope_id) = scope_id {
                    payload.insert("scope_id", Value::String(scope_id.into()));
                }
                Value::Object(payload)
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use semantic_data::value::{FromValue, IntoValue};
    use semantic_rpc::{
        RpcClient,
        client::{RpcClientDyn, RpcClientFuture},
    };
    use semantic_rpc_core::RpcClientError;
    use std::sync::{Arc, Mutex};

    pub(crate) fn contains_string(query: &SelectQuery, text: &str) -> bool {
        fn walk(value: &Value, text: &str) -> bool {
            match value {
                Value::String(value) => value == text,
                Value::List(values) => values.iter().any(|value| walk(value, text)),
                Value::Object(values) => values.values().any(|value| walk(value, text)),
                _ => false,
            }
        }
        walk(&query.clone().into_value(), text)
    }

    pub(crate) type CapturedCalls = Arc<Mutex<Vec<(String, Value)>>>;

    pub(crate) fn capture_client() -> (RpcClient, CapturedCalls) {
        struct Capture(CapturedCalls);
        impl RpcClientDyn for Capture {
            fn invoke_value(
                &self,
                command: String,
                payload: Value,
            ) -> RpcClientFuture<Result<Value, RpcClientError>> {
                self.0.lock().unwrap().push((command, payload));
                Box::pin(async { Ok(Value::Object(Object::new())) })
            }
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        (RpcClient::new(Capture(calls.clone())), calls)
    }

    pub(crate) fn assert_ast_call(
        call: &(String, Value),
        query: &SelectQuery,
        scope_id: Option<&str>,
    ) {
        assert_eq!(call.0, "semantic.db.query");
        let Value::Object(payload) = &call.1 else {
            panic!("object request")
        };
        assert!(payload.get("format").is_none());
        assert!(payload.get("params").is_none());
        assert_eq!(payload.get("scope_id").and_then(Value::as_str), scope_id);
        assert_eq!(
            Query::from_value(payload.get("query").unwrap().clone()).unwrap(),
            Query::Select(query.clone())
        );
    }

    pub(crate) async fn memory_db(
        collection: &str,
        rows: impl IntoIterator<Item = Object>,
    ) -> semantic_db_core::Db {
        use semantic_data::query::{DdlBatch, DdlCollectionKind, DdlOperation, IntegrityMode};
        let db = semantic_db_core::Db::new(semantic_db_kv::MemoryBackend::new(
            semantic_db_kv::open_memory().unwrap(),
        ));
        db.execute_ddl(DdlBatch::new().with_op(DdlOperation::UpsertCollection {
            name: collection.into(),
            kind: DdlCollectionKind::Untyped,
            integrity_mode: IntegrityMode::Permissive,
        }))
        .await
        .unwrap();
        for row in rows {
            let id = row.get("id").unwrap().as_str().unwrap().to_string();
            db.insert(collection, id, row).await.unwrap();
        }
        db
    }

    pub(crate) fn row(id: &str, fields: impl IntoIterator<Item = (&'static str, Value)>) -> Object {
        let mut object = Object::new();
        object.insert("id", Value::String(id.into()));
        for (name, value) in fields {
            object.insert(name, value);
        }
        object
    }

    #[test]
    fn typed_literals_survive_ast_payloads() {
        let query = SelectQuery::new().with_predicate(binary(
            BinaryOp::Eq,
            field(None, "score"),
            literal(Value::F64(1.0.into())),
        ));
        assert_ast_call(
            &(
                "semantic.db.query".into(),
                QueryRequest::Ast(query.clone()).payload(Some("scope")),
            ),
            &query,
            Some("scope"),
        );
    }
}
