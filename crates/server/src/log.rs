use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use objstore::DynObjStore;
use semantic_app::{AppError, DbBackend, Principal, SemanticDb};
use semantic_data::schema::DbOpenMode;
use semantic_db_core::{Db, DbError};
use semantic_db_log::{EventId, LogStore, ObjStoreLogStore};

/// Storage source for the object-store-backed log database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogDbConfig {
    /// Share the server's already-opened blob store.
    BlobStore,
}

impl LogDbConfig {
    pub fn from_uri(uri: &str) -> Result<Self, AppError> {
        match uri {
            "log:<blob>" => Ok(Self::BlobStore),
            _ => Err(AppError::InvalidRequest(
                "log database URI must be 'log:<blob>' to share the configured blob store".into(),
            )),
        }
    }
}

/// Holds one database instance, ensuring one WAL writer for the shared store.
pub struct LogDbProvider {
    store: DynObjStore,
    db: Arc<Mutex<Option<Arc<dyn SemanticDb>>>>,
}

impl LogDbProvider {
    pub fn new(store: DynObjStore) -> Self {
        Self {
            store,
            db: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl DbBackend for LogDbProvider {
    type Config = LogDbConfig;

    fn scheme(&self) -> &str {
        "log"
    }

    fn parse_uri(&self, uri: &str) -> Result<Self::Config, AppError> {
        LogDbConfig::from_uri(uri)
    }

    async fn open_config(
        &self,
        _config: Self::Config,
        mode: DbOpenMode,
        _principal: &Principal,
    ) -> Result<Arc<dyn SemanticDb>, AppError> {
        let store = Arc::clone(&self.store);
        let db = Arc::clone(&self.db);
        // Initialize within the blocking task so cancellation of its async
        // caller cannot leave an untracked writer while another open starts.
        tokio::task::spawn_blocking(move || {
            let mut db = db.lock().map_err(|_| {
                AppError::Db(DbError::Storage(
                    "shared log database initialization lock poisoned".into(),
                ))
            })?;
            if let Some(db) = &*db {
                return Ok(Arc::clone(db));
            }
            let wal = ObjStoreLogStore::with_prefix(store, "db/default/wal/v1")?;
            if mode == DbOpenMode::OpenExisting && wal.event_ids(EventId::FIRST)?.is_empty() {
                return Err(AppError::Db(DbError::Storage(
                    "shared blob store contains no database WAL".into(),
                )));
            }
            let backend = semantic_db_log::open_backend_from_store(wal)?;
            let opened = Arc::new(Db::new(backend)) as Arc<dyn SemanticDb>;
            *db = Some(Arc::clone(&opened));
            Ok(opened)
        })
        .await
        .map_err(|err| {
            AppError::Db(DbError::Storage(format!(
                "log database open task failed: {err}"
            )))
        })?
    }
}
