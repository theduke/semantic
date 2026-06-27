use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};

use crate::protocol::{RpcRequest, RpcResponse};
use crate::registry::RpcRegistry;

pub struct RpcState<Ctx> {
    pub registry: Arc<RpcRegistry<Ctx>>,
    pub ctx: Arc<Ctx>,
}

impl<Ctx> Clone for RpcState<Ctx> {
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            ctx: Arc::clone(&self.ctx),
        }
    }
}

impl<Ctx> RpcState<Ctx> {
    pub fn new(registry: Arc<RpcRegistry<Ctx>>, ctx: Arc<Ctx>) -> Self {
        Self { registry, ctx }
    }
}

pub fn rpc_router<Ctx>(state: RpcState<Ctx>) -> Router
where
    Ctx: Send + Sync + 'static,
{
    Router::new()
        .route("/rpc", post(rpc_http_handler::<Ctx>))
        .with_state(state)
}

pub async fn rpc_http_handler<Ctx>(
    State(state): State<RpcState<Ctx>>,
    Json(request): Json<RpcRequest>,
) -> Json<RpcResponse>
where
    Ctx: Send + Sync + 'static,
{
    Json(state.registry.invoke(state.ctx.as_ref(), request).await)
}
