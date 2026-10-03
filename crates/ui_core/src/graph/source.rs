use std::collections::BTreeMap;

use futures::future::LocalBoxFuture;
use semantic_data::{
    attr::{ATTR_PARENT, ATTR_TITLE},
    query::{BinaryOp, Expr, Operand, SelectQuery, SortDirection},
    value::{Object, Value},
};
use semantic_rpc::RpcClient;

use crate::{
    EntityTarget,
    query_ast::{
        RELATION_EDGES_COLLECTION, all, any, binary, field, order, query_payload, select, wildcard,
    },
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
    fn entities<'a>(
        &'a self,
        ids: &'a [EntityTarget],
    ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>>;
    fn children<'a>(
        &'a self,
        parents: &'a [EntityTarget],
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
        ids: &'a [EntityTarget],
    ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
        Box::pin(async move {
            let mut rows = Vec::new();
            for (collection, ids) in grouped_ids(ids) {
                // Bound request size even when this source is used independently of the explorer.
                for chunk in ids.chunks(50) {
                    rows.extend(
                        self.query(entities_query(&collection), ids_params(chunk))
                            .await?,
                    );
                }
            }
            Ok(rows)
        })
    }

    fn children<'a>(
        &'a self,
        parents: &'a [EntityTarget],
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<Object>, String>> {
        Box::pin(async move {
            let mut rows = Vec::new();
            for (collection, ids) in grouped_ids(parents) {
                if rows.len() >= limit {
                    break;
                }
                for chunk in ids.chunks(50) {
                    let remaining = limit.saturating_sub(rows.len());
                    if remaining == 0 {
                        break;
                    }
                    rows.extend(
                        self.query(
                            children_query(&collection),
                            limited_params(chunk, remaining),
                        )
                        .await?,
                    );
                }
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

fn grouped_ids(targets: &[EntityTarget]) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for target in targets {
        groups
            .entry(target.collection_or_default().into())
            .or_default()
            .push(target.id.clone());
    }
    groups
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
fn entities_query(collection: &str) -> SelectQuery {
    select("entity")
        .with_collection(collection)
        .with_projection(vec![wildcard("entity")])
        .with_predicate(membership("entity", "id"))
        .with_order_by(vec![order(&["entity", "id"], SortDirection::Asc)])
}
fn children_query(collection: &str) -> SelectQuery {
    select("child")
        .with_collection(collection)
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

    #[test]
    fn queries_use_list_parameters_and_bounded_direct_relations() {
        let params = limited_params(&["a".into(), "b".into()], 51);
        assert_eq!(
            params.get("ids"),
            Some(&Value::List(vec![
                Value::String("a".into()),
                Value::String("b".into())
            ]))
        );
        assert_eq!(params.get("limit"), Some(&Value::U64(51)));
        for query in [
            entities_query("custom"),
            children_query("custom"),
            relations_query(),
        ] {
            let value = query.clone().into_value();
            assert_eq!(SelectQuery::from_value(value).unwrap(), query);
        }
        assert_eq!(
            entities_query("custom").predicate,
            Some(membership("entity", "id"))
        );
        assert_eq!(
            children_query("custom").predicate,
            Some(membership("child", ATTR_PARENT))
        );
        assert_eq!(
            relations_query().predicate,
            Some(all([
                binary(
                    BinaryOp::Eq,
                    field(&["edge", "depth"]),
                    Expr::Operand(Operand::Literal(Value::U64(1)))
                ),
                any([membership("edge", "source"), membership("edge", "target")])
            ]))
        );
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
                entities_query("graph_test"),
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
                children_query("graph_test"),
                limited_params(&["root".into()], 1).into_iter().collect(),
            ))
            .await
            .unwrap();
        let semantic_db_core::QueryResult::Select(rows) = result else {
            panic!("select result")
        };
        assert_eq!(rows.len(), 1);
    }
}
