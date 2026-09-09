mod auth;
mod config;
mod error;
mod file;
#[cfg(feature = "logfs")]
mod log;
mod logfs;
#[cfg(feature = "logfs")]
mod logfs_blob;
mod redb;
mod router;
mod startup;
mod storage;
#[cfg(feature = "embed-ui")]
mod ui;
mod ws;

pub use auth::{HeaderPrincipalResolver, NoAuthPrincipalResolver, PrincipalResolver};
pub use config::ServerConfig;
pub use error::ServerError;
#[cfg(feature = "logfs")]
pub use log::{LogDbConfig, LogDbProvider};
#[cfg(feature = "logfs")]
pub use logfs::LogFsDbProvider;
#[cfg(feature = "redb")]
pub use redb::RedbDbProvider;
pub use router::SemanticServer;
pub use startup::prompt_blob_password;
pub use storage::{LocalDbConfig, is_logfs_blob_uri, resolve_db_uri};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use http::Request;
    use semantic_app::{DbScopeId, SemanticDb};
    use semantic_data::value::{Object, Value};
    use semantic_db_core::catalog::Catalog;
    use semantic_db_core::{
        Batch, BatchOutcome, DbError, EntityRecord, PackageRegistrationOutcome, QueryResult,
        TextQueryInput,
    };

    use semantic_rpc_core::{RpcRequest, RpcResponse, RpcResult};
    use tower::ServiceExt;

    use super::*;

    struct MockDb {
        name: String,
        records: Mutex<BTreeMap<(String, String), Object>>,
    }

    #[async_trait]
    impl SemanticDb for MockDb {
        async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
            Err(DbError::InvalidQuery(
                "catalog not used in tests".to_string(),
            ))
        }

        async fn query(&self, _query: TextQueryInput) -> std::result::Result<QueryResult, DbError> {
            let mut row = Object::new();
            row.insert("db", Value::String(self.name.clone()));
            Ok(QueryResult::Select(vec![row]))
        }

        async fn get(
            &self,
            collection: String,
            id: String,
        ) -> std::result::Result<Option<EntityRecord>, DbError> {
            if let Some(object) = self
                .records
                .lock()
                .unwrap()
                .get(&(collection.clone(), id.clone()))
                .cloned()
            {
                return Ok(Some(EntityRecord {
                    collection,
                    id,
                    object,
                }));
            }
            Ok(Some(EntityRecord {
                collection,
                id,
                object: Object::new(),
            }))
        }

        async fn insert(
            &self,
            _collection: String,
            _id: String,
            _object: Object,
        ) -> std::result::Result<(), DbError> {
            self.records
                .lock()
                .unwrap()
                .insert((_collection, _id), _object);
            Ok(())
        }

        async fn delete(
            &self,
            _collection: String,
            _id: String,
        ) -> std::result::Result<(), DbError> {
            Ok(())
        }

        async fn execute_batch(&self, _batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
            Err(DbError::InvalidQuery("batch not used in tests".to_string()))
        }

        async fn upsert_package(
            &self,
            package: semantic_data::schema::Package,
        ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
            assert!(
                package.name == "semantic.base"
                    || package.name == semantic_data::filestore::PACKAGE_NAME
            );
            Ok(PackageRegistrationOutcome {
                executed_migrations: vec![],
            })
        }
    }

    fn mock_db(name: &str) -> Arc<dyn SemanticDb> {
        Arc::new(MockDb {
            name: name.to_string(),
            records: Mutex::new(BTreeMap::new()),
        })
    }

    fn test_app() -> semantic_app::SemanticApp {
        let default_db = mock_db("default");
        let header_db = mock_db("header");
        let query_db = mock_db("query");
        let data_dir = std::env::temp_dir().join("semantic-server-test-blob");
        let blob_uri = semantic_app::AppConfig::new()
            .with_data_dir(&data_dir)
            .default_blob_uri()
            .unwrap();
        let app = semantic_app::SemanticApp::builder()
            .with_default_scope(DbScopeId::new("default"), default_db)
            .with_default_file_store_uri(DbScopeId::new("default"), blob_uri)
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        app.scopes()
            .add_default_scope(DbScopeId::new("header"), header_db)
            .unwrap();
        app.scopes()
            .add_default_scope(DbScopeId::new("query"), query_db)
            .unwrap();
        app
    }

    fn value_object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        let mut object = Object::new();
        for (key, value) in fields {
            object.insert(key, value);
        }
        Value::Object(object)
    }

    async fn post_rpc(
        server: &SemanticServer,
        uri: &str,
        scope_header: Option<&str>,
        command: &str,
        payload: Value,
    ) -> RpcResponse {
        let body = serde_json::to_vec(&RpcRequest {
            id: 11,
            command: command.to_string(),
            payload,
        })
        .unwrap();
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(scope_header) = scope_header {
            builder = builder.header("x-semantic-scope", scope_header);
        }
        let response = server
            .router()
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert!(response.status().is_success());
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<RpcResponse>(&bytes).unwrap()
    }

    fn select_db_name(response: RpcResponse) -> String {
        let RpcResult::Ok(Value::Object(object)) = response.result else {
            panic!("expected ok object");
        };
        let Some(Value::List(rows)) = object.get("rows") else {
            panic!("expected rows");
        };
        let Some(Value::Object(row)) = rows.first() else {
            panic!("expected row");
        };
        let Some(Value::String(db)) = row.get("db") else {
            panic!("expected db string");
        };
        db.clone()
    }

    #[tokio::test]
    async fn http_rpc_uses_system_principal_by_default() {
        let server = SemanticServer::new(test_app());
        let response = post_rpc(
            &server,
            "/api/v1/rpc",
            None,
            "semantic.scope.current",
            Value::Void,
        )
        .await;
        let RpcResult::Ok(Value::Object(object)) = response.result else {
            panic!("expected ok object");
        };
        assert_eq!(object.get("scope_id"), Some(&Value::Null));
    }

    #[tokio::test]
    async fn http_scope_header_is_used() {
        let server = SemanticServer::new(test_app());
        let response = post_rpc(
            &server,
            "/api/v1/rpc",
            Some("header"),
            "semantic.db.query",
            value_object([("query", Value::String("select * from _".to_string()))]),
        )
        .await;
        assert_eq!(select_db_name(response), "header");
    }

    #[tokio::test]
    async fn query_param_scope_fallback_is_used() {
        let server = SemanticServer::new(test_app());
        let response = post_rpc(
            &server,
            "/api/v1/rpc?scope=query",
            None,
            "semantic.db.query",
            value_object([("query", Value::String("select * from _".to_string()))]),
        )
        .await;
        assert_eq!(select_db_name(response), "query");
    }

    #[tokio::test]
    async fn invalid_rpc_payload_returns_rpc_error() {
        let server = SemanticServer::new(test_app());
        let response = post_rpc(
            &server,
            "/api/v1/rpc",
            None,
            "semantic.db.query",
            Value::Void,
        )
        .await;
        let RpcResult::Err(err) = response.result else {
            panic!("expected rpc error");
        };
        assert_eq!(err.code, "invalid_request");
    }

    #[tokio::test]
    async fn file_upload_and_download_round_trip() {
        let server = SemanticServer::new(test_app());
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .header("content-type", "text/plain")
                    .header("x-semantic-filename", "hello.txt")
                    .header(
                        "x-semantic-file-entity",
                        r#"{"semantic:title":"Hello","filename":"wrong.txt","filestore_locator":"wrong"}"#,
                    )
                    .body(Body::from("hello"))
                    .unwrap(),
            )
            .await
            .unwrap();
        if response.status() != http::StatusCode::CREATED {
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            panic!(
                "expected 201, got {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let Value::Object(upload) = serde_json::from_slice::<Value>(&bytes).unwrap() else {
            panic!("expected upload object");
        };
        let Some(Value::String(id)) = upload.get("id") else {
            panic!("expected file id");
        };
        let Some(Value::Object(object)) = upload.get("object") else {
            panic!("expected file object");
        };
        assert_eq!(
            object.get("filestore_locator").and_then(Value::as_str),
            Some(id.as_str())
        );
        assert_eq!(
            object.get("semantic:title").and_then(Value::as_str),
            Some("Hello")
        );
        assert_eq!(
            object.get("filename").and_then(Value::as_str),
            Some("hello.txt")
        );
        assert_eq!(
            object.get("mime_type").and_then(Value::as_str),
            Some("text/plain")
        );
        assert_eq!(object.get("filekind").and_then(Value::as_str), Some("text"));

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            response.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "text/plain"
        );
        assert_eq!(
            response.headers().get(http::header::ACCEPT_RANGES).unwrap(),
            "bytes"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"hello");
    }

    #[tokio::test]
    async fn file_upload_streams_bodies_larger_than_axums_default_limit() {
        const CHUNK_SIZE: usize = 64 * 1024;
        const CHUNK_COUNT: usize = 48;
        const TOTAL_SIZE: usize = CHUNK_SIZE * CHUNK_COUNT;

        let chunks = (0..CHUNK_COUNT)
            .map(|_| Ok::<_, std::convert::Infallible>(bytes::Bytes::from(vec![0x5a; CHUNK_SIZE])));
        let server = SemanticServer::new(test_app());
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .header("content-length", TOTAL_SIZE)
                    .body(Body::from_stream(futures_util::stream::iter(chunks)))
                    .unwrap(),
            )
            .await
            .unwrap();
        if response.status() != http::StatusCode::CREATED {
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            panic!(
                "expected 201, got {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let Value::Object(upload) = serde_json::from_slice::<Value>(&bytes).unwrap() else {
            panic!("expected upload object");
        };
        let Some(Value::Object(object)) = upload.get("object") else {
            panic!("expected file object");
        };
        assert_eq!(
            object.get("byte_size"),
            Some(&Value::U64(TOTAL_SIZE as u64))
        );
    }

    #[tokio::test]
    async fn file_upload_enforces_configured_streaming_limit() {
        let mut config = ServerConfig::default();
        config.max_file_upload_size = 4;
        let server = SemanticServer::new(test_app()).with_config(config);
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .header("content-length", 5)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PAYLOAD_TOO_LARGE);

        let chunks = [
            Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"1234")),
            Ok(bytes::Bytes::from_static(b"5")),
        ];
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .body(Body::from_stream(futures_util::stream::iter(chunks)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn file_download_supports_byte_ranges() {
        let server = SemanticServer::new(test_app());
        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/file")
                    .header("x-semantic-scope", "default")
                    .body(Body::from("abcdef"))
                    .unwrap(),
            )
            .await
            .unwrap();
        if response.status() != http::StatusCode::CREATED {
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            panic!(
                "expected 201, got {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let Value::Object(upload) = serde_json::from_slice::<Value>(&bytes).unwrap() else {
            panic!("expected upload object");
        };
        let Some(Value::String(id)) = upload.get("id") else {
            panic!("expected file id");
        };

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .header("range", "bytes=1-3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response.headers().get(http::header::CONTENT_RANGE).unwrap(),
            "bytes 1-3/6"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"bcd");

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .header("range", "bytes=3-")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response.headers().get(http::header::CONTENT_RANGE).unwrap(),
            "bytes 3-5/6"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"def");

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .header("range", "bytes=-2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::PARTIAL_CONTENT);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"ef");

        let response = server
            .router()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/file/{id}"))
                    .header("x-semantic-scope", "default")
                    .header("range", "bytes=99-100")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::RANGE_NOT_SATISFIABLE);
    }
}
