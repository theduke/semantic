use bytes::{Bytes, BytesMut};
use futures_util::stream::BoxStream;
use futures_util::{StreamExt as _, TryStreamExt as _};
use objstore::{DataSource, ObjStore as _, Put};
use semantic_data::filestore::{
    FILE_BYTE_SIZE_ATTRIBUTE_ID, FILE_CLASS_ID, FILE_CONTENT_HASH_SHA256_ATTRIBUTE_ID,
    FILE_FILENAME_ATTRIBUTE_ID, FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID, FILE_MIME_TYPE_ATTRIBUTE_ID,
};
use semantic_data::value::{Object, Value};
use semantic_db_core::{DEFAULT_COLLECTION, EntityRecord};
use sha2::{Digest as _, Sha256};

use crate::{AppError, AppRequestContext, DbScopeId};

pub type FileByteStream = BoxStream<'static, std::result::Result<Bytes, AppError>>;

pub struct FileService;

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

impl FileService {
    pub fn new() -> Self {
        Self
    }

    pub async fn create(
        &self,
        ctx: &AppRequestContext,
        request: FileCreateRequest,
    ) -> std::result::Result<FileRecord, AppError> {
        let scope_id = ctx.resolve_scope_id(request.scope_id.clone()).await?;
        let db = ctx.resolve_db(Some(scope_id.clone())).await?;
        let store = ctx.default_file_store(Some(scope_id)).await?;

        let bytes = content_bytes(request.content).await?;
        let computed_sha256 = sha256_hex(&bytes);
        let id = request
            .id
            .or_else(|| object_string(&request.entity, "id"))
            .unwrap_or_else(|| format!("file-sha256-{computed_sha256}"));
        let filestore_locator = request.filestore_locator.unwrap_or_else(|| id.clone());

        let mut put = Put::new(filestore_locator.clone(), DataSource::Data(bytes.clone()));
        put.mime_type = request.mime_type.clone();
        let meta = store.send_put(put).await?;

        let content_hash_sha256 = meta.hash_sha256.map(hex::encode).unwrap_or(computed_sha256);
        let byte_size = meta.size.or(Some(bytes.len() as u64));
        let mime_type = request.mime_type.or(meta.mime_type);

        let mut object = request.entity;
        object.insert("id", Value::String(id.clone()));
        object.insert("type", Value::String(FILE_CLASS_ID.to_string()));
        object.insert("filestore_locator", Value::String(filestore_locator));
        if let Some(filename) = request.filename {
            object.insert("filename", Value::String(filename));
        }
        if let Some(byte_size) = byte_size {
            object.insert("byte_size", Value::U64(byte_size));
        }
        if let Some(mime_type) = mime_type {
            object.insert("mime_type", Value::String(mime_type));
        }
        object.insert("content_hash_sha256", Value::String(content_hash_sha256));

        db.insert(DEFAULT_COLLECTION.to_string(), id.clone(), object.clone())
            .await?;

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
        let Some((meta, stream)) = store.get_stream_with_meta(&filestore_locator).await? else {
            return Err(AppError::FileNotFound(id));
        };
        let stream = stream.map_err(AppError::ObjectStore).boxed();

        Ok(FileReadResult {
            record,
            filestore_locator,
            byte_size: byte_size.or(meta.size),
            mime_type: mime_type.or(meta.mime_type),
            content_hash_sha256: content_hash_sha256.or(meta.hash_sha256.map(hex::encode)),
            stream,
        })
    }
}

impl Default for FileService {
    fn default() -> Self {
        Self::new()
    }
}

async fn content_bytes(content: FileContent) -> std::result::Result<Bytes, AppError> {
    match content {
        FileContent::Bytes(bytes) => Ok(bytes),
        FileContent::Stream(stream) => {
            let data = stream.stream.try_collect::<BytesMut>().await?;
            if let Some(size) = stream.size
                && data.len() as u64 != size
            {
                return Err(AppError::InvalidFileMetadata(format!(
                    "stream size declared {size}, received {} bytes",
                    data.len()
                )));
            }
            Ok(data.freeze())
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn object_string(object: &Object, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn validate_file_record(record: &EntityRecord) -> std::result::Result<(), AppError> {
    match record.object.get("type").and_then(Value::as_str) {
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
        "filestore_locator" => Some(FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID),
        "filename" => Some(FILE_FILENAME_ATTRIBUTE_ID),
        "byte_size" => Some(FILE_BYTE_SIZE_ATTRIBUTE_ID),
        "mime_type" => Some(FILE_MIME_TYPE_ATTRIBUTE_ID),
        "content_hash_sha256" => Some(FILE_CONTENT_HASH_SHA256_ATTRIBUTE_ID),
        _ => None,
    }
}
