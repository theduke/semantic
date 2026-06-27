use futures::future::LocalBoxFuture;
use semantic_app::{AppRequestContext, AppSession, DbScopeId, Principal, SemanticApp};
use semantic_data::value::Value;
use semantic_rpc::{RpcClientDyn, RpcClientError, RpcRequest, client::resolve_response};
use std::sync::Arc;

#[derive(Clone)]
pub struct EmbeddedRpcClient {
    app: SemanticApp,
    session: Arc<AppSession>,
    principal: Principal,
    request_scope: Option<DbScopeId>,
}

impl EmbeddedRpcClient {
    pub fn new(
        app: SemanticApp,
        session: Arc<AppSession>,
        principal: Principal,
        request_scope: Option<DbScopeId>,
    ) -> Self {
        Self {
            app,
            session,
            principal,
            request_scope,
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
        let request_scope = self.request_scope.clone();
        Box::pin(async move {
            let ctx = AppRequestContext {
                app: app.clone(),
                principal,
                session: Some(session),
                request_scope,
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
    let backend =
        semantic_db_redb::open_backend(db_path, semantic_data::schema::DbOpenMode::AutoCreate)
            .map_err(|err| err.to_string())?;
    let db = Arc::new(semantic_db_core::Db::new(backend));
    let scope_id = DbScopeId::new("local");
    let app = SemanticApp::builder()
        .with_default_scope(scope_id.clone(), db)
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
            Some(scope_id.clone()),
        )),
        scope_id.to_string(),
    ))
}
