use semantic_data::value::Value;

use crate::client::{request, resolve_response};
use crate::command::RpcCommandSpec;
use crate::error::RpcClientError;
use crate::protocol::RpcResponse;

#[derive(Clone)]
pub struct HttpRpcClient {
    endpoint: String,
    client: reqwest::Client,
}

impl HttpRpcClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_client(endpoint: impl Into<String>, client: reqwest::Client) -> Self {
        Self {
            endpoint: endpoint.into(),
            client,
        }
    }

    pub async fn invoke_value(
        &self,
        command: impl Into<String>,
        payload: Value,
    ) -> Result<Value, RpcClientError> {
        let response = self
            .client
            .post(&self.endpoint)
            .json(&request(command, payload))
            .send()
            .await
            .map_err(|err| RpcClientError::Transport(err.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            return Err(RpcClientError::Transport(format!(
                "HTTP RPC request failed with status {status}"
            )));
        }

        let response = response
            .json::<RpcResponse>()
            .await
            .map_err(|err| RpcClientError::Protocol(err.to_string()))?;

        resolve_response(response)
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
