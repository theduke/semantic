use bytes::Bytes;
use futures::StreamExt as _;
use futures::stream;
use semantic_data::value::Value;

use crate::client::{RpcClient, RpcClientDyn, request, resolve_response};
use crate::command::RpcCommandSpec;
use crate::error::RpcClientError;
use crate::file::{
    FileUploadByteStream, FileUploadContent, FileUploadPhase, FileUploadProgressSender,
    FileUploadRequest, FileUploadResponse, derive_file_api_prefix, emit_progress,
};
use crate::protocol::RpcResponse;

#[derive(Clone)]
pub struct HttpRpcClient {
    endpoint: String,
    file_api_prefix: String,
    client: reqwest::Client,
}

impl HttpRpcClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into();
        Self {
            file_api_prefix: derive_file_api_prefix(&endpoint),
            endpoint,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_client(endpoint: impl Into<String>, client: reqwest::Client) -> Self {
        let endpoint = endpoint.into();
        Self {
            file_api_prefix: derive_file_api_prefix(&endpoint),
            endpoint,
            client,
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

    pub async fn upload_file(
        &self,
        request: FileUploadRequest,
        progress: Option<FileUploadProgressSender>,
    ) -> Result<FileUploadResponse, RpcClientError> {
        let total = request.content.size();
        emit_progress(&progress, FileUploadPhase::Preparing, 0, total);
        let mut url = self.file_api_prefix.clone();
        if let Some(scope_id) = &request.scope_id {
            let separator = if url.contains('?') { '&' } else { '?' };
            url.push(separator);
            url.push_str("scope=");
            url.push_str(&form_encode(scope_id));
        }

        let entity_header = serde_json::to_string(&Value::Object(request.entity.clone()))
            .map_err(|err| RpcClientError::Protocol(err.to_string()))?;
        let mut builder = self
            .client
            .post(url)
            .header("x-semantic-file-entity", entity_header);
        if let Some(mime_type) = &request.mime_type {
            builder = builder.header(reqwest::header::CONTENT_TYPE, mime_type);
        }
        if let Some(filename) = &request.filename {
            builder = builder.header("x-semantic-filename", filename);
        }
        if let Some(id) = &request.id {
            builder = builder.header("x-semantic-file-id", id);
        }
        if let Some(total) = total {
            builder = builder.header(reqwest::header::CONTENT_LENGTH, total);
        }

        let body = upload_body_stream(request.content, progress.clone());
        let response = builder
            .body(reqwest::Body::wrap_stream(body))
            .send()
            .await
            .map_err(|err| RpcClientError::Transport(err.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(RpcClientError::Transport(format!(
                "file upload failed with status {status}: {}",
                short_body(&body)
            )));
        }
        emit_progress(
            &progress,
            FileUploadPhase::Finalizing,
            total.unwrap_or(0),
            total,
        );
        let value = response
            .json::<Value>()
            .await
            .map_err(|err| RpcClientError::Protocol(err.to_string()))?;
        let response = FileUploadResponse::from_value(value)?;
        emit_progress(&progress, FileUploadPhase::Done, total.unwrap_or(0), total);
        Ok(response)
    }

    pub async fn read_file_range(
        &self,
        id: &str,
        scope_id: Option<&str>,
        offset: u64,
        size: u32,
    ) -> Result<Bytes, RpcClientError> {
        if size == 0 {
            return Ok(Bytes::new());
        }
        let mut url = format!("{}/{}", self.file_api_prefix, path_encode(id));
        if let Some(scope_id) = scope_id {
            url.push_str("?scope=");
            url.push_str(&form_encode(scope_id));
        }
        let end = offset.saturating_add(u64::from(size)).saturating_sub(1);
        let response = self
            .client
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"))
            .send()
            .await
            .map_err(|err| RpcClientError::Transport(err.to_string()))?;
        let status = response.status();
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            return Ok(Bytes::new());
        }
        if !status.is_success() {
            return Err(RpcClientError::Transport(format!(
                "file download failed with status {status}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|err| RpcClientError::Transport(err.to_string()))
    }
}

impl RpcClientDyn for HttpRpcClient {
    fn invoke_value(
        &self,
        command: String,
        payload: Value,
    ) -> crate::client::RpcClientFuture<std::result::Result<Value, RpcClientError>> {
        let client = self.clone();
        Box::pin(async move { client.invoke_value(command, payload).await })
    }

    fn upload_file(
        &self,
        request: FileUploadRequest,
        progress: Option<FileUploadProgressSender>,
    ) -> crate::client::RpcClientFuture<std::result::Result<FileUploadResponse, RpcClientError>>
    {
        let client = self.clone();
        Box::pin(async move { client.upload_file(request, progress).await })
    }

    fn file_url(&self, id: &str) -> Option<String> {
        Some(format!("{}/{}", self.file_api_prefix, id))
    }

    fn read_file_range(
        &self,
        id: String,
        scope_id: Option<String>,
        offset: u64,
        size: u32,
    ) -> crate::client::RpcClientFuture<Result<Bytes, RpcClientError>> {
        let client = self.clone();
        Box::pin(async move {
            client
                .read_file_range(&id, scope_id.as_deref(), offset, size)
                .await
        })
    }
}

impl From<HttpRpcClient> for RpcClient {
    fn from(value: HttpRpcClient) -> Self {
        RpcClient::new(value)
    }
}

fn progress_stream(
    bytes: Bytes,
    progress: Option<FileUploadProgressSender>,
) -> FileUploadByteStream {
    const CHUNK_SIZE: usize = 64 * 1024;
    let total = bytes.len() as u64;
    stream::unfold(0usize, move |offset| {
        let bytes = bytes.clone();
        let progress = progress.clone();
        async move {
            if offset >= bytes.len() {
                return None;
            }
            let end = (offset + CHUNK_SIZE).min(bytes.len());
            let chunk = bytes.slice(offset..end);
            emit_progress(
                &progress,
                FileUploadPhase::Uploading,
                end as u64,
                Some(total),
            );
            Some((Ok(chunk), end))
        }
    })
    .boxed()
}

fn upload_body_stream(
    content: FileUploadContent,
    progress: Option<FileUploadProgressSender>,
) -> FileUploadByteStream {
    match content {
        FileUploadContent::Bytes(bytes) => progress_stream(bytes, progress),
        FileUploadContent::Stream { stream, size } => {
            let mut uploaded = 0_u64;
            stream
                .map(move |result| {
                    if let Ok(chunk) = &result {
                        uploaded = uploaded.saturating_add(chunk.len() as u64);
                        emit_progress(&progress, FileUploadPhase::Uploading, uploaded, size);
                    }
                    result
                })
                .boxed()
        }
    }
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

fn path_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
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
