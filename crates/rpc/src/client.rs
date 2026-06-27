use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

use semantic_data::value::Value;

use crate::command::RpcCommandSpec;
use crate::convert::{RpcDecode, RpcEncode};
use crate::error::RpcClientError;
use crate::protocol::{RpcRequest, RpcResponse, RpcResult};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_request_id() -> u64 {
    NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

pub fn request(command: impl Into<String>, payload: Value) -> RpcRequest {
    RpcRequest {
        id: next_request_id(),
        command: command.into(),
        payload,
    }
}

pub fn resolve_response(response: RpcResponse) -> Result<Value, RpcClientError> {
    match response.result {
        RpcResult::Ok(value) => Ok(value),
        RpcResult::Err(err) => Err(err.into()),
    }
}

pub async fn invoke_typed<C, F, Fut>(
    payload: C::Payload,
    invoke: F,
) -> Result<C::Output, RpcClientError>
where
    C: RpcCommandSpec,
    F: FnOnce(String, Value) -> Fut,
    Fut: Future<Output = Result<Value, RpcClientError>>,
{
    let payload = payload
        .encode_rpc()
        .map_err(|err| RpcClientError::Encode(err.message))?;
    let output = invoke(C::NAME.to_owned(), payload).await?;
    C::Output::decode_rpc(output).map_err(|err| RpcClientError::Decode(err.message))
}
