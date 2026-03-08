use std::future::Future;
use std::pin::Pin;

use semantic_db_core::DbError;

pub trait KvBackendSpawner: Send + Sync + 'static {
    fn spawn_blocking<R, F>(
        &self,
        op: F,
    ) -> Pin<Box<dyn Future<Output = Result<R, DbError>> + Send>>
    where
        R: Send + 'static,
        F: FnOnce() -> Result<R, DbError> + Send + 'static;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct InlineSpawner;

impl KvBackendSpawner for InlineSpawner {
    fn spawn_blocking<R, F>(
        &self,
        op: F,
    ) -> Pin<Box<dyn Future<Output = Result<R, DbError>> + Send>>
    where
        R: Send + 'static,
        F: FnOnce() -> Result<R, DbError> + Send + 'static,
    {
        Box::pin(async move { op() })
    }
}

#[cfg(feature = "tokio")]
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioSpawner;

#[cfg(feature = "tokio")]
impl KvBackendSpawner for TokioSpawner {
    fn spawn_blocking<R, F>(
        &self,
        op: F,
    ) -> Pin<Box<dyn Future<Output = Result<R, DbError>> + Send>>
    where
        R: Send + 'static,
        F: FnOnce() -> Result<R, DbError> + Send + 'static,
    {
        Box::pin(async move {
            tokio::task::spawn_blocking(op)
                .await
                .map_err(|err| DbError::Storage(format!("tokio spawn_blocking failed: {err}")))?
        })
    }
}

#[cfg(feature = "tokio")]
pub type DefaultKvBackendSpawner = TokioSpawner;

#[cfg(not(feature = "tokio"))]
pub type DefaultKvBackendSpawner = InlineSpawner;
