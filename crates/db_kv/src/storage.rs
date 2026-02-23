use std::collections::BTreeSet;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use semantic_data::value::{Object, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};

pub mod memory;
pub use memory::MemoryKvEngine;

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

    pub fn scan_raw_prefix(
        &self,
        prefix: &[u8],
    ) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.engine.scan_prefix(prefix)
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
