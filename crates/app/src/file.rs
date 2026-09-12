use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::{StreamExt as _, TryStreamExt as _};
use objstore::{
    Copy, DataSource, DynObjStore, ObjStore as _, ObjStoreError, Operation, Put, SizedValueStream,
};
use semantic_data::builtin::{ATTR_ID, ATTR_TYPE};
use semantic_data::filestore::{
    ATTR_FILE_BYTE_SIZE, ATTR_FILE_CONTENT_HASH_SHA256, ATTR_FILE_FILENAME,
    ATTR_FILE_FILESTORE_LOCATOR, ATTR_FILE_MIME_TYPE, FILE_CLASS_ID,
};
use semantic_data::value::{DateTime, Object, Value};
use semantic_db_core::{Batch, BatchOperation, DEFAULT_COLLECTION, DbError, EntityRecord};
use sha2::{Digest as _, Sha256};

use crate::{
    AppError, AppRequestContext, DbScopeId, MediaAnalysisConfig, MediaAnalysisService,
    media::merge_analysis_attributes,
};

pub use objstore::ByteRange as FileByteRange;

pub type FileByteStream = BoxStream<'static, std::result::Result<Bytes, AppError>>;

#[derive(Clone, Debug)]
pub struct FileService {
    media_analysis: MediaAnalysisService,
}

pub struct FileCreateRequest {
    pub scope_id: Option<DbScopeId>,
    pub id: Option<String>,
    pub filestore_locator: Option<String>,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub entity: Object,
    pub content: FileContent,
}

pub enum FileContent {
    Bytes(Bytes),
    Stream(FileSizedStream),
}

pub struct FileSizedStream {
    pub stream: FileByteStream,
    pub size: Option<u64>,
}

pub struct FileRecord {
    pub id: String,
    pub collection: String,
    pub object: Object,
}

pub struct FileReadResult {
    pub record: EntityRecord,
    pub filestore_locator: String,
    pub byte_size: Option<u64>,
    pub mime_type: Option<String>,
    pub content_hash_sha256: Option<String>,
    pub stream: FileByteStream,
}

pub struct FileReader {
    pub record: EntityRecord,
    pub filestore_locator: String,
    pub byte_size: Option<u64>,
    pub mime_type: Option<String>,
    pub content_hash_sha256: Option<String>,
    store: DynObjStore,
}

impl FileReader {
    pub async fn read(
        self,
        range: Option<FileByteRange>,
    ) -> std::result::Result<FileReadResult, AppError> {
        let builder = self.store.build_stream(&self.filestore_locator);
        let builder = match range {
            Some(range) => builder.with_range(range),
            None => builder,
        };
        let stream = builder
            .send()
            .await?
            .ok_or_else(|| AppError::FileNotFound(self.record.id.clone()))?
            .map_err(AppError::ObjectStore)
            .boxed();

        Ok(FileReadResult {
            record: self.record,
            filestore_locator: self.filestore_locator,
            byte_size: self.byte_size,
            mime_type: self.mime_type,
            content_hash_sha256: self.content_hash_sha256,
            stream,
        })
    }
}

impl FileService {
    /// Prepare imported bytes under a content-addressed locator. The caller owns
    /// the separate entity publication, so a failed replacement preserves old bytes.
    /// Optional media analysis is deliberately left to the ordinary analysis action.
    pub(crate) async fn prepare_import(
        &self,
        store: &dyn objstore::ObjStore,
        id: String,
        filename: Option<String>,
        mime_type: String,
        mut object: Object,
        content: FileSizedStream,
    ) -> Result<Object, AppError> {
        let persisted =
            persist_content(store, Some(mime_type.clone()), FileContent::Stream(content)).await?;
        object.insert(ATTR_ID, id);
        object.insert(ATTR_TYPE, FILE_CLASS_ID.to_string());
        object.insert("uploaded_at", Value::DateTime(DateTime::now_utc()));
        object.insert("filestore_locator", persisted.filestore_locator);
        object.insert("byte_size", persisted.byte_size);
        object.insert("mime_type", mime_type.clone());
        object.insert("filekind", filekind_from_mime_type(&mime_type).to_string());
        object.insert("content_hash_sha256", persisted.content_hash_sha256);
        if let Some(filename) = filename {
            object.insert("filename", filename);
        }
        Ok(object)
    }

    pub fn new(media_analysis_config: MediaAnalysisConfig) -> Self {
        Self {
            media_analysis: MediaAnalysisService::new(media_analysis_config),
        }
    }

    pub fn media_analysis(&self) -> &MediaAnalysisService {
        &self.media_analysis
    }

    pub async fn create(
        &self,
        ctx: &AppRequestContext,
        request: FileCreateRequest,
    ) -> std::result::Result<FileRecord, AppError> {
        let scope_id = ctx.resolve_scope_id(request.scope_id.clone()).await?;
        let db = ctx.resolve_db(Some(scope_id.clone())).await?;
        let store = ctx.default_file_store(Some(scope_id)).await?;

        let requested_id = request
            .id
            .or_else(|| object_string(&request.entity, ATTR_ID));
        // Identity is checked again by Create inside the committing transaction.
        // This early check only avoids unnecessary uploads for known duplicates.
        if let Some(id) = &requested_id
            && db
                .get(DEFAULT_COLLECTION.into(), id.clone())
                .await?
                .is_some()
        {
            return Err(AppError::FileAlreadyExists { id: id.clone() });
        }
        if request.filestore_locator.is_some() {
            return Err(AppError::InvalidFileMetadata(
                "custom locators cannot be published by ordinary upload; existing locators remain readable".into(),
            ));
        }
        let filename = request.filename.clone();
        let PersistedContent {
            meta,
            filestore_locator,
            content_hash_sha256,
            byte_size: streamed_byte_size,
            bytes,
        } = persist_content(store.as_ref(), request.mime_type.clone(), request.content).await?;
        let id = requested_id.unwrap_or_else(|| format!("file-sha256-{content_hash_sha256}"));
        let byte_size = meta.size.or(Some(streamed_byte_size));
        let mime_type = request.mime_type.or(meta.mime_type);

        let mut object = request.entity;
        object.insert(ATTR_ID, Value::String(id.clone()));
        object.insert(ATTR_TYPE, Value::String(FILE_CLASS_ID.to_string()));
        object.insert("uploaded_at", Value::DateTime(DateTime::now_utc()));
        object.insert(
            "filestore_locator",
            Value::String(filestore_locator.clone()),
        );
        if let Some(filename) = filename.clone() {
            object.insert("filename", Value::String(filename));
        }
        if let Some(byte_size) = byte_size {
            object.insert("byte_size", Value::U64(byte_size));
        }
        if let Some(mime_type) = mime_type {
            object.insert(
                "filekind",
                Value::String(filekind_from_mime_type(&mime_type).to_string()),
            );
            object.insert("mime_type", Value::String(mime_type));
        } else {
            object.insert("filekind", Value::String("other".to_string()));
        }
        object.insert("content_hash_sha256", Value::String(content_hash_sha256));
        if self.media_analysis.config().auto_analyze_media {
            let declared_mime_type = object_string(&object, "mime_type");
            let analysis = match bytes {
                Some(bytes) => {
                    self.media_analysis
                        .analyze_created_bytes(
                            bytes,
                            filename.as_deref(),
                            declared_mime_type.as_deref(),
                        )
                        .await
                }
                None => match store.get_stream(&filestore_locator).await {
                    Ok(Some(stream)) => {
                        self.media_analysis
                            .analyze_stream(
                                stream.map_err(AppError::ObjectStore).boxed(),
                                filename.as_deref(),
                                declared_mime_type.as_deref(),
                            )
                            .await
                    }
                    Ok(None) => Ok(None),
                    Err(err) => Err(AppError::ObjectStore(err)),
                },
            };
            if let Ok(Some(analysis)) = analysis {
                merge_analysis_attributes(&mut object, &analysis);
            }
        }

        db.execute_batch(Batch::new().with_op(BatchOperation::Create {
            collection: DEFAULT_COLLECTION.to_string(),
            id: id.clone(),
            object: object.clone(),
        }))
        .await
        .map_err(|error| match error {
            DbError::EntityExists { id, .. } => AppError::FileAlreadyExists { id },
            other => AppError::Db(other),
        })?;

        Ok(FileRecord {
            id,
            collection: DEFAULT_COLLECTION.to_string(),
            object,
        })
    }

    pub async fn read(
        &self,
        ctx: &AppRequestContext,
        scope_id: Option<DbScopeId>,
        id: String,
    ) -> std::result::Result<FileReadResult, AppError> {
        self.open(ctx, scope_id, id).await?.read(None).await
    }

    /// Remove native file metadata explicitly and durably record cleanup intent.
    /// Bytes are retained until exclusive store ownership and retention are configured.
    /// Removing a domain reference never invokes this operation.
    pub async fn delete(
        &self,
        ctx: &AppRequestContext,
        scope: Option<DbScopeId>,
        id: String,
    ) -> Result<(), AppError> {
        let scope = ctx.resolve_scope_id(scope).await?;
        let db = ctx.resolve_db(Some(scope.clone())).await?;
        let Some(record) = db.get(DEFAULT_COLLECTION.into(), id.clone()).await? else {
            return Ok(());
        };
        validate_file_record(&record)?;
        let locator = required_file_string(&record, "filestore_locator")?;
        // A scope-local alias is not a physical store identity: different Apps
        // commonly name different stores "default".
        let store = ctx.default_file_store(Some(scope)).await?;
        let cleanup_id = format!("file-cleanup-{}", uuid::Uuid::new_v4());
        let mut cleanup = Object::new();
        cleanup.insert(ATTR_ID, cleanup_id.clone());
        cleanup.insert(
            ATTR_TYPE,
            semantic_data::filestore::CLEANUP_CLASS_ID.to_owned(),
        );
        cleanup.insert(
            "semantic:filestore:cleanup:store",
            store.safe_uri().to_string(),
        );
        cleanup.insert("semantic:filestore:cleanup:locator", locator);
        cleanup.insert(
            "semantic:filestore:cleanup:not_before",
            Value::DateTime(DateTime::now_utc()),
        );
        cleanup.insert("semantic:filestore:cleanup:attempts", 0_u64);
        db.execute_batch(
            Batch::new()
                .with_op(BatchOperation::DeleteById {
                    collection: DEFAULT_COLLECTION.into(),
                    id: id.clone(),
                })
                .with_op(BatchOperation::Create {
                    collection: DEFAULT_COLLECTION.into(),
                    id: cleanup_id,
                    object: cleanup,
                }),
        )
        .await
        .map_err(|error| match error {
            DbError::ReferenceTargetNotFound { id: target, .. } if target == id => {
                AppError::FileReferenced { id }
            }
            DbError::Validation(error) if error.rule == "reference" && error.actual == id => {
                AppError::FileReferenced { id }
            }
            other => AppError::Db(other),
        })?;
        Ok(())
    }

    pub async fn open(
        &self,
        ctx: &AppRequestContext,
        scope_id: Option<DbScopeId>,
        id: String,
    ) -> std::result::Result<FileReader, AppError> {
        let scope_id = ctx.resolve_scope_id(scope_id).await?;
        let db = ctx.resolve_db(Some(scope_id.clone())).await?;
        let Some(record) = db.get(DEFAULT_COLLECTION.to_string(), id.clone()).await? else {
            return Err(AppError::FileNotFound(id));
        };
        validate_file_record(&record)?;
        let filestore_locator = required_file_string(&record, "filestore_locator")?;
        let byte_size = optional_file_u64(&record, "byte_size")?;
        let mime_type = optional_file_string(&record, "mime_type")?;
        let content_hash_sha256 = optional_file_string(&record, "content_hash_sha256")?;

        let store = ctx.default_file_store(Some(scope_id)).await?;
        let Some(meta) = store.meta(&filestore_locator).await? else {
            return Err(AppError::FileNotFound(id));
        };

        Ok(FileReader {
            record,
            filestore_locator,
            byte_size: byte_size.or(meta.size),
            mime_type: mime_type.or(meta.mime_type),
            content_hash_sha256: content_hash_sha256.or(meta.hash_sha256.map(hex::encode)),
            store,
        })
    }
}

impl Default for FileService {
    fn default() -> Self {
        Self::new(MediaAnalysisConfig::default())
    }
}

struct PersistedContent {
    meta: objstore::ObjectMeta,
    filestore_locator: String,
    content_hash_sha256: String,
    byte_size: u64,
    bytes: Option<Bytes>,
}

#[derive(Default)]
struct StreamUploadState {
    hasher: Sha256,
    byte_size: u64,
    input_error: Option<StreamInputError>,
}

enum StreamInputError {
    UploadTooLarge(u64),
    Other(String),
}

async fn persist_content(
    store: &dyn objstore::ObjStore,
    mime_type: Option<String>,
    content: FileContent,
) -> std::result::Result<PersistedContent, AppError> {
    match content {
        FileContent::Bytes(bytes) => {
            let content_hash_sha256 = sha256_hex(&bytes);
            let byte_size = bytes.len() as u64;
            // Some stores ignore conditional writes. A fresh locator prevents a
            // losing publication (including another App process) from replacing
            // live bytes, even when callers choose the same identity or content.
            let filestore_locator =
                format!("file-sha256-{content_hash_sha256}/{}", uuid::Uuid::new_v4());
            let mut put = Put::new(&filestore_locator, DataSource::Data(bytes.clone()));
            put.mime_type = mime_type;
            let meta = store.send_put(put).await?;
            Ok(PersistedContent {
                meta,
                filestore_locator,
                content_hash_sha256,
                byte_size,
                bytes: Some(bytes),
            })
        }
        FileContent::Stream(stream) => {
            let temporary_locator = format!("upload-tmp-{}", uuid::Uuid::new_v4());
            let expected_size = stream.size;
            let state = Arc::new(Mutex::new(StreamUploadState::default()));
            let stream_state = Arc::clone(&state);
            let stream = stream
                .stream
                .map(move |result| match result {
                    Ok(chunk) => {
                        let mut state = stream_state.lock().expect("file stream state poisoned");
                        state.hasher.update(&chunk);
                        state.byte_size = state.byte_size.saturating_add(chunk.len() as u64);
                        Ok(chunk)
                    }
                    Err(err) => {
                        let input_error = match &err {
                            AppError::FileUploadTooLarge { limit } => {
                                StreamInputError::UploadTooLarge(*limit)
                            }
                            _ => StreamInputError::Other(err.to_string()),
                        };
                        stream_state
                            .lock()
                            .expect("file stream state poisoned")
                            .input_error = Some(input_error);
                        Err(ObjStoreError::Io {
                            operation: Operation::Put,
                            source: Some(Box::new(err)),
                        })
                    }
                })
                .boxed();
            let stream = match expected_size {
                Some(size) => SizedValueStream::new(stream, size),
                None => SizedValueStream::new_without_size(stream),
            };
            let mut put = Put::new(&temporary_locator, DataSource::Stream(stream));
            put.mime_type = mime_type;
            if let Err(err) = store.send_put(put).await {
                let input_error = state
                    .lock()
                    .expect("file stream state poisoned")
                    .input_error
                    .take();
                let _ = store.delete(&temporary_locator).await;
                return match input_error {
                    Some(StreamInputError::UploadTooLarge(limit)) => {
                        Err(AppError::FileUploadTooLarge { limit })
                    }
                    Some(StreamInputError::Other(message)) => {
                        Err(AppError::InvalidRequest(message))
                    }
                    None => Err(AppError::ObjectStore(err)),
                };
            }
            let (content_hash_sha256, byte_size) = {
                let state = state.lock().expect("file stream state poisoned");
                (
                    hex::encode(state.hasher.clone().finalize()),
                    state.byte_size,
                )
            };
            if let Some(expected_size) = expected_size
                && byte_size != expected_size
            {
                let _ = store.delete(&temporary_locator).await;
                return Err(AppError::InvalidFileMetadata(format!(
                    "stream size declared {expected_size}, received {byte_size} bytes"
                )));
            }
            let filestore_locator =
                format!("file-sha256-{content_hash_sha256}/{}", uuid::Uuid::new_v4());
            let persist_result = async {
                let meta = store
                    .send_copy(Copy::new(&temporary_locator, &filestore_locator))
                    .await?;

                store.delete(&temporary_locator).await?;

                std::result::Result::<_, ObjStoreError>::Ok(meta)
            }
            .await;

            let meta = match persist_result {
                Ok(meta) => meta,
                Err(err) => {
                    let _ = store.delete(&temporary_locator).await;
                    return Err(AppError::ObjectStore(err));
                }
            };
            Ok(PersistedContent {
                meta,
                filestore_locator,
                content_hash_sha256,
                byte_size,
                bytes: None,
            })
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn filekind_from_mime_type(mime_type: &str) -> &'static str {
    let mime_type = semantic_media::mime::normalize_declared(Some(mime_type)).unwrap_or_default();
    let mime_type = mime_type.as_str();

    if mime_type.starts_with("image/") {
        "image"
    } else if mime_type.starts_with("video/") {
        "video"
    } else if mime_type.starts_with("audio/") {
        "audio"
    } else if mime_type.starts_with("text/")
        || matches!(
            mime_type,
            "application/json"
                | "application/javascript"
                | "application/sql"
                | "application/toml"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
        )
        || mime_type.ends_with("+json")
        || mime_type.ends_with("+xml")
    {
        "text"
    } else if matches!(
        mime_type,
        "application/pdf" | "application/rtf" | "application/msword" | "application/epub+zip"
    ) || mime_type.starts_with("application/vnd.ms-")
        || mime_type.starts_with("application/vnd.oasis.opendocument.")
        || mime_type.starts_with("application/vnd.openxmlformats-officedocument.")
    {
        "document"
    } else if matches!(
        mime_type,
        "application/gzip"
            | "application/vnd.rar"
            | "application/x-7z-compressed"
            | "application/x-bzip2"
            | "application/x-gzip"
            | "application/x-rar-compressed"
            | "application/x-tar"
            | "application/x-xz"
            | "application/x-zip-compressed"
            | "application/zip"
            | "application/zstd"
    ) {
        "archive"
    } else {
        "other"
    }
}

fn object_string(object: &Object, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn validate_file_record(record: &EntityRecord) -> std::result::Result<(), AppError> {
    match record.object.get(ATTR_TYPE).and_then(Value::as_str) {
        Some(FILE_CLASS_ID) => Ok(()),
        Some(other) => Err(AppError::InvalidFileEntity(format!(
            "entity '{}' is type '{other}', not '{}'",
            record.id, FILE_CLASS_ID
        ))),
        None => Err(AppError::InvalidFileEntity(format!(
            "entity '{}' has no type",
            record.id
        ))),
    }
}

fn required_file_string(
    record: &EntityRecord,
    field: &str,
) -> std::result::Result<String, AppError> {
    optional_file_string(record, field)?.ok_or_else(|| {
        AppError::InvalidFileEntity(format!("file entity '{}' has no {field}", record.id))
    })
}

fn optional_file_string(
    record: &EntityRecord,
    field: &str,
) -> std::result::Result<Option<String>, AppError> {
    match file_value(record, field) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Null) | Some(Value::Void) | None => Ok(None),
        Some(_) => Err(AppError::InvalidFileEntity(format!(
            "file entity '{}' field '{field}' must be a string",
            record.id
        ))),
    }
}

fn optional_file_u64(
    record: &EntityRecord,
    field: &str,
) -> std::result::Result<Option<u64>, AppError> {
    match file_value(record, field) {
        Some(Value::U64(value)) => Ok(Some(*value)),
        Some(Value::U8(value)) => Ok(Some((*value).into())),
        Some(Value::U16(value)) => Ok(Some((*value).into())),
        Some(Value::U32(value)) => Ok(Some((*value).into())),
        Some(Value::Null) | Some(Value::Void) | None => Ok(None),
        Some(_) => Err(AppError::InvalidFileEntity(format!(
            "file entity '{}' field '{field}' must be an unsigned integer",
            record.id
        ))),
    }
}

fn file_value<'a>(record: &'a EntityRecord, field: &str) -> Option<&'a Value> {
    record
        .object
        .get(field)
        .or_else(|| file_field_id(field).and_then(|field_id| record.object.get(field_id)))
}

fn file_field_id(field: &str) -> Option<&'static str> {
    match field {
        "filestore_locator" => Some(ATTR_FILE_FILESTORE_LOCATOR),
        "filename" => Some(ATTR_FILE_FILENAME),
        "byte_size" => Some(ATTR_FILE_BYTE_SIZE),
        "mime_type" => Some(ATTR_FILE_MIME_TYPE),
        "content_hash_sha256" => Some(ATTR_FILE_CONTENT_HASH_SHA256),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::filekind_from_mime_type;

    #[test]
    fn filekind_is_derived_from_mime_type() {
        assert_eq!(filekind_from_mime_type("image/png"), "image");
        assert_eq!(filekind_from_mime_type("video/mp4"), "video");
        assert_eq!(filekind_from_mime_type("audio/mpeg"), "audio");
        assert_eq!(filekind_from_mime_type("text/plain; charset=utf-8"), "text");
        assert_eq!(filekind_from_mime_type("application/json"), "text");
        assert_eq!(filekind_from_mime_type("application/pdf"), "document");
        assert_eq!(filekind_from_mime_type("application/zip"), "archive");
        assert_eq!(filekind_from_mime_type("application/octet-stream"), "other");
    }
}
