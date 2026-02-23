use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};

use crate::error::DbError;

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum StoredEntityKind {
    Untyped,
    Record,
    Class,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct StoredEntity {
    pub id: String,
    pub collection: usize,
    pub kind: StoredEntityKind,
    pub object: Object,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum KvWriteOp {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub struct KvTransactionCapabilities {
    pub conflict_detection: bool,
    pub mvcc: bool,
    pub snapshot_reads: bool,
}

impl Default for KvTransactionCapabilities {
    fn default() -> Self {
        Self {
            conflict_detection: false,
            mvcc: false,
            snapshot_reads: false,
        }
    }
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum KvCommitOutcome {
    Committed {
        revision: Option<u64>,
    },
    Conflict {
        expected_revision: Option<u64>,
        actual_revision: Option<u64>,
    },
}

pub trait KvEngine: std::fmt::Debug + Send + Sync + 'static {
    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError>;
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError>;
    fn delete(&mut self, key: &[u8]) -> std::result::Result<(), DbError>;
    fn scan_prefix(&self, prefix: &[u8]) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError>;

    fn tx_capabilities(&self) -> KvTransactionCapabilities {
        KvTransactionCapabilities::default()
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        Ok(None)
    }

    fn scan_prefix_at_revision(
        &self,
        prefix: &[u8],
        _revision: u64,
    ) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.scan_prefix(prefix)
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        _expected_revision: Option<u64>,
    ) -> std::result::Result<KvCommitOutcome, DbError> {
        self.write_batch(ops)?;
        Ok(KvCommitOutcome::Committed {
            revision: self.current_revision()?,
        })
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> std::result::Result<(), DbError> {
        for op in ops {
            match op {
                KvWriteOp::Put { key, value } => self.put(key.clone(), value.clone())?,
                KvWriteOp::Delete { key } => self.delete(key)?,
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryKvEngine {
    map: BTreeMap<Vec<u8>, Vec<u8>>,
    revision: u64,
    mvcc_snapshots: Option<BTreeMap<u64, BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl MemoryKvEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mvcc(enable_mvcc: bool) -> Self {
        let mut engine = Self::default();
        if enable_mvcc {
            engine.mvcc_snapshots = Some(BTreeMap::new());
            engine.capture_snapshot();
        }
        engine
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.capture_snapshot();
    }

    fn capture_snapshot(&mut self) {
        if let Some(snapshots) = &mut self.mvcc_snapshots {
            snapshots.insert(self.revision, self.map.clone());
            if snapshots.len() > 64 {
                let keep_from = snapshots
                    .keys()
                    .rev()
                    .nth(63)
                    .copied()
                    .unwrap_or(self.revision);
                snapshots.retain(|rev, _| *rev >= keep_from);
            }
        }
    }
}

impl KvEngine for MemoryKvEngine {
    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError> {
        Ok(self.map.get(key).cloned())
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError> {
        self.map.insert(key, value);
        self.bump_revision();
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> std::result::Result<(), DbError> {
        self.map.remove(key);
        self.bump_revision();
        Ok(())
    }

    fn scan_prefix(&self, prefix: &[u8]) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        Ok(self
            .map
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> std::result::Result<(), DbError> {
        for op in ops {
            match op {
                KvWriteOp::Put { key, value } => {
                    self.map.insert(key.clone(), value.clone());
                }
                KvWriteOp::Delete { key } => {
                    self.map.remove(key);
                }
            }
        }
        if !ops.is_empty() {
            self.bump_revision();
        }
        Ok(())
    }

    fn tx_capabilities(&self) -> KvTransactionCapabilities {
        KvTransactionCapabilities {
            conflict_detection: true,
            mvcc: self.mvcc_snapshots.is_some(),
            snapshot_reads: self.mvcc_snapshots.is_some(),
        }
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        Ok(Some(self.revision))
    }

    fn scan_prefix_at_revision(
        &self,
        prefix: &[u8],
        revision: u64,
    ) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        if revision == self.revision {
            return self.scan_prefix(prefix);
        }
        let Some(snapshots) = &self.mvcc_snapshots else {
            return Err(DbError::Storage(
                "snapshot reads require an mvcc-enabled engine".to_string(),
            ));
        };
        let Some(snapshot) = snapshots.get(&revision) else {
            return Err(DbError::Storage(format!(
                "mvcc snapshot for revision {revision} not available"
            )));
        };
        Ok(snapshot
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<KvCommitOutcome, DbError> {
        let actual = Some(self.revision);
        if let Some(expected) = expected_revision
            && actual != Some(expected)
        {
            return Ok(KvCommitOutcome::Conflict {
                expected_revision: Some(expected),
                actual_revision: actual,
            });
        }
        self.write_batch(ops)?;
        Ok(KvCommitOutcome::Committed {
            revision: Some(self.revision),
        })
    }
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub struct FileKvConfig {
    pub enable_mvcc: bool,
}

#[derive(Debug, Clone)]
pub struct FileKvEngine {
    path: PathBuf,
    map: BTreeMap<Vec<u8>, Vec<u8>>,
    revision: u64,
    config: FileKvConfig,
    mvcc_snapshots: Option<BTreeMap<u64, BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl FileKvEngine {
    pub fn open(path: impl Into<PathBuf>) -> std::result::Result<Self, DbError> {
        Self::open_with_config(path, FileKvConfig::default())
    }

    pub fn open_with_config(
        path: impl Into<PathBuf>,
        config: FileKvConfig,
    ) -> std::result::Result<Self, DbError> {
        let path = path.into();
        let (map, revision) = if path.exists() {
            load_snapshot(&path)?
        } else {
            (BTreeMap::new(), 0)
        };
        let mut engine = Self {
            path,
            map,
            revision,
            config,
            mvcc_snapshots: None,
        };
        if engine.config.enable_mvcc {
            engine.mvcc_snapshots = Some(BTreeMap::new());
            engine.capture_snapshot();
        }
        Ok(engine)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.capture_snapshot();
    }

    fn capture_snapshot(&mut self) {
        if let Some(snapshots) = &mut self.mvcc_snapshots {
            snapshots.insert(self.revision, self.map.clone());
            if snapshots.len() > 64 {
                let keep_from = snapshots
                    .keys()
                    .rev()
                    .nth(63)
                    .copied()
                    .unwrap_or(self.revision);
                snapshots.retain(|rev, _| *rev >= keep_from);
            }
        }
    }

    fn persist(&self) -> std::result::Result<(), DbError> {
        let snapshot = FileKvSnapshot {
            revision: self.revision,
            entries: self
                .map
                .iter()
                .map(|(key, value)| KvEntry {
                    key: key.clone(),
                    value: value.clone(),
                })
                .collect(),
        };

        let bytes =
            facet_json::to_vec(&snapshot).map_err(|err| DbError::Serialization(err.to_string()))?;
        std::fs::write(&self.path, bytes).map_err(|err| DbError::Storage(err.to_string()))
    }
}

impl KvEngine for FileKvEngine {
    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError> {
        Ok(self.map.get(key).cloned())
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError> {
        self.map.insert(key, value);
        self.bump_revision();
        self.persist()
    }

    fn delete(&mut self, key: &[u8]) -> std::result::Result<(), DbError> {
        self.map.remove(key);
        self.bump_revision();
        self.persist()
    }

    fn scan_prefix(&self, prefix: &[u8]) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        Ok(self
            .map
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> std::result::Result<(), DbError> {
        for op in ops {
            match op {
                KvWriteOp::Put { key, value } => {
                    self.map.insert(key.clone(), value.clone());
                }
                KvWriteOp::Delete { key } => {
                    self.map.remove(key);
                }
            }
        }
        if !ops.is_empty() {
            self.bump_revision();
        }
        self.persist()
    }

    fn tx_capabilities(&self) -> KvTransactionCapabilities {
        KvTransactionCapabilities {
            conflict_detection: true,
            mvcc: self.config.enable_mvcc,
            snapshot_reads: self.config.enable_mvcc,
        }
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        Ok(Some(self.revision))
    }

    fn scan_prefix_at_revision(
        &self,
        prefix: &[u8],
        revision: u64,
    ) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        if revision == self.revision {
            return self.scan_prefix(prefix);
        }
        let Some(snapshots) = &self.mvcc_snapshots else {
            return Err(DbError::Storage(
                "snapshot reads require an mvcc-enabled engine".to_string(),
            ));
        };
        let Some(snapshot) = snapshots.get(&revision) else {
            return Err(DbError::Storage(format!(
                "mvcc snapshot for revision {revision} not available"
            )));
        };
        Ok(snapshot
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<KvCommitOutcome, DbError> {
        let actual = Some(self.revision);
        if let Some(expected) = expected_revision {
            if actual != Some(expected) {
                return Ok(KvCommitOutcome::Conflict {
                    expected_revision: Some(expected),
                    actual_revision: actual,
                });
            }
        }
        self.write_batch(ops)?;
        Ok(KvCommitOutcome::Committed {
            revision: Some(self.revision),
        })
    }
}

#[derive(Debug)]
pub struct EntityStore<E: KvEngine> {
    engine: E,
}

impl<E: KvEngine> EntityStore<E> {
    pub fn new(engine: E) -> Self {
        Self { engine }
    }

    pub fn engine(&self) -> &E {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut E {
        &mut self.engine
    }

    pub fn into_inner(self) -> E {
        self.engine
    }

    pub fn tx_capabilities(&self) -> KvTransactionCapabilities {
        self.engine.tx_capabilities()
    }

    pub fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        self.engine.current_revision()
    }

    pub fn get_raw(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError> {
        self.engine.get(key)
    }

    pub fn put_raw(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError> {
        self.engine.put(key, value)
    }

    pub fn put_entity(&mut self, entity: &StoredEntity) -> std::result::Result<(), DbError> {
        let key = entity_key(LocalCollectionId(entity.collection), &entity.id);
        let payload = encode_entity(entity)?;
        self.engine.put(key, payload)
    }

    pub fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> std::result::Result<Option<StoredEntity>, DbError> {
        let key = entity_key(collection, id);
        let Some(payload) = self.engine.get(&key)? else {
            return Ok(None);
        };
        decode_entity(&payload).map(Some)
    }

    pub fn delete_entity(
        &mut self,
        collection: LocalCollectionId,
        id: &str,
    ) -> std::result::Result<(), DbError> {
        let key = entity_key(collection, id);
        self.engine.delete(&key)
    }

    pub fn scan_collection(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<Vec<StoredEntity>, DbError> {
        let prefix = entity_prefix(collection);
        let pairs = self.engine.scan_prefix(&prefix)?;
        pairs
            .into_iter()
            .map(|(_, payload)| decode_entity(&payload))
            .collect()
    }

    pub fn scan_collection_at_revision(
        &self,
        collection: LocalCollectionId,
        revision: u64,
    ) -> std::result::Result<Vec<StoredEntity>, DbError> {
        let prefix = entity_prefix(collection);
        let pairs = self.engine.scan_prefix_at_revision(&prefix, revision)?;
        pairs
            .into_iter()
            .map(|(_, payload)| decode_entity(&payload))
            .collect()
    }

    pub fn put_index_entry(
        &mut self,
        index: LocalIndexId,
        value: &Value,
        entity_id: &str,
    ) -> std::result::Result<(), DbError> {
        let key = index_key(index, value, entity_id)?;
        self.engine.put(key, Vec::new())
    }

    pub fn delete_index_entry(
        &mut self,
        index: LocalIndexId,
        value: &Value,
        entity_id: &str,
    ) -> std::result::Result<(), DbError> {
        let key = index_key(index, value, entity_id)?;
        self.engine.delete(&key)
    }

    pub fn scan_index_value(
        &self,
        index: LocalIndexId,
        value: &Value,
    ) -> std::result::Result<Vec<String>, DbError> {
        let prefix = index_value_prefix(index, value)?;
        let pairs = self.engine.scan_prefix(&prefix)?;
        let mut ids = BTreeSet::new();
        for (key, _) in pairs {
            if let Some(id) = extract_index_entity_id(&key) {
                ids.insert(id);
            }
        }
        Ok(ids.into_iter().collect())
    }

    pub fn collection_keys(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<Vec<Vec<u8>>, DbError> {
        let prefix = entity_prefix(collection);
        let pairs = self.engine.scan_prefix(&prefix)?;
        Ok(pairs.into_iter().map(|(k, _)| k).collect())
    }

    pub fn index_keys(&self, index: LocalIndexId) -> std::result::Result<Vec<Vec<u8>>, DbError> {
        let prefix = index_prefix(index);
        let pairs = self.engine.scan_prefix(&prefix)?;
        Ok(pairs.into_iter().map(|(k, _)| k).collect())
    }

    pub fn write_batch(&mut self, ops: &[KvWriteOp]) -> std::result::Result<(), DbError> {
        self.engine.write_batch(ops)
    }

    pub fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<KvCommitOutcome, DbError> {
        self.engine.write_batch_conditional(ops, expected_revision)
    }
}

fn decode_entity(payload: &[u8]) -> std::result::Result<StoredEntity, DbError> {
    facet_json::from_slice(payload).map_err(|err| DbError::Deserialization(err.to_string()))
}

pub(crate) fn encode_entity(entity: &StoredEntity) -> std::result::Result<Vec<u8>, DbError> {
    facet_json::to_vec(entity).map_err(|err| DbError::Serialization(err.to_string()))
}

fn entity_prefix(collection: LocalCollectionId) -> Vec<u8> {
    format!("c/{}/e/", collection.0).into_bytes()
}

pub(crate) fn entity_key(collection: LocalCollectionId, id: &str) -> Vec<u8> {
    format!("c/{}/e/{}", collection.0, id).into_bytes()
}

fn index_value_prefix(index: LocalIndexId, value: &Value) -> std::result::Result<Vec<u8>, DbError> {
    let value_bytes =
        facet_json::to_vec(value).map_err(|err| DbError::Serialization(err.to_string()))?;
    let token = URL_SAFE_NO_PAD.encode(value_bytes);
    Ok(format!("i/{}/v/{}/e/", index.0, token).into_bytes())
}

pub(crate) fn index_key(
    index: LocalIndexId,
    value: &Value,
    entity_id: &str,
) -> std::result::Result<Vec<u8>, DbError> {
    let mut key = index_value_prefix(index, value)?;
    key.extend_from_slice(entity_id.as_bytes());
    Ok(key)
}

pub(crate) fn index_prefix(index: LocalIndexId) -> Vec<u8> {
    format!("i/{}/v/", index.0).into_bytes()
}

fn extract_index_entity_id(key: &[u8]) -> Option<String> {
    let marker = b"/e/";
    let pos = key.windows(marker.len()).position(|w| w == marker)?;
    let id = &key[(pos + marker.len())..];
    std::str::from_utf8(id).ok().map(|s| s.to_string())
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq, Default)]
struct FileKvSnapshot {
    revision: u64,
    entries: Vec<KvEntry>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
struct KvEntry {
    key: Vec<u8>,
    value: Vec<u8>,
}

fn load_snapshot(path: &Path) -> std::result::Result<(BTreeMap<Vec<u8>, Vec<u8>>, u64), DbError> {
    let bytes = std::fs::read(path).map_err(|err| DbError::Storage(err.to_string()))?;
    if bytes.is_empty() {
        return Ok((BTreeMap::new(), 0));
    }

    let snapshot: FileKvSnapshot =
        facet_json::from_slice(&bytes).map_err(|err| DbError::Deserialization(err.to_string()))?;

    Ok((
        snapshot
            .entries
            .into_iter()
            .map(|entry| (entry.key, entry.value))
            .collect(),
        snapshot.revision,
    ))
}

#[cfg(test)]
mod tests {
    use super::{KvCommitOutcome, KvEngine, KvWriteOp, MemoryKvEngine};

    #[test]
    fn conditional_write_reports_conflict() {
        let mut engine = MemoryKvEngine::new();
        engine
            .write_batch(&[KvWriteOp::Put {
                key: b"k".to_vec(),
                value: b"v1".to_vec(),
            }])
            .unwrap();

        let out = engine
            .write_batch_conditional(
                &[KvWriteOp::Put {
                    key: b"k".to_vec(),
                    value: b"v2".to_vec(),
                }],
                Some(0),
            )
            .unwrap();
        assert_eq!(
            out,
            KvCommitOutcome::Conflict {
                expected_revision: Some(0),
                actual_revision: Some(1),
            }
        );
    }

    #[test]
    fn mvcc_snapshot_read_returns_historic_state() {
        let mut engine = MemoryKvEngine::with_mvcc(true);
        engine
            .write_batch(&[KvWriteOp::Put {
                key: b"pref/a".to_vec(),
                value: b"one".to_vec(),
            }])
            .unwrap();
        let rev1 = engine.current_revision().unwrap().unwrap();

        engine
            .write_batch(&[KvWriteOp::Put {
                key: b"pref/a".to_vec(),
                value: b"two".to_vec(),
            }])
            .unwrap();

        let past = engine.scan_prefix_at_revision(b"pref/", rev1).unwrap();
        assert_eq!(past.len(), 1);
        assert_eq!(past[0].1, b"one".to_vec());
    }
}
