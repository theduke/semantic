mod logfs;
mod object;

pub use logfs::LogFsLogStore;
pub use object::ObjStoreLogStore;

use std::fmt::Debug;

use semantic_db_core::DbError;

use crate::EventId;

/// Immutable, contiguous event storage for a single-writer WAL.
///
/// Event IDs begin at one. A successful append is the store's durability
/// boundary and must never overwrite an existing event.
pub trait LogStore: Debug + Send + Sync + 'static {
    fn event_ids(&self, from: EventId) -> std::result::Result<Vec<EventId>, DbError>;
    fn read_event(&self, id: EventId) -> std::result::Result<Option<Vec<u8>>, DbError>;
    fn append_event(&mut self, id: EventId, bytes: Vec<u8>) -> std::result::Result<(), DbError>;
}

const DEFAULT_PREFIX: &str = "__semantic/wal/v1/";
const MAX_PREFIX_LEN: usize = 512;

fn normalize_prefix(prefix: impl Into<String>) -> std::result::Result<String, DbError> {
    let mut prefix = prefix.into();
    if prefix.is_empty() || prefix.len() > MAX_PREFIX_LEN || prefix.contains('\0') {
        return Err(DbError::Storage(format!(
            "WAL prefix must contain 1..={MAX_PREFIX_LEN} non-NUL bytes"
        )));
    }
    if !prefix.ends_with('/') {
        prefix.push('/');
    }
    Ok(prefix)
}

fn event_key(prefix: &str, id: EventId) -> String {
    format!("{prefix}{:020}", id.get())
}

fn parse_event_key(prefix: &str, key: &str) -> std::result::Result<EventId, DbError> {
    let suffix = key.strip_prefix(prefix).ok_or_else(|| {
        DbError::Storage(format!(
            "WAL event key '{key}' is outside prefix '{prefix}'"
        ))
    })?;
    if suffix.len() != 20 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DbError::Storage(format!("invalid WAL event key '{key}'")));
    }
    let value = suffix
        .parse::<u64>()
        .map_err(|err| DbError::Storage(format!("invalid WAL event key '{key}': {err}")))?;
    EventId::new(value).ok_or_else(|| DbError::Storage(format!("invalid WAL event key '{key}'")))
}
