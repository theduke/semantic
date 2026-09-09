#[cfg(feature = "redb")]
use std::sync::Arc;

#[cfg(feature = "redb")]
use async_trait::async_trait;
#[cfg(feature = "redb")]
use semantic_app::{AppError, DbBackend, Principal, SemanticDb};
#[cfg(feature = "redb")]
use semantic_data::schema::DbOpenMode;
#[cfg(feature = "redb")]
use semantic_db_core::Db;

#[cfg(feature = "redb")]
#[derive(Debug)]
pub struct RedbDbProvider;

#[cfg(feature = "redb")]
#[async_trait]
impl DbBackend for RedbDbProvider {
    type Config = crate::LocalDbConfig;

    fn scheme(&self) -> &str {
        "redb"
    }

    fn parse_uri(&self, uri: &str) -> Result<Self::Config, AppError> {
        crate::LocalDbConfig::from_uri("redb", uri)
    }

    async fn open_config(
        &self,
        config: Self::Config,
        mode: DbOpenMode,
        _principal: &Principal,
    ) -> Result<Arc<dyn SemanticDb>, AppError> {
        let backend = tokio::task::spawn_blocking(move || {
            config.ensure_parent(mode)?;
            semantic_db_redb::open_backend(config.path, mode).map_err(AppError::from)
        })
        .await
        .map_err(|err| {
            AppError::Db(semantic_db_core::DbError::Storage(format!(
                "redb database open task failed: {err}"
            )))
        })??;
        Ok(Arc::new(Db::new(backend)))
    }
}
