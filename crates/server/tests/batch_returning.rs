#![cfg(feature = "redb")]

use axum::body::{Body, to_bytes};
use http::Request;
use semantic_app::{DbScopeId, SemanticApp};
use semantic_data::{
    query::{Batch, BatchOperation},
    schema::DbOpenMode,
    value::{Object, Value},
};
use semantic_db_core::{Db, catalog::CollectionKind};
use semantic_rpc_core::{RpcRequest, RpcResponse, RpcResult};
use semantic_server::SemanticServer;
use std::sync::Arc;
use tower::ServiceExt;

async fn batch(
    server: &SemanticServer,
    operations: Vec<Value>,
    returning: Option<Value>,
) -> RpcResult {
    let mut payload = Object::new();
    payload.insert("operations", Value::List(operations));
    if let Some(returning) = returning {
        payload.insert("returning", returning);
    }
    let request = RpcRequest {
        id: 1,
        command: "semantic.db.batch".into(),
        payload: Value::Object(payload),
    };
    let response = server
        .router()
        .oneshot(
            Request::post("/api/v1/rpc")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    serde_json::from_slice::<RpcResponse>(
        &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
    )
    .unwrap()
    .result
}
fn row(id: &str, name: &str) -> Object {
    Object::from_iter([
        ("id".into(), Value::String(id.into())),
        ("batch_label".into(), Value::String(name.into())),
    ])
}
fn operation(kind: &str, id: &str, name: &str) -> Value {
    let mut op = Object::new();
    op.insert("kind", kind.to_string());
    op.insert("collection", "items".to_string());
    op.insert("id", id.to_string());
    if kind == "upsert" || kind == "create" {
        op.insert("object", Value::Object(row(id, name)));
    }
    Value::Object(op)
}
fn projection(fields: &[&str]) -> Value {
    Value::Object(Object::from_iter([(
        "projection".into(),
        Value::Object(Object::from_iter([(
            "fields".into(),
            Value::List(
                fields
                    .iter()
                    .map(|field| Value::String((*field).into()))
                    .collect(),
            ),
        )])),
    )]))
}
fn output(result: RpcResult) -> Object {
    let RpcResult::Ok(Value::Object(out)) = result else {
        panic!("{result:?}")
    };
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_returning_http_modes_errors_size_and_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    {
        let db = Arc::new(Db::new(
            semantic_db_redb::open_backend(&path, DbOpenMode::AutoCreate).unwrap(),
        ));
        db.create_collection("items", CollectionKind::Polymorphic)
            .await
            .unwrap();
        let app = SemanticApp::builder()
            .with_default_scope(DbScopeId::new("default"), db.clone())
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        let server = SemanticServer::new(app.clone());
        let initial = output(batch(&server, vec![operation("upsert", "a", "one")], None).await);
        assert!(initial.contains_key("dataset"));
        assert!(!initial.contains_key("changes"));
        let compact = output(
            batch(
                &server,
                vec![operation("upsert", "a", "two")],
                Some(Value::String("changes".into())),
            )
            .await,
        );
        assert_eq!(compact.len(), 2);
        let compact_bytes = serde_json::to_vec(&Value::Object(compact)).unwrap().len();
        let mut seed = Batch::new();
        for index in 0..300 {
            let id = format!("background-{index:04}");
            seed = seed.with_op(BatchOperation::Upsert {
                collection: "items".into(),
                id: id.clone(),
                object: row(&id, "background data"),
            });
        }
        db.execute_batch(seed).await.unwrap();
        let compact = output(
            batch(
                &server,
                vec![operation("upsert", "a", "three")],
                Some(Value::String("changes".into())),
            )
            .await,
        );
        assert_eq!(
            serde_json::to_vec(&Value::Object(compact)).unwrap().len(),
            compact_bytes
        );
        let stats = output(
            batch(
                &server,
                vec![operation("upsert", "a", "four")],
                Some(Value::String("stats".into())),
            )
            .await,
        );
        assert_eq!(stats.len(), 1);
        assert!(stats.contains_key("stats"));
        let projected = output(
            batch(
                &server,
                vec![
                    operation("upsert", "a", "intermediate"),
                    operation("upsert", "a", "final"),
                    operation("create", "new", "created"),
                ],
                Some(projection(&["batch_label"])),
            )
            .await,
        );
        assert!(!projected.contains_key("dataset"));
        let Some(Value::List(rows)) = projected.get("rows") else {
            panic!("rows")
        };
        assert_eq!(rows.len(), 2);
        let Value::Object(first) = &rows[0] else {
            panic!("row")
        };
        assert_eq!(first.get("id"), Some(&Value::String("a".into())));
        assert_eq!(
            first.get("object"),
            Some(&Value::Object(Object::from_iter([(
                "batch_label".into(),
                Value::String("final".into())
            )])))
        );
        for (returning, reason) in [
            (Value::String("bad".into()), "unknown_mode"),
            (Value::Null, "unknown_mode"),
            (projection(&["missing"]), "unknown_field"),
            (
                projection(&["batch_label", "batch_label"]),
                "duplicate_field",
            ),
        ] {
            let RpcResult::Err(error) = batch(
                &server,
                vec![operation("upsert", "a", "must not commit")],
                Some(returning),
            )
            .await
            else {
                panic!("error")
            };
            assert_eq!(error.code, "batch_return_error");
            let Some(Value::Object(data)) = error.data else {
                panic!("data")
            };
            assert_eq!(data.get("reason"), Some(&Value::String(reason.into())));
            assert_eq!(
                db.get("items", "a")
                    .await
                    .unwrap()
                    .unwrap()
                    .object
                    .get("batch_label"),
                Some(&Value::String("final".into()))
            );
        }
        let empty = output(
            batch(
                &server,
                vec![operation("upsert", "a", "last")],
                Some(projection(&[])),
            )
            .await,
        );
        let Some(Value::List(rows)) = empty.get("rows") else {
            panic!("rows")
        };
        let Value::Object(row) = &rows[0] else {
            panic!("row")
        };
        assert_eq!(row.get("object"), Some(&Value::Object(Object::new())));
        app.shutdown().await.unwrap();
    }
    let db = Db::new(semantic_db_redb::open_backend(&path, DbOpenMode::OpenExisting).unwrap());
    assert_eq!(
        db.get("items", "a")
            .await
            .unwrap()
            .unwrap()
            .object
            .get("batch_label"),
        Some(&Value::String("last".into()))
    );
    assert!(db.get("items", "new").await.unwrap().is_some());
}
