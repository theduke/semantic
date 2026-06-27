use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use http::HeaderMap;
use semantic_app::AppRequestContext;
use semantic_rpc::{RpcError, RpcRequest, RpcResponse};
use tokio::sync::mpsc;

use crate::router::{ServerState, scope_from_parts};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

pub async fn rpc_ws_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let principal = match state.resolver.resolve_ws(&headers).await {
        Ok(principal) => principal,
        Err(err) => return (http::StatusCode::UNAUTHORIZED, err.to_string()).into_response(),
    };
    let request_scope = scope_from_parts(&headers, &query, &state.config);
    ws.on_upgrade(move |socket| async move {
        let session = state.app.new_session(format!(
            "ws-{}",
            NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
        ));
        if let Some(scope_id) = request_scope {
            session.set_current_scope(Some(scope_id)).await;
        }
        handle_socket(socket, state, principal, session).await;
    })
    .into_response()
}

async fn handle_socket(
    socket: WebSocket,
    state: ServerState,
    principal: semantic_app::Principal,
    session: std::sync::Arc<semantic_app::AppSession>,
) {
    let (mut sender, mut receiver) = socket.split();
    let (response_sender, mut response_receiver) = mpsc::unbounded_channel::<RpcResponse>();

    let writer = tokio::spawn(async move {
        while let Some(response) = response_receiver.recv().await {
            let Ok(text) = serde_json::to_string(&response) else {
                break;
            };
            if sender.send(Message::Text(text.into())).await.is_err() {
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
        let principal = principal.clone();
        let session = std::sync::Arc::clone(&session);
        let response_sender = response_sender.clone();
        tokio::spawn(async move {
            let ctx = AppRequestContext {
                app: state.app.clone(),
                principal,
                session: Some(session),
                request_scope: None,
            };
            let response = state.app.invoke(ctx, request).await;
            let _ = response_sender.send(response);
        });
    }

    drop(response_sender);
    let _ = writer.await;
}
