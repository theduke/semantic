use semantic_app::AppRequestContext;
use semantic_base::server::{ConfigGet, ServerConfig};
use semantic_rpc_core::{RpcCommand, RpcError};

use crate::router::ServerState;

impl RpcCommand<ServerState> for ConfigGet {
    fn call<'a>(
        &'a self,
        _state: &'a ServerState,
        _payload: (),
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ServerConfig, RpcError>> + Send + 'a>,
    > {
        Box::pin(async move { Ok(ServerConfig::default()) })
    }
}

impl ServerState {
    pub(crate) async fn call(
        &self,
        ctx: AppRequestContext,
        command: &str,
        payload: semantic_data::value::Value,
    ) -> Result<semantic_data::value::Value, semantic_rpc_core::CallError<semantic_app::AppError>>
    {
        if let Some(handler) = self.commands.get(command) {
            handler.call_value(self, payload).await
        } else {
            self.app.call(ctx, command, payload).await
        }
    }

    pub(crate) async fn invoke(
        &self,
        ctx: AppRequestContext,
        request: semantic_rpc_core::RpcRequest,
    ) -> semantic_rpc_core::RpcResponse {
        match self.call(ctx, &request.command, request.payload).await {
            Ok(value) => semantic_rpc_core::RpcResponse::ok(request.id, value),
            Err(error) => semantic_rpc_core::RpcResponse::err(request.id, error.into()),
        }
    }
}
