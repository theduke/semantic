mod auth;
mod command;
mod config;
mod context;
mod db;
mod error;
mod file;
mod media;
mod object_store;
mod scope;
mod session;

pub use auth::{Principal, PrincipalId, PrincipalKind};
pub use command::{SemanticApp, SemanticAppBuilder, SemanticAppInner};
pub use config::AppConfig;
pub use context::AppRequestContext;
pub use db::{DbOpenRequest, DbProvider, SemanticDb};
pub use error::AppError;
pub use file::{
    FileByteStream, FileContent, FileCreateRequest, FileReadResult, FileRecord, FileService,
    FileSizedStream,
};
pub use media::{MediaAnalysisConfig, MediaAnalysisOutcome, MediaAnalysisService};
pub use scope::{DbScopeId, ScopeInfo, ScopeManager, ScopeOpenOptions, ScopeVisibility};
pub use session::{AppSession, AppSessionId};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use std::time::{SystemTime, UNIX_EPOCH};

    use async_trait::async_trait;
    use semantic_data::value::{Object, Value};
    use semantic_db_core::catalog::{Catalog, CatalogStorageSnapshot};
    use semantic_db_core::{
        Batch, BatchOperation, BatchOutcome, BatchStats, DbError, EntityRecord,
        PackageRegistrationOutcome, QueryResult, TextQueryInput,
    };
    use semantic_rpc::{RpcRequest, RpcResponse, RpcResult};

    use super::*;

    struct MockDb {
        name: String,
        query_count: AtomicUsize,
        batch_count: AtomicUsize,
        records: Mutex<BTreeMap<(String, String), Object>>,
        #[cfg(feature = "base")]
        package_count: Arc<AtomicUsize>,
    }

    impl MockDb {
        fn new(name: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                query_count: AtomicUsize::new(0),
                batch_count: AtomicUsize::new(0),
                records: Mutex::new(BTreeMap::new()),
                #[cfg(feature = "base")]
                package_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        #[cfg(feature = "base")]
        fn with_package_count(name: impl Into<String>, package_count: Arc<AtomicUsize>) -> Self {
            Self {
                name: name.into(),
                query_count: AtomicUsize::new(0),
                batch_count: AtomicUsize::new(0),
                records: Mutex::new(BTreeMap::new()),
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

        async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
            self.batch_count.fetch_add(1, Ordering::Relaxed);
            let mut stats = BatchStats {
                upserted: 0,
                deleted: 0,
                updated: 0,
            };
            let mut records = self.records.lock().unwrap();
            for operation in batch.operations {
                match operation {
                    BatchOperation::Upsert {
                        collection,
                        id,
                        object,
                    } => {
                        records.insert((collection, id), object);
                        stats.upserted += 1;
                    }
                    BatchOperation::DeleteById { collection, id } => {
                        records.remove(&(collection, id));
                        stats.deleted += 1;
                    }
                    BatchOperation::DeleteByIds { ids, .. } => stats.deleted += ids.len(),
                    BatchOperation::Update { .. } => stats.updated += 1,
                    BatchOperation::Delete { .. } => stats.deleted += 1,
                }
            }
            Ok(BatchOutcome {
                dataset: Default::default(),
                stats,
            })
        }

        #[cfg(feature = "base")]
        async fn upsert_package(
            &self,
            package: semantic_data::schema::Package,
        ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
            assert!(
                package.name == semantic_base::PACKAGE_NAME
                    || package.name == semantic_data::filestore::PACKAGE_NAME
            );
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

    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5,
        0x1c, 0x0c, 0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xfa,
        0xcf, 0x00, 0x00, 0x02, 0x07, 0x01, 0x02, 0x9a, 0x1c, 0x31, 0x71, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

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
        assert_eq!(package_count.load(Ordering::Relaxed), 2);
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
        assert_eq!(package_count.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn default_file_store_resolves_for_default_scope() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!("semantic-app-blob-{suffix}"));
        let config = AppConfig::new().with_data_dir(&data_dir);
        let scope_id = DbScopeId::new("default");
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::new("default"));
        let app = SemanticApp::builder()
            .with_default_scope(scope_id.clone(), default_db)
            .with_default_file_store_uri(scope_id.clone(), config.default_blob_uri().unwrap())
            .build()
            .unwrap();

        let store = ctx(&app, Principal::system())
            .default_file_store(None)
            .await
            .unwrap();

        assert_eq!(store.kind(), "objstore.fs");
        assert!(config.default_blob_path().is_dir());
    }

    #[tokio::test]
    async fn file_service_creates_and_reads_file() {
        use bytes::Bytes;
        use futures_util::TryStreamExt as _;

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!("semantic-app-file-{suffix}"));
        let config = AppConfig::new().with_data_dir(&data_dir);
        let scope_id = DbScopeId::new("default");
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::new("default"));
        let app = SemanticApp::builder()
            .with_default_scope(scope_id.clone(), default_db)
            .with_default_file_store_uri(scope_id, config.default_blob_uri().unwrap())
            .build()
            .unwrap();
        let ctx = ctx(&app, Principal::system());

        let mut entity = Object::new();
        entity.insert("semantic:title", Value::String("Hello".to_string()));
        entity.insert("type", Value::String("wrong".to_string()));
        entity.insert("filestore_locator", Value::String("wrong".to_string()));

        let record = app
            .files()
            .create(
                &ctx,
                FileCreateRequest {
                    scope_id: None,
                    id: None,
                    filestore_locator: Some("uploads/hello.txt".to_string()),
                    filename: Some("hello.txt".to_string()),
                    mime_type: Some("text/plain".to_string()),
                    entity,
                    content: FileContent::Bytes(Bytes::from_static(b"hello")),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            record.id,
            "file-sha256-2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(
            record.object.get("type").and_then(Value::as_str),
            Some(semantic_data::filestore::FILE_CLASS_ID)
        );
        assert_eq!(
            record
                .object
                .get("filestore_locator")
                .and_then(Value::as_str),
            Some("uploads/hello.txt")
        );
        assert_eq!(record.object.get("byte_size"), Some(&Value::U64(5)));
        assert_eq!(
            record.object.get("semantic:title").and_then(Value::as_str),
            Some("Hello")
        );

        let read = app.files().read(&ctx, None, record.id).await.unwrap();
        let bytes = read.stream.try_collect::<bytes::BytesMut>().await.unwrap();
        assert_eq!(&bytes[..], b"hello");
    }

    #[tokio::test]
    async fn file_service_auto_analyzes_image_when_enabled() {
        use bytes::Bytes;

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!("semantic-app-image-auto-{suffix}"));
        let config = AppConfig::new()
            .with_data_dir(&data_dir)
            .with_auto_analyze_media(true);
        let scope_id = DbScopeId::new("default");
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::new("default"));
        let app = SemanticApp::builder()
            .with_config(config.clone())
            .with_default_scope(scope_id.clone(), default_db)
            .with_default_file_store_uri(scope_id, config.default_blob_uri().unwrap())
            .build()
            .unwrap();
        let ctx = ctx(&app, Principal::system());

        let record = app
            .files()
            .create(
                &ctx,
                FileCreateRequest {
                    scope_id: None,
                    id: Some("image-1".to_string()),
                    filestore_locator: None,
                    filename: Some("image.png".to_string()),
                    mime_type: Some("image/png".to_string()),
                    entity: Object::new(),
                    content: FileContent::Bytes(Bytes::from_static(PNG_1X1)),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            record
                .object
                .get(semantic_data::filestore::ATTR_FILE_MEDIA_PIXEL_WIDTH),
            Some(&Value::U64(1))
        );
        assert_eq!(
            record
                .object
                .get(semantic_data::filestore::ATTR_FILE_MEDIA_PIXEL_HEIGHT),
            Some(&Value::U64(1))
        );
    }

    #[tokio::test]
    async fn file_service_auto_analysis_failure_keeps_file() {
        use bytes::Bytes;

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!("semantic-app-video-auto-{suffix}"));
        let config = AppConfig::new()
            .with_data_dir(&data_dir)
            .with_auto_analyze_media(true);
        let scope_id = DbScopeId::new("default");
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::new("default"));
        let app = SemanticApp::builder()
            .with_config(config.clone())
            .with_default_scope(scope_id.clone(), default_db)
            .with_default_file_store_uri(scope_id, config.default_blob_uri().unwrap())
            .build()
            .unwrap();
        let ctx = ctx(&app, Principal::system());

        let record = app
            .files()
            .create(
                &ctx,
                FileCreateRequest {
                    scope_id: None,
                    id: Some("video-1".to_string()),
                    filestore_locator: None,
                    filename: Some("video.mp4".to_string()),
                    mime_type: Some("video/mp4".to_string()),
                    entity: Object::new(),
                    content: FileContent::Bytes(Bytes::from_static(b"not video")),
                },
            )
            .await
            .unwrap();

        assert_eq!(record.id, "video-1");
        assert!(
            !record
                .object
                .contains_key(semantic_data::filestore::ATTR_FILE_MEDIA_DURATION)
        );
    }

    #[tokio::test]
    async fn file_analyze_command_updates_existing_image() {
        use bytes::Bytes;

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!("semantic-app-image-command-{suffix}"));
        let config = AppConfig::new().with_data_dir(&data_dir);
        let scope_id = DbScopeId::new("default");
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::new("default"));
        let app = SemanticApp::builder()
            .with_config(config.clone())
            .with_default_scope(scope_id.clone(), default_db)
            .with_default_file_store_uri(scope_id, config.default_blob_uri().unwrap())
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        let ctx = ctx(&app, Principal::system());

        app.files()
            .create(
                &ctx,
                FileCreateRequest {
                    scope_id: None,
                    id: Some("image-1".to_string()),
                    filestore_locator: None,
                    filename: Some("image.png".to_string()),
                    mime_type: Some("image/png".to_string()),
                    entity: Object::new(),
                    content: FileContent::Bytes(Bytes::from_static(PNG_1X1)),
                },
            )
            .await
            .unwrap();

        let response = app
            .invoke(
                ctx.clone(),
                request(
                    "semantic.file.analyze",
                    value_object([("id", Value::String("image-1".to_string()))]),
                ),
            )
            .await;

        let RpcResult::Ok(Value::Object(out)) = response.result else {
            panic!("expected ok object");
        };
        assert_eq!(out.get("analyzed"), Some(&Value::Bool(true)));
        assert_eq!(
            out.get("analysis_kind").and_then(Value::as_str),
            Some("image")
        );
        let Some(Value::Object(attributes)) = out.get("attributes") else {
            panic!("expected attributes object");
        };
        assert_eq!(
            attributes.get(semantic_data::filestore::ATTR_FILE_MEDIA_PIXEL_WIDTH),
            Some(&Value::U64(1))
        );

        let read = app
            .files()
            .read(&ctx, None, "image-1".to_string())
            .await
            .unwrap();
        assert_eq!(
            read.record
                .object
                .get(semantic_data::filestore::ATTR_FILE_MEDIA_PIXEL_WIDTH),
            Some(&Value::U64(1))
        );
    }

    #[tokio::test]
    async fn file_analyze_command_returns_false_for_non_media() {
        use bytes::Bytes;

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!("semantic-app-text-command-{suffix}"));
        let config = AppConfig::new().with_data_dir(&data_dir);
        let scope_id = DbScopeId::new("default");
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::new("default"));
        let app = SemanticApp::builder()
            .with_config(config.clone())
            .with_default_scope(scope_id.clone(), default_db)
            .with_default_file_store_uri(scope_id, config.default_blob_uri().unwrap())
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        let ctx = ctx(&app, Principal::system());

        app.files()
            .create(
                &ctx,
                FileCreateRequest {
                    scope_id: None,
                    id: Some("text-1".to_string()),
                    filestore_locator: None,
                    filename: Some("hello.txt".to_string()),
                    mime_type: Some("text/plain".to_string()),
                    entity: Object::new(),
                    content: FileContent::Bytes(Bytes::from_static(b"hello")),
                },
            )
            .await
            .unwrap();

        let response = app
            .invoke(
                ctx,
                request(
                    "semantic.file.analyze",
                    value_object([("id", Value::String("text-1".to_string()))]),
                ),
            )
            .await;

        let RpcResult::Ok(Value::Object(out)) = response.result else {
            panic!("expected ok object");
        };
        assert_eq!(out.get("analyzed"), Some(&Value::Bool(false)));
        assert!(
            matches!(out.get("attributes"), Some(Value::Object(attributes)) if attributes.is_empty())
        );
    }

    #[tokio::test]
    async fn file_analyze_command_surfaces_missing_ffprobe_temp_dir() {
        use bytes::Bytes;

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!("semantic-app-video-command-{suffix}"));
        let config = AppConfig::new().with_data_dir(&data_dir);
        let scope_id = DbScopeId::new("default");
        let default_db: Arc<dyn SemanticDb> = Arc::new(MockDb::new("default"));
        let app = SemanticApp::builder()
            .with_config(config.clone())
            .with_default_scope(scope_id.clone(), default_db)
            .with_default_file_store_uri(scope_id, config.default_blob_uri().unwrap())
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        let ctx = ctx(&app, Principal::system());

        app.files()
            .create(
                &ctx,
                FileCreateRequest {
                    scope_id: None,
                    id: Some("video-1".to_string()),
                    filestore_locator: None,
                    filename: Some("video.mp4".to_string()),
                    mime_type: Some("video/mp4".to_string()),
                    entity: Object::new(),
                    content: FileContent::Bytes(Bytes::from_static(b"not video")),
                },
            )
            .await
            .unwrap();

        let response = app
            .invoke(
                ctx,
                request(
                    "semantic.file.analyze",
                    value_object([("id", Value::String("video-1".to_string()))]),
                ),
            )
            .await;

        let RpcResult::Err(error) = response.result else {
            panic!("expected media analysis error");
        };
        assert_eq!(error.code, "media_analysis_failed");
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

    #[tokio::test]
    async fn batch_command_executes_upsert_batch() {
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
                    "semantic.db.batch",
                    value_object([(
                        "operations",
                        Value::List(vec![value_object([
                            ("kind", Value::String("upsert".to_string())),
                            ("collection", Value::String("entities".to_string())),
                            ("id", Value::String("entity-1".to_string())),
                            (
                                "object",
                                value_object([("name", Value::String("Ada".to_string()))]),
                            ),
                        ])]),
                    )]),
                ),
            )
            .await;

        let RpcResult::Ok(Value::Object(object)) = response.result else {
            panic!("expected ok object");
        };
        let Some(Value::Object(stats)) = object.get("stats") else {
            panic!("expected stats");
        };
        assert_eq!(stats.get("upserted"), Some(&Value::U64(1)));
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
