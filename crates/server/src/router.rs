use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use http::HeaderMap;
use semantic_app::{AppRequestContext, DbScopeId, SemanticApp};
use semantic_data::value::Value;
use semantic_data::value::serde::typed::TypedValue;

use semantic_rpc_core::{CallError, RpcError, RpcRequest, RpcResponse};
use tokio::net::TcpListener;

use crate::error::{call_error_response, server_error_response};
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
        let command_path = format!("{}/{{command}}", self.config.rpc_path.trim_end_matches('/'));
        let router = Router::new()
            .route(
                &self.config.rpc_path,
                post(rpc_http_handler)
                    .layer(DefaultBodyLimit::max(self.config.max_rpc_request_size)),
            )
            .route(
                &command_path,
                post(command_http_handler)
                    .layer(DefaultBodyLimit::max(self.config.max_rpc_request_size)),
            )
            .route(
                &semantic_rpc_core::interface_protocol::interface_ws_path(&self.config.rpc_path),
                get(crate::interface::handler),
            )
            .route(
                &self.config.file_api_prefix,
                post(crate::file::upload_handler),
            )
            .route(
                &file_get_path,
                get(crate::file::download_handler).delete(crate::file::delete_handler),
            )
            .with_state(state);

        #[cfg(feature = "embed-ui")]
        let router = router.fallback_service(get(crate::ui::handler));

        router
    }

    pub async fn serve(self, listener: TcpListener) -> std::result::Result<(), ServerError> {
        let result = axum::serve(listener, self.router()).await;
        self.app.shutdown().await?;
        result?;
        Ok(())
    }

    /// Drain HTTP requests and await cooperative job cleanup before returning.
    pub async fn serve_with_shutdown(
        self,
        listener: TcpListener,
        signal: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<(), ServerError> {
        let result = axum::serve(listener, self.router())
            .with_graceful_shutdown(signal)
            .await;
        self.app.shutdown().await?;
        result?;
        Ok(())
    }
}

pub async fn rpc_http_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
    Json(request): Json<RpcRequest>,
) -> Json<RpcResponse> {
    match request_context(&state, &headers, &query).await {
        Ok(ctx) => Json(state.app.invoke(ctx, request).await),
        Err(err) => Json(RpcResponse::err(request.id, err.into())),
    }
}

/// Invoke a single command named by the path, with the typed JSON payload as
/// the request body. An empty body is treated as a `Void` payload.
///
/// Responds with the typed JSON output value on success, or with the
/// [`RpcError`] and a status derived from the error on failure.
pub async fn command_http_handler(
    State(state): State<ServerState>,
    Path(command): Path<String>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
    body: Bytes,
) -> Response {
    let payload = if body.is_empty() {
        Value::Void
    } else {
        match serde_json::from_slice::<TypedValue>(&body) {
            Ok(TypedValue(value)) => value,
            Err(err) => {
                let err = RpcError::invalid_payload(format!("invalid JSON payload: {err}"));
                return call_error_response(CallError::InvalidPayload(err));
            }
        }
    };
    let ctx = match request_context(&state, &headers, &query).await {
        Ok(ctx) => ctx,
        Err(err) => return server_error_response(err),
    };
    match state.app.call(ctx, &command, payload).await {
        Ok(value) => Json(TypedValue(value)).into_response(),
        Err(err) => call_error_response(err),
    }
}

pub(crate) async fn request_context(
    state: &ServerState,
    headers: &HeaderMap,
    query: &BTreeMap<String, String>,
) -> Result<AppRequestContext, ServerError> {
    let principal = state.resolver.resolve_http(headers).await?;
    Ok(AppRequestContext {
        app: state.app.clone(),
        principal,
        session: None,
        request_scope: scope_from_parts(headers, query, &state.config),
    })
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
