#[cfg(feature = "storage-logfs")]
use std::sync::Arc;

#[cfg(feature = "storage-logfs")]
use crate::{AppError, DbBackend, Principal, SemanticDb};
#[cfg(feature = "storage-logfs")]
use async_trait::async_trait;
#[cfg(feature = "storage-logfs")]
use semantic_data::schema::DbOpenMode;
#[cfg(feature = "storage-logfs")]
use semantic_db_core::Db;

#[cfg(feature = "storage-logfs")]
#[derive(Debug)]
pub struct LogFsDbProvider;

#[cfg(feature = "storage-logfs")]
#[async_trait]
impl DbBackend for LogFsDbProvider {
    type Config = super::LocalDbConfig;

    fn scheme(&self) -> &str {
        "logfs"
    }

    fn parse_uri(&self, uri: &str) -> Result<Self::Config, AppError> {
        super::LocalDbConfig::from_uri("logfs", uri)
    }

    async fn open_config(
        &self,
        config: Self::Config,
        mode: DbOpenMode,
        _principal: &Principal,
    ) -> Result<Arc<dyn SemanticDb>, AppError> {
        let backend = tokio::task::spawn_blocking(move || {
            config.ensure_parent(mode)?;
            semantic_db_log::open_backend(config.path, mode).map_err(AppError::from)
        })
        .await
        .map_err(|err| {
            AppError::Db(semantic_db_core::DbError::Storage(format!(
                "logfs database open task failed: {err}"
            )))
        })??;
        Ok(Arc::new(Db::new(backend)))
    }
}
