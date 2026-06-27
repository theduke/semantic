use futures::future::LocalBoxFuture;
#[cfg(feature = "desktop")]
use semantic_app::{AppError, DbOpenRequest, DbProvider};
use semantic_app::{AppRequestContext, AppSession, DbScopeId, Principal, SemanticApp};
#[cfg(feature = "desktop")]
use semantic_data::schema::DbOpenMode;
use semantic_data::value::Value;
use semantic_rpc::{RpcClientDyn, RpcClientError, RpcRequest, client::resolve_response};
use std::sync::Arc;

#[cfg(feature = "desktop")]
struct RedbDbProvider;

#[cfg(feature = "desktop")]
#[async_trait::async_trait]
impl DbProvider for RedbDbProvider {
    fn scheme(&self) -> &str {
        "redb"
    }

    async fn open(
        &self,
        request: DbOpenRequest,
        _principal: &Principal,
    ) -> std::result::Result<Arc<dyn semantic_app::SemanticDb>, AppError> {
        let path = request.uri.strip_prefix("redb://").ok_or_else(|| {
            AppError::InvalidRequest(format!("invalid redb uri '{}'", request.uri))
        })?;
        if path.is_empty() {
            return Err(AppError::InvalidRequest(
                "invalid redb uri: missing database path".to_string(),
            ));
        }
        let backend = semantic_db_redb::open_backend(path, request.mode)?;
        Ok(Arc::new(semantic_db_core::Db::new(backend)))
    }
}

#[derive(Clone)]
pub struct EmbeddedRpcClient {
    app: SemanticApp,
    session: Arc<AppSession>,
    principal: Principal,
    scope_id: DbScopeId,
}

impl EmbeddedRpcClient {
    pub fn new(
        app: SemanticApp,
        session: Arc<AppSession>,
        principal: Principal,
        scope_id: DbScopeId,
    ) -> Self {
        Self {
            app,
            session,
            principal,
            scope_id,
        }
    }
}

impl RpcClientDyn for EmbeddedRpcClient {
    fn invoke_value(
        &self,
        command: String,
        payload: Value,
    ) -> LocalBoxFuture<'static, std::result::Result<Value, RpcClientError>> {
        let app = self.app.clone();
        let session = Arc::clone(&self.session);
        let principal = self.principal.clone();
        let scope_id = self.scope_id.clone();
        Box::pin(async move {
            let ctx = AppRequestContext {
                app: app.clone(),
                principal,
                session: Some(session),
                request_scope: Some(scope_id),
            };
            let response = app
                .invoke(
                    ctx,
                    RpcRequest {
                        id: semantic_rpc::client::next_request_id(),
                        command,
                        payload,
                    },
                )
                .await;
            resolve_response(response)
        })
    }
}

#[cfg(feature = "desktop")]
pub fn build_embedded_client(
    db_path: impl AsRef<std::path::Path>,
) -> std::result::Result<(semantic_rpc::RpcClient, String), String> {
    let blob_uri = semantic_app::AppConfig::from_env().default_blob_uri()?;
    build_embedded_client_with_blob_store(db_path, blob_uri)
}

#[cfg(feature = "desktop")]
pub fn build_embedded_client_with_blob_store(
    db_path: impl AsRef<std::path::Path>,
    blob_uri: String,
) -> std::result::Result<(semantic_rpc::RpcClient, String), String> {
    let scope_id = DbScopeId::new("local");
    let db_uri = format!("redb://{}", db_path.as_ref().to_string_lossy());
    let app = SemanticApp::builder()
        .with_provider(RedbDbProvider)
        .with_default_scope_request(
            scope_id.clone(),
            DbOpenRequest {
                uri: db_uri,
                mode: DbOpenMode::AutoCreate,
            },
        )
        .with_default_object_store_request(
            scope_id.clone(),
            semantic_app::ObjectStoreId::new("default"),
            semantic_app::ObjectStoreOpenRequest { uri: blob_uri },
        )
        .register_builtin_commands()
        .map_err(|err| err.to_string())?
        .build()
        .map_err(|err| err.to_string())?;
    let session = app.new_session("semantic-ui");
    Ok((
        semantic_rpc::RpcClient::new(EmbeddedRpcClient::new(
            app,
            session,
            Principal::system(),
            scope_id.clone(),
        )),
        scope_id.to_string(),
    ))
}

#[cfg(all(test, feature = "desktop"))]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn embedded_client_loads_base_catalog() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("semantic-ui-standalone-{suffix}.redb"));
        let (client, scope_id) = build_embedded_client(&db_path).unwrap();

        let catalog = futures::executor::block_on(semantic_ui_core::ui_catalog::load_catalog(
            client,
            Some(scope_id),
        ))
        .unwrap();

        assert!(catalog.class_by_id("semantic.base.person").is_some());
        assert!(catalog.class_by_id("semantic.base.file").is_some());
    }
}
