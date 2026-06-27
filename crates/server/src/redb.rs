#[cfg(feature = "redb")]
use std::sync::Arc;

#[cfg(feature = "redb")]
use async_trait::async_trait;
#[cfg(feature = "redb")]
use semantic_app::{AppError, DbOpenRequest, DbProvider, Principal, SemanticDb};
#[cfg(feature = "redb")]
use semantic_db_core::Db;

#[cfg(feature = "redb")]
pub struct RedbDbProvider;

#[cfg(feature = "redb")]
#[async_trait]
impl DbProvider for RedbDbProvider {
    fn scheme(&self) -> &str {
        "redb"
    }

    async fn open(
        &self,
        request: DbOpenRequest,
        _principal: &Principal,
    ) -> std::result::Result<Arc<dyn SemanticDb>, AppError> {
        let path = request.uri.strip_prefix("redb://").ok_or_else(|| {
            AppError::InvalidRequest(format!("invalid redb uri '{}'", request.uri))
        })?;
        if path.is_empty() {
            return Err(AppError::InvalidRequest(
                "invalid redb uri: missing database path".to_string(),
            ));
        }
        let backend = semantic_db_redb::open_backend(path, request.mode)?;
        Ok(Arc::new(Db::new(backend)))
    }
}
