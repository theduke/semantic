#![cfg(feature = "redb")]

use axum::body::{Body, to_bytes};
use http::Request;
use semantic_app::{DbScopeId, SemanticApp};
use semantic_data::{
    schema::DbOpenMode,
    value::{Object, Value},
};
use semantic_db_core::Db;
use semantic_rpc_core::{RpcRequest, RpcResponse, RpcResult};
use semantic_server::SemanticServer;
use std::sync::Arc;
use tower::ServiceExt;

async fn rpc(server: &SemanticServer, command: &str, payload: Object) -> Object {
    let request = RpcRequest {
        id: 1,
        command: command.into(),
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
    assert!(response.status().is_success());
    let response = serde_json::from_slice::<RpcResponse>(
        &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
    )
    .unwrap();
    match response.result {
        RpcResult::Ok(Value::Object(object)) => object,
        other => panic!("{command}: object expected, got {other:?}"),
    }
}

fn payload(fields: &[(&str, Value)]) -> Object {
    let mut object = Object::new();
    for (name, value) in fields {
        object.insert(*name, value.clone());
    }
    object
}

#[tokio::test(flavor = "multi_thread")]
async fn http_maintenance_commands() {
    let directory = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::new(
        semantic_db_redb::open_backend(directory.path().join("db"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let mut row = Object::new();
    row.insert("id", "one".to_string());
    db.insert(None::<&str>, "one", row).await.unwrap();
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("default"), db.clone())
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    let server = SemanticServer::new(app);

    let verify = rpc(&server, "semantic.db.maintenance.verify", Object::new()).await;
    assert_eq!(
        verify.get("semantic:maintenance:ok"),
        Some(&Value::Bool(true)),
        "{verify:?}"
    );
    assert_eq!(
        verify.get("semantic:maintenance:problem_count"),
        Some(&Value::U64(0))
    );

    let reindex = rpc(
        &server,
        "semantic.db.maintenance.reindex",
        payload(&[("collection", Value::String("entities".into()))]),
    )
    .await;
    let Some(Value::List(indexes)) = reindex.get("semantic:maintenance:indexes") else {
        panic!("reindexed indexes expected: {reindex:?}")
    };
    assert!(!indexes.is_empty());

    let repair = rpc(
        &server,
        "semantic.db.maintenance.repair",
        payload(&[("check_storage_integrity", Value::Bool(false))]),
    )
    .await;
    let Some(Value::Object(after)) = repair.get("semantic:maintenance:after") else {
        panic!("repair report expected: {repair:?}")
    };
    assert_eq!(
        after.get("semantic:maintenance:ok"),
        Some(&Value::Bool(true))
    );

    let stats = rpc(&server, "semantic.db.maintenance.stats", Object::new()).await;
    assert!(matches!(
        stats.get("semantic:maintenance:file_size_bytes"),
        Some(Value::U64(_))
    ));
    let compact = rpc(&server, "semantic.db.maintenance.compact", Object::new()).await;
    assert!(matches!(
        compact.get("semantic:maintenance:compact:compacted"),
        Some(Value::Bool(_))
    ));
    let rewrite = rpc(
        &server,
        "semantic.db.maintenance.rewrite_payloads",
        payload(&[("batch_size", Value::I64(10))]),
    )
    .await;
    assert_eq!(
        rewrite.get("semantic:maintenance:rewritten"),
        Some(&Value::U64(0))
    );

    let target = directory.path().join("backup.redb");
    let backup = rpc(
        &server,
        "semantic.db.maintenance.backup",
        payload(&[("path", Value::String(target.display().to_string()))]),
    )
    .await;
    assert!(
        matches!(backup.get("semantic:maintenance:entries"), Some(Value::U64(entries)) if *entries > 0)
    );
    assert!(target.exists());
}
