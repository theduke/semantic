use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use http::HeaderMap;
use semantic_app::{AppRequestContext, DbScopeId, SemanticApp};

use semantic_rpc_core::{RpcRequest, RpcResponse};
use tokio::net::TcpListener;

use crate::{NoAuthPrincipalResolver, PrincipalResolver, ServerConfig, ServerError};

#[derive(Clone)]
pub struct SemanticServer {
    app: SemanticApp,
    config: ServerConfig,
    resolver: Arc<dyn PrincipalResolver>,
}

#[derive(Clone)]
pub(crate) struct ServerState {
    pub app: SemanticApp,
    pub config: ServerConfig,
    pub resolver: Arc<dyn PrincipalResolver>,
}

impl SemanticServer {
    pub fn new(app: SemanticApp) -> Self {
        Self {
            app,
            config: ServerConfig::default(),
            resolver: Arc::new(NoAuthPrincipalResolver),
        }
    }

    pub fn with_config(mut self, config: ServerConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_principal_resolver(mut self, resolver: impl PrincipalResolver) -> Self {
        self.resolver = Arc::new(resolver);
        self
    }

    pub fn router(&self) -> Router {
        let state = ServerState {
            app: self.app.clone(),
            config: self.config.clone(),
            resolver: Arc::clone(&self.resolver),
        };
        let file_get_path = format!("{}/{{id}}", self.config.file_api_prefix);
        let router = Router::new()
            .route(&self.config.rpc_path, post(rpc_http_handler))
            .route(
                &self.config.file_api_prefix,
                post(crate::file::upload_handler),
            )
            .route(&file_get_path, get(crate::file::download_handler))
            .route(
                &self.config.ws_path,
                axum::routing::get(crate::ws::rpc_ws_handler),
            )
            .with_state(state);

        #[cfg(feature = "embed-ui")]
        let router = router.fallback_service(get(crate::ui::handler));

        router
    }

    pub async fn serve(self, listener: TcpListener) -> std::result::Result<(), ServerError> {
        axum::serve(listener, self.router()).await?;
        Ok(())
    }
}

pub async fn rpc_http_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
    Json(request): Json<RpcRequest>,
) -> Json<RpcResponse> {
    let principal = match state.resolver.resolve_http(&headers).await {
        Ok(principal) => principal,
        Err(err) => return Json(RpcResponse::err(request.id, err.into())),
    };
    let request_scope = scope_from_parts(&headers, &query, &state.config);
    let ctx = AppRequestContext {
        app: state.app.clone(),
        principal,
        session: None,
        request_scope,
    };
    Json(state.app.invoke(ctx, request).await)
}

pub(crate) fn scope_from_parts(
    headers: &HeaderMap,
    query: &BTreeMap<String, String>,
    config: &ServerConfig,
) -> Option<DbScopeId> {
    headers
        .get(&config.scope_header)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(DbScopeId::new)
        .or_else(|| {
            query
                .get("scope")
                .filter(|value| !value.is_empty())
                .cloned()
                .map(DbScopeId::new)
        })
}
