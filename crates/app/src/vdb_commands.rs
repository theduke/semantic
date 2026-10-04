use std::{collections::BTreeSet, future::Future, pin::Pin, sync::Arc};

use semantic_data::{
    Value,
    value::{FromValue, IntoValue, SemanticType},
    vdb::{VdbInfo, VdbSchemaRequest},
};
use semantic_db_core::FederatedExplain;
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{CommandDef, RpcCommand, RpcCommandSpec};
use semantic_vdb::{DatabaseSchema, VdbSet, VdbStatus};

use crate::{
    AppError, AppRequestContext, DbScopeId, SemanticDb,
    command::{CommandDictionary, QueryArgument, ScopeParams},
    vdb::{FederatedScopeDb, VdbAccess},
};

pub(crate) fn register(
    registry: &mut RpcRegistry<AppRequestContext, AppError>,
) -> Result<(), AppError> {
    registry.register(List)?;
    registry.register(Schema)?;
    registry.register(Explain)?;
    Ok(())
}

#[derive(SemanticType, IntoValue, FromValue)]
struct ExplainPayload {
    scope_id: Option<String>,
    query: QueryArgument,
    #[semantic(default)]
    params: CommandDictionary<Value>,
}

type CommandFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

async fn access(
    ctx: &AppRequestContext,
    scope_id: Option<DbScopeId>,
) -> Result<(Arc<dyn SemanticDb>, VdbAccess), AppError> {
    let scope = ctx.resolve_scope_id(scope_id).await?;
    let db = ctx
        .app
        .scopes()
        .resolve_scope(&ctx.principal, Some(scope.clone()), None)
        .await?;
    Ok((
        db,
        VdbAccess::new(ctx.app.clone(), ctx.principal.clone(), scope),
    ))
}

async fn snapshot(
    ctx: &AppRequestContext,
    scope_id: Option<DbScopeId>,
    names: Option<&BTreeSet<String>>,
) -> Result<(VdbAccess, VdbSet), AppError> {
    let (db, access) = access(ctx, scope_id).await?;
    let (_, set) = access.snapshot(db.catalog().await?, names).await?;
    Ok((access, set))
}

struct List;
impl RpcCommandSpec for List {
    type Payload = Option<ScopeParams>;
    type Output = Vec<VdbInfo>;
    type Error = AppError;
    const NAME: &'static str = "semantic.vdb.list";
}
impl RpcCommand<AppRequestContext> for List {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Self::Payload,
    ) -> CommandFuture<'a, Self::Output> {
        Box::pin(async move {
            let (_, set) = snapshot(ctx, payload.unwrap_or_default().scope_id(), None).await?;
            Ok(set
                .entries()
                .iter()
                .map(|entry| VdbInfo {
                    name: entry.name.clone(),
                    plugin_id: entry.plugin_id.clone(),
                    export: entry.export.clone(),
                    generation: entry.generation,
                    available: matches!(entry.status, VdbStatus::Available),
                    reason: match &entry.status {
                        VdbStatus::Available => None,
                        VdbStatus::Unavailable { reason } => Some(reason.clone()),
                    },
                    title: entry
                        .descriptor
                        .as_ref()
                        .map(|descriptor| descriptor.title.clone()),
                    schema_revision: entry
                        .descriptor
                        .as_ref()
                        .map(|descriptor| descriptor.schema_revision.clone()),
                    classes: entry
                        .descriptor
                        .as_ref()
                        .map(|descriptor| {
                            descriptor.schema.class_ids().map(str::to_owned).collect()
                        })
                        .unwrap_or_default(),
                })
                .collect())
        })
    }
}

struct Schema;
impl RpcCommandSpec for Schema {
    type Payload = VdbSchemaRequest;
    type Output = DatabaseSchema;
    type Error = AppError;
    const NAME: &'static str = "semantic.vdb.schema";
}
impl RpcCommand<AppRequestContext> for Schema {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Self::Payload,
    ) -> CommandFuture<'a, Self::Output> {
        Box::pin(async move {
            let names = BTreeSet::from([payload.name.clone()]);
            let (access, set) =
                snapshot(ctx, payload.scope_id.map(DbScopeId::new), Some(&names)).await?;
            if let Some(error) = access
                .unavailable(&BTreeSet::from([payload.name.clone()]), &set)
                .await?
            {
                return Err(error.into());
            }
            set.schema(&payload.name).cloned().ok_or_else(|| {
                crate::plugins::error(format!("unknown virtual database '{}'", payload.name))
            })
        })
    }
}

struct Explain;
impl RpcCommandSpec for Explain {
    type Payload = ExplainPayload;
    type Output = Value;
    type Error = AppError;
    const NAME: &'static str = "semantic.vdb.explain";
    fn definition(&self) -> CommandDef {
        CommandDef::new(
            Self::NAME,
            ExplainPayload::semantic_type(),
            FederatedExplain::semantic_type(),
        )
    }
}
impl RpcCommand<AppRequestContext> for Explain {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Self::Payload,
    ) -> CommandFuture<'a, Self::Output> {
        Box::pin(async move {
            let (db, access) = access(ctx, payload.scope_id.map(DbScopeId::new)).await?;
            let query = match payload.query {
                QueryArgument::Ast(query) => query,
                QueryArgument::Text(sql) => db.parse_sql(sql).await?,
            };
            let semantic_data::query::Query::Select(query) = query else {
                return Err(crate::plugins::error(
                    "virtual database explain supports SELECT only",
                ));
            };
            FederatedScopeDb::new(db, access)
                .explain(query, &payload.params.0)
                .await
                .map(IntoValue::into_value)
                .map_err(AppError::from)
        })
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use semantic_data::{
        Object,
        schema::{ClassType, DbOpenMode, Meta},
    };
    use semantic_db_core::{Db, catalog::Catalog};
    use semantic_vdb::{
        AcceptedScan, CancellationToken, DatabaseDescriptor, EntityStream, ScanRequest,
        VirtualDatabase, VirtualDatabasePlugin,
    };

    use super::*;
    use crate::{Principal, SemanticApp};

    #[derive(Clone)]
    struct CommandFixture;

    fn fixture_schema() -> DatabaseSchema {
        DatabaseSchema {
            classes: vec![ClassType {
                id: "command_fixture:Item".into(),
                name: "Item".into(),
                inherits: None,
                extends: vec![],
                strict_schema: true,
                creatable_in_ui: None,
                include_in_ui_listings: None,
                attributes: Default::default(),
                constraints: vec![],
                meta: Meta::default(),
            }],
            ..Default::default()
        }
    }

    #[async_trait]
    impl VirtualDatabase for CommandFixture {
        async fn describe(&self) -> Result<DatabaseDescriptor, semantic_vdb::VdbError> {
            Ok(DatabaseDescriptor {
                title: "Command fixture".into(),
                description: None,
                schema: fixture_schema(),
                schema_revision: "1".into(),
                allow_untyped: false,
            })
        }
        fn schema_revision(&self) -> String {
            "1".into()
        }
        fn scan(
            &self,
            _request: ScanRequest,
            _plan: AcceptedScan,
            _bindings: Object,
            _cancellation: CancellationToken,
        ) -> EntityStream {
            let mut row = Object::new();
            row.insert("id", Value::String("one".into()));
            row.insert("type", Value::String("command_fixture:Item".into()));
            Box::pin(futures_util::stream::once(async move { Ok(row) }))
        }
    }

    #[tokio::test]
    async fn list_schema_and_explain_expose_only_runtime_definitions() {
        let temp = tempfile::tempdir().unwrap();
        let db: Arc<dyn SemanticDb> = Arc::new(Db::new(
            semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
                .unwrap(),
        ));
        db.query_data(
            semantic_data::query::DdlQuery {
                batch: semantic_data::query::DdlBatch {
                    operations: vec![semantic_data::query::DdlOperation::UpsertCollection {
                        name: "fx2".into(),
                        kind: semantic_data::query::DdlCollectionKind::Polymorphic,
                        integrity_mode: semantic_data::query::IntegrityMode::Permissive,
                    }],
                },
            }
            .into(),
        )
        .await
        .unwrap();
        let mut catalog = Catalog::new();
        catalog.upsert_package(semantic_data::bundles::query::package());
        catalog.upsert_package(semantic_vdb::package());
        let plugin = VirtualDatabasePlugin::new(
            semantic_plugin::PluginManifest {
                id: "fx".into(),
                revision: "1".into(),
                title: "Command fixture".into(),
                exports: vec![
                    semantic_vdb::implementation_descriptor(&catalog, "database").unwrap(),
                ],
                configuration_schema: None,
                source_bindings: Default::default(),
            },
            |_context| async { Ok(CommandFixture) },
        );
        let app = SemanticApp::builder()
            .with_default_scope(DbScopeId::new("main"), db.clone())
            .register_plugin(plugin)
            .unwrap()
            .register_builtin_commands()
            .unwrap()
            .build()
            .unwrap();
        let ctx = AppRequestContext {
            app: app.clone(),
            principal: Principal::system(),
            session: None,
            request_scope: None,
        };
        let plugins = app
            .plugins(&ctx.principal, DbScopeId::new("main"))
            .await
            .unwrap();
        let mut activation = plugins
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|activation| activation.id == "fx")
            .unwrap();
        activation.id = "fx2".into();
        plugins.configure(activation).await.unwrap();
        let Value::List(entries) = app
            .call(
                ctx.clone(),
                "semantic.vdb.list",
                Value::Object(Object::new()),
            )
            .await
            .unwrap()
        else {
            panic!("list")
        };
        assert_eq!(entries.len(), 2);
        let entry = |name: &str| {
            entries
                .iter()
                .find(|entry| entry.get_field("name").and_then(Value::as_str) == Some(name))
                .unwrap()
        };
        assert_eq!(entry("fx").get_field("available"), Some(&Value::Bool(true)));
        assert_eq!(
            entry("fx2").get_field("available"),
            Some(&Value::Bool(false))
        );
        assert!(
            entry("fx2")
                .get_field("reason")
                .and_then(Value::as_str)
                .unwrap()
                .contains("local collection")
        );
        let mut payload = Object::new();
        payload.insert("name", Value::String("fx".into()));
        let schema = app
            .call(
                ctx.clone(),
                "semantic.vdb.schema",
                Value::Object(payload.clone()),
            )
            .await
            .unwrap();
        assert_eq!(
            DatabaseSchema::from_value(schema).unwrap(),
            fixture_schema()
        );
        payload.insert("name", Value::String("fx2".into()));
        assert!(
            format!(
                "{:?}",
                app.call(ctx.clone(), "semantic.vdb.schema", Value::Object(payload))
                    .await
                    .unwrap_err()
            )
            .contains("unavailable")
        );
        let mut payload = Object::new();
        payload.insert(
            "query",
            Value::String("SELECT id FROM fx WHERE id = 'one'".into()),
        );
        let explain = app
            .call(
                ctx.clone(),
                "semantic.vdb.explain",
                Value::Object(payload.clone()),
            )
            .await
            .unwrap();
        let Value::List(leaves) = explain.get_field("leaves").unwrap() else {
            panic!("leaves")
        };
        assert_eq!(leaves.len(), 1);
        assert_eq!(
            leaves[0].get_field("collection").and_then(Value::as_str),
            Some("fx")
        );
        let Value::List(filters) = leaves[0].get_field("filters").unwrap() else {
            panic!("filters")
        };
        assert_eq!(filters.len(), 1);
        assert_eq!(
            filters[0].get_field("support").and_then(Value::as_str),
            Some("unsupported")
        );
        payload.insert("query", Value::String("SELECT id FROM fx2".into()));
        assert!(
            format!(
                "{:?}",
                app.call(ctx.clone(), "semantic.vdb.explain", Value::Object(payload))
                    .await
                    .unwrap_err()
            )
            .contains("no virtual database")
        );
        assert!(
            db.catalog()
                .await
                .unwrap()
                .class_id("command_fixture:Item")
                .is_none()
        );
        app.shutdown().await.unwrap();
    }
}
