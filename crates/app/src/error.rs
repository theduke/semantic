#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("authentication required")]
    AuthenticationRequired,
    #[error("database scope required")]
    ScopeRequired,
    #[error("unknown database scope '{0}'")]
    UnknownScope(String),
    #[error("unsupported database uri scheme '{0}'")]
    UnsupportedDbScheme(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error(transparent)]
    Db(#[from] semantic_db_core::DbError),
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
            AppError::UnknownScope(_) => {
                semantic_rpc::RpcError::new("unknown_scope", value.to_string())
            }
            AppError::UnsupportedDbScheme(_) => {
                semantic_rpc::RpcError::new("unsupported_db_scheme", value.to_string())
            }
            AppError::InvalidRequest(_) => {
                semantic_rpc::RpcError::new("invalid_request", value.to_string())
            }
            AppError::Db(_) => semantic_rpc::RpcError::new("db_error", value.to_string()),
            AppError::RpcRegister(_) => semantic_rpc::RpcError::new("internal", value.to_string()),
        }
    }
}
