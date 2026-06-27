use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use futures::channel::oneshot;
use futures::lock::Mutex;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::Message;
use gloo_net::websocket::futures::WebSocket;
use semantic_data::value::Value;
use wasm_bindgen_futures::spawn_local;

use crate::client::{request, resolve_response};
use crate::command::RpcCommandSpec;
use crate::error::RpcClientError;
use crate::protocol::RpcResponse;

type Pending = Rc<RefCell<BTreeMap<u64, oneshot::Sender<Result<Value, RpcClientError>>>>>;

#[derive(Clone)]
pub struct WsRpcClient {
    sender: Rc<Mutex<SplitSink<WebSocket, Message>>>,
    pending: Pending,
}

impl WsRpcClient {
    pub async fn connect(url: impl Into<String>) -> Result<Self, RpcClientError> {
        let socket = WebSocket::open(&url.into())
            .map_err(|err| RpcClientError::Transport(err.to_string()))?;
        let (sender, receiver) = socket.split();
        let pending = Rc::new(RefCell::new(BTreeMap::new()));

        spawn_local(read_responses(receiver, Rc::clone(&pending)));

        Ok(Self {
            sender: Rc::new(Mutex::new(sender)),
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

        self.pending.borrow_mut().insert(id, response_sender);

        let send_result = self.sender.lock().await.send(Message::Text(text)).await;
        if let Err(err) = send_result {
            self.pending.borrow_mut().remove(&id);
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

async fn read_responses(mut receiver: SplitStream<WebSocket>, pending: Pending) {
    while let Some(message) = receiver.next().await {
        let response = match message {
            Ok(Message::Text(text)) => serde_json::from_str::<RpcResponse>(&text)
                .map_err(|err| RpcClientError::Protocol(err.to_string())),
            Ok(Message::Bytes(bytes)) => serde_json::from_slice::<RpcResponse>(&bytes)
                .map_err(|err| RpcClientError::Protocol(err.to_string())),
            Err(err) => {
                fail_all_pending(&pending, RpcClientError::Transport(err.to_string()));
                return;
            }
        };

        match response {
            Ok(response) => {
                let sender = pending.borrow_mut().remove(&response.id);
                if let Some(sender) = sender {
                    let _ = sender.send(resolve_response(response));
                }
            }
            Err(err) => {
                fail_all_pending(&pending, err);
                return;
            }
        }
    }

    fail_all_pending(
        &pending,
        RpcClientError::Transport("websocket closed".to_string()),
    );
}

fn fail_all_pending(pending: &Pending, err: RpcClientError) {
    for (_, sender) in std::mem::take(&mut *pending.borrow_mut()) {
        let _ = sender.send(Err(RpcClientError::Transport(err.to_string())));
    }
}
