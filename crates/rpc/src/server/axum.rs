use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};

use crate::registry::RpcRegistry;
use semantic_rpc_core::error::RpcError;
use semantic_rpc_core::protocol::{RpcRequest, RpcResponse};

pub struct RpcState<Ctx, E> {
    pub registry: Arc<RpcRegistry<Ctx, E>>,
    pub ctx: Arc<Ctx>,
}

impl<Ctx, E> Clone for RpcState<Ctx, E> {
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            ctx: Arc::clone(&self.ctx),
        }
    }
}

impl<Ctx, E> RpcState<Ctx, E> {
    pub fn new(registry: Arc<RpcRegistry<Ctx, E>>, ctx: Arc<Ctx>) -> Self {
        Self { registry, ctx }
    }
}

pub fn rpc_router<Ctx, E>(state: RpcState<Ctx, E>) -> Router
where
    Ctx: Send + Sync + 'static,
    E: Into<RpcError> + 'static,
{
    Router::new()
        .route("/api/v1/rpc", post(rpc_http_handler::<Ctx, E>))
        .with_state(state)
}

pub async fn rpc_http_handler<Ctx, E>(
    State(state): State<RpcState<Ctx, E>>,
    Json(request): Json<RpcRequest>,
) -> Json<RpcResponse>
where
    Ctx: Send + Sync + 'static,
    E: Into<RpcError> + 'static,
{
    Json(state.registry.invoke(state.ctx.as_ref(), request).await)
}
