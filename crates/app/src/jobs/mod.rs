//! Scope-bound jobs persistence and application controls.
mod commands;
mod store;
pub(crate) use commands::register as register_commands;
pub use store::DbJobStore;

pub(crate) fn error(error: impl std::fmt::Display) -> crate::AppError {
    crate::AppError::InvalidRequest(error.to_string())
}
