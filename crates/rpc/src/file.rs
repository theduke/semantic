use std::pin::Pin;

use bytes::Bytes;
use futures::Stream;
use futures::channel::mpsc::UnboundedSender;
use semantic_data::value::{Object, Value};

use crate::RpcClientError;

pub type FileUploadByteStream =
    Pin<Box<dyn Stream<Item = std::result::Result<Bytes, RpcClientError>> + Send + 'static>>;

pub enum FileUploadContent {
    Bytes(Bytes),
    Stream {
        stream: FileUploadByteStream,
        size: Option<u64>,
    },
    #[cfg(all(target_arch = "wasm32", feature = "client-http-web"))]
    Blob(web_sys::Blob),
}

impl FileUploadContent {
    pub fn size(&self) -> Option<u64> {
        match self {
            Self::Bytes(bytes) => Some(bytes.len() as u64),
            Self::Stream { size, .. } => *size,
            #[cfg(all(target_arch = "wasm32", feature = "client-http-web"))]
            Self::Blob(blob) => Some(blob.size() as u64),
        }
    }
}

impl std::fmt::Debug for FileUploadContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileUploadContent")
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct FileUploadRequest {
    pub scope_id: Option<String>,
    pub id: Option<String>,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub entity: Object,
    pub content: FileUploadContent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileUploadProgress {
    pub uploaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub phase: FileUploadPhase,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FileUploadPhase {
    Preparing,
    Uploading,
    Finalizing,
    Done,
}

pub type FileUploadProgressSender = UnboundedSender<FileUploadProgress>;

#[derive(Clone, Debug, PartialEq)]
pub struct FileUploadResponse {
    pub id: String,
    pub collection: String,
    pub object: Object,
}

impl FileUploadResponse {
    pub fn from_value(value: Value) -> std::result::Result<Self, RpcClientError> {
        let Value::Object(mut object) = value else {
            return Err(RpcClientError::Decode(
                "file upload response must be an object".to_string(),
            ));
        };
        let id = match object.remove("id") {
            Some(Value::String(id)) => id,
            Some(_) => {
                return Err(RpcClientError::Decode(
                    "file upload response field 'id' must be a string".to_string(),
                ));
            }
            None => {
                return Err(RpcClientError::Decode(
                    "file upload response missing field 'id'".to_string(),
                ));
            }
        };
        let collection = match object.remove("collection") {
            Some(Value::String(collection)) => collection,
            Some(_) => {
                return Err(RpcClientError::Decode(
                    "file upload response field 'collection' must be a string".to_string(),
                ));
            }
            None => {
                return Err(RpcClientError::Decode(
                    "file upload response missing field 'collection'".to_string(),
                ));
            }
        };
        let object = match object.remove("object") {
            Some(Value::Object(object)) => object,
            Some(_) => {
                return Err(RpcClientError::Decode(
                    "file upload response field 'object' must be an object".to_string(),
                ));
            }
            None => {
                return Err(RpcClientError::Decode(
                    "file upload response missing field 'object'".to_string(),
                ));
            }
        };
        Ok(Self {
            id,
            collection,
            object,
        })
    }

    pub fn into_value(self) -> Value {
        let mut object = Object::new();
        object.insert("id", Value::String(self.id));
        object.insert("collection", Value::String(self.collection));
        object.insert("object", Value::Object(self.object));
        Value::Object(object)
    }
}

pub fn emit_progress(
    progress: &Option<FileUploadProgressSender>,
    phase: FileUploadPhase,
    uploaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    if let Some(progress) = progress {
        let _ = progress.unbounded_send(FileUploadProgress {
            uploaded_bytes,
            total_bytes,
            phase,
        });
    }
}

pub fn derive_file_api_prefix(endpoint: &str) -> String {
    if let Some(prefix) = endpoint.strip_suffix("/rpc") {
        return format!("{prefix}/file");
    }
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        if let Some((origin, _)) = split_absolute_origin(endpoint) {
            return format!("{origin}/api/v1/file");
        }
    }
    "/api/v1/file".to_string()
}

fn split_absolute_origin(value: &str) -> Option<(&str, &str)> {
    let scheme_end = value.find("://")? + 3;
    let path_start = value[scheme_end..]
        .find('/')
        .map(|idx| scheme_end + idx)
        .unwrap_or(value.len());
    Some((&value[..path_start], &value[path_start..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RpcClientDyn, error::RpcClientError};
    use futures::FutureExt as _;

    struct UnsupportedClient;

    impl RpcClientDyn for UnsupportedClient {
        fn invoke_value(
            &self,
            _command: String,
            _payload: Value,
        ) -> crate::client::RpcClientFuture<std::result::Result<Value, RpcClientError>> {
            async { Ok(Value::Null) }.boxed()
        }
    }

    #[test]
    fn derives_file_api_prefix() {
        assert_eq!(derive_file_api_prefix("/api/v1/rpc"), "/api/v1/file");
        assert_eq!(
            derive_file_api_prefix("http://127.0.0.1:8888/api/v1/rpc"),
            "http://127.0.0.1:8888/api/v1/file"
        );
        assert_eq!(derive_file_api_prefix("/custom"), "/api/v1/file");
        assert_eq!(
            derive_file_api_prefix("http://127.0.0.1:8888/custom"),
            "http://127.0.0.1:8888/api/v1/file"
        );
    }

    #[test]
    fn upload_response_from_value_accepts_valid_response() {
        let mut file = Object::new();
        file.insert("filename", Value::String("image.png".to_string()));
        let response = FileUploadResponse {
            id: "file-1".to_string(),
            collection: "entities".to_string(),
            object: file,
        };
        let parsed = FileUploadResponse::from_value(response.clone().into_value()).unwrap();
        assert_eq!(parsed.id, response.id);
        assert_eq!(parsed.collection, response.collection);
        assert_eq!(parsed.object, response.object);
    }

    #[test]
    fn upload_response_from_value_rejects_missing_fields() {
        assert!(FileUploadResponse::from_value(Value::Object(Object::new())).is_err());
    }

    #[test]
    fn default_upload_file_is_unsupported() {
        let request = FileUploadRequest {
            scope_id: None,
            id: None,
            filename: None,
            mime_type: None,
            entity: Object::new(),
            content: FileUploadContent::Bytes(Bytes::new()),
        };
        let err = futures::executor::block_on(UnsupportedClient.upload_file(request, None))
            .expect_err("default upload must fail");
        assert!(
            matches!(err, RpcClientError::Transport(message) if message.contains("not supported"))
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_rpc_clients_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<crate::RpcClient>();
        assert_send_sync::<UnsupportedClient>();
    }
}
