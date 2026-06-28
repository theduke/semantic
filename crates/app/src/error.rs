#[derive(Debug, thiserror::Error)]
pub enum AppError {
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
    #[error("invalid file entity: {0}")]
    InvalidFileEntity(String),
    #[error("invalid file metadata: {0}")]
    InvalidFileMetadata(String),
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
    RpcRegister(#[from] semantic_rpc::RegisterError),
}

impl From<AppError> for semantic_rpc::RpcError {
    fn from(value: AppError) -> Self {
        match value {
            AppError::AuthenticationRequired => {
                semantic_rpc::RpcError::new("authentication_required", value.to_string())
            }
            AppError::ScopeRequired => {
                semantic_rpc::RpcError::new("scope_required", value.to_string())
            }
            AppError::ObjectStoreRequired(_) => {
                semantic_rpc::RpcError::new("object_store_required", value.to_string())
            }
            AppError::FileStoreRequired(_) => {
                semantic_rpc::RpcError::new("file_store_required", value.to_string())
            }
            AppError::FileNotFound(_) => {
                semantic_rpc::RpcError::new("file_not_found", value.to_string())
            }
            AppError::InvalidFileEntity(_) => {
                semantic_rpc::RpcError::new("invalid_file_entity", value.to_string())
            }
            AppError::InvalidFileMetadata(_) => {
                semantic_rpc::RpcError::new("invalid_file_metadata", value.to_string())
            }
            AppError::InvalidRange(_) => {
                semantic_rpc::RpcError::new("invalid_range", value.to_string())
            }
            AppError::MediaAnalysis(_) => {
                semantic_rpc::RpcError::new("media_analysis_failed", value.to_string())
            }
            AppError::UnknownScope(_) => {
                semantic_rpc::RpcError::new("unknown_scope", value.to_string())
            }
            AppError::UnknownObjectStoreScope(_) => {
                semantic_rpc::RpcError::new("unknown_object_store_scope", value.to_string())
            }
            AppError::UnknownObjectStore(_, _) => {
                semantic_rpc::RpcError::new("unknown_object_store", value.to_string())
            }
            AppError::UnsupportedDbScheme(_) => {
                semantic_rpc::RpcError::new("unsupported_db_scheme", value.to_string())
            }
            AppError::InvalidRequest(_) => {
                semantic_rpc::RpcError::new("invalid_request", value.to_string())
            }
            AppError::Db(_) => semantic_rpc::RpcError::new("db_error", value.to_string()),
            AppError::ObjectStore(_) => {
                semantic_rpc::RpcError::new("object_store_error", value.to_string())
            }
            AppError::RpcRegister(_) => semantic_rpc::RpcError::new("internal", value.to_string()),
        }
    }
}
