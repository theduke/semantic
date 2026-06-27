use gloo_net::http::Request;
use semantic_data::value::Value;

use crate::client::{RpcClient, RpcClientDyn, request, resolve_response};
use crate::command::RpcCommandSpec;
use crate::error::RpcClientError;
use crate::protocol::RpcResponse;

#[derive(Clone)]
pub struct HttpRpcClient {
    endpoint: String,
}

impl HttpRpcClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    pub async fn invoke_value(
        &self,
        command: impl Into<String>,
        payload: Value,
    ) -> std::result::Result<Value, RpcClientError> {
        let response = Request::post(&self.endpoint)
            .json(&request(command, payload))
            .map_err(|err| RpcClientError::Protocol(err.to_string()))?
            .send()
            .await
            .map_err(|err| RpcClientError::Transport(err.to_string()))?;

        let status = response.status();
        if !(200..300).contains(&status) {
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

    pub async fn invoke<C>(
        &self,
        payload: C::Payload,
    ) -> std::result::Result<C::Output, RpcClientError>
    where
        C: RpcCommandSpec,
    {
        crate::client::invoke_typed::<C, _, _>(payload, |command, payload| {
            self.invoke_value(command, payload)
        })
        .await
    }
}

impl RpcClientDyn for HttpRpcClient {
    fn invoke_value(
        &self,
        command: String,
        payload: Value,
    ) -> futures::future::LocalBoxFuture<'static, std::result::Result<Value, RpcClientError>> {
        let client = self.clone();
        Box::pin(async move { client.invoke_value(command, payload).await })
    }
}

impl From<HttpRpcClient> for RpcClient {
    fn from(value: HttpRpcClient) -> Self {
        RpcClient::new(value)
    }
}
