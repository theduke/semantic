#![cfg(feature = "redb")]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use http::Request;
use semantic_app::{DbScopeId, SemanticApp};
use semantic_data::query::{
    DdlBatch, DdlCollectionKind, DdlOperation, DdlQuery, Expr, IntegrityMode, Operand, Query,
    QueryField, SelectQuery,
};
use semantic_data::schema::DbOpenMode;
use semantic_data::value::{FromValue, IntoValue, Object, Value};
use semantic_db_core::{Db, catalog::CollectionKind};
use semantic_rpc_core::{RpcRequest, RpcResponse, RpcResult};
use semantic_server::SemanticServer;
use tower::ServiceExt;

async fn invoke(server: &SemanticServer, command: &str, payload: Object) -> RpcResult {
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
    serde_json::from_slice::<RpcResponse>(
        &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
    )
    .unwrap()
    .result
}

async fn fixture() -> (tempfile::TempDir, Arc<Db>, SemanticApp, SemanticServer) {
    let directory = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::new(
        semantic_db_redb::open_backend(directory.path().join("db"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    db.create_collection("items", CollectionKind::Polymorphic)
        .await
        .unwrap();
    db.insert(
        "items",
        "row",
        Object::from_iter([("id".into(), Value::String("row".into()))]),
    )
    .await
    .unwrap();
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("default"), db.clone())
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    let server = SemanticServer::new(app.clone());
    (directory, db, app, server)
}

fn field(expr: Expr, alias: &str) -> QueryField {
    QueryField {
        expr: Box::new(expr),
        alias: Some(alias.into()),
        wildcard: None,
    }
}

fn rows(result: RpcResult) -> Vec<Object> {
    let RpcResult::Ok(Value::Object(mut output)) = result else {
        panic!("{result:?}")
    };
    let Some(Value::List(rows)) = output.remove("rows") else {
        panic!("rows")
    };
    rows.into_iter()
        .map(|row| {
            let Value::Object(row) = row else {
                panic!("row")
            };
            row
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn http_client_constructs_ast_and_reuses_unbound_parameters_with_exact_values() {
    let (_directory, _db, app, server) = fixture().await;
    let reusable = Query::Select(
        SelectQuery::new()
            .with_collection("items")
            .with_projection(vec![field(Expr::parameter("value"), "value")]),
    );
    for value in [
        Value::Null,
        Value::I8(-5),
        Value::U64(u64::MAX),
        Value::I128(i128::MAX),
        Value::Bytes(vec![0, 255].into()),
        Value::String("' OR true -- :not_a_binding".into()),
        Value::Uuid(semantic_data::value::Uuid::NIL),
        Value::List(vec![Value::I16(7)]),
    ] {
        let payload = Object::from_iter([
            ("query".into(), reusable.clone().into_value()),
            (
                "params".into(),
                Value::Object(Object::from_iter([("value".into(), value.clone())])),
            ),
        ]);
        assert_eq!(
            rows(invoke(&server, "semantic.db.query", payload).await)[0].get("value"),
            Some(&value)
        );
        let literal = Query::Select(SelectQuery::new().with_collection("items").with_projection(
            vec![field(
                Expr::Operand(Operand::Literal(value.clone())),
                "value",
            )],
        ));
        let payload = Object::from_iter([("query".into(), literal.into_value())]);
        assert_eq!(
            rows(invoke(&server, "semantic.db.query", payload).await)[0].get("value"),
            Some(&value)
        );
    }
    assert!(
        matches!(reusable, Query::Select(ref select) if matches!(&*select.projection[0].expr, Expr::Operand(Operand::Parameter(name)) if name == "value"))
    );
    for (params, expected) in [
        (Object::new(), "missing"),
        (
            Object::from_iter([
                ("value".into(), Value::Null),
                ("unused".into(), Value::Null),
            ]),
            "unused",
        ),
    ] {
        let payload = Object::from_iter([
            ("query".into(), reusable.clone().into_value()),
            ("params".into(), Value::Object(params)),
        ]);
        let RpcResult::Err(error) = invoke(&server, "semantic.db.query", payload).await else {
            panic!("parameter error")
        };
        assert_eq!(error.code, "query_parameter_error");
        let Some(Value::Object(data)) = error.data else {
            panic!("error data")
        };
        assert_eq!(data.get("reason").and_then(Value::as_str), Some(expected));
    }
    let payload = Object::from_iter([
        ("query".into(), reusable.into_value()),
        ("format".into(), Value::String("sql".into())),
    ]);
    let RpcResult::Err(error) = invoke(&server, "semantic.db.query", payload).await else {
        panic!("format conflict")
    };
    assert!(error.message.contains("format must be omitted"));
    app.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn http_parse_preserves_nested_parameters_and_ddl_without_execution() {
    let (_directory, db, app, server) = fixture().await;
    for sql in [
        "SELECT :value AS value FROM items LIMIT :limit",
        "SELECT id FROM items WHERE EXISTS (SELECT inner_items.id FROM items AS inner_items WHERE inner_items.id = :id)",
        "CREATE ATTRIBUTE \"test:unexecuted\" TYPE string",
    ] {
        let result = invoke(
            &server,
            "semantic.db.query.parse_sql",
            Object::from_iter([("query".into(), Value::String(sql.into()))]),
        )
        .await;
        let RpcResult::Ok(value) = result else {
            panic!("{result:?}")
        };
        let ast = Query::from_value(value).unwrap();
        assert_eq!(ast, db.parse_sql(sql).unwrap());
        if sql.starts_with("CREATE") {
            assert!(matches!(ast, Query::Ddl(_)));
        }
    }
    assert!(
        db.catalog()
            .await
            .unwrap()
            .attribute_by_id("test:unexecuted")
            .is_none()
    );
    let ddl = Query::Ddl(DdlQuery {
        batch: DdlBatch::new().with_op(DdlOperation::UpsertCollection {
            name: "created_by_ast".into(),
            kind: DdlCollectionKind::Polymorphic,
            integrity_mode: IntegrityMode::Permissive,
        }),
    });
    assert!(matches!(
        invoke(
            &server,
            "semantic.db.query",
            Object::from_iter([("query".into(), ddl.into_value())])
        )
        .await,
        RpcResult::Ok(_)
    ));
    let select = Query::Select(SelectQuery::new().with_collection("created_by_ast"));
    assert!(
        rows(
            invoke(
                &server,
                "semantic.db.query",
                Object::from_iter([("query".into(), select.into_value())])
            )
            .await
        )
        .is_empty()
    );
    for (format, text) in [
        ("sql", "SELECT id FROM items"),
        ("prql", "from items | select {id}"),
    ] {
        let payload = Object::from_iter([
            ("query".into(), Value::String(text.into())),
            ("format".into(), Value::String(format.into())),
        ]);
        assert_eq!(
            rows(invoke(&server, "semantic.db.query", payload).await)[0]
                .get("id")
                .and_then(Value::as_str),
            Some("row")
        );
    }
    app.shutdown().await.unwrap();
}
