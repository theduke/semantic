use std::future::Future;
#[cfg(feature = "client")]
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "client")]
use futures::future::LocalBoxFuture;
use semantic_data::value::Value;

use crate::command::RpcCommandSpec;
use crate::convert::{RpcDecode, RpcEncode};
use crate::error::RpcClientError;
#[cfg(feature = "client")]
use crate::file::{FileUploadProgressSender, FileUploadRequest, FileUploadResponse};
use crate::protocol::{RpcRequest, RpcResponse, RpcResult};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "client")]
#[derive(Clone)]
pub struct RpcClient {
    inner: Rc<dyn RpcClientDyn>,
}

#[cfg(feature = "client")]
pub trait RpcClientDyn: 'static {
    fn invoke_value(
        &self,
        command: String,
        payload: Value,
    ) -> LocalBoxFuture<'static, std::result::Result<Value, RpcClientError>>;

    fn upload_file(
        &self,
        _request: FileUploadRequest,
        _progress: Option<FileUploadProgressSender>,
    ) -> LocalBoxFuture<'static, std::result::Result<FileUploadResponse, RpcClientError>> {
        Box::pin(async move {
            Err(RpcClientError::Transport(
                "file upload is not supported by this RPC client".to_string(),
            ))
        })
    }

    fn file_url(&self, _id: &str) -> Option<String> {
        None
    }
}

#[cfg(feature = "client")]
impl RpcClient {
    pub fn new(client: impl RpcClientDyn) -> Self {
        Self {
            inner: Rc::new(client),
        }
    }

    pub fn from_rc(client: Rc<dyn RpcClientDyn>) -> Self {
        Self { inner: client }
    }

    pub async fn invoke_value(
        &self,
        command: impl Into<String>,
        payload: Value,
    ) -> std::result::Result<Value, RpcClientError> {
        self.inner.invoke_value(command.into(), payload).await
    }

    pub async fn invoke<C>(
        &self,
        payload: C::Payload,
    ) -> std::result::Result<C::Output, RpcClientError>
    where
        C: RpcCommandSpec,
    {
        invoke_typed::<C, _, _>(payload, |command, payload| {
            self.invoke_value(command, payload)
        })
        .await
    }

    pub async fn upload_file(
        &self,
        request: FileUploadRequest,
        progress: Option<FileUploadProgressSender>,
    ) -> std::result::Result<FileUploadResponse, RpcClientError> {
        self.inner.upload_file(request, progress).await
    }

    pub fn file_url(&self, id: &str) -> Option<String> {
        self.inner.file_url(id)
    }
}

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
