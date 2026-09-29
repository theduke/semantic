use axum::Json;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use semantic_app::AppError;
use semantic_rpc_core::{CallError, RpcError};

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

/// Respond with the [`RpcError`] form of `err` and a matching HTTP status.
pub(crate) fn server_error_response(err: ServerError) -> Response {
    let status = match &err {
        ServerError::InvalidHeader(_) => StatusCode::BAD_REQUEST,
        ServerError::App(err) => app_error_status(err),
        ServerError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(RpcError::from(err))).into_response()
}

/// Respond with the [`RpcError`] form of `err` and its HTTP status.
pub(crate) fn app_error_response(err: AppError) -> Response {
    (app_error_status(&err), Json(RpcError::from(err))).into_response()
}

/// Respond to a failed command call with the folded [`RpcError`] and a
/// matching HTTP status.
pub(crate) fn call_error_response(err: CallError<AppError>) -> Response {
    let status = match &err {
        CallError::UnknownCommand(_) => StatusCode::NOT_FOUND,
        CallError::InvalidPayload(_) => StatusCode::BAD_REQUEST,
        CallError::InvalidOutput(_) => StatusCode::INTERNAL_SERVER_ERROR,
        CallError::Command(err) => app_error_status(err),
    };
    (status, Json(RpcError::from(err))).into_response()
}

fn app_error_status(err: &AppError) -> StatusCode {
    StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}
