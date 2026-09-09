#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error(transparent)]
    App(#[from] semantic_app::AppError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("invalid header value: {0}")]
    InvalidHeader(String),
}

impl From<semantic_db_core::DbError> for ServerError {
    fn from(value: semantic_db_core::DbError) -> Self {
        Self::App(semantic_app::AppError::Db(value))
    }
}

impl From<ServerError> for semantic_rpc_core::RpcError {
    fn from(value: ServerError) -> Self {
        match value {
            ServerError::App(err) => err.into(),
            ServerError::Io(err) => semantic_rpc_core::RpcError::internal(err.to_string()),
            ServerError::InvalidHeader(message) => {
                semantic_rpc_core::RpcError::new("invalid_request", message)
            }
        }
    }
}
