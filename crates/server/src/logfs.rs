#[cfg(feature = "logfs")]
use std::sync::Arc;

#[cfg(feature = "logfs")]
use async_trait::async_trait;
#[cfg(feature = "logfs")]
use semantic_app::{AppError, DbOpenRequest, DbProvider, Principal, SemanticDb};
#[cfg(feature = "logfs")]
use semantic_db_core::Db;

#[cfg(feature = "logfs")]
pub struct LogFsDbProvider;

#[cfg(feature = "logfs")]
#[async_trait]
impl DbProvider for LogFsDbProvider {
    fn scheme(&self) -> &str {
        "logfs"
    }

    async fn open(
        &self,
        request: DbOpenRequest,
        _principal: &Principal,
    ) -> std::result::Result<Arc<dyn SemanticDb>, AppError> {
        let path = request.uri.strip_prefix("logfs://").ok_or_else(|| {
            AppError::InvalidRequest(format!("invalid logfs uri '{}'", request.uri))
        })?;
        if path.is_empty() {
            return Err(AppError::InvalidRequest(
                "invalid logfs uri: missing database path".to_string(),
            ));
        }
        let path = std::path::PathBuf::from(path);
        let mode = request.mode;
        let backend =
            tokio::task::spawn_blocking(move || semantic_db_log::open_backend(path, mode))
                .await
                .map_err(|err| {
                    AppError::Db(semantic_db_core::DbError::Storage(format!(
                        "logfs database open task failed: {err}"
                    )))
                })??;
        Ok(Arc::new(Db::new(backend)))
    }
}
