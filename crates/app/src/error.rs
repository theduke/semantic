#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Jobs(#[from] semantic_jobs::JobsError),
    #[error("authentication required")]
    AuthenticationRequired,
    #[error("database scope required")]
    ScopeRequired,
    #[error("object store required for scope '{0}'")]
    ObjectStoreRequired(String),
    #[error("file store required for scope '{0}'")]
    FileStoreRequired(String),
    #[error("file '{0}' not found")]
    FileNotFound(String),
    #[error("file '{id}' already exists")]
    FileAlreadyExists { id: String },
    #[error("file '{id}' is still referenced")]
    FileReferenced { id: String },
    #[error("invalid file entity: {0}")]
    InvalidFileEntity(String),
    #[error("invalid file metadata: {0}")]
    InvalidFileMetadata(String),
    #[error("file upload exceeds the {limit} byte limit")]
    FileUploadTooLarge { limit: u64 },
    #[error("invalid range: {0}")]
    InvalidRange(String),
    #[error("media analysis failed: {0}")]
    MediaAnalysis(#[from] semantic_media::MediaAnalysisError),
    #[error("unknown database scope '{0}'")]
    UnknownScope(String),
    #[error("unknown object store scope '{0}'")]
    UnknownObjectStoreScope(String),
    #[error("unknown object store '{1}' for scope '{0}'")]
    UnknownObjectStore(String, String),
    #[error("unsupported database uri scheme '{0}'")]
    UnsupportedDbScheme(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error(transparent)]
    Db(#[from] semantic_db_core::DbError),
    #[error(transparent)]
    ObjectStore(#[from] objstore::ObjStoreError),
    #[error(transparent)]
    RpcRegister(#[from] semantic_rpc_core::RegisterError),
}

impl From<AppError> for semantic_rpc_core::RpcError {
    fn from(value: AppError) -> Self {
        match value {
            AppError::Db(semantic_db_core::DbError::Validation(error)) => {
                semantic_rpc_core::RpcError::with_data(
                    "validation_failed",
                    error.to_string(),
                    validation_error_data(error),
                )
            }
            AppError::Db(semantic_db_core::DbError::UnsupportedConstraint { kind }) => {
                let mut data = semantic_data::value::Object::new();
                data.insert("kind", kind);
                semantic_rpc_core::RpcError::with_data(
                    "unsupported_constraint",
                    "unsupported constraint",
                    semantic_data::value::Value::Object(data),
                )
            }
            AppError::Db(semantic_db_core::DbError::BatchReturn { reason, field }) => {
                let mut data = semantic_data::value::Object::new();
                data.insert("reason", reason.as_str().to_string());
                if let Some(field) = field {
                    data.insert("field", field);
                }
                semantic_rpc_core::RpcError::with_data(
                    "batch_return_error",
                    "invalid batch returning",
                    semantic_data::value::Value::Object(data),
                )
            }
            AppError::FileAlreadyExists { id } => semantic_rpc_core::RpcError::with_data(
                "file_already_exists",
                "file already exists",
                semantic_data::value::Value::Object(semantic_data::value::Object::from_iter([(
                    "id".to_string(),
                    semantic_data::value::Value::String(id),
                )])),
            ),
            AppError::FileReferenced { id } => semantic_rpc_core::RpcError::with_data(
                "file_referenced",
                "file is still referenced",
                semantic_data::value::Value::Object(semantic_data::value::Object::from_iter([(
                    "id".to_string(),
                    semantic_data::value::Value::String(id),
                )])),
            ),
            AppError::Db(semantic_db_core::DbError::EntityExists { collection, id }) => {
                let mut data = semantic_data::value::Object::new();
                data.insert("collection", collection);
                data.insert("id", id);
                semantic_rpc_core::RpcError::with_data(
                    "entity_exists",
                    "entity already exists",
                    semantic_data::value::Value::Object(data),
                )
            }
            AppError::Jobs(_) => semantic_rpc_core::RpcError::new("jobs_error", value.to_string()),
            AppError::AuthenticationRequired => {
                semantic_rpc_core::RpcError::new("authentication_required", value.to_string())
            }
            AppError::ScopeRequired => {
                semantic_rpc_core::RpcError::new("scope_required", value.to_string())
            }
            AppError::ObjectStoreRequired(_) => {
                semantic_rpc_core::RpcError::new("object_store_required", value.to_string())
            }
            AppError::FileStoreRequired(_) => {
                semantic_rpc_core::RpcError::new("file_store_required", value.to_string())
            }
            AppError::FileNotFound(_) => {
                semantic_rpc_core::RpcError::new("file_not_found", value.to_string())
            }
            AppError::InvalidFileEntity(_) => {
                semantic_rpc_core::RpcError::new("invalid_file_entity", value.to_string())
            }
            AppError::InvalidFileMetadata(_) => {
                semantic_rpc_core::RpcError::new("invalid_file_metadata", value.to_string())
            }
            AppError::FileUploadTooLarge { .. } => {
                semantic_rpc_core::RpcError::new("file_upload_too_large", value.to_string())
            }
            AppError::InvalidRange(_) => {
                semantic_rpc_core::RpcError::new("invalid_range", value.to_string())
            }
            AppError::MediaAnalysis(_) => {
                semantic_rpc_core::RpcError::new("media_analysis_failed", value.to_string())
            }
            AppError::UnknownScope(_) => {
                semantic_rpc_core::RpcError::new("unknown_scope", value.to_string())
            }
            AppError::UnknownObjectStoreScope(_) => {
                semantic_rpc_core::RpcError::new("unknown_object_store_scope", value.to_string())
            }
            AppError::UnknownObjectStore(_, _) => {
                semantic_rpc_core::RpcError::new("unknown_object_store", value.to_string())
            }
            AppError::UnsupportedDbScheme(_) => {
                semantic_rpc_core::RpcError::new("unsupported_db_scheme", value.to_string())
            }
            AppError::InvalidRequest(_) => {
                semantic_rpc_core::RpcError::new("invalid_request", value.to_string())
            }
            AppError::Db(semantic_db_core::DbError::QueryParameter { reason, name }) => {
                let mut data = semantic_data::value::Object::new();
                data.insert("reason", semantic_data::value::Value::String(reason));
                if let Some(name) = name {
                    data.insert("name", semantic_data::value::Value::String(name));
                }
                semantic_rpc_core::RpcError::with_data(
                    "query_parameter_error",
                    "invalid SQL parameters",
                    semantic_data::value::Value::Object(data),
                )
            }
            AppError::Db(_) => semantic_rpc_core::RpcError::new("db_error", value.to_string()),
            AppError::ObjectStore(_) => {
                semantic_rpc_core::RpcError::new("object_store_error", value.to_string())
            }
            AppError::RpcRegister(_) => {
                semantic_rpc_core::RpcError::new("internal", value.to_string())
            }
        }
    }
}

pub(crate) fn validation_error_data(
    error: semantic_db_core::ValidationError,
) -> semantic_data::value::Value {
    use semantic_data::value::{Object, PathSegment, Value};
    let mut data = Object::new();
    data.insert("class", error.class);
    data.insert("attribute", error.attribute);
    data.insert(
        "path",
        Value::List(
            error
                .path
                .0
                .into_iter()
                .map(|part| match part {
                    PathSegment::Field(name) => {
                        Value::Object(Object::from_iter([("field".into(), Value::String(name))]))
                    }
                    PathSegment::Index(index) => Value::Object(Object::from_iter([(
                        "index".into(),
                        Value::U64(index as u64),
                    )])),
                })
                .collect(),
        ),
    );
    data.insert("rule", error.rule);
    data.insert("expected", error.expected);
    data.insert("actual", error.actual);
    Value::Object(data)
}
