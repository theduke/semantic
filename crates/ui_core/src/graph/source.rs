use futures::future::LocalBoxFuture;
use semantic_data::{
    attr::{ATTR_PARENT, ATTR_TITLE},
    builtin::DEFAULT_COLLECTION,
    query::{BinaryOp, Expr, Operand, SelectQuery, SortDirection},
    value::{Object, Value},
};
use semantic_rpc::RpcClient;

use crate::query_ast::{
    RELATION_EDGES_COLLECTION, all, any, binary, field, order, query_payload, select, wildcard,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationEdgeRow {
    pub relation: String,
    pub source: String,
    pub target: String,
}

impl RelationEdgeRow {
    pub fn from_object(row: &Object) -> Result<Self, String> {
        let get = |field: &str| {
            row.get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| format!("relationship row has invalid {field}"))
        };
        Ok(Self {
            relation: get("relation")?,
            source: get("source")?,
            target: get("target")?,
        })
    }
}

/// Local futures match the UI's single-threaded RPC and Dioxus runtime.
pub trait GraphSource {
    fn entities<'a>(&'a self, ids: &'a [String])
    -> LocalBoxFuture<'a, Result<Vec<Object>, String>>;
    /// Hierarchy children are entities in the default collection.
    fn children<'a>(
        &'a self,
        parents: &'a [String],
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>>;
    fn relation_edges<'a>(
        &'a self,
        ids: &'a [String],
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<RelationEdgeRow>, String>>;
}

#[derive(Clone)]
pub struct RpcGraphSource {
    client: RpcClient,
    scope_id: Option<String>,
}

impl RpcGraphSource {
    pub fn new(client: RpcClient, scope_id: Option<String>) -> Self {
        Self { client, scope_id }
    }

    async fn query(&self, query: SelectQuery, params: Object) -> Result<Vec<Object>, String> {
        let response = self
            .client
            .invoke_value(
                "semantic.db.query",
                query_payload(query, self.scope_id.as_deref(), Some(params)),
            )
            .await
            .map_err(|error| error.to_string())?;
        parse_rows(response)
    }
}

impl GraphSource for RpcGraphSource {
    fn entities<'a>(
        &'a self,
        ids: &'a [String],
    ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
        Box::pin(async move {
            let mut rows = Vec::new();
            // Bound request size even when this source is used independently of the explorer.
            for chunk in ids.chunks(50) {
                rows.extend(self.query(entities_query(), ids_params(chunk)).await?);
            }
            Ok(rows)
        })
    }

    fn children<'a>(
        &'a self,
        parents: &'a [String],
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
        Box::pin(async move {
            let mut rows = Vec::new();
            for chunk in parents.chunks(50) {
                let remaining = limit.saturating_sub(rows.len());
                if remaining == 0 {
                    break;
                }
                rows.extend(
                    self.query(children_query(), limited_params(chunk, remaining))
                        .await?,
                );
            }
            Ok(rows)
        })
    }

    fn relation_edges<'a>(
        &'a self,
        ids: &'a [String],
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<RelationEdgeRow>, String>> {
        Box::pin(async move {
            if ids.is_empty() || limit == 0 {
                return Ok(Vec::new());
            }
            self.query(relations_query(), limited_params(ids, limit))
                .await?
                .iter()
                .map(RelationEdgeRow::from_object)
                .collect()
        })
    }
}

fn ids_params(ids: &[String]) -> Object {
    let mut params = Object::new();
    params.insert(
        "ids",
        Value::List(ids.iter().cloned().map(Value::String).collect()),
    );
    params
}
fn limited_params(ids: &[String], limit: usize) -> Object {
    let mut params = ids_params(ids);
    params.insert("limit", Value::U64(limit as u64));
    params
}
fn membership(alias: &str, name: &str) -> Expr {
    binary(BinaryOp::In, field(&[alias, name]), Expr::parameter("ids"))
}
fn entities_query() -> SelectQuery {
    select("entity")
        .with_collection(DEFAULT_COLLECTION)
        .with_projection(vec![wildcard("entity")])
        .with_predicate(membership("entity", "id"))
        .with_order_by(vec![order(&["entity", "id"], SortDirection::Asc)])
}
fn children_query() -> SelectQuery {
    select("child")
        .with_collection(DEFAULT_COLLECTION)
        .with_projection(vec![wildcard("child")])
        .with_predicate(membership("child", ATTR_PARENT))
        .with_order_by(vec![
            order(&["child", ATTR_TITLE], SortDirection::Asc),
            order(&["child", "id"], SortDirection::Asc),
        ])
        .with_limit(Expr::parameter("limit"))
}
fn relations_query() -> SelectQuery {
    select("edge")
        .with_collection(RELATION_EDGES_COLLECTION)
        .with_projection(vec![wildcard("edge")])
        .with_predicate(all([
            binary(
                BinaryOp::Eq,
                field(&["edge", "depth"]),
                Expr::Operand(Operand::Literal(Value::U64(1))),
            ),
            any([membership("edge", "source"), membership("edge", "target")]),
        ]))
        .with_order_by(
            ["relation", "source", "target"]
                .map(|name| order(&["edge", name], SortDirection::Asc))
                .to_vec(),
        )
        .with_limit(Expr::parameter("limit"))
}
fn parse_rows(response: Value) -> Result<Vec<Object>, String> {
    let Value::Object(response) = response else {
        return Err("query response must be an object".into());
    };
    let Some(Value::List(rows)) = response.get("rows") else {
        return Err("query response missing rows".into());
    };
    rows.iter()
        .map(|row| match row {
            Value::Object(row) => Ok(row.clone()),
            _ => Err("invalid query row".into()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::{
        query::{DdlBatch, DdlCollectionKind, DdlOperation, IntegrityMode, QueryInput},
        value::{FromValue, IntoValue},
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct RecordingClient {
        requests: Arc<Mutex<Vec<Value>>>,
        response_sizes: Arc<Mutex<std::collections::VecDeque<usize>>>,
    }

    impl semantic_rpc::RpcClientDyn for RecordingClient {
        fn invoke_value(
            &self,
            command: String,
            payload: Value,
        ) -> futures::future::BoxFuture<'static, Result<Value, semantic_rpc_core::RpcClientError>>
        {
            assert_eq!(command, "semantic.db.query");
            self.requests.lock().unwrap().push(payload);
            let count = self.response_sizes.lock().unwrap().pop_front().unwrap_or(0);
            let rows = (0..count)
                .map(|index| {
                    let mut row = Object::new();
                    row.insert("id", Value::String(format!("child-{index}")));
                    Value::Object(row)
                })
                .collect();
            let mut response = Object::new();
            response.insert("rows", Value::List(rows));
            Box::pin(async move { Ok(Value::Object(response)) })
        }
    }

    #[tokio::test]
    async fn rpc_entities_use_default_collection_and_scope_in_bounded_batches() {
        let client = RecordingClient::default();
        let source = RpcGraphSource::new(RpcClient::new(client.clone()), Some("scope".into()));
        let ids = (0..101)
            .map(|index| format!("entity-{index}"))
            .collect::<Vec<_>>();
        assert_eq!(
            entities_query().collection.as_deref(),
            Some(DEFAULT_COLLECTION)
        );
        source.entities(&ids).await.unwrap();
        let expected = ids
            .chunks(50)
            .map(|chunk| query_payload(entities_query(), Some("scope"), Some(ids_params(chunk))))
            .collect::<Vec<_>>();
        assert_eq!(*client.requests.lock().unwrap(), expected);
    }

    #[tokio::test]
    async fn rpc_children_share_the_limit_across_default_collection_batches() {
        let client = RecordingClient::default();
        client.response_sizes.lock().unwrap().extend([2, 1]);
        let source = RpcGraphSource::new(RpcClient::new(client.clone()), Some("scope".into()));
        let ids = (0..101)
            .map(|index| format!("parent-{index}"))
            .collect::<Vec<_>>();
        assert_eq!(
            children_query().collection.as_deref(),
            Some(DEFAULT_COLLECTION)
        );
        assert_eq!(source.children(&ids, 3).await.unwrap().len(), 3);
        assert_eq!(
            *client.requests.lock().unwrap(),
            vec![
                query_payload(
                    children_query(),
                    Some("scope"),
                    Some(limited_params(&ids[..50], 3))
                ),
                query_payload(
                    children_query(),
                    Some("scope"),
                    Some(limited_params(&ids[50..100], 1))
                ),
            ]
        );
    }

    #[tokio::test]
    async fn rpc_empty_or_zero_limit_requests_do_not_query_and_relations_keep_scope() {
        let client = RecordingClient::default();
        let source = RpcGraphSource::new(RpcClient::new(client.clone()), Some("scope".into()));
        let ids = vec!["root".into()];
        assert!(source.entities(&[]).await.unwrap().is_empty());
        assert!(source.children(&[], 3).await.unwrap().is_empty());
        assert!(source.children(&ids, 0).await.unwrap().is_empty());
        assert!(source.relation_edges(&[], 3).await.unwrap().is_empty());
        assert!(source.relation_edges(&ids, 0).await.unwrap().is_empty());
        assert!(client.requests.lock().unwrap().is_empty());
        source.relation_edges(&ids, 7).await.unwrap();
        assert_eq!(
            *client.requests.lock().unwrap(),
            vec![query_payload(
                relations_query(),
                Some("scope"),
                Some(limited_params(&ids, 7))
            )]
        );
    }

    #[test]
    fn query_parameters_and_ast_round_trip() {
        let params = limited_params(&["a".into(), "b".into()], 51);
        assert_eq!(
            params.get("ids"),
            Some(&Value::List(vec![
                Value::String("a".into()),
                Value::String("b".into())
            ]))
        );
        assert_eq!(params.get("limit"), Some(&Value::U64(51)));
        for query in [entities_query(), children_query(), relations_query()] {
            let value = query.clone().into_value();
            assert_eq!(SelectQuery::from_value(value).unwrap(), query);
        }
    }

    #[test]
    fn malformed_relationships_and_responses_are_reported() {
        let mut row = Object::new();
        for name in ["relation", "source", "target"] {
            row.insert(name, Value::String(name.into()));
        }
        assert_eq!(RelationEdgeRow::from_object(&row).unwrap().target, "target");
        row.insert("source", Value::U64(7));
        assert!(RelationEdgeRow::from_object(&row).is_err());
        assert!(parse_rows(Value::Null).is_err());
        assert!(parse_rows(Value::Object(Object::new())).is_err());
        row.insert("source", Value::String(String::new()));
        assert!(RelationEdgeRow::from_object(&row).is_err());
    }

    #[tokio::test]
    async fn kv_supports_in_with_list_parameter_and_limit() {
        let db = semantic_db_core::Db::new(semantic_db_kv::MemoryBackend::new(
            semantic_db_kv::open_memory().unwrap(),
        ));
        db.execute_ddl(DdlBatch::new().with_op(DdlOperation::UpsertCollection {
            name: "graph_test".into(),
            kind: DdlCollectionKind::Untyped,
            integrity_mode: IntegrityMode::Permissive,
        }))
        .await
        .unwrap();
        for id in ["a", "b", "c"] {
            let mut row = Object::new();
            row.insert("id", Value::String(id.into()));
            row.insert(ATTR_PARENT, Value::String("root".into()));
            db.insert("graph_test", id, row).await.unwrap();
        }
        let result = db
            .query(QueryInput::ast_with_params(
                entities_query().with_collection("graph_test"),
                ids_params(&["a".into(), "c".into()]).into_iter().collect(),
            ))
            .await
            .unwrap();
        let semantic_db_core::QueryResult::Select(rows) = result else {
            panic!("select result")
        };
        assert_eq!(rows.len(), 2);
        let result = db
            .query(QueryInput::ast_with_params(
                children_query().with_collection("graph_test"),
                limited_params(&["root".into()], 1).into_iter().collect(),
            ))
            .await
            .unwrap();
        let semantic_db_core::QueryResult::Select(rows) = result else {
            panic!("select result")
        };
        assert_eq!(rows.len(), 1);
        // Execute the actual relationship query, including incoming/outgoing
        // edges, unrelated edges, transitive rows and the bounded limit.
        use semantic_data::{
            attr::{ATTR_RELATION_FROM, ATTR_RELATION_RELATION, ATTR_RELATION_TO},
            schema::{RelationIndexingMode, RelationMode, RelationType},
        };
        db.upsert_relationship(RelationType {
            id: "graph_test:links".into(),
            name: "links".into(),
            source_collection: "graph_test".into(),
            mode: RelationMode::External,
            indexing_mode: RelationIndexingMode::Enabled,
            meta: Default::default(),
        })
        .await
        .unwrap();
        for (id, from, to) in [("ab", "a", "b"), ("bc", "b", "c"), ("unrelated", "c", "a")] {
            let mut row = Object::new();
            for (key, value) in [
                ("id", id),
                (ATTR_RELATION_FROM, from),
                (ATTR_RELATION_TO, to),
                (ATTR_RELATION_RELATION, "graph_test:links"),
            ] {
                row.insert(key, Value::String(value.into()));
            }
            db.insert("graph_test", id, row).await.unwrap();
        }
        let result = db
            .query(QueryInput::ast_with_params(
                relations_query(),
                limited_params(&["b".into()], 10).into_iter().collect(),
            ))
            .await
            .unwrap();
        let semantic_db_core::QueryResult::Select(rows) = result else {
            panic!("select result")
        };
        let endpoints = rows
            .iter()
            .map(|row| {
                let edge = RelationEdgeRow::from_object(row).unwrap();
                (edge.source, edge.target)
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            endpoints,
            std::collections::BTreeSet::from([
                ("a".into(), "b".into()),
                ("b".into(), "c".into()),
                ("b".into(), "root".into()),
            ])
        );
        assert_eq!(rows.len(), 3);
        assert!(
            rows.iter()
                .all(|row| row.get("depth") == Some(&Value::U64(1)))
        );
        let result = db
            .query(QueryInput::ast_with_params(
                relations_query(),
                limited_params(&["b".into()], 1).into_iter().collect(),
            ))
            .await
            .unwrap();
        let semantic_db_core::QueryResult::Select(rows) = result else {
            panic!("select result")
        };
        assert_eq!(rows.len(), 1);
    }
}
