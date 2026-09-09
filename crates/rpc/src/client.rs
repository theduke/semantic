use std::future::Future;
#[cfg(all(feature = "client", target_arch = "wasm32"))]
use std::rc::Rc;
#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
use futures::future::BoxFuture;
#[cfg(all(feature = "client", target_arch = "wasm32"))]
use futures::future::LocalBoxFuture;
use semantic_data::value::Value;

#[cfg(feature = "client")]
use crate::file::{
    FileDownloadByteStream, FileUploadProgressSender, FileUploadRequest, FileUploadResponse,
};
#[cfg(feature = "client")]
use bytes::Bytes;
use semantic_rpc_core::command::RpcCommandSpec;
use semantic_rpc_core::convert::{RpcDecode, RpcEncode};
use semantic_rpc_core::error::RpcClientError;
use semantic_rpc_core::protocol::{RpcRequest, RpcResponse, RpcResult};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "client")]
#[derive(Clone)]
pub struct RpcClient {
    #[cfg(target_arch = "wasm32")]
    inner: Rc<dyn RpcClientDyn>,
    #[cfg(not(target_arch = "wasm32"))]
    inner: Arc<dyn RpcClientDyn>,
}

#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
pub type RpcClientFuture<T> = BoxFuture<'static, T>;
#[cfg(all(feature = "client", target_arch = "wasm32"))]
pub type RpcClientFuture<T> = LocalBoxFuture<'static, T>;

#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
pub trait RpcClientThreadBounds: Send + Sync {}
#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
impl<T: Send + Sync> RpcClientThreadBounds for T {}
#[cfg(all(feature = "client", target_arch = "wasm32"))]
pub trait RpcClientThreadBounds {}
#[cfg(all(feature = "client", target_arch = "wasm32"))]
impl<T> RpcClientThreadBounds for T {}

#[cfg(feature = "client")]
impl PartialEq for RpcClient {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(target_arch = "wasm32")]
        return Rc::ptr_eq(&self.inner, &other.inner);
        #[cfg(not(target_arch = "wasm32"))]
        return Arc::ptr_eq(&self.inner, &other.inner);
    }
}

#[cfg(feature = "client")]
impl Eq for RpcClient {}

#[cfg(feature = "client")]
pub trait RpcClientDyn: RpcClientThreadBounds + 'static {
    fn invoke_value(
        &self,
        command: String,
        payload: Value,
    ) -> RpcClientFuture<std::result::Result<Value, RpcClientError>>;

    fn upload_file(
        &self,
        _request: FileUploadRequest,
        _progress: Option<FileUploadProgressSender>,
    ) -> RpcClientFuture<std::result::Result<FileUploadResponse, RpcClientError>> {
        Box::pin(async move {
            Err(RpcClientError::Transport(
                "file upload is not supported by this RPC client".to_string(),
            ))
        })
    }

    fn file_url(&self, _id: &str) -> Option<String> {
        None
    }

    fn stream_file_from(
        &self,
        _id: String,
        _scope_id: Option<String>,
        _offset: u64,
    ) -> RpcClientFuture<std::result::Result<FileDownloadByteStream, RpcClientError>> {
        Box::pin(async {
            Err(RpcClientError::Transport(
                "file downloads are not supported by this client".to_string(),
            ))
        })
    }

    #[cfg(feature = "client")]
    fn read_file_range(
        &self,
        _id: String,
        _scope_id: Option<String>,
        _offset: u64,
        _size: u32,
    ) -> RpcClientFuture<std::result::Result<Bytes, RpcClientError>> {
        Box::pin(async {
            Err(RpcClientError::Transport(
                "file downloads are not supported by this client".to_string(),
            ))
        })
    }
}

#[cfg(feature = "client")]
impl RpcClient {
    pub fn new(client: impl RpcClientDyn) -> Self {
        Self {
            #[cfg(target_arch = "wasm32")]
            inner: Rc::new(client),
            #[cfg(not(target_arch = "wasm32"))]
            inner: Arc::new(client),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn from_rc(client: Rc<dyn RpcClientDyn>) -> Self {
        Self { inner: client }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_arc(client: Arc<dyn RpcClientDyn>) -> Self {
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

    pub async fn stream_file_from(
        &self,
        id: impl Into<String>,
        scope_id: Option<String>,
        offset: u64,
    ) -> std::result::Result<FileDownloadByteStream, RpcClientError> {
        self.inner
            .stream_file_from(id.into(), scope_id, offset)
            .await
    }

    #[cfg(feature = "client")]
    pub async fn read_file_range(
        &self,
        id: impl Into<String>,
        scope_id: Option<String>,
        offset: u64,
        size: u32,
    ) -> std::result::Result<Bytes, RpcClientError> {
        self.inner
            .read_file_range(id.into(), scope_id, offset, size)
            .await
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
