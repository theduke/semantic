//! Plain ws:// plugin adapter. One text message contains one interface envelope.
use super::{ProviderConnection, error, negotiate};
use crate::interface::{ImplementationDescriptor, InvocationError, session::Session};
use futures::{SinkExt, StreamExt};
use semantic_data::value::Value;
use semantic_rpc_core::interface_protocol::{InterfaceMessage, PLUGIN_SUBPROTOCOL};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

pub async fn connect(
    url: &str,
    exports: Vec<ImplementationDescriptor>,
    revision: Option<String>,
    configuration: Value,
) -> Result<ProviderConnection, InvocationError> {
    connect_with_cancellation(
        url,
        exports,
        revision,
        configuration,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
}

pub async fn connect_with_cancellation(
    url: &str,
    exports: Vec<ImplementationDescriptor>,
    revision: Option<String>,
    configuration: Value,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<ProviderConnection, InvocationError> {
    if !url.starts_with("ws://") {
        return Err(InvocationError::new(
            "provider_unavailable",
            "only plain ws:// plugin endpoints are supported",
        ));
    }
    connect_protocol_cancellable(
        url,
        exports,
        revision,
        configuration,
        PLUGIN_SUBPROTOCOL,
        cancellation,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::{
        InterfaceImplementation, InvocationContext, InvocationFuture, InvocationOutput,
        ValidatedInvocation,
    };

    struct Fixture;
    impl InterfaceImplementation for Fixture {
        fn descriptors(&self) -> &[ImplementationDescriptor] {
            &[]
        }
        fn invoke<'a>(
            &'a self,
            _: ValidatedInvocation,
            _: InvocationContext,
        ) -> InvocationFuture<'a> {
            Box::pin(async { Ok(InvocationOutput::Values(vec![Value::U64(u64::MAX)])) })
        }
    }

    #[tokio::test]
    async fn loopback_upgrade_handshake_call_and_shutdown() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_hdr_async(tcp, |request: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                assert_eq!(request.headers()["Sec-WebSocket-Protocol"], PLUGIN_SUBPROTOCOL);
                response.headers_mut().insert("Sec-WebSocket-Protocol", PLUGIN_SUBPROTOCOL.parse().unwrap());
                Ok(response)
            }).await.unwrap();
            let (mut sink, mut source) = socket.split();
            let (outgoing, mut output) = mpsc::unbounded_channel::<InterfaceMessage>();
            let (input, incoming) = mpsc::unbounded_channel();
            let reader = tokio::spawn(async move {
                while let Some(Ok(Message::Text(text))) = source.next().await {
                    if input
                        .send(serde_json::from_str(&text).map_err(error))
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let writer = tokio::spawn(async move {
                while let Some(message) = output.recv().await {
                    if sink
                        .send(Message::Text(
                            serde_json::to_string(&message).unwrap().into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let session = crate::plugin::accept(
                incoming,
                outgoing,
                Arc::new(Fixture),
                None,
                |value| async move {
                    assert_eq!(value, Value::Bool(true));
                    Ok(())
                },
            )
            .await
            .unwrap();
            session.closed().await;
            writer.await.unwrap();
            reader.abort();
        });
        let connection = connect(&format!("ws://{address}"), vec![], None, Value::Bool(true))
            .await
            .unwrap();
        let output = connection
            .implementation
            .invoke(
                ValidatedInvocation {
                    export: "fixture".into(),
                    method: "values".into(),
                    arguments: vec![],
                },
                InvocationContext::default(),
            )
            .await
            .unwrap();
        let InvocationOutput::Values(values) = output else {
            panic!("values")
        };
        assert_eq!(values, vec![Value::U64(u64::MAX)]);
        connection.shutdown().await.unwrap();
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_during_negotiation_closes_owned_transport() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (hello, received_hello) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(tcp, |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert("Sec-WebSocket-Protocol", PLUGIN_SUBPROTOCOL.parse().unwrap());
                Ok(response)
            }).await.unwrap();
            assert!(matches!(socket.next().await, Some(Ok(Message::Text(_)))));
            hello.send(()).unwrap();
            // Do not answer Hello. The only way forward is host cancellation.
            assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        });
        let cancellation = tokio_util::sync::CancellationToken::new();
        let connecting = {
            let cancellation = cancellation.clone();
            tokio::spawn(async move {
                connect_with_cancellation(
                    &format!("ws://{address}"),
                    vec![],
                    None,
                    Value::Null,
                    cancellation,
                )
                .await
            })
        };
        received_hello.await.unwrap();
        cancellation.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), connecting)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(error) if error.code == "cancelled"));
        tokio::time::timeout(std::time::Duration::from_secs(5), peer)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn rejects_wrong_scheme_and_missing_subprotocol() {
        assert!(
            connect("wss://localhost", vec![], None, Value::Null)
                .await
                .is_err()
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let _socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        });
        assert!(
            connect(&format!("ws://{address}"), vec![], None, Value::Null)
                .await
                .is_err()
        );
        peer.await.unwrap();
    }
}

#[cfg(any(feature = "client-http-native", feature = "client-http-web"))]
pub(crate) async fn connect_protocol(
    url: &str,
    exports: Vec<ImplementationDescriptor>,
    revision: Option<String>,
    configuration: Value,
    subprotocol: &str,
) -> Result<ProviderConnection, InvocationError> {
    connect_protocol_cancellable(
        url,
        exports,
        revision,
        configuration,
        subprotocol,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
}

async fn connect_protocol_cancellable(
    url: &str,
    exports: Vec<ImplementationDescriptor>,
    revision: Option<String>,
    configuration: Value,
    subprotocol: &str,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<ProviderConnection, InvocationError> {
    let mut request = url.into_client_request().map_err(error)?;
    if request
        .uri()
        .authority()
        .is_some_and(|authority| authority.as_str().contains('@'))
    {
        return Err(InvocationError::new(
            "provider_unavailable",
            "plugin WebSocket credentials are not supported",
        ));
    }
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        subprotocol.parse().map_err(error)?,
    );
    let (socket, response) = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(InvocationError::new("cancelled", "Plugin startup cancelled")),
        result = connect_async(request) => result.map_err(error)?,
    };
    if response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok())
        != Some(subprotocol)
    {
        return Err(InvocationError::new(
            "interface_incompatible",
            "missing or incorrect plugin WebSocket subprotocol",
        ));
    }
    let (mut sink, mut source) = socket.split();
    let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<InterfaceMessage>();
    let (incoming_tx, mut incoming) = mpsc::unbounded_channel();
    let read_tx = incoming_tx.clone();
    let reader = tokio::spawn(async move {
        while let Some(message) = source.next().await {
            let result = match message {
                Ok(Message::Text(text)) => serde_json::from_str(&text).map_err(error),
                Ok(Message::Ping(_) | Message::Pong(_)) => continue,
                Ok(Message::Close(_)) => Err(error("plugin WebSocket closed")),
                Ok(_) => Err(InvocationError::new(
                    "protocol_violation",
                    "unexpected binary WebSocket message",
                )),
                Err(e) => Err(error(e)),
            };
            let terminal = result.is_err();
            if read_tx.send(result).is_err() || terminal {
                return;
            }
        }
        let _ = read_tx.send(Err(error("plugin WebSocket disconnected")));
    });
    let writer = tokio::spawn(async move {
        while let Some(message) = outgoing_rx.recv().await {
            let message = match serde_json::to_string(&message) {
                Ok(text) => Message::Text(text.into()),
                Err(e) => {
                    let _ = incoming_tx.send(Err(error(e)));
                    break;
                }
            };
            if let Err(e) = sink.send(message).await {
                let _ = incoming_tx.send(Err(error(e)));
                break;
            }
        }
        let _ = sink.close().await;
    });
    let result = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(InvocationError::new("cancelled", "Plugin startup cancelled")),
        result = negotiate(&mut incoming, &outgoing, &exports, revision, configuration) => result,
    };
    if let Err(failure) = result {
        drop(outgoing);
        let _ = writer.await;
        reader.abort();
        return Err(failure);
    }
    let session = Session::start(incoming, outgoing, None, exports);
    Ok(ProviderConnection {
        implementation: Arc::new(session.clone()),
        session,
        cleanup: Box::pin(async move {
            writer.await.map_err(error)?;
            reader.abort();
            Ok(())
        }),
    })
}
