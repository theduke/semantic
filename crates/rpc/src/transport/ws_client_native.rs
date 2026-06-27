use std::collections::BTreeMap;
use std::sync::Arc;

use futures::channel::oneshot;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use semantic_data::value::Value;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::client::{RpcClient, RpcClientDyn, request, resolve_response};
use crate::command::RpcCommandSpec;
use crate::error::RpcClientError;
use crate::protocol::RpcResponse;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Pending = Arc<Mutex<BTreeMap<u64, oneshot::Sender<Result<Value, RpcClientError>>>>>;

#[derive(Clone)]
pub struct WsRpcClient {
    sender: Arc<Mutex<SplitSink<Socket, Message>>>,
    pending: Pending,
}

impl WsRpcClient {
    pub async fn connect(url: impl Into<String>) -> Result<Self, RpcClientError> {
        let (socket, _) = connect_async(url.into())
            .await
            .map_err(|err| RpcClientError::Transport(err.to_string()))?;
        let (sender, receiver) = socket.split();
        let pending = Arc::new(Mutex::new(BTreeMap::new()));

        tokio::spawn(read_responses(receiver, Arc::clone(&pending)));

        Ok(Self {
            sender: Arc::new(Mutex::new(sender)),
            pending,
        })
    }

    pub async fn invoke_value(
        &self,
        command: impl Into<String>,
        payload: Value,
    ) -> Result<Value, RpcClientError> {
        let request = request(command, payload);
        let id = request.id;
        let text = serde_json::to_string(&request)
            .map_err(|err| RpcClientError::Protocol(err.to_string()))?;
        let (response_sender, response_receiver) = oneshot::channel();

        self.pending.lock().await.insert(id, response_sender);

        let send_result = self
            .sender
            .lock()
            .await
            .send(Message::Text(text.into()))
            .await;
        if let Err(err) = send_result {
            self.pending.lock().await.remove(&id);
            return Err(RpcClientError::Transport(err.to_string()));
        }

        response_receiver.await.map_err(|_| {
            RpcClientError::Transport("websocket response channel closed".to_string())
        })?
    }

    pub async fn invoke<C>(&self, payload: C::Payload) -> Result<C::Output, RpcClientError>
    where
        C: RpcCommandSpec,
    {
        crate::client::invoke_typed::<C, _, _>(payload, |command, payload| {
            self.invoke_value(command, payload)
        })
        .await
    }
}

impl RpcClientDyn for WsRpcClient {
    fn invoke_value(
        &self,
        command: String,
        payload: Value,
    ) -> futures::future::LocalBoxFuture<'static, std::result::Result<Value, RpcClientError>> {
        let client = self.clone();
        Box::pin(async move { client.invoke_value(command, payload).await })
    }
}

impl From<WsRpcClient> for RpcClient {
    fn from(value: WsRpcClient) -> Self {
        RpcClient::new(value)
    }
}

async fn read_responses(mut receiver: SplitStream<Socket>, pending: Pending) {
    while let Some(message) = receiver.next().await {
        let response = match message {
            Ok(Message::Text(text)) => serde_json::from_str::<RpcResponse>(&text)
                .map_err(|err| RpcClientError::Protocol(err.to_string())),
            Ok(Message::Binary(bytes)) => serde_json::from_slice::<RpcResponse>(&bytes)
                .map_err(|err| RpcClientError::Protocol(err.to_string())),
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => continue,
            Err(err) => Err(RpcClientError::Transport(err.to_string())),
        };

        match response {
            Ok(response) => {
                let sender = pending.lock().await.remove(&response.id);
                if let Some(sender) = sender {
                    let _ = sender.send(resolve_response(response));
                }
            }
            Err(err) => {
                fail_all_pending(&pending, err).await;
                break;
            }
        }
    }

    fail_all_pending(
        &pending,
        RpcClientError::Transport("websocket closed".to_string()),
    )
    .await;
}

async fn fail_all_pending(pending: &Pending, err: RpcClientError) {
    let mut pending = pending.lock().await;
    for (_, sender) in std::mem::take(&mut *pending) {
        let _ = sender.send(Err(RpcClientError::Transport(err.to_string())));
    }
}
