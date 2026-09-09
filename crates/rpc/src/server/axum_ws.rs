use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use semantic_rpc_core::error::RpcError;
use semantic_rpc_core::protocol::{RpcRequest, RpcResponse};

pub use super::axum::RpcState;

pub fn rpc_ws_router<Ctx>(state: RpcState<Ctx>) -> Router
where
    Ctx: Send + Sync + 'static,
{
    Router::new()
        .route("/api/v1/rpc/ws", get(rpc_ws_handler::<Ctx>))
        .with_state(state)
}

pub async fn rpc_ws_handler<Ctx>(
    State(state): State<RpcState<Ctx>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse
where
    Ctx: Send + Sync + 'static,
{
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket<Ctx>(socket: WebSocket, state: RpcState<Ctx>)
where
    Ctx: Send + Sync + 'static,
{
    let (mut sender, mut receiver) = socket.split();
    let (response_sender, mut response_receiver) = mpsc::unbounded_channel::<RpcResponse>();

    let writer = tokio::spawn(async move {
        while let Some(response) = response_receiver.recv().await {
            if send_response(&mut sender, response).await.is_err() {
                break;
            }
        }
    });

    while let Some(message) = receiver.next().await {
        let request = match message {
            Ok(Message::Text(text)) => match serde_json::from_str::<RpcRequest>(&text) {
                Ok(request) => request,
                Err(err) => {
                    let _ = response_sender
                        .send(RpcResponse::err(0, RpcError::protocol(err.to_string())));
                    continue;
                }
            },
            Ok(Message::Binary(bytes)) => match serde_json::from_slice::<RpcRequest>(&bytes) {
                Ok(request) => request,
                Err(err) => {
                    let _ = response_sender
                        .send(RpcResponse::err(0, RpcError::protocol(err.to_string())));
                    continue;
                }
            },
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
            Err(err) => {
                let _ =
                    response_sender.send(RpcResponse::err(0, RpcError::protocol(err.to_string())));
                break;
            }
        };

        let state = state.clone();
        let response_sender = response_sender.clone();
        tokio::spawn(async move {
            let response = state.registry.invoke(state.ctx.as_ref(), request).await;
            let _ = response_sender.send(response);
        });
    }

    drop(response_sender);
    let _ = writer.await;
}

async fn send_response(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    response: RpcResponse,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(&response)
        .map_err(|err| axum::Error::new(std::io::Error::other(err)))?;
    sender.send(Message::Text(text.into())).await
}
