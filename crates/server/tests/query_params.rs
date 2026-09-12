#![cfg(feature = "redb")]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use http::Request;
use semantic_app::{DbScopeId, SemanticApp};
use semantic_data::schema::DbOpenMode;
use semantic_data::value::{Object, Value};
use semantic_db_core::{Db, catalog::CollectionKind};
use semantic_rpc_core::{RpcRequest, RpcResponse, RpcResult};
use semantic_server::SemanticServer;
use tower::ServiceExt;

async fn query(server: &SemanticServer, sql: &str, format: &str, params: Object) -> RpcResult {
    let mut payload = Object::new();
    payload.insert("query", sql.to_string());
    payload.insert("format", format.to_string());
    if !params.is_empty() {
        payload.insert("params", Value::Object(params));
    }
    let request = RpcRequest {
        id: 1,
        command: "semantic.db.query".into(),
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
    serde_json::from_slice::<RpcResponse>(
        &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
    )
    .unwrap()
    .result
}

#[tokio::test(flavor = "multi_thread")]
async fn http_named_sql_parameters() {
    let directory = tempfile::tempdir().unwrap();
    let db = Db::new(
        semantic_db_redb::open_backend(directory.path().join("db"), DbOpenMode::AutoCreate)
            .unwrap(),
    );
    db.create_collection("items", CollectionKind::Polymorphic)
        .await
        .unwrap();
    let mut row = Object::new();
    row.insert("id", "row".to_string());
    db.insert("items", "row", row).await.unwrap();
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("default"), Arc::new(db))
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    let server = SemanticServer::new(app.clone());

    for value in [
        Value::Null,
        Value::Bool(true),
        Value::U64(u64::MAX),
        Value::I128(i128::MAX),
        Value::String("' OR true -- :not_a_binding".into()),
        Value::Bytes(vec![0, 255].into()),
        Value::List(vec![Value::I64(9)]),
        Value::Uuid(semantic_data::value::Uuid::NIL),
        Value::DateTime(semantic_data::value::DateTime::now_utc()),
        Value::Object(Object::new()),
    ] {
        let mut params = Object::new();
        params.insert("value", value.clone());
        let result = query(
            &server,
            "SELECT :value AS first, :value AS second, ':quoted' AS text FROM items /* :comment */",
            "sql",
            params,
        )
        .await;
        let RpcResult::Ok(Value::Object(result)) = result else {
            panic!("{result:?}")
        };
        let Some(Value::List(rows)) = result.get("rows") else {
            panic!("rows")
        };
        let Value::Object(row) = &rows[0] else {
            panic!("row")
        };
        assert_eq!(row.get("first"), Some(&value));
        assert_eq!(row.get("second"), Some(&value));
    }

    for (sql, format, entries, reason) in [
        ("SELECT :missing FROM items", "sql", vec![], "missing"),
        (
            "SELECT id FROM items",
            "sql",
            vec![("extra", Value::Null)],
            "unused",
        ),
        (
            "SELECT :Name FROM items",
            "sql",
            vec![("name", Value::Null)],
            "missing",
        ),
        ("SELECT :1 FROM items", "sql", vec![], "invalid_name"),
        (
            "SELECT :\"quoted\" FROM items",
            "sql",
            vec![],
            "invalid_name",
        ),
        (
            "SELECT id FROM :table",
            "sql",
            vec![("table", Value::String("items".into()))],
            "invalid_position",
        ),
        (
            "from items",
            "prql",
            vec![("name", Value::Null)],
            "unsupported_format",
        ),
    ] {
        let params = entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();
        let RpcResult::Err(error) = query(&server, sql, format, params).await else {
            panic!("expected {reason}: {sql}")
        };
        assert_eq!(error.code, "query_parameter_error", "{sql}: {error:?}");
        let Some(Value::Object(data)) = error.data else {
            panic!("error data")
        };
        assert_eq!(data.get("reason"), Some(&Value::String(reason.into())));
    }
    assert!(matches!(
        query(&server, "SELECT id FROM items", "sql", Object::new()).await,
        RpcResult::Ok(_)
    ));
    let mut params = Object::new();
    params.insert("value", Value::String("text".into()));
    let RpcResult::Err(error) =
        query(&server, "SELECT :value::text FROM items", "sql", params).await
    else {
        panic!("casts are currently unsupported")
    };
    assert_eq!(
        error.code, "db_error",
        ":: must not be interpreted as a binding"
    );
    app.shutdown().await.unwrap();
}
