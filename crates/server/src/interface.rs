//! Streaming application WebSocket endpoint with existing principal/scope resolution.
use crate::router::{ServerState, scope_from_parts};
use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use http::HeaderMap;
use semantic_rpc::interface::{ExportRouter, InterfaceImplementation};
use semantic_rpc::plugin::ExportMatch;
use semantic_rpc_core::interface::InvocationError;
use semantic_rpc_core::interface_protocol::InterfaceMessage;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// Both exports share one request context, so they share the session scope.
fn implementation(
    context: semantic_app::AppRequestContext,
) -> Result<Arc<dyn InterfaceImplementation>, InvocationError> {
    Ok(Arc::new(
        ExportRouter::new()
            .with(semantic_app::interface::implementation(context.clone())?)?
            .with(semantic_app::interface::command_implementation(context)?)?,
    ))
}

pub async fn handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let principal = match state.resolver.resolve_ws(&headers).await {
        Ok(principal) => principal,
        Err(error) => return (http::StatusCode::UNAUTHORIZED, error.to_string()).into_response(),
    };
    let subprotocol = "semantic.interface.v1";
    if !headers
        .get_all("Sec-WebSocket-Protocol")
        .iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|value| value.trim() == subprotocol)
    {
        return (
            http::StatusCode::BAD_REQUEST,
            "semantic.interface.v1 subprotocol required",
        )
            .into_response();
    }
    // Like any other connection, the interface session owns a scope selection
    // that `semantic.scope.use` changes for its subsequent calls.
    let session = state.app.new_session(format!(
        "interface-{}",
        NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
    ));
    if let Some(scope_id) = scope_from_parts(&headers, &query, &state.config) {
        session.set_current_scope(Some(scope_id)).await;
    }
    let context = semantic_app::AppRequestContext {
        app: state.app,
        principal,
        session: Some(session),
        request_scope: None,
    };
    let implementation = match implementation(context) {
        Ok(implementation) => implementation,
        Err(error) => {
            return (http::StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    ws.protocols([subprotocol])
        .on_upgrade(move |socket| async move {
            let (mut sink, mut source) = socket.split();
            let (outgoing, mut output) = mpsc::unbounded_channel::<InterfaceMessage>();
            let (input, incoming) = mpsc::unbounded_channel();
            let writer_errors = input.clone();
            let writer = tokio::spawn(async move {
                while let Some(message) = output.recv().await {
                    let text = match serde_json::to_string(&message) {
                        Ok(text) => text,
                        Err(error) => {
                            let _ = writer_errors.send(Err(failure(error)));
                            break;
                        }
                    };
                    if let Err(error) = sink.send(Message::Text(text.into())).await {
                        let _ = writer_errors.send(Err(failure(error)));
                        break;
                    }
                }
                let _ = sink.close().await;
            });
            let reader = tokio::spawn(async move {
                while let Some(message) = source.next().await {
                    let result = match message {
                        Ok(Message::Text(text)) => serde_json::from_str(&text).map_err(failure),
                        Ok(Message::Ping(_) | Message::Pong(_)) => continue,
                        Ok(_) => Err(failure("WebSocket closed or unsupported binary data")),
                        Err(error) => Err(failure(error)),
                    };
                    let terminal = result.is_err();
                    if input.send(result).is_err() || terminal {
                        return;
                    }
                }
                let _ = input.send(Err(failure("WebSocket disconnected")));
            });
            if let Ok(session) = semantic_rpc::plugin::accept(
                incoming,
                outgoing,
                implementation,
                None,
                ExportMatch::RequiredSubset,
                |_| async { Ok(()) },
            )
            .await
            {
                session.closed().await;
            }
            reader.abort();
            let _ = writer.await;
        })
        .into_response()
}
fn failure(error: impl ToString) -> InvocationError {
    InvocationError::new("connection_lost", error.to_string())
}
