mod lifecycle;
mod options;

use std::sync::Arc;

use bytes::Bytes;
use futures_util::TryStreamExt as _;
use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use semantic_app::{DbScopeId, FileByteRange, FileContent, FileCreateRequest};
use semantic_data::value::serde::typed::{TypedRef, TypedValue};
use semantic_data::value::{Object, Value};
use semantic_rpc_core::RpcRequest;
use serde::{Deserialize, Serialize};

pub use options::EmbeddedOptions;

const PROTOCOL_VERSION: u32 = 1;

#[napi]
pub async fn open(options: EmbeddedOptions) -> napi::Result<NativeEmbedded> {
    let options = options.validate().map_err(invalid_argument)?;
    let storage = semantic_app::storage::resolve_storage(&options.app, options.storage)
        .map_err(open_error)?;
    let app = semantic_app::storage::open_app(options.app, storage)
        .await
        .map_err(open_error)?;
    Ok(NativeEmbedded {
        lifecycle: Arc::new(lifecycle::Lifecycle::new(
            app,
            options.max_concurrent_requests,
        )),
        max_buffered_file_bytes: options.max_buffered_file_bytes,
    })
}

#[napi]
pub fn protocol_version() -> u32 {
    PROTOCOL_VERSION
}

#[napi]
pub fn build_info() -> String {
    format!(
        "semantic-js-embed/{} napi8 protocol/{}",
        env!("CARGO_PKG_VERSION"),
        PROTOCOL_VERSION
    )
}

#[napi]
pub struct NativeEmbedded {
    lifecycle: Arc<lifecycle::Lifecycle>,
    max_buffered_file_bytes: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadOptions {
    #[serde(default)]
    scope_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    filestore_locator: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    entity: Option<TypedValue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadOptions {
    #[serde(default)]
    scope_id: Option<String>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    end: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadResult<'a> {
    id: &'a str,
    collection: &'a str,
    object: TypedRef<'a>,
}

#[napi]
impl NativeEmbedded {
    #[napi]
    pub async fn invoke_json(&self, request: String) -> napi::Result<String> {
        let request: RpcRequest = serde_json::from_str(&request)
            .map_err(|err| protocol_error(format!("invalid RPC request: {err}")))?;
        let (ctx, _permit) = self.lifecycle.context().await.map_err(lifecycle_error)?;
        let response = ctx.app.invoke(ctx.clone(), request).await;
        serde_json::to_string(&response)
            .map_err(|err| protocol_error(format!("could not encode RPC response: {err}")))
    }

    #[napi]
    pub async fn upload_file(&self, bytes: Buffer, options_json: String) -> napi::Result<String> {
        if bytes.len() > self.max_buffered_file_bytes {
            return Err(lifecycle_error(format!(
                "EMBEDDED_FILE_TOO_LARGE: buffered file exceeds {} bytes",
                self.max_buffered_file_bytes
            )));
        }
        let options: UploadOptions = serde_json::from_str(&options_json)
            .map_err(|err| protocol_error(format!("invalid upload options: {err}")))?;
        let entity = match options.entity.map(|value| value.0) {
            None => Object::new(),
            Some(Value::Object(value)) => value,
            Some(_) => return Err(protocol_error("upload entity must be an object")),
        };
        let (ctx, _permit) = self.lifecycle.context().await.map_err(lifecycle_error)?;
        let record = ctx
            .app
            .files()
            .create(
                &ctx,
                FileCreateRequest {
                    scope_id: options.scope_id.map(DbScopeId::new),
                    id: options.id,
                    filestore_locator: options.filestore_locator,
                    filename: options.filename,
                    mime_type: options.mime_type,
                    entity,
                    content: FileContent::Bytes(Bytes::copy_from_slice(bytes.as_ref())),
                },
            )
            .await
            .map_err(app_error)?;
        serde_json::to_string(&UploadResult {
            id: &record.id,
            collection: &record.collection,
            object: TypedRef(&Value::Object(record.object)),
        })
        .map_err(|err| protocol_error(format!("could not encode upload result: {err}")))
    }

    #[napi]
    pub async fn read_file(&self, id: String, options_json: String) -> napi::Result<Buffer> {
        let options: ReadOptions = serde_json::from_str(&options_json)
            .map_err(|err| protocol_error(format!("invalid read options: {err}")))?;
        let range = match (options.offset, options.end) {
            (None, None) => None,
            (Some(offset), None) => Some(FileByteRange::from_offset(offset)),
            (offset, Some(end)) => Some(FileByteRange::bounded(offset.unwrap_or(0), end)),
        };
        let (ctx, _permit) = self.lifecycle.context().await.map_err(lifecycle_error)?;
        let result = ctx
            .app
            .files()
            .open(&ctx, options.scope_id.map(DbScopeId::new), id)
            .await
            .map_err(app_error)?
            .read(range)
            .await
            .map_err(app_error)?;
        if result
            .byte_size
            .is_some_and(|size| size > self.max_buffered_file_bytes as u64)
        {
            return Err(lifecycle_error(format!(
                "EMBEDDED_FILE_TOO_LARGE: buffered file exceeds {} bytes",
                self.max_buffered_file_bytes
            )));
        }
        let chunks: Vec<Bytes> = result.stream.try_collect().await.map_err(app_error)?;
        let size: usize = chunks.iter().map(Bytes::len).sum();
        if size > self.max_buffered_file_bytes {
            return Err(lifecycle_error(format!(
                "EMBEDDED_FILE_TOO_LARGE: buffered file exceeds {} bytes",
                self.max_buffered_file_bytes
            )));
        }
        let mut output = Vec::with_capacity(size);
        for chunk in chunks {
            output.extend_from_slice(&chunk);
        }
        Ok(output.into())
    }

    #[napi]
    pub async fn close(&self) -> napi::Result<()> {
        self.lifecycle.close().await.map_err(lifecycle_error)
    }
}

fn invalid_argument(message: String) -> napi::Error {
    napi::Error::from_reason(format!("EMBEDDED_INVALID_OPTIONS: {message}"))
}
fn protocol_error(message: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(format!("EMBEDDED_PROTOCOL_ERROR: {message}"))
}
fn lifecycle_error(message: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(message.to_string())
}
fn open_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(format!("EMBEDDED_OPEN_FAILED: {error}"))
}
fn app_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(format!("EMBEDDED_FILE_ERROR: {error}"))
}
