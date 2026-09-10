use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use semantic_data::schema::{DbOpenMode, FunctionType, Package};
use semantic_data::value::{Object, Value};
use semantic_db_core::{
    Batch, BatchOperation, BatchOutcome, BatchStats, DEFAULT_COLLECTION, DeleteResult,
    InsertResult, MutationStats, QueryResult, TextQueryFormat, TextQueryInput, UpdateResult,
};
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{RpcCommand, RpcCommandSpec, RpcRequest, RpcResponse, RuntimePackage};

use crate::object_store::{ObjectStoreId, ObjectStoreManager, ObjectStoreOpenRequest};
use crate::{
    AppConfig, AppError, AppRequestContext, AppSession, DbOpenRequest, DbProvider, DbScopeId,
    FileService, MediaAnalysisConfig, ScopeInfo, ScopeManager, ScopeOpenOptions, ScopeVisibility,
    SemanticDb,
};

const DEFAULT_FILE_STORE_ID: &str = "default";

#[derive(Clone)]
pub struct SemanticApp {
    inner: Arc<SemanticAppInner>,
}

pub struct SemanticAppInner {
    registry: Arc<RpcRegistry<AppRequestContext>>,
    scopes: ScopeManager,
    object_stores: ObjectStoreManager,
    file_service: FileService,
    jobs_registry: semantic_jobs::JobsRegistry,
    jobs_config: semantic_jobs::JobsConfig,
    plugins: semantic_plugin::PluginRegistry,
    pub(crate) import_job: semantic_jobs::RegisteredJob<semantic_import::ImportJobHandler>,
}

enum DefaultScope {
    Opened(DbScopeId, Arc<dyn SemanticDb>),
    Request(DbScopeId, DbOpenRequest),
}

enum DefaultObjectStore {
    Request(DbScopeId, ObjectStoreId, ObjectStoreOpenRequest),
    Opened(DbScopeId, ObjectStoreId, objstore::DynObjStore),
}

pub struct SemanticAppBuilder {
    providers: BTreeMap<String, Arc<dyn DbProvider>>,
    registry: RpcRegistry<AppRequestContext>,
    packages: Vec<Package>,
    default_scope: Option<DefaultScope>,
    default_object_store: Option<DefaultObjectStore>,
    idle_ttl: Duration,
    media_analysis_config: MediaAnalysisConfig,
    jobs_registry: semantic_jobs::JobsRegistry,
    jobs_config: semantic_jobs::JobsConfig,
    plugins: semantic_plugin::PluginRegistry,
}

impl SemanticApp {
    pub async fn plugins(
        &self,
        principal: &crate::Principal,
        scope_id: DbScopeId,
    ) -> Result<Arc<crate::plugins::AppScopePlugins>, AppError> {
        let app = self.clone();
        let principal = principal.clone();
        // The task owns startup and the scope lifecycle lock through publication.
        // Dropping an RPC caller cannot drop partially started providers or let a
        // second caller create another runtime for the same scope.
        tokio::spawn(async move {
            app.inner
                .scopes
                .resolve_plugins(
                    &principal,
                    scope_id,
                    app.inner.plugins.clone(),
                    app.inner.jobs_registry.clone(),
                    app.inner.jobs_config.clone(),
                )
                .await
        })
        .await
        .map_err(crate::plugins::error)?
    }
    pub(crate) fn import_registration(
        &self,
    ) -> &semantic_jobs::RegisteredJob<semantic_import::ImportJobHandler> {
        &self.inner.import_job
    }
    pub fn builder() -> SemanticAppBuilder {
        SemanticAppBuilder {
            providers: BTreeMap::new(),
            registry: RpcRegistry::new(),
            packages: Vec::new(),
            default_scope: None,
            default_object_store: None,
            idle_ttl: Duration::from_secs(15 * 60),
            media_analysis_config: MediaAnalysisConfig::default(),
            jobs_registry: semantic_jobs::JobsRegistry::default(),
            jobs_config: semantic_jobs::JobsConfig::default(),
            plugins: semantic_plugin::PluginRegistry::new().with_host_providers(),
        }
    }

    pub async fn invoke(&self, ctx: AppRequestContext, request: RpcRequest) -> RpcResponse {
        self.inner.registry.invoke(&ctx, request).await
    }

    pub fn scopes(&self) -> &ScopeManager {
        &self.inner.scopes
    }

    pub async fn jobs(
        &self,
        principal: &crate::Principal,
        scope_id: DbScopeId,
    ) -> Result<semantic_jobs::ScopeJobs, AppError> {
        self.inner
            .scopes
            .resolve_jobs(
                principal,
                scope_id,
                self.inner.jobs_registry.clone(),
                self.inner.jobs_config.clone(),
            )
            .await
    }

    pub async fn shutdown(&self) -> Result<(), AppError> {
        self.inner.scopes.shutdown_jobs().await
    }

    pub(crate) fn object_stores(&self) -> &ObjectStoreManager {
        &self.inner.object_stores
    }

    pub fn files(&self) -> &FileService {
        &self.inner.file_service
    }

    pub fn new_session(&self, id: impl Into<String>) -> Arc<AppSession> {
        Arc::new(AppSession::new(id))
    }
}

impl SemanticAppBuilder {
    pub fn register_plugin(
        mut self,
        plugin: impl semantic_plugin::Plugin,
    ) -> Result<Self, AppError> {
        self.plugins
            .register(plugin)
            .map_err(crate::plugins::error)?;
        Ok(self)
    }
    pub fn with_jobs(
        mut self,
        registry: semantic_jobs::JobsRegistry,
        config: semantic_jobs::JobsConfig,
    ) -> Self {
        self.jobs_registry = registry;
        self.jobs_config = config;
        self
    }
    pub fn with_provider(mut self, provider: impl DbProvider) -> Self {
        let scheme = provider.scheme().to_string();
        self.providers.insert(scheme, Arc::new(provider));
        self
    }

    pub fn with_default_scope(mut self, scope_id: DbScopeId, db: Arc<dyn SemanticDb>) -> Self {
        self.default_scope = Some(DefaultScope::Opened(scope_id, db));
        self
    }

    pub fn with_default_scope_request(
        mut self,
        scope_id: DbScopeId,
        request: DbOpenRequest,
    ) -> Self {
        self.default_scope = Some(DefaultScope::Request(scope_id, request));
        self
    }

    pub fn with_default_file_store_uri(mut self, scope_id: DbScopeId, uri: String) -> Self {
        self.default_object_store = Some(DefaultObjectStore::Request(
            scope_id,
            ObjectStoreId::new(DEFAULT_FILE_STORE_ID),
            ObjectStoreOpenRequest { uri },
        ));
        self
    }

    /// Attach an already-opened store, preserving shared ownership of its handle.
    pub fn with_default_file_store(
        mut self,
        scope_id: DbScopeId,
        store: objstore::DynObjStore,
    ) -> Self {
        self.default_object_store = Some(DefaultObjectStore::Opened(
            scope_id,
            ObjectStoreId::new(DEFAULT_FILE_STORE_ID),
            store,
        ));
        self
    }

    pub fn with_idle_ttl(mut self, ttl: Duration) -> Self {
        self.idle_ttl = ttl;
        self
    }

    pub fn with_config(mut self, config: AppConfig) -> Self {
        self.jobs_config = config.jobs.clone();
        self.media_analysis_config = MediaAnalysisConfig::from(&config);
        self
    }

    pub fn with_media_temp_dir(mut self, temp_dir: impl Into<std::path::PathBuf>) -> Self {
        self.media_analysis_config.temp_dir = Some(temp_dir.into());
        self
    }

    pub fn with_auto_analyze_media(mut self, enabled: bool) -> Self {
        self.media_analysis_config.auto_analyze_media = enabled;
        self
    }

    pub fn register_command<C>(mut self, command: C) -> std::result::Result<Self, AppError>
    where
        C: RpcCommand<AppRequestContext>,
        AppRequestContext: Sync,
    {
        self.registry.register(command)?;
        Ok(self)
    }

    /// Register a package's handlers and schema.
    ///
    /// Handlers are available application-wide. Schemas are applied lazily to the
    /// default database, alongside the built-in packages, before its first use.
    /// Explicitly opened scopes retain their existing schema initialization behavior.
    pub fn register_package(
        mut self,
        package: impl RuntimePackage<AppRequestContext>,
    ) -> Result<Self, AppError> {
        for command in package.commands() {
            self.registry.register_dyn(command)?;
        }
        self.packages.push(package.schema());
        Ok(self)
    }

    pub fn register_builtin_commands(mut self) -> std::result::Result<Self, AppError> {
        self.registry.register(ScopeOpenCommand)?;
        self.registry.register(ScopeUseCommand)?;
        self.registry.register(ScopeCurrentCommand)?;
        self.registry.register(ScopeListCommand)?;
        self.registry.register(DbCatalogCommand)?;
        self.registry.register(DbPackageUpsertCommand)?;
        self.registry.register(DbQueryCommand)?;
        self.registry.register(DbGetCommand)?;
        self.registry.register(DbInsertCommand)?;
        self.registry.register(DbDeleteCommand)?;
        self.registry.register(DbBatchCommand)?;
        self.registry.register(FileAnalyzeCommand)?;
        crate::jobs::register_commands(&mut self.registry)?;
        crate::import_commands::register(&mut self.registry)?;
        Ok(self)
    }

    pub fn build(mut self) -> std::result::Result<SemanticApp, AppError> {
        let import_package = semantic_data::import::package();
        let mut catalog = semantic_db_core::catalog::Catalog::new();
        catalog.upsert_package(import_package.clone());
        let exports = ["Source", "Fetcher", "Importer"]
            .into_iter()
            .map(|name| {
                let resolved = catalog
                    .resolve_interface(semantic_data::import::PACKAGE_NAME, "v1", None, name)
                    .map_err(crate::plugins::error)?;
                Ok(semantic_rpc_core::interface::ImplementationDescriptor {
                    export: name.to_lowercase(),
                    interface: semantic_rpc_core::interface::InterfaceRef {
                        package: semantic_data::import::PACKAGE_NAME.into(),
                        module: "v1".into(),
                        contract: None,
                        name: name.into(),
                    },
                    package_version: "1.0.0".into(),
                    fingerprint: resolved.fingerprint,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        self.plugins
            .register(semantic_import::GenericUrlPlugin::new(exports))
            .map_err(crate::plugins::error)?;
        let import_job = self
            .jobs_registry
            .register(semantic_import::ImportJobHandler::default())?;
        let packages = std::mem::take(&mut self.packages);
        #[cfg(feature = "base")]
        {
            self = self.register_package(semantic_base::BasePackage)?;
        }
        self.packages.push(semantic_data::filestore::package());
        self.packages.push(import_package);
        self.packages.extend(packages);
        let scopes = ScopeManager::with_packages(self.providers, self.idle_ttl, self.packages);
        let object_stores = ObjectStoreManager::new(Vec::new());
        if let Some(default_scope) = self.default_scope {
            match default_scope {
                DefaultScope::Opened(scope_id, db) => scopes.add_default_scope(scope_id, db)?,
                DefaultScope::Request(scope_id, request) => {
                    scopes.add_default_scope_request(scope_id, request)?;
                }
            }
        }
        if let Some(default_object_store) = self.default_object_store {
            match default_object_store {
                DefaultObjectStore::Request(scope_id, store_id, request) => {
                    object_stores.attach_store_request(scope_id, store_id, request, true)?;
                }
                DefaultObjectStore::Opened(scope_id, store_id, store) => {
                    object_stores.attach_store(scope_id, store_id, store, true)?;
                }
            }
        }
        Ok(SemanticApp {
            inner: Arc::new(SemanticAppInner {
                registry: Arc::new(self.registry),
                scopes,
                object_stores,
                file_service: FileService::new(self.media_analysis_config),
                jobs_registry: self.jobs_registry,
                jobs_config: self.jobs_config,
                plugins: self.plugins,
                import_job,
            }),
        })
    }
}

struct ScopeOpenCommand;
struct ScopeUseCommand;
struct ScopeCurrentCommand;
struct ScopeListCommand;
struct DbCatalogCommand;
struct DbPackageUpsertCommand;
struct DbQueryCommand;
struct DbGetCommand;
struct DbInsertCommand;
struct DbDeleteCommand;
struct DbBatchCommand;
struct FileAnalyzeCommand;

macro_rules! command_spec {
    ($ty:ty, $name:literal) => {
        impl RpcCommandSpec for $ty {
            type Payload = Value;
            type Output = Value;
            type Error = AppError;

            const NAME: &'static str = $name;

            fn signature(&self) -> FunctionType {
                FunctionType {
                    params: Vec::new(),
                    results: Vec::new(),
                    throws: None,
                    async_fn: true,
                }
            }
        }
    };
}

command_spec!(ScopeOpenCommand, "semantic.scope.open");
command_spec!(ScopeUseCommand, "semantic.scope.use");
command_spec!(ScopeCurrentCommand, "semantic.scope.current");
command_spec!(ScopeListCommand, "semantic.scope.list");
command_spec!(DbCatalogCommand, "semantic.db.catalog");
command_spec!(DbPackageUpsertCommand, "semantic.db.package.upsert");
command_spec!(DbQueryCommand, "semantic.db.query");
command_spec!(DbGetCommand, "semantic.db.get");
command_spec!(DbInsertCommand, "semantic.db.insert");
command_spec!(DbDeleteCommand, "semantic.db.delete");
command_spec!(DbBatchCommand, "semantic.db.batch");
command_spec!(FileAnalyzeCommand, "semantic.file.analyze");

impl RpcCommand<AppRequestContext> for ScopeOpenCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let object = expect_object(payload)?;
            let uri = required_string(&object, "uri")?;
            let scope_id = optional_string(&object, "scope_id")?.map(DbScopeId::new);
            let mode = match optional_string(&object, "mode")?.as_deref() {
                Some("open_existing") => DbOpenMode::OpenExisting,
                Some("auto_create") | None => DbOpenMode::AutoCreate,
                Some(other) => {
                    return Err(AppError::InvalidRequest(format!(
                        "unsupported open mode '{other}'"
                    )));
                }
            };
            let visibility = match optional_string(&object, "visibility")?.as_deref() {
                Some("system") => ScopeVisibility::System,
                Some("principal") | None => ScopeVisibility::Principal,
                Some(other) => {
                    return Err(AppError::InvalidRequest(format!(
                        "unsupported scope visibility '{other}'"
                    )));
                }
            };
            let set_current = optional_bool(&object, "set_current")?.unwrap_or(true);
            let info = ctx
                .app
                .scopes()
                .open_scope(
                    &ctx.principal,
                    ScopeOpenOptions {
                        scope_id,
                        request: DbOpenRequest {
                            uri: uri.clone(),
                            mode,
                        },
                        visibility,
                        set_current,
                    },
                )
                .await?;
            let mut current = false;
            if set_current {
                if ctx.session.is_some() {
                    ctx.set_session_scope(Some(info.scope_id.clone())).await?;
                    current = true;
                }
            }
            let mut out = scope_info_object(&info);
            out.insert("current", Value::Bool(current));
            Ok(Value::Object(out))
        })
    }
}

impl RpcCommand<AppRequestContext> for ScopeUseCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let scope_id = match payload {
                Value::Null | Value::Void => None,
                Value::String(value) => Some(DbScopeId::new(value)),
                Value::Object(object) => optional_string(&object, "scope_id")?.map(DbScopeId::new),
                _ => {
                    return Err(AppError::InvalidRequest(
                        "expected scope id string, null, or object".to_string(),
                    ));
                }
            };
            if let Some(scope_id) = &scope_id {
                let _ = ctx.resolve_db(Some(scope_id.clone())).await?;
            }
            ctx.set_session_scope(scope_id.clone()).await?;
            let mut out = Object::new();
            out.insert(
                "scope_id",
                scope_id
                    .map(|scope_id| Value::String(scope_id.to_string()))
                    .unwrap_or(Value::Null),
            );
            Ok(Value::Object(out))
        })
    }
}

impl RpcCommand<AppRequestContext> for ScopeCurrentCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        _payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let scope_id = ctx.effective_scope_hint().await;
            let mut out = Object::new();
            out.insert(
                "scope_id",
                scope_id
                    .map(|scope_id| Value::String(scope_id.to_string()))
                    .unwrap_or(Value::Null),
            );
            Ok(Value::Object(out))
        })
    }
}

impl RpcCommand<AppRequestContext> for ScopeListCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        _payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            Ok(Value::List(
                ctx.app
                    .scopes()
                    .list_scopes(&ctx.principal)
                    .into_iter()
                    .map(|info| Value::Object(scope_info_object(&info)))
                    .collect(),
            ))
        })
    }
}

impl RpcCommand<AppRequestContext> for DbCatalogCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let scope_id = match payload {
                Value::Null | Value::Void => None,
                Value::Object(object) => optional_string(&object, "scope_id")?.map(DbScopeId::new),
                _ => {
                    return Err(AppError::InvalidRequest(
                        "expected object, null, or void payload".to_string(),
                    ));
                }
            };
            let db = ctx.resolve_db(scope_id).await?;
            let catalog = db.catalog().await?;
            let catalog = facet_json::to_string(&catalog.to_storage_snapshot())
                .map_err(|err| AppError::InvalidRequest(err.to_string()))?;
            let mut out = Object::new();
            out.insert("format", Value::String("facet-json".to_string()));
            out.insert("catalog", Value::String(catalog));
            Ok(Value::Object(out))
        })
    }
}

impl RpcCommand<AppRequestContext> for DbPackageUpsertCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let object = expect_object(payload)?;
            let scope_id = optional_string(&object, "scope_id")?.map(DbScopeId::new);
            let format = optional_string(&object, "format")?;
            if !matches!(format.as_deref(), None | Some("facet-json")) {
                return Err(AppError::InvalidRequest(format!(
                    "unsupported package format '{}'",
                    format.expect("checked above")
                )));
            }
            let package = facet_json::from_str::<Package>(&required_string(&object, "package")?)
                .map_err(|err| AppError::InvalidRequest(format!("invalid package: {err}")))?;
            let scope_id = ctx.resolve_scope_id(scope_id).await?;
            let outcome = ctx
                .app
                .scopes()
                .update_package(&ctx.principal, &scope_id, package)
                .await?;
            let outcome = facet_json::to_string(&outcome)
                .map_err(|err| AppError::InvalidRequest(err.to_string()))?;
            let mut out = Object::new();
            out.insert("format", Value::String("facet-json".to_string()));
            out.insert("outcome", Value::String(outcome));
            Ok(Value::Object(out))
        })
    }
}

impl RpcCommand<AppRequestContext> for DbQueryCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let object = expect_object(payload)?;
            let scope_id = optional_string(&object, "scope_id")?.map(DbScopeId::new);
            let query = required_string(&object, "query")?;
            let format = match optional_string(&object, "format")?.as_deref() {
                Some("prql") => TextQueryFormat::Prql,
                Some("sql") | None => TextQueryFormat::Sql,
                Some(other) => {
                    return Err(AppError::InvalidRequest(format!(
                        "unsupported query format '{other}'"
                    )));
                }
            };
            let db = ctx.resolve_db(scope_id).await?;
            let result = db.query(TextQueryInput::Text { format, query }).await?;
            Ok(query_result_to_value(result))
        })
    }
}

impl RpcCommand<AppRequestContext> for DbGetCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let object = expect_object(payload)?;
            let scope_id = optional_string(&object, "scope_id")?.map(DbScopeId::new);
            let collection = optional_string(&object, "collection")?
                .unwrap_or_else(|| DEFAULT_COLLECTION.into());
            let id = required_string(&object, "id")?;
            let db = ctx.resolve_db(scope_id).await?;
            match db.get(collection, id).await? {
                Some(record) => {
                    let mut out = Object::new();
                    out.insert("collection", Value::String(record.collection));
                    out.insert("id", Value::String(record.id));
                    out.insert("object", Value::Object(record.object));
                    Ok(Value::Object(out))
                }
                None => Ok(Value::Null),
            }
        })
    }
}

impl RpcCommand<AppRequestContext> for DbInsertCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let object = expect_object(payload)?;
            let scope_id = optional_string(&object, "scope_id")?.map(DbScopeId::new);
            let collection = optional_string(&object, "collection")?
                .unwrap_or_else(|| DEFAULT_COLLECTION.into());
            let id = required_string(&object, "id")?;
            let row = match object.get("object") {
                Some(Value::Object(object)) => object.clone(),
                Some(_) => {
                    return Err(AppError::InvalidRequest(
                        "field 'object' must be an object".to_string(),
                    ));
                }
                None => return Err(missing_field("object")),
            };
            let db = ctx.resolve_db(scope_id).await?;
            db.insert(collection, id, row).await?;
            Ok(Value::Void)
        })
    }
}

impl RpcCommand<AppRequestContext> for DbDeleteCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let object = expect_object(payload)?;
            let scope_id = optional_string(&object, "scope_id")?.map(DbScopeId::new);
            let collection = optional_string(&object, "collection")?
                .unwrap_or_else(|| DEFAULT_COLLECTION.into());
            let id = required_string(&object, "id")?;
            let db = ctx.resolve_db(scope_id).await?;
            db.delete(collection, id).await?;
            Ok(Value::Void)
        })
    }
}

impl RpcCommand<AppRequestContext> for DbBatchCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let object = expect_object(payload)?;
            let scope_id = optional_string(&object, "scope_id")?.map(DbScopeId::new);
            let batch = batch_from_payload(&object)?;
            let db = ctx.resolve_db(scope_id).await?;
            let outcome = db.execute_batch(batch).await?;
            Ok(batch_outcome_to_value(outcome))
        })
    }
}

impl RpcCommand<AppRequestContext> for FileAnalyzeCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<Value, AppError>> + Send + 'a>> {
        Box::pin(async move {
            let object = expect_object(payload)?;
            let scope_id = optional_string(&object, "scope_id")?.map(DbScopeId::new);
            let id = required_string(&object, "id")?;
            let outcome = ctx
                .app
                .files()
                .media_analysis()
                .analyze_persisted_file(ctx, scope_id, id)
                .await?;
            let mut out = Object::new();
            out.insert("id", Value::String(outcome.id));
            out.insert("collection", Value::String(outcome.collection));
            out.insert("analyzed", Value::Bool(outcome.analyzed));
            match outcome.analysis_kind {
                Some(kind) => out.insert("analysis_kind", Value::String(kind.to_string())),
                None => out.insert("analysis_kind", Value::Null),
            };
            out.insert("attributes", Value::Object(outcome.attributes));
            out.insert("object", Value::Object(outcome.object));
            Ok(Value::Object(out))
        })
    }
}

fn expect_object(value: Value) -> std::result::Result<Object, AppError> {
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(AppError::InvalidRequest(
            "expected object payload".to_string(),
        )),
    }
}

fn batch_from_payload(object: &Object) -> std::result::Result<Batch, AppError> {
    let operations = match object.get("operations") {
        Some(Value::List(operations)) => operations,
        Some(_) => {
            return Err(AppError::InvalidRequest(
                "field 'operations' must be a list".to_string(),
            ));
        }
        None => return Err(missing_field("operations")),
    };
    let mut batch = Batch::new();
    for operation in operations {
        batch = batch.with_op(batch_operation_from_value(operation)?);
    }
    Ok(batch)
}

fn batch_operation_from_value(value: &Value) -> std::result::Result<BatchOperation, AppError> {
    let Value::Object(object) = value else {
        return Err(AppError::InvalidRequest(
            "batch operation must be an object".to_string(),
        ));
    };
    let kind = required_string(object, "kind")?;
    match kind.as_str() {
        "upsert" => {
            let collection =
                optional_string(object, "collection")?.unwrap_or_else(|| DEFAULT_COLLECTION.into());
            let id = required_string(object, "id")?;
            let object = required_object(object, "object")?;
            Ok(BatchOperation::Upsert {
                collection,
                id,
                object,
            })
        }
        "delete_by_id" => {
            let collection =
                optional_string(object, "collection")?.unwrap_or_else(|| DEFAULT_COLLECTION.into());
            let id = required_string(object, "id")?;
            Ok(BatchOperation::DeleteById { collection, id })
        }
        "delete_by_ids" => {
            let collection =
                optional_string(object, "collection")?.unwrap_or_else(|| DEFAULT_COLLECTION.into());
            let ids = required_string_list(object, "ids")?;
            Ok(BatchOperation::DeleteByIds { collection, ids })
        }
        other => Err(AppError::InvalidRequest(format!(
            "unsupported batch operation kind '{other}'"
        ))),
    }
}

fn required_object(object: &Object, field: &str) -> std::result::Result<Object, AppError> {
    match object.get(field) {
        Some(Value::Object(value)) => Ok(value.clone()),
        Some(_) => Err(AppError::InvalidRequest(format!(
            "field '{field}' must be an object"
        ))),
        None => Err(missing_field(field)),
    }
}

fn required_string_list(
    object: &Object,
    field: &str,
) -> std::result::Result<Vec<String>, AppError> {
    let values = match object.get(field) {
        Some(Value::List(values)) => values,
        Some(_) => {
            return Err(AppError::InvalidRequest(format!(
                "field '{field}' must be a list"
            )));
        }
        None => return Err(missing_field(field)),
    };
    values
        .iter()
        .map(|value| match value {
            Value::String(value) => Ok(value.clone()),
            _ => Err(AppError::InvalidRequest(format!(
                "field '{field}' must contain only strings"
            ))),
        })
        .collect()
}

fn required_string(object: &Object, field: &str) -> std::result::Result<String, AppError> {
    optional_string(object, field)?.ok_or_else(|| missing_field(field))
}

fn optional_string(object: &Object, field: &str) -> std::result::Result<Option<String>, AppError> {
    match object.get(field) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Null) | Some(Value::Void) | None => Ok(None),
        Some(_) => Err(AppError::InvalidRequest(format!(
            "field '{field}' must be a string"
        ))),
    }
}

fn optional_bool(object: &Object, field: &str) -> std::result::Result<Option<bool>, AppError> {
    match object.get(field) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(Value::Null) | Some(Value::Void) | None => Ok(None),
        Some(_) => Err(AppError::InvalidRequest(format!(
            "field '{field}' must be a boolean"
        ))),
    }
}

fn missing_field(field: &str) -> AppError {
    AppError::InvalidRequest(format!("missing field '{field}'"))
}

fn scope_info_object(info: &ScopeInfo) -> Object {
    let mut object = Object::new();
    object.insert("scope_id", Value::String(info.scope_id.to_string()));
    object.insert("owner", Value::String(info.owner.to_string()));
    object.insert(
        "visibility",
        Value::String(match info.visibility {
            ScopeVisibility::Principal => "principal".to_string(),
            ScopeVisibility::System => "system".to_string(),
        }),
    );
    object.insert("uri", Value::String(info.uri.clone()));
    object.insert("loaded", Value::Bool(info.loaded));
    object
}

fn query_result_to_value(result: QueryResult) -> Value {
    let mut object = Object::new();
    match result {
        QueryResult::Select(rows) => {
            object.insert("kind", Value::String("select".to_string()));
            object.insert(
                "rows",
                Value::List(rows.into_iter().map(Value::Object).collect()),
            );
        }
        QueryResult::Insert(result) => {
            object.insert("kind", Value::String("insert".to_string()));
            insert_insert_result(&mut object, result);
        }
        QueryResult::Update(result) => {
            object.insert("kind", Value::String("update".to_string()));
            insert_update_result(&mut object, result);
        }
        QueryResult::Delete(result) => {
            object.insert("kind", Value::String("delete".to_string()));
            insert_delete_result(&mut object, result);
        }
        QueryResult::Ddl(()) => {
            object.insert("kind", Value::String("ddl".to_string()));
        }
    }
    Value::Object(object)
}

fn batch_outcome_to_value(outcome: BatchOutcome) -> Value {
    let mut object = Object::new();
    insert_batch_stats(&mut object, outcome.stats);
    object.insert(
        "dataset",
        Value::Object(
            outcome
                .dataset
                .into_iter()
                .map(|(collection, rows)| {
                    (
                        collection,
                        Value::Object(
                            rows.into_iter()
                                .map(|(id, object)| (id, Value::Object(object)))
                                .collect(),
                        ),
                    )
                })
                .collect(),
        ),
    );
    Value::Object(object)
}

fn insert_batch_stats(object: &mut Object, stats: BatchStats) {
    let mut stats_object = Object::new();
    stats_object.insert("upserted", Value::U64(stats.upserted as u64));
    stats_object.insert("deleted", Value::U64(stats.deleted as u64));
    stats_object.insert("updated", Value::U64(stats.updated as u64));
    object.insert("stats", Value::Object(stats_object));
}

fn insert_insert_result(object: &mut Object, result: InsertResult) {
    object.insert("inserted", Value::U64(result.inserted as u64));
    object.insert(
        "returning",
        Value::List(result.returning.into_iter().map(Value::Object).collect()),
    );
}

fn insert_update_result(object: &mut Object, result: UpdateResult) {
    insert_stats(object, result.stats);
    object.insert(
        "returning",
        Value::List(result.returning.into_iter().map(Value::Object).collect()),
    );
}

fn insert_delete_result(object: &mut Object, result: DeleteResult) {
    object.insert("deleted", Value::U64(result.deleted as u64));
    object.insert(
        "returning",
        Value::List(result.returning.into_iter().map(Value::Object).collect()),
    );
}

fn insert_stats(object: &mut Object, stats: MutationStats) {
    let mut stats_object = Object::new();
    stats_object.insert("matched", Value::U64(stats.matched as u64));
    stats_object.insert("affected", Value::U64(stats.affected as u64));
    object.insert("stats", Value::Object(stats_object));
}
