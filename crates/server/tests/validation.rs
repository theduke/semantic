#![cfg(feature = "redb")]

use axum::body::{Body, to_bytes};
use http::Request;
use semantic_app::{DbScopeId, SemanticApp};
use semantic_data::{
    schema::{
        AttributeType, ClassAttribute, ClassType, Constraint, DbOpenMode, Field, Meta, RecordType,
        StringType, Type, TypeKind, attribute::attribute_ref::AttributeRef,
    },
    value::{Object, Value},
};
use semantic_db_core::{Db, DdlBatch, DdlOperation};
use semantic_rpc_core::{RpcRequest, RpcResponse, RpcResult};
use semantic_server::SemanticServer;
use std::{collections::BTreeMap, sync::Arc};
use tower::ServiceExt;

async fn rpc(server: &SemanticServer, command: &str, payload: Object) -> RpcResult {
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

#[tokio::test(flavor = "multi_thread")]
async fn http_validation_preflight_activation_and_structured_nested_errors() {
    let directory = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::new(
        semantic_db_redb::open_backend(directory.path().join("db"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let mut string = Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }));
    string
        .constraints
        .push(Constraint::Pattern("^[a-z]+$".into()));
    let payload = Type::new(TypeKind::Record(RecordType {
        fields: BTreeMap::from([(
            "name".into(),
            Field {
                ty: string,
                required: true,
                readonly: false,
                writeonly: false,
                default: None,
                meta: Meta::default(),
            },
        )]),
        open: false,
        additional: None,
        required_order: None,
    }));
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "http:payload".into(),
                    name: "Payload".into(),
                    ty: payload,
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertClass {
                class: ClassType {
                    id: "http:Holder".into(),
                    name: "Holder".into(),
                    inherits: None,
                    extends: vec![],
                    strict_schema: false,
                    creatable_in_ui: None,
                    attributes: BTreeMap::from([(
                        "payload".into(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: "http:payload".into(),
                            },
                            required: true,
                            ui_order: None,
                            computed: None,
                            constraints: vec![],
                            meta: Meta::default(),
                        },
                    )]),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            }),
    )
    .await
    .unwrap();
    let mut legacy = Object::new();
    legacy.insert("id", "legacy".to_string());
    legacy.insert("type", "http:Holder".to_string());
    db.insert(None::<&str>, "legacy", legacy).await.unwrap();
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("default"), db.clone())
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    let server = SemanticServer::new(app);
    let RpcResult::Ok(Value::List(violations)) =
        rpc(&server, "semantic.db.validation.preflight", Object::new()).await
    else {
        panic!("preflight report expected")
    };
    assert_eq!(violations.len(), 1);
    let RpcResult::Err(error) =
        rpc(&server, "semantic.db.validation.activate", Object::new()).await
    else {
        panic!("activation must refuse legacy violations")
    };
    assert_eq!(error.code, "validation_failed");
    let Some(Value::Object(data)) = error.data else {
        panic!("structured data expected")
    };
    assert_eq!(data.get("rule"), Some(&Value::String("required".into())));
    db.delete(None::<&str>, "legacy").await.unwrap();
    assert!(matches!(
        rpc(&server, "semantic.db.validation.activate", Object::new()).await,
        RpcResult::Ok(_)
    ));
    for (name, rule) in [
        (Value::String("INVALID".into()), "pattern"),
        (Value::I64(1), "type"),
    ] {
        let mut object = Object::new();
        object.insert("id", "bad".to_string());
        object.insert("type", "http:Holder".to_string());
        object.insert(
            "http:payload",
            Value::Object(Object::from_iter([("name".into(), name)])),
        );
        let mut insert = Object::new();
        insert.insert("id", "bad".to_string());
        insert.insert("object", Value::Object(object));
        let RpcResult::Err(error) = rpc(&server, "semantic.db.insert", insert).await else {
            panic!("write should fail")
        };
        assert_eq!(error.code, "validation_failed");
        let Some(Value::Object(data)) = error.data else {
            panic!("structured data expected")
        };
        assert_eq!(
            data.get("class"),
            Some(&Value::String("http:Holder".into()))
        );
        assert_eq!(
            data.get("attribute"),
            Some(&Value::String("http:payload".into()))
        );
        assert_eq!(data.get("rule"), Some(&Value::String(rule.into())));
        assert_eq!(
            data.get("path"),
            Some(&Value::List(vec![
                Value::Object(Object::from_iter([(
                    "field".into(),
                    Value::String("http:payload".into())
                )])),
                Value::Object(Object::from_iter([(
                    "field".into(),
                    Value::String("name".into())
                )]))
            ]))
        );
        assert!(data.contains_key("expected") && data.contains_key("actual"));
        assert!(db.get(None::<&str>, "bad").await.unwrap().is_none());
    }
}
