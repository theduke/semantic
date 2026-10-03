use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use semantic_data::schema::{
    DbOpenMode, EnumRepr, EnumType, EnumVariant, Meta, Package, Type, TypeKind, UnionType,
};
use semantic_data::value::{FromValue, FromValueError, IntoValue, Object, SemanticType, Value};
use semantic_db_core::{
    Batch, BatchOperation, BatchStats, DEFAULT_COLLECTION, DeleteResult, InsertResult, QueryResult,
    TextQueryFormat, TextQueryInput, UpdateResult,
};
use semantic_rpc::RpcRegistry;
use semantic_rpc::stream_command::RpcStreamCommand;
use semantic_rpc_core::{
    CallError, RpcCommand, RpcCommandSpec, RpcRequest, RpcResponse, RuntimePackage,
};

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
    registry: Arc<RpcRegistry<AppRequestContext, AppError>>,
    /// Descriptor of `registry`, served as the command export.
    command_descriptor: semantic_rpc_core::interface::ImplementationDescriptor,
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
    registry: RpcRegistry<AppRequestContext, AppError>,
    packages: Vec<Package>,
    default_scope: Option<DefaultScope>,
    default_object_store: Option<DefaultObjectStore>,
    idle_ttl: Duration,
    media_analysis_config: MediaAnalysisConfig,
    jobs_registry: semantic_jobs::JobsRegistry,
    jobs_config: semantic_jobs::JobsConfig,
    plugins: semantic_plugin::PluginRegistry,
    tasks_enabled: bool,
    comments_enabled: bool,
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
            tasks_enabled: true,
            comments_enabled: true,
        }
    }

    pub async fn invoke(&self, ctx: AppRequestContext, request: RpcRequest) -> RpcResponse {
        self.inner.registry.invoke(&ctx, request).await
    }

    /// Call a command by name, preserving its typed [`AppError`].
    pub async fn call(
        &self,
        ctx: AppRequestContext,
        command: &str,
        payload: Value,
    ) -> Result<Value, CallError<AppError>> {
        self.inner.registry.call(&ctx, command, payload).await
    }

    pub(crate) fn registry_arc(&self) -> Arc<RpcRegistry<AppRequestContext, AppError>> {
        self.inner.registry.clone()
    }

    pub(crate) fn command_descriptor(
        &self,
    ) -> &semantic_rpc_core::interface::ImplementationDescriptor {
        &self.inner.command_descriptor
    }

    /// The registry of all commands exposed by this app.
    pub(crate) fn registry(&self) -> &RpcRegistry<AppRequestContext, AppError> {
        &self.inner.registry
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

    pub fn with_tasks(mut self, enabled: bool) -> Self {
        self.tasks_enabled = enabled;
        self
    }

    pub fn with_comments(mut self, enabled: bool) -> Self {
        self.comments_enabled = enabled;
        self
    }

    pub fn with_config(mut self, config: AppConfig) -> Self {
        self.tasks_enabled = config.tasks_enabled;
        self.comments_enabled = config.comments_enabled;
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
        C::Error: Into<AppError>,
        AppRequestContext: Sync,
    {
        self.registry.register(command)?;
        Ok(self)
    }

    /// Register a streaming command, served over interface sessions.
    pub fn register_stream_command<C>(mut self, command: C) -> std::result::Result<Self, AppError>
    where
        C: RpcStreamCommand<AppRequestContext>,
        C::Error: Into<AppError>,
        AppRequestContext: Sync,
    {
        self.registry.register_stream(command)?;
        Ok(self)
    }

    /// Register a package's handlers and schema.
    ///
    /// Handlers are available application-wide. Schemas are applied lazily to the
    /// default database, alongside the built-in packages, before its first use.
    /// Explicitly opened scopes retain their existing schema initialization behavior.
    pub fn register_package(
        mut self,
        package: impl RuntimePackage<AppRequestContext, AppError>,
    ) -> Result<Self, AppError> {
        for command in package.commands() {
            self.registry.register_dyn(command)?;
        }
        self.packages.push(package.schema());
        Ok(self)
    }

    pub fn register_builtin_commands(mut self) -> std::result::Result<Self, AppError> {
        self.registry
            .register_definitions(semantic_data::query::semantic::definitions())?;
        self.registry.register(ScopeOpenCommand)?;
        self.registry.register(ScopeUseCommand)?;
        self.registry.register(ScopeCurrentCommand)?;
        self.registry.register(ScopeListCommand)?;
        self.registry.register(DbCatalogCommand)?;
        self.registry.register(DbPackageUpsertCommand)?;
        self.registry.register(DbQueryCommand)?;
        self.registry.register(DbParseSqlCommand)?;
        self.registry.register(DbGetCommand)?;
        self.registry.register(DbInsertCommand)?;
        self.registry.register(DbDeleteCommand)?;
        self.registry.register(DbBatchCommand)?;
        self.registry.register(DbValidationPreflightCommand)?;
        self.registry.register(DbValidationActivateCommand)?;
        crate::db_maintenance_commands::register(&mut self.registry)?;
        self.registry.register(FileAnalyzeCommand)?;
        crate::jobs::register_commands(&mut self.registry)?;
        crate::import_commands::register(&mut self.registry)?;
        crate::command_introspection::register(&mut self.registry)?;
        crate::capabilities::register(&mut self.registry)?;
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
            if self.comments_enabled || self.tasks_enabled {
                self = self.register_package(semantic_base::CommentsPackage)?;
            }
            if self.tasks_enabled {
                self = self.register_package(semantic_base::TasksPackage)?;
            }
        }
        self.packages.push(semantic_data::filestore::package());
        self.packages.push(import_package);
        self.packages.push(semantic_data::bundles::query::package());
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
        let command_interface =
            semantic_rpc::interface::registry::registry_interface(&self.registry);
        let command_descriptor =
            semantic_rpc::interface::registry::registry_descriptor_with_definitions(
                &command_interface,
                self.registry.definitions(),
            )
            .map_err(crate::plugins::error)?;
        Ok(SemanticApp {
            inner: Arc::new(SemanticAppInner {
                registry: Arc::new(self.registry),
                command_descriptor,
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
struct DbParseSqlCommand;
struct DbGetCommand;
struct DbInsertCommand;
struct DbDeleteCommand;
struct DbBatchCommand;
struct DbValidationPreflightCommand;
struct DbValidationActivateCommand;
struct FileAnalyzeCommand;

macro_rules! command_spec {
    ($ty:ty, $name:literal, $payload:ty => $output:ty) => {
        impl RpcCommandSpec for $ty {
            type Payload = $payload;
            type Output = $output;
            type Error = AppError;

            const NAME: &'static str = $name;
        }
    };
}

command_spec!(ScopeOpenCommand, "semantic.scope.open", ScopeOpenPayload => ScopeOpenOutput);
command_spec!(ScopeUseCommand, "semantic.scope.use", Option<ScopeSelector> => CurrentScope);
command_spec!(ScopeCurrentCommand, "semantic.scope.current", Option<NoParams> => CurrentScope);
command_spec!(ScopeListCommand, "semantic.scope.list", Option<NoParams> => Vec<ScopeInfoOutput>);
command_spec!(DbCatalogCommand, "semantic.db.catalog", Option<ScopeParams> => CatalogOutput);
command_spec!(
    DbPackageUpsertCommand,
    "semantic.db.package.upsert",
    PackageUpsertPayload => PackageUpsertOutput
);
command_spec!(DbQueryCommand, "semantic.db.query", QueryPayload => QueryOutput);
command_spec!(DbParseSqlCommand, "semantic.db.query.parse_sql", ParseSqlPayload => semantic_data::query::Query);
command_spec!(DbGetCommand, "semantic.db.get", EntityPayload => Option<EntityOutput>);
command_spec!(DbInsertCommand, "semantic.db.insert", InsertPayload => ());
command_spec!(DbDeleteCommand, "semantic.db.delete", EntityPayload => ());
command_spec!(DbBatchCommand, "semantic.db.batch", BatchPayload => BatchOutput);
command_spec!(
    DbValidationPreflightCommand,
    "semantic.db.validation.preflight",
    ScopeParams => Vec<ValidationViolationOutput>
);
command_spec!(
    DbValidationActivateCommand,
    "semantic.db.validation.activate",
    ScopeParams => ()
);
command_spec!(FileAnalyzeCommand, "semantic.file.analyze", FileAnalyzePayload => FileAnalyzeOutput);

/// Encoding of serialized documents, like catalogs and packages.
#[derive(SemanticType, IntoValue, FromValue, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DocumentFormat {
    #[semantic(rename = "facet-json")]
    FacetJson,
}

/// A payload with only an optional scope; the current scope applies without it.
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, Default)]
pub(crate) struct ScopeParams {
    pub scope_id: Option<String>,
}

impl ScopeParams {
    pub(crate) fn scope_id(self) -> Option<DbScopeId> {
        self.scope_id.map(DbScopeId::new)
    }
}

/// A payload without fields.
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, Default)]
pub(crate) struct NoParams {}

#[derive(SemanticType, IntoValue, FromValue, Clone, Copy, Debug, Default)]
#[semantic(rename_all = "snake_case")]
enum OpenMode {
    OpenExisting,
    #[default]
    AutoCreate,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct ScopeOpenPayload {
    uri: String,
    scope_id: Option<String>,
    /// Defaults to `auto_create`.
    mode: Option<OpenMode>,
    /// Defaults to `principal`.
    visibility: Option<ScopeVisibility>,
    /// Defaults to `true`.
    set_current: Option<bool>,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct ScopeInfoOutput {
    scope_id: String,
    owner: String,
    visibility: ScopeVisibility,
    uri: String,
    loaded: bool,
}

impl From<ScopeInfo> for ScopeInfoOutput {
    fn from(info: ScopeInfo) -> Self {
        Self {
            scope_id: info.scope_id.to_string(),
            owner: info.owner.to_string(),
            visibility: info.visibility,
            uri: info.uri,
            loaded: info.loaded,
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
struct ScopeOpenOutput {
    #[semantic(flatten)]
    info: ScopeInfoOutput,
    /// Whether the scope became the session's current scope.
    current: bool,
}

/// A scope id, or an object with an optional `scope_id`.
enum ScopeSelector {
    Id(String),
    Params(ScopeParams),
}

impl SemanticType for ScopeSelector {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Union(UnionType {
            variants: vec![String::semantic_type(), ScopeParams::semantic_type()],
        }))
    }
}

impl IntoValue for ScopeSelector {
    fn into_value(self) -> Value {
        match self {
            Self::Id(id) => Value::String(id),
            Self::Params(params) => params.into_value(),
        }
    }
}

impl FromValue for ScopeSelector {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::String(id) => Ok(Self::Id(id)),
            value @ Value::Object(_) => ScopeParams::from_value(value).map(Self::Params),
            other => Err(FromValueError::expected(
                "scope id string or object",
                &other,
            )),
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
struct CurrentScope {
    #[semantic(required)]
    scope_id: Option<String>,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct CatalogOutput {
    format: DocumentFormat,
    /// The catalog storage snapshot, encoded in `format`.
    catalog: String,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct PackageUpsertPayload {
    scope_id: Option<String>,
    format: Option<DocumentFormat>,
    /// The package, encoded in `format`.
    package: String,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct PackageUpsertOutput {
    format: DocumentFormat,
    /// The update outcome, encoded in `format`.
    outcome: String,
}

#[derive(SemanticType, IntoValue, FromValue, Clone, Copy, Default)]
#[semantic(rename_all = "snake_case")]
enum QueryFormat {
    #[default]
    Sql,
    Prql,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct QueryPayload {
    scope_id: Option<String>,
    /// Construct an AST directly, or supply query text as a fallback.
    query: QueryArgument,
    /// Text language; defaults to `sql`. Must be omitted for ASTs.
    format: Option<QueryFormat>,
    #[semantic(default)]
    params: CommandDictionary<Value>,
}

/// Open, typed command dictionaries. Keep this correction local until the
/// generic BTreeMap schema can change independently of existing contracts.
pub(crate) struct CommandDictionary<T>(pub BTreeMap<String, T>);

impl<T> Default for CommandDictionary<T> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl<T: SemanticType> SemanticType for CommandDictionary<T> {
    fn semantic_type() -> Type {
        let mut ty = BTreeMap::<String, T>::semantic_type();
        if let TypeKind::Record(record) = &mut ty.kind {
            record.open = true;
        }
        ty
    }
}

impl<T: IntoValue> IntoValue for CommandDictionary<T> {
    fn into_value(self) -> Value {
        self.0.into_value()
    }
}

impl<T: FromValue> FromValue for CommandDictionary<T> {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        BTreeMap::<String, T>::from_value(value).map(Self)
    }
}

/// An untagged query AST or the existing query text input.
enum QueryArgument {
    Ast(semantic_data::query::Query),
    Text(String),
}

impl SemanticType for QueryArgument {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Union(UnionType {
            variants: vec![
                semantic_data::query::Query::semantic_type(),
                String::semantic_type(),
            ],
        }))
    }
}

impl IntoValue for QueryArgument {
    fn into_value(self) -> Value {
        match self {
            Self::Ast(query) => query.into_value(),
            Self::Text(query) => query.into_value(),
        }
    }
}

impl FromValue for QueryArgument {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::String(query) => Ok(Self::Text(query)),
            value @ Value::Object(_) => {
                semantic_data::query::Query::from_value(value).map(Self::Ast)
            }
            other => Err(FromValueError::expected("query AST or query text", &other)),
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
struct ParseSqlPayload {
    scope_id: Option<String>,
    query: String,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(tag = "kind", rename_all = "snake_case")]
enum QueryOutput {
    Select {
        rows: Vec<Object>,
    },
    Insert {
        inserted: usize,
        returning: Vec<Object>,
    },
    Update {
        stats: MutationStatsOutput,
        returning: Vec<Object>,
    },
    Delete {
        deleted: usize,
        returning: Vec<Object>,
    },
    Ddl,
}

impl From<QueryResult> for QueryOutput {
    fn from(result: QueryResult) -> Self {
        match result {
            QueryResult::Select(rows) => Self::Select { rows },
            QueryResult::Insert(InsertResult {
                inserted,
                returning,
            }) => Self::Insert {
                inserted,
                returning,
            },
            QueryResult::Update(UpdateResult { stats, returning }) => Self::Update {
                stats: MutationStatsOutput {
                    matched: stats.matched,
                    affected: stats.affected,
                },
                returning,
            },
            QueryResult::Delete(DeleteResult { deleted, returning }) => {
                Self::Delete { deleted, returning }
            }
            QueryResult::Ddl(()) => Self::Ddl,
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
struct MutationStatsOutput {
    matched: usize,
    affected: usize,
}

/// Identifies an entity; `collection` defaults to the default collection.
#[derive(SemanticType, IntoValue, FromValue)]
struct EntityPayload {
    scope_id: Option<String>,
    collection: Option<String>,
    id: String,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct EntityOutput {
    collection: String,
    id: String,
    object: Object,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct InsertPayload {
    scope_id: Option<String>,
    collection: Option<String>,
    id: String,
    object: Object,
}

fn collection_or_default(collection: Option<String>) -> String {
    collection.unwrap_or_else(|| DEFAULT_COLLECTION.into())
}

#[derive(SemanticType, IntoValue, FromValue)]
struct BatchPayload {
    scope_id: Option<String>,
    operations: Vec<BatchOperationPayload>,
    #[semantic(default)]
    returning: BatchReturning,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(tag = "kind", rename_all = "snake_case")]
enum BatchOperationPayload {
    Create {
        collection: Option<String>,
        id: String,
        object: Object,
    },
    Upsert {
        collection: Option<String>,
        id: String,
        object: Object,
    },
    DeleteById {
        collection: Option<String>,
        id: String,
    },
    DeleteByIds {
        collection: Option<String>,
        ids: Vec<String>,
    },
}

impl From<BatchOperationPayload> for BatchOperation {
    fn from(operation: BatchOperationPayload) -> Self {
        match operation {
            BatchOperationPayload::Create {
                collection,
                id,
                object,
            } => BatchOperation::Create {
                collection: collection_or_default(collection),
                id,
                object,
            },
            BatchOperationPayload::Upsert {
                collection,
                id,
                object,
            } => BatchOperation::Upsert {
                collection: collection_or_default(collection),
                id,
                object,
            },
            BatchOperationPayload::DeleteById { collection, id } => BatchOperation::DeleteById {
                collection: collection_or_default(collection),
                id,
            },
            BatchOperationPayload::DeleteByIds { collection, ids } => BatchOperation::DeleteByIds {
                collection: collection_or_default(collection),
                ids,
            },
        }
    }
}

/// The batch reply mode: `"dataset"` (the default), `"stats"`, `"changes"`, or
/// `{"projection": {"fields": [...]}}`.
///
/// Decoding keeps the raw value, so invalid modes fail as a
/// [`semantic_db_core::DbError::BatchReturn`] error rather than an invalid payload.
#[derive(Default)]
struct BatchReturning(Option<Value>);

impl SemanticType for BatchReturning {
    fn semantic_type() -> Type {
        let modes = Type::new(TypeKind::Enum(EnumType {
            repr: EnumRepr::String,
            variants: ["dataset", "stats", "changes"]
                .into_iter()
                .map(|name| EnumVariant {
                    name: name.into(),
                    value: None,
                    symbol: Some(name.into()),
                    meta: Meta::default(),
                })
                .collect(),
        }));
        Type::new(TypeKind::Union(UnionType {
            variants: vec![modes, BatchProjectionMode::semantic_type()],
        }))
    }
}

impl IntoValue for BatchReturning {
    fn into_value(self) -> Value {
        self.0
            .unwrap_or_else(|| Value::String("dataset".to_string()))
    }
}

impl FromValue for BatchReturning {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        Ok(Self(Some(value)))
    }
}

/// Declares the projection mode of [`BatchReturning`].
#[derive(SemanticType)]
#[allow(dead_code)]
struct BatchProjectionMode {
    projection: BatchProjection,
}

#[derive(SemanticType)]
#[allow(dead_code)]
struct BatchProjection {
    fields: Vec<String>,
}

impl BatchReturning {
    fn parse(self) -> Result<semantic_db_core::BatchReturn, AppError> {
        use semantic_db_core::{BatchReturn, BatchReturnErrorReason, DbError};
        let invalid = || {
            AppError::Db(DbError::BatchReturn {
                reason: BatchReturnErrorReason::UnknownMode,
                field: None,
            })
        };
        match self.0 {
            None => Ok(BatchReturn::Dataset),
            Some(Value::String(mode)) => match mode.as_str() {
                "dataset" => Ok(BatchReturn::Dataset),
                "stats" => Ok(BatchReturn::Stats),
                "changes" => Ok(BatchReturn::Changes),
                _ => Err(invalid()),
            },
            Some(Value::Object(mode)) if mode.len() == 1 => {
                let Some(Value::Object(projection)) = mode.get("projection") else {
                    return Err(invalid());
                };
                if projection.len() != 1 {
                    return Err(invalid());
                }
                let Some(Value::List(fields)) = projection.get("fields") else {
                    return Err(invalid());
                };
                let fields = fields
                    .iter()
                    .map(|field| match field {
                        Value::String(field) => Ok(field.clone()),
                        _ => Err(invalid()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(BatchReturn::Projection { fields })
            }
            _ => Err(invalid()),
        }
    }
}

/// The batch reply; its fields depend on the `returning` mode.
#[derive(SemanticType, IntoValue, FromValue)]
struct BatchOutput {
    stats: BatchStatsOutput,
    /// Rows written, by collection and id (mode `dataset`).
    dataset: Option<BTreeMap<String, BTreeMap<String, Object>>>,
    /// Changed entities (modes `changes` and `projection`).
    changes: Option<Vec<EntityChangeOutput>>,
    /// Projected rows (mode `projection`).
    rows: Option<Vec<EntityOutput>>,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct BatchStatsOutput {
    upserted: usize,
    deleted: usize,
    updated: usize,
}

impl From<BatchStats> for BatchStatsOutput {
    fn from(stats: BatchStats) -> Self {
        Self {
            upserted: stats.upserted,
            deleted: stats.deleted,
            updated: stats.updated,
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(rename_all = "snake_case")]
enum EntityChangeKindOutput {
    Upsert,
    Delete,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct EntityChangeOutput {
    collection: String,
    id: String,
    kind: EntityChangeKindOutput,
}

impl From<semantic_db_core::BatchReply> for BatchOutput {
    fn from(reply: semantic_db_core::BatchReply) -> Self {
        use semantic_db_core::{BatchReply, EntityChangeKind};
        let (stats, dataset, changes, rows) = match reply {
            BatchReply::Dataset(outcome) => {
                let dataset = outcome
                    .dataset
                    .into_iter()
                    .map(|(collection, rows)| (collection, rows.into_iter().collect()))
                    .collect();
                (outcome.stats, Some(dataset), None, None)
            }
            BatchReply::Stats { stats, .. } => (stats, None, None, None),
            BatchReply::Changes { stats, changes, .. } => (stats, None, Some(changes), None),
            BatchReply::Projection {
                stats,
                changes,
                rows,
                ..
            } => (stats, None, Some(changes), Some(rows)),
        };
        Self {
            stats: stats.into(),
            dataset,
            changes: changes.map(|changes| {
                changes
                    .into_iter()
                    .map(|change| EntityChangeOutput {
                        collection: change.collection,
                        id: change.id,
                        kind: match change.kind {
                            EntityChangeKind::Upsert => EntityChangeKindOutput::Upsert,
                            EntityChangeKind::Delete => EntityChangeKindOutput::Delete,
                        },
                    })
                    .collect()
            }),
            rows: rows.map(|rows| {
                rows.into_iter()
                    .map(|row| EntityOutput {
                        collection: row.collection,
                        id: row.id,
                        object: row.object,
                    })
                    .collect()
            }),
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
struct ValidationViolationOutput {
    collection: String,
    id: String,
    /// The validation error data, as in `validation_failed` errors.
    error: Value,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct FileAnalyzePayload {
    scope_id: Option<String>,
    id: String,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct FileAnalyzeOutput {
    id: String,
    collection: String,
    analyzed: bool,
    #[semantic(required)]
    analysis_kind: Option<String>,
    attributes: Object,
    object: Object,
}

type CommandFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

impl RpcCommand<AppRequestContext> for ScopeOpenCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: ScopeOpenPayload,
    ) -> CommandFuture<'a, ScopeOpenOutput> {
        Box::pin(async move {
            let mode = match payload.mode.unwrap_or_default() {
                OpenMode::OpenExisting => DbOpenMode::OpenExisting,
                OpenMode::AutoCreate => DbOpenMode::AutoCreate,
            };
            let set_current = payload.set_current.unwrap_or(true);
            let info = ctx
                .app
                .scopes()
                .open_scope(
                    &ctx.principal,
                    ScopeOpenOptions {
                        scope_id: payload.scope_id.map(DbScopeId::new),
                        request: DbOpenRequest {
                            uri: payload.uri,
                            mode,
                        },
                        visibility: payload.visibility.unwrap_or(ScopeVisibility::Principal),
                        set_current,
                    },
                )
                .await?;
            let mut current = false;
            if set_current && ctx.session.is_some() {
                ctx.set_session_scope(Some(info.scope_id.clone())).await?;
                current = true;
            }
            Ok(ScopeOpenOutput {
                info: info.into(),
                current,
            })
        })
    }
}

impl RpcCommand<AppRequestContext> for ScopeUseCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Option<ScopeSelector>,
    ) -> CommandFuture<'a, CurrentScope> {
        Box::pin(async move {
            let scope_id = match payload {
                None => None,
                Some(ScopeSelector::Id(id)) => Some(DbScopeId::new(id)),
                Some(ScopeSelector::Params(params)) => params.scope_id(),
            };
            if let Some(scope_id) = &scope_id {
                let _ = ctx.resolve_db(Some(scope_id.clone())).await?;
            }
            ctx.set_session_scope(scope_id.clone()).await?;
            Ok(CurrentScope {
                scope_id: scope_id.map(|scope_id| scope_id.to_string()),
            })
        })
    }
}

impl RpcCommand<AppRequestContext> for ScopeCurrentCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        _payload: Option<NoParams>,
    ) -> CommandFuture<'a, CurrentScope> {
        Box::pin(async move {
            let scope_id = ctx.effective_scope_hint().await;
            Ok(CurrentScope {
                scope_id: scope_id.map(|scope_id| scope_id.to_string()),
            })
        })
    }
}

impl RpcCommand<AppRequestContext> for ScopeListCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        _payload: Option<NoParams>,
    ) -> CommandFuture<'a, Vec<ScopeInfoOutput>> {
        Box::pin(async move {
            Ok(ctx
                .app
                .scopes()
                .list_scopes(&ctx.principal)
                .into_iter()
                .map(ScopeInfoOutput::from)
                .collect())
        })
    }
}

impl RpcCommand<AppRequestContext> for DbCatalogCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Option<ScopeParams>,
    ) -> CommandFuture<'a, CatalogOutput> {
        Box::pin(async move {
            let db = ctx
                .resolve_db(payload.unwrap_or_default().scope_id())
                .await?;
            let catalog = db.catalog().await?;
            let catalog = facet_json::to_string(&catalog.to_storage_snapshot())
                .map_err(|err| AppError::InvalidRequest(err.to_string()))?;
            Ok(CatalogOutput {
                format: DocumentFormat::FacetJson,
                catalog,
            })
        })
    }
}

impl RpcCommand<AppRequestContext> for DbPackageUpsertCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: PackageUpsertPayload,
    ) -> CommandFuture<'a, PackageUpsertOutput> {
        Box::pin(async move {
            let package = facet_json::from_str::<Package>(&payload.package)
                .map_err(|err| AppError::InvalidRequest(format!("invalid package: {err}")))?;
            let scope_id = ctx
                .resolve_scope_id(payload.scope_id.map(DbScopeId::new))
                .await?;
            let outcome = ctx
                .app
                .scopes()
                .update_package(&ctx.principal, &scope_id, package)
                .await?;
            let outcome = facet_json::to_string(&outcome)
                .map_err(|err| AppError::InvalidRequest(err.to_string()))?;
            Ok(PackageUpsertOutput {
                format: DocumentFormat::FacetJson,
                outcome,
            })
        })
    }
}

impl RpcCommand<AppRequestContext> for DbQueryCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: QueryPayload,
    ) -> CommandFuture<'a, QueryOutput> {
        Box::pin(async move {
            if matches!(payload.query, QueryArgument::Ast(_)) && payload.format.is_some() {
                return Err(AppError::InvalidRequest(
                    "format must be omitted for query ASTs".into(),
                ));
            }
            let db = ctx.resolve_db(payload.scope_id.map(DbScopeId::new)).await?;
            let result = match payload.query {
                QueryArgument::Ast(query) => {
                    db.query_data(semantic_data::query::QueryInput::ast_with_params(
                        query,
                        payload.params.0,
                    ))
                    .await?
                }
                QueryArgument::Text(query) => {
                    let format = match payload.format.unwrap_or_default() {
                        QueryFormat::Sql => TextQueryFormat::Sql,
                        QueryFormat::Prql => TextQueryFormat::Prql,
                    };
                    db.query(TextQueryInput::Text {
                        format,
                        query,
                        params: payload.params.0,
                    })
                    .await?
                }
            };
            Ok(result.into())
        })
    }
}

impl RpcCommand<AppRequestContext> for DbParseSqlCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: ParseSqlPayload,
    ) -> CommandFuture<'a, semantic_data::query::Query> {
        Box::pin(async move {
            let db = ctx.resolve_db(payload.scope_id.map(DbScopeId::new)).await?;
            Ok(db.parse_sql(payload.query).await?)
        })
    }
}

impl RpcCommand<AppRequestContext> for DbGetCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: EntityPayload,
    ) -> CommandFuture<'a, Option<EntityOutput>> {
        Box::pin(async move {
            let db = ctx.resolve_db(payload.scope_id.map(DbScopeId::new)).await?;
            let record = db
                .get(collection_or_default(payload.collection), payload.id)
                .await?;
            Ok(record.map(|record| EntityOutput {
                collection: record.collection,
                id: record.id,
                object: record.object,
            }))
        })
    }
}

impl RpcCommand<AppRequestContext> for DbInsertCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: InsertPayload,
    ) -> CommandFuture<'a, ()> {
        Box::pin(async move {
            let db = ctx.resolve_db(payload.scope_id.map(DbScopeId::new)).await?;
            db.insert(
                collection_or_default(payload.collection),
                payload.id,
                payload.object,
            )
            .await?;
            Ok(())
        })
    }
}

impl RpcCommand<AppRequestContext> for DbDeleteCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: EntityPayload,
    ) -> CommandFuture<'a, ()> {
        Box::pin(async move {
            let db = ctx.resolve_db(payload.scope_id.map(DbScopeId::new)).await?;
            db.delete(collection_or_default(payload.collection), payload.id)
                .await?;
            Ok(())
        })
    }
}

impl RpcCommand<AppRequestContext> for DbBatchCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: BatchPayload,
    ) -> CommandFuture<'a, BatchOutput> {
        Box::pin(async move {
            let batch = payload
                .operations
                .into_iter()
                .fold(Batch::new(), |batch, operation| {
                    batch.with_op(operation.into())
                });
            let returning = payload.returning.parse()?;
            let db = ctx.resolve_db(payload.scope_id.map(DbScopeId::new)).await?;
            let reply = db.execute_batch_returning(batch, returning).await?;
            Ok(reply.into())
        })
    }
}

impl RpcCommand<AppRequestContext> for DbValidationPreflightCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: ScopeParams,
    ) -> CommandFuture<'a, Vec<ValidationViolationOutput>> {
        Box::pin(async move {
            let db = ctx.resolve_db(payload.scope_id()).await?;
            let violations = db.validation_preflight().await?;
            Ok(violations
                .into_iter()
                .map(|violation| ValidationViolationOutput {
                    collection: violation.collection,
                    id: violation.id,
                    error: crate::error::validation_error_data(violation.error),
                })
                .collect())
        })
    }
}

impl RpcCommand<AppRequestContext> for DbValidationActivateCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: ScopeParams,
    ) -> CommandFuture<'a, ()> {
        Box::pin(async move {
            let db = ctx.resolve_db(payload.scope_id()).await?;
            db.activate_validation().await?;
            Ok(())
        })
    }
}

impl RpcCommand<AppRequestContext> for FileAnalyzeCommand {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: FileAnalyzePayload,
    ) -> CommandFuture<'a, FileAnalyzeOutput> {
        Box::pin(async move {
            let outcome = ctx
                .app
                .files()
                .media_analysis()
                .analyze_persisted_file(ctx, payload.scope_id.map(DbScopeId::new), payload.id)
                .await?;
            Ok(FileAnalyzeOutput {
                id: outcome.id,
                collection: outcome.collection,
                analyzed: outcome.analyzed,
                analysis_kind: outcome.analysis_kind.map(str::to_owned),
                attributes: outcome.attributes,
                object: outcome.object,
            })
        })
    }
}
