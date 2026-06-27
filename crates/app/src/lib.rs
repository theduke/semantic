mod auth;
mod command;
mod config;
mod context;
mod db;
mod error;
mod scope;
mod session;

pub use auth::{Principal, PrincipalId, PrincipalKind};
pub use command::{SemanticApp, SemanticAppBuilder, SemanticAppInner};
pub use config::AppConfig;
pub use context::AppRequestContext;
pub use db::{DbOpenRequest, DbProvider, SemanticDb};
pub use error::AppError;
pub use scope::{DbScopeId, ScopeInfo, ScopeManager, ScopeOpenOptions, ScopeVisibility};
pub use session::{AppSession, AppSessionId};

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use semantic_data::value::{Object, Value};
    use semantic_db_core::catalog::{Catalog, CatalogStorageSnapshot};
    use semantic_db_core::{
        Batch, BatchOutcome, DbError, EntityRecord, PackageRegistrationOutcome, QueryResult,
        TextQueryInput,
    };
    use semantic_rpc::{RpcRequest, RpcResponse, RpcResult};

    use super::*;

    struct MockDb {
        name: String,
        query_count: AtomicUsize,
        #[cfg(feature = "base")]
        package_count: Arc<AtomicUsize>,
    }

    impl MockDb {
        fn new(name: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                query_count: AtomicUsize::new(0),
                #[cfg(feature = "base")]
                package_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        #[cfg(feature = "base")]
        fn with_package_count(name: impl Into<String>, package_count: Arc<AtomicUsize>) -> Self {
            Self {
                name: name.into(),
                query_count: AtomicUsize::new(0),
                package_count,
            }
        }
    }

    #[async_trait]
    impl SemanticDb for MockDb {
        async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
            Ok(Arc::new(Catalog::new()))
        }

        async fn query(&self, _query: TextQueryInput) -> std::result::Result<QueryResult, DbError> {
            self.query_count.fetch_add(1, Ordering::Relaxed);
            let mut row = Object::new();
            row.insert("db", Value::String(self.name.clone()));
            Ok(QueryResult::Select(vec![row]))
        }

        async fn get(
            &self,
            collection: String,
            id: String,
        ) -> std::result::Result<Option<EntityRecord>, DbError> {
            let mut object = Object::new();
            object.insert("db", Value::String(self.name.clone()));
            Ok(Some(EntityRecord {
                collection,
                id,
                object,
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

        #[cfg(feature = "base")]
        async fn upsert_package(
            &self,
            package: semantic_data::schema::Package,
        ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
            assert_eq!(package.name, semantic_base::PACKAGE_NAME);
            self.package_count.fetch_add(1, Ordering::Relaxed);
            Ok(PackageRegistrationOutcome {
                executed_migrations: vec![],
            })
        }
    }

    struct MockProvider {
        opened: Arc<Mutex<Vec<String>>>,
        #[cfg(feature = "base")]
        package_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl DbProvider for MockProvider {
        fn scheme(&self) -> &str {
            "mock"
        }

        async fn open(
            &self,
            request: DbOpenRequest,
            _principal: &Principal,
        ) -> std::result::Result<Arc<dyn SemanticDb>, AppError> {
            self.opened.lock().unwrap().push(request.uri.clone());
            #[cfg(feature = "base")]
            {
                Ok(Arc::new(MockDb::with_package_count(
                    request.uri,
                    Arc::clone(&self.package_count),
                )))
            }
            #[cfg(not(feature = "base"))]
            {
                Ok(Arc::new(MockDb::new(request.uri)))
            }
        }
    }

    fn mock_provider(opened: Arc<Mutex<Vec<String>>>) -> MockProvider {
        MockProvider {
            opened,
            #[cfg(feature = "base")]
            package_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[cfg(feature = "base")]
    fn mock_provider_with_package_count(
        opened: Arc<Mutex<Vec<String>>>,
        package_count: Arc<AtomicUsize>,
    ) -> MockProvider {
        MockProvider {
            opened,
            package_count,
        }
    }

    fn value_object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        let mut object = Object::new();
        for (key, value) in fields {
            object.insert(key, value);
        }
        Value::Object(object)
    }

    fn request(command: &str, payload: Value) -> RpcRequest {
        RpcRequest {
            id: 1,
            command: command.to_string(),
            payload,
        }
    }

    fn ctx(app: &SemanticApp, principal: Principal) -> AppRequestContext {
        AppRequestContext {
            app: app.clone(),
            principal,
            session: None,
            request_scope: None,
        }
    }

    fn select_db_name(response: RpcResponse) -> String {
        let RpcResult::Ok(Value::Object(object)) = response.result else {
            panic!("expected ok object");
        };
        let Some(Value::List(rows)) = object.get("rows") else {
            panic!("expected rows");
        };
        let Some(Value::Object(row)) = rows.first() else {
            panic!("expected first row");
        };
        let Some(Value::String(db)) = row.get("db") else {
            panic!("expected db string");
        };
        db.clone()
    }

    #[tokio::test]
    async fn no_auth_default_scope_resolves() {
        #[cfg(feature = "base")]
        let package_count = Arc::new(AtomicUsize::new(0));
        #[cfg(feature = "base")]
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::with_package_count(
            "default",
            Arc::clone(&package_count),
        ));
        #[cfg(not(feature = "base"))]
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::new("default"));
        let app = SemanticApp::builder()
            .with_default_scope(DbScopeId::new("default"), default_db)
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();

        let response = app
            .invoke(
                ctx(&app, Principal::system()),
                request(
                    "semantic.db.query",
                    value_object([("query", Value::String("select * from _".to_string()))]),
                ),
            )
            .await;

        assert_eq!(select_db_name(response), "default");
        #[cfg(feature = "base")]
        assert_eq!(package_count.load(Ordering::Relaxed), 1);
    }

    #[cfg(feature = "base")]
    #[tokio::test]
    async fn opened_scope_does_not_register_base_package() {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let package_count = Arc::new(AtomicUsize::new(0));
        let app = SemanticApp::builder()
            .with_provider(mock_provider_with_package_count(
                opened,
                Arc::clone(&package_count),
            ))
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();

        let response = app
            .invoke(
                ctx(&app, Principal::system()),
                request(
                    "semantic.scope.open",
                    value_object([
                        ("uri", Value::String("mock://opened".to_string())),
                        ("scope_id", Value::String("opened".to_string())),
                    ]),
                ),
            )
            .await;
        assert!(matches!(response.result, RpcResult::Ok(_)));

        let response = app
            .invoke(
                ctx(&app, Principal::system()),
                request(
                    "semantic.db.query",
                    value_object([
                        ("scope_id", Value::String("opened".to_string())),
                        ("query", Value::String("select * from _".to_string())),
                    ]),
                ),
            )
            .await;
        assert_eq!(select_db_name(response), "mock://opened");
        assert_eq!(package_count.load(Ordering::Relaxed), 0);
    }

    #[cfg(feature = "base")]
    #[tokio::test]
    async fn default_scope_request_registers_base_package() {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let package_count = Arc::new(AtomicUsize::new(0));
        let app = SemanticApp::builder()
            .with_provider(mock_provider_with_package_count(
                opened,
                Arc::clone(&package_count),
            ))
            .with_default_scope_request(
                DbScopeId::new("default"),
                DbOpenRequest {
                    uri: "mock://default".to_string(),
                    mode: semantic_data::schema::DbOpenMode::AutoCreate,
                },
            )
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();

        let response = app
            .invoke(
                ctx(&app, Principal::system()),
                request(
                    "semantic.db.query",
                    value_object([("query", Value::String("select * from _".to_string()))]),
                ),
            )
            .await;

        assert_eq!(select_db_name(response), "mock://default");
        assert_eq!(package_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn principal_scopes_are_isolated() {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let app = SemanticApp::builder()
            .with_provider(mock_provider(Arc::clone(&opened)))
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();

        for (principal, uri) in [
            (Principal::user("user_a"), "mock://a"),
            (Principal::user("user_b"), "mock://b"),
        ] {
            let response = app
                .invoke(
                    ctx(&app, principal),
                    request(
                        "semantic.scope.open",
                        value_object([
                            ("uri", Value::String(uri.to_string())),
                            ("scope_id", Value::String("shared".to_string())),
                        ]),
                    ),
                )
                .await;
            assert!(matches!(response.result, RpcResult::Ok(_)));
        }

        for (principal, expected) in [
            (Principal::user("user_a"), "mock://a"),
            (Principal::user("user_b"), "mock://b"),
        ] {
            let response = app
                .invoke(
                    ctx(&app, principal),
                    request(
                        "semantic.db.query",
                        value_object([
                            ("scope_id", Value::String("shared".to_string())),
                            ("query", Value::String("select * from _".to_string())),
                        ]),
                    ),
                )
                .await;
            assert_eq!(select_db_name(response), expected);
        }
    }

    #[tokio::test]
    async fn scope_open_sets_session_current() {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let app = SemanticApp::builder()
            .with_provider(mock_provider(opened))
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        let session = app.new_session("s1");
        let ctx = AppRequestContext {
            app: app.clone(),
            principal: Principal::system(),
            session: Some(Arc::clone(&session)),
            request_scope: None,
        };

        let response = app
            .invoke(
                ctx.clone(),
                request(
                    "semantic.scope.open",
                    value_object([
                        ("uri", Value::String("mock://current".to_string())),
                        ("scope_id", Value::String("current".to_string())),
                    ]),
                ),
            )
            .await;
        assert!(matches!(response.result, RpcResult::Ok(_)));

        let response = app
            .invoke(ctx, request("semantic.scope.current", Value::Void))
            .await;
        let RpcResult::Ok(Value::Object(object)) = response.result else {
            panic!("expected current response");
        };
        assert_eq!(
            object.get("scope_id"),
            Some(&Value::String("current".to_string()))
        );
    }

    #[tokio::test]
    async fn payload_scope_overrides_session_scope() {
        let db_a: Arc<dyn SemanticDb> = Arc::new(MockDb::new("a"));
        let db_b: Arc<dyn SemanticDb> = Arc::new(MockDb::new("b"));
        let app = SemanticApp::builder()
            .with_default_scope(DbScopeId::new("a"), db_a)
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        app.scopes()
            .add_default_scope(DbScopeId::new("b"), db_b)
            .unwrap();
        let session = app.new_session("s1");
        session.set_current_scope(Some(DbScopeId::new("a"))).await;
        let ctx = AppRequestContext {
            app: app.clone(),
            principal: Principal::system(),
            session: Some(session),
            request_scope: None,
        };

        let response = app
            .invoke(
                ctx,
                request(
                    "semantic.db.query",
                    value_object([
                        ("scope_id", Value::String("b".to_string())),
                        ("query", Value::String("select * from _".to_string())),
                    ]),
                ),
            )
            .await;

        assert_eq!(select_db_name(response), "b");
    }

    #[tokio::test]
    async fn retired_scope_reopens_from_descriptor() {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let app = SemanticApp::builder()
            .with_provider(mock_provider(Arc::clone(&opened)))
            .with_idle_ttl(Duration::ZERO)
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        let principal = Principal::system();
        let ctx = ctx(&app, principal.clone());
        let response = app
            .invoke(
                ctx.clone(),
                request(
                    "semantic.scope.open",
                    value_object([
                        ("uri", Value::String("mock://reopen".to_string())),
                        ("scope_id", Value::String("reopen".to_string())),
                    ]),
                ),
            )
            .await;
        assert!(matches!(response.result, RpcResult::Ok(_)));
        assert_eq!(opened.lock().unwrap().len(), 1);

        assert_eq!(app.scopes().retire_idle_scopes(Instant::now()), 1);
        let _ = app
            .scopes()
            .resolve_scope(&principal, Some(DbScopeId::new("reopen")), None)
            .await
            .unwrap();

        assert_eq!(opened.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn unknown_scope_returns_rpc_error() {
        let app = SemanticApp::builder()
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        let response = app
            .invoke(
                ctx(&app, Principal::system()),
                request(
                    "semantic.db.query",
                    value_object([
                        ("scope_id", Value::String("missing".to_string())),
                        ("query", Value::String("select * from _".to_string())),
                    ]),
                ),
            )
            .await;

        let RpcResult::Err(err) = response.result else {
            panic!("expected error");
        };
        assert_eq!(err.code, "unknown_scope");
    }

    #[tokio::test]
    async fn catalog_command_returns_facet_json_snapshot() {
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::new("default"));
        let app = SemanticApp::builder()
            .with_default_scope(DbScopeId::new("default"), default_db)
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();

        let response = app
            .invoke(
                ctx(&app, Principal::system()),
                request("semantic.db.catalog", Value::Void),
            )
            .await;

        let RpcResult::Ok(Value::Object(object)) = response.result else {
            panic!("expected ok object");
        };
        assert_eq!(
            object.get("format"),
            Some(&Value::String("facet-json".to_string()))
        );
        let Some(Value::String(catalog)) = object.get("catalog") else {
            panic!("expected catalog string");
        };
        let snapshot = facet_json::from_str::<CatalogStorageSnapshot>(catalog)
            .expect("catalog snapshot should decode");
        assert!(snapshot.attributes.is_empty());
    }

    #[test]
    fn duplicate_command_registration_fails() {
        let result = SemanticApp::builder()
            .register_builtin_commands()
            .unwrap()
            .register_builtin_commands();
        let Err(err) = result else {
            panic!("expected duplicate registration error");
        };
        assert!(matches!(err, AppError::RpcRegister(_)));
    }
}
