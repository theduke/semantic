use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use semantic_data::schema::{DbOpenMode, FunctionType};
use semantic_data::value::{Object, Value};
use semantic_db_core::{
    DEFAULT_COLLECTION, DeleteResult, InsertResult, MutationStats, QueryResult, TextQueryFormat,
    TextQueryInput, UpdateResult,
};
use semantic_rpc::{RpcCommand, RpcCommandSpec, RpcRegistry, RpcRequest, RpcResponse};

use crate::{
    AppError, AppRequestContext, AppSession, DbOpenRequest, DbProvider, DbScopeId, ScopeInfo,
    ScopeManager, ScopeOpenOptions, ScopeVisibility, SemanticDb,
};

#[derive(Clone)]
pub struct SemanticApp {
    inner: Arc<SemanticAppInner>,
}

pub struct SemanticAppInner {
    registry: Arc<RpcRegistry<AppRequestContext>>,
    scopes: ScopeManager,
}

enum DefaultScope {
    Opened(DbScopeId, Arc<dyn SemanticDb>),
    Request(DbScopeId, DbOpenRequest),
}

pub struct SemanticAppBuilder {
    providers: BTreeMap<String, Arc<dyn DbProvider>>,
    registry: RpcRegistry<AppRequestContext>,
    default_scope: Option<DefaultScope>,
    idle_ttl: Duration,
}

impl SemanticApp {
    pub fn builder() -> SemanticAppBuilder {
        SemanticAppBuilder {
            providers: BTreeMap::new(),
            registry: RpcRegistry::new(),
            default_scope: None,
            idle_ttl: Duration::from_secs(15 * 60),
        }
    }

    pub async fn invoke(&self, ctx: AppRequestContext, request: RpcRequest) -> RpcResponse {
        self.inner.registry.invoke(&ctx, request).await
    }

    pub fn scopes(&self) -> &ScopeManager {
        &self.inner.scopes
    }

    pub fn new_session(&self, id: impl Into<String>) -> Arc<AppSession> {
        Arc::new(AppSession::new(id))
    }
}

impl SemanticAppBuilder {
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

    pub fn with_idle_ttl(mut self, ttl: Duration) -> Self {
        self.idle_ttl = ttl;
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

    pub fn register_builtin_commands(mut self) -> std::result::Result<Self, AppError> {
        self.registry.register(ScopeOpenCommand)?;
        self.registry.register(ScopeUseCommand)?;
        self.registry.register(ScopeCurrentCommand)?;
        self.registry.register(ScopeListCommand)?;
        self.registry.register(DbCatalogCommand)?;
        self.registry.register(DbQueryCommand)?;
        self.registry.register(DbGetCommand)?;
        self.registry.register(DbInsertCommand)?;
        self.registry.register(DbDeleteCommand)?;
        Ok(self)
    }

    pub fn build(self) -> std::result::Result<SemanticApp, AppError> {
        let scopes = ScopeManager::new(self.providers, self.idle_ttl);
        if let Some(default_scope) = self.default_scope {
            match default_scope {
                DefaultScope::Opened(scope_id, db) => scopes.add_default_scope(scope_id, db)?,
                DefaultScope::Request(scope_id, request) => {
                    scopes.add_default_scope_request(scope_id, request)?;
                }
            }
        }
        Ok(SemanticApp {
            inner: Arc::new(SemanticAppInner {
                registry: Arc::new(self.registry),
                scopes,
            }),
        })
    }
}

struct ScopeOpenCommand;
struct ScopeUseCommand;
struct ScopeCurrentCommand;
struct ScopeListCommand;
struct DbCatalogCommand;
struct DbQueryCommand;
struct DbGetCommand;
struct DbInsertCommand;
struct DbDeleteCommand;

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
command_spec!(DbQueryCommand, "semantic.db.query");
command_spec!(DbGetCommand, "semantic.db.get");
command_spec!(DbInsertCommand, "semantic.db.insert");
command_spec!(DbDeleteCommand, "semantic.db.delete");

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

fn expect_object(value: Value) -> std::result::Result<Object, AppError> {
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(AppError::InvalidRequest(
            "expected object payload".to_string(),
        )),
    }
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
