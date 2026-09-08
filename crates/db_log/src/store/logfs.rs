use std::path::Path;

use logfs::{ConfigBuilder, LogFs};
use semantic_data::schema::DbOpenMode;
use semantic_db_core::DbError;

use super::{DEFAULT_PREFIX, LogStore, event_key, normalize_prefix, parse_event_key};
use crate::EventId;

/// Durable local event storage backed by logfs.
pub struct LogFsLogStore {
    log: LogFs,
    prefix: String,
}

impl std::fmt::Debug for LogFsLogStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogFsLogStore")
            .field("path", &self.log.path())
            .field("prefix", &self.prefix)
            .finish()
    }
}

impl LogFsLogStore {
    pub fn open(path: impl AsRef<Path>, mode: DbOpenMode) -> std::result::Result<Self, DbError> {
        Self::open_with_prefix(path, mode, DEFAULT_PREFIX)
    }

    pub fn open_with_prefix(
        path: impl AsRef<Path>,
        mode: DbOpenMode,
        prefix: impl Into<String>,
    ) -> std::result::Result<Self, DbError> {
        let path = path.as_ref();
        let mut builder = ConfigBuilder::new(path);
        if mode == DbOpenMode::AutoCreate {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent).map_err(|err| {
                    DbError::Storage(format!("create logfs parent '{}': {err}", parent.display()))
                })?;
            }
            builder = builder.allow_create();
        }
        let log = LogFs::open_durable(builder.build()).map_err(|err| {
            DbError::Storage(format!("open logfs WAL '{}': {err}", path.display()))
        })?;
        Ok(Self {
            log,
            prefix: normalize_prefix(prefix)?,
        })
    }
}

impl LogStore for LogFsLogStore {
    fn event_ids(&self, from: EventId) -> std::result::Result<Vec<EventId>, DbError> {
        let keys = self.log.paths_prefix(&self.prefix).map_err(|err| {
            DbError::Storage(format!("list logfs WAL prefix '{}': {err}", self.prefix))
        })?;
        let mut ids = keys
            .iter()
            .map(|key| parse_event_key(&self.prefix, key))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.retain(|id| *id >= from);
        ids.sort_unstable();
        Ok(ids)
    }

    fn read_event(&self, id: EventId) -> std::result::Result<Option<Vec<u8>>, DbError> {
        let key = event_key(&self.prefix, id);
        self.log
            .get(&key)
            .map_err(|err| DbError::Storage(format!("read logfs WAL event {}: {err}", id.get())))
    }

    fn append_event(&mut self, id: EventId, bytes: Vec<u8>) -> std::result::Result<(), DbError> {
        let key = event_key(&self.prefix, id);
        if self
            .log
            .get_meta(&key)
            .map_err(|err| DbError::Storage(format!("check logfs WAL event {}: {err}", id.get())))?
            .is_some()
        {
            return Err(DbError::Storage(format!(
                "refusing to overwrite logfs WAL event {}",
                id.get()
            )));
        }
        self.log
            .insert(key, bytes)
            .map_err(|err| DbError::Storage(format!("append logfs WAL event {}: {err}", id.get())))
    }
}
