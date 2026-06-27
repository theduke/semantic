mod auth;
mod config;
mod error;
mod redb;
mod router;
mod ws;

pub use auth::{HeaderPrincipalResolver, NoAuthPrincipalResolver, PrincipalResolver};
pub use config::ServerConfig;
pub use error::ServerError;
#[cfg(feature = "redb")]
pub use redb::RedbDbProvider;
pub use router::SemanticServer;

#[cfg(feature = "redb")]
impl SemanticServer {
    pub fn local_redb(path: impl AsRef<std::path::Path>) -> std::result::Result<Self, ServerError> {
        let app_config = semantic_app::AppConfig::from_env();
        let blob_uri = app_config
            .default_blob_uri()
            .map_err(|err| ServerError::App(semantic_app::AppError::InvalidRequest(err)))?;
        Self::local_redb_with_blob_store(path, blob_uri)
    }

    pub fn local_redb_with_blob_store(
        path: impl AsRef<std::path::Path>,
        blob_uri: String,
    ) -> std::result::Result<Self, ServerError> {
        let path = path.as_ref();
        let backend =
            semantic_db_redb::open_backend(path, semantic_data::schema::DbOpenMode::AutoCreate)?;
        let db: std::sync::Arc<dyn semantic_app::SemanticDb> =
            std::sync::Arc::new(semantic_db_core::Db::new(backend));
        let scope_id = semantic_app::DbScopeId::new("default");
        let app = semantic_app::SemanticApp::builder()
            .with_provider(RedbDbProvider)
            .with_default_scope(scope_id.clone(), db)
            .with_default_object_store_request(
                scope_id,
                semantic_app::ObjectStoreId::new("default"),
                semantic_app::ObjectStoreOpenRequest { uri: blob_uri },
            )
            .register_builtin_commands()?
            .build()?;
        Ok(Self::new(app))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
    use semantic_rpc::{RpcRequest, RpcResponse, RpcResult};
    use tower::ServiceExt;

    use super::*;

    struct MockDb {
        name: String,
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
            assert_eq!(package.name, "semantic.base");
            Ok(PackageRegistrationOutcome {
                executed_migrations: vec![],
            })
        }
    }

    fn test_app() -> semantic_app::SemanticApp {
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb {
            name: "default".to_string(),
        });
        let header_db: Arc<dyn SemanticDb> = Arc::new(MockDb {
            name: "header".to_string(),
        });
        let query_db: Arc<dyn SemanticDb> = Arc::new(MockDb {
            name: "query".to_string(),
        });
        let app = semantic_app::SemanticApp::builder()
            .with_default_scope(DbScopeId::new("default"), default_db)
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
        let response = post_rpc(&server, "/rpc", None, "semantic.scope.current", Value::Void).await;
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
            "/rpc",
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
            "/rpc?scope=query",
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
        let response = post_rpc(&server, "/rpc", None, "semantic.db.query", Value::Void).await;
        let RpcResult::Err(err) = response.result else {
            panic!("expected rpc error");
        };
        assert_eq!(err.code, "invalid_request");
    }
}
