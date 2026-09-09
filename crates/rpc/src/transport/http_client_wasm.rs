use gloo_net::http::Request;
use js_sys::Uint8Array;
use semantic_data::value::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::Closure;
use web_sys::{Blob, ProgressEvent, XmlHttpRequest};

use crate::client::{RpcClient, RpcClientDyn, request, resolve_response};
use crate::file::{
    FileUploadContent, FileUploadPhase, FileUploadProgressSender, FileUploadRequest,
    FileUploadResponse, derive_file_api_prefix, emit_progress,
};
use semantic_rpc_core::command::RpcCommandSpec;
use semantic_rpc_core::error::RpcClientError;
use semantic_rpc_core::protocol::RpcResponse;

#[derive(Clone)]
pub struct HttpRpcClient {
    endpoint: String,
    file_api_prefix: String,
}

impl HttpRpcClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into();
        Self {
            file_api_prefix: derive_file_api_prefix(&endpoint),
            endpoint,
        }
    }

    pub fn with_file_api_prefix(mut self, file_api_prefix: impl Into<String>) -> Self {
        self.file_api_prefix = file_api_prefix.into();
        self
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

    pub async fn upload_file(
        &self,
        request: FileUploadRequest,
        progress: Option<FileUploadProgressSender>,
    ) -> std::result::Result<FileUploadResponse, RpcClientError> {
        upload_file_xhr(self.file_api_prefix.clone(), request, progress).await
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

    fn upload_file(
        &self,
        request: FileUploadRequest,
        progress: Option<FileUploadProgressSender>,
    ) -> futures::future::LocalBoxFuture<
        'static,
        std::result::Result<FileUploadResponse, RpcClientError>,
    > {
        let client = self.clone();
        Box::pin(async move { client.upload_file(request, progress).await })
    }

    fn file_url(&self, id: &str) -> Option<String> {
        Some(format!("{}/{}", self.file_api_prefix, id))
    }
}

impl From<HttpRpcClient> for RpcClient {
    fn from(value: HttpRpcClient) -> Self {
        RpcClient::new(value)
    }
}

async fn upload_file_xhr(
    file_api_prefix: String,
    request: FileUploadRequest,
    progress: Option<FileUploadProgressSender>,
) -> std::result::Result<FileUploadResponse, RpcClientError> {
    let total = request.content.size();
    emit_progress(&progress, FileUploadPhase::Preparing, 0, total);
    let mut url = file_api_prefix;
    if let Some(scope_id) = &request.scope_id {
        let separator = if url.contains('?') { '&' } else { '?' };
        url.push(separator);
        url.push_str("scope=");
        url.push_str(&form_encode(scope_id));
    }

    let xhr = XmlHttpRequest::new().map_err(js_transport)?;
    xhr.open_with_async("POST", &url, true)
        .map_err(js_transport)?;
    xhr.set_request_header(
        "x-semantic-file-entity",
        &serde_json::to_string(&Value::Object(request.entity.clone()))
            .map_err(|err| RpcClientError::Protocol(err.to_string()))?,
    )
    .map_err(js_transport)?;
    if let Some(mime_type) = &request.mime_type {
        xhr.set_request_header("content-type", mime_type)
            .map_err(js_transport)?;
    }
    if let Some(filename) = &request.filename {
        xhr.set_request_header("x-semantic-filename", filename)
            .map_err(js_transport)?;
    }
    if let Some(id) = &request.id {
        xhr.set_request_header("x-semantic-file-id", id)
            .map_err(js_transport)?;
    }

    let upload_progress = progress.clone();
    let onprogress = Closure::<dyn FnMut(ProgressEvent)>::new(move |event: ProgressEvent| {
        let total_bytes = event.length_computable().then_some(event.total() as u64);
        emit_progress(
            &upload_progress,
            FileUploadPhase::Uploading,
            event.loaded() as u64,
            total_bytes,
        );
    });
    xhr.upload()
        .map_err(js_transport)?
        .set_onprogress(Some(onprogress.as_ref().unchecked_ref()));
    onprogress.forget();

    let (sender, receiver) = futures::channel::oneshot::channel();
    let sender = std::rc::Rc::new(std::cell::RefCell::new(Some(sender)));
    let onload = {
        let xhr = xhr.clone();
        let sender = std::rc::Rc::clone(&sender);
        Closure::<dyn FnMut()>::new(move || {
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(Ok(xhr.clone()));
            }
        })
    };
    let onerror = {
        let sender = std::rc::Rc::clone(&sender);
        Closure::<dyn FnMut()>::new(move || {
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(Err(RpcClientError::Transport(
                    "file upload network error".to_string(),
                )));
            }
        })
    };
    xhr.set_onload(Some(onload.as_ref().unchecked_ref()));
    xhr.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    onload.forget();
    onerror.forget();

    let blob = match request.content {
        FileUploadContent::Bytes(bytes) => {
            let array = Uint8Array::from(bytes.as_ref());
            let parts = js_sys::Array::new();
            parts.push(&array.buffer());
            Blob::new_with_u8_array_sequence(&parts).map_err(js_transport)?
        }
        FileUploadContent::Blob(blob) => blob,
        FileUploadContent::Stream { .. } => {
            return Err(RpcClientError::Transport(
                "stream-backed uploads are not supported in the web client; use a Blob".to_string(),
            ));
        }
    };
    xhr.send_with_opt_blob(Some(&blob)).map_err(js_transport)?;

    let xhr = receiver.await.map_err(|_| {
        RpcClientError::Transport("file upload response channel closed".to_string())
    })??;
    let status = xhr.status().map_err(js_transport)?;
    let text = xhr
        .response_text()
        .map_err(js_transport)?
        .unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(RpcClientError::Transport(format!(
            "file upload failed with status {status}: {}",
            short_body(&text)
        )));
    }
    emit_progress(
        &progress,
        FileUploadPhase::Finalizing,
        total.unwrap_or(0),
        total,
    );
    let value = serde_json::from_str::<Value>(&text)
        .map_err(|err| RpcClientError::Protocol(err.to_string()))?;
    let response = FileUploadResponse::from_value(value)?;
    emit_progress(&progress, FileUploadPhase::Done, total.unwrap_or(0), total);
    Ok(response)
}

fn js_transport(value: wasm_bindgen::JsValue) -> RpcClientError {
    RpcClientError::Transport(format!("{value:?}"))
}

fn form_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn short_body(body: &str) -> String {
    const MAX: usize = 240;
    if body.chars().count() <= MAX {
        body.to_string()
    } else {
        format!("{}...", body.chars().take(MAX).collect::<String>())
    }
}
