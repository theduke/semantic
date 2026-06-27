use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use semantic_data::value::serde::typed::{TypedRef, TypedValue};
use semantic_data::value::{FieldPath, Object, PathSegment, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};
use serde::{Deserialize, Serialize};

pub mod memory;
pub use memory::MemoryKvEngine;

const ENTITY_FORMAT_VERSION_PREFIX_LEN: usize = std::mem::size_of::<u16>();
const ENTITY_FORMAT_VERSION_V1_MSGPACK: u16 = 1;
const INDEX_FORMAT_VERSION_V1_MSGPACK: u16 = 1;

#[derive(facet::Facet, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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

#[derive(facet::Facet, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum KvWriteOp {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

#[derive(facet::Facet, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(facet::Facet, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
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

pub type KvScanItem = std::result::Result<(Vec<u8>, Vec<u8>), DbError>;
pub type BoxKvPrefixScan = Box<dyn Iterator<Item = KvScanItem> + Send>;

pub trait KvEngine: std::fmt::Debug + Send + Sync + 'static {
    type PrefixScan: Iterator<Item = KvScanItem> + Send + 'static;

    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError>;
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError>;
    fn delete(&mut self, key: &[u8]) -> std::result::Result<(), DbError>;
    fn scan_prefix_stream(&self, prefix: Vec<u8>)
    -> std::result::Result<Self::PrefixScan, DbError>;

    fn scan_prefix(&self, prefix: &[u8]) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.scan_prefix_stream(prefix.to_vec())?.collect()
    }

    fn tx_capabilities(&self) -> KvTransactionCapabilities {
        KvTransactionCapabilities::default()
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        Ok(None)
    }

    fn scan_prefix_at_revision_stream(
        &self,
        prefix: Vec<u8>,
        _revision: u64,
    ) -> std::result::Result<Self::PrefixScan, DbError> {
        self.scan_prefix_stream(prefix)
    }

    fn scan_prefix_at_revision(
        &self,
        prefix: &[u8],
        revision: u64,
    ) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.scan_prefix_at_revision_stream(prefix.to_vec(), revision)?
            .collect()
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

pub struct EntityScan<I> {
    inner: I,
}

impl<I> EntityScan<I> {
    fn new(inner: I) -> Self {
        Self { inner }
    }
}

impl<I> Iterator for EntityScan<I>
where
    I: Iterator<Item = KvScanItem>,
{
    type Item = std::result::Result<StoredEntity, DbError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|item| item.and_then(|(_, payload)| decode_entity(&payload)))
    }
}

pub struct IndexEntityIdScan<I> {
    inner: I,
    seen: BTreeSet<String>,
}

impl<I> IndexEntityIdScan<I> {
    fn new(inner: I) -> Self {
        Self {
            inner,
            seen: BTreeSet::new(),
        }
    }
}

impl<I> Iterator for IndexEntityIdScan<I>
where
    I: Iterator<Item = KvScanItem>,
{
    type Item = std::result::Result<String, DbError>;

    fn next(&mut self) -> Option<Self::Item> {
        for item in self.inner.by_ref() {
            match item {
                Ok((key, _)) => {
                    let Some(id) = extract_index_entity_id(&key) else {
                        continue;
                    };
                    if self.seen.insert(id.clone()) {
                        return Some(Ok(id));
                    }
                }
                Err(err) => return Some(Err(err)),
            }
        }
        None
    }
}

pub struct KvKeyScan<I> {
    inner: I,
}

impl<I> KvKeyScan<I> {
    fn new(inner: I) -> Self {
        Self { inner }
    }
}

impl<I> Iterator for KvKeyScan<I>
where
    I: Iterator<Item = KvScanItem>,
{
    type Item = std::result::Result<Vec<u8>, DbError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|item| item.map(|(key, _)| key))
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
        self.scan_raw_prefix_stream(prefix)?.collect()
    }

    pub fn scan_raw_prefix_stream(
        &self,
        prefix: &[u8],
    ) -> std::result::Result<E::PrefixScan, DbError> {
        self.engine.scan_prefix_stream(prefix.to_vec())
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
        self.scan_collection_stream(collection)?.collect()
    }

    pub fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<EntityScan<E::PrefixScan>, DbError> {
        let prefix = entity_prefix(collection);
        Ok(EntityScan::new(self.engine.scan_prefix_stream(prefix)?))
    }

    pub fn scan_collection_at_revision(
        &self,
        collection: LocalCollectionId,
        revision: u64,
    ) -> std::result::Result<Vec<StoredEntity>, DbError> {
        self.scan_collection_at_revision_stream(collection, revision)?
            .collect()
    }

    pub fn scan_collection_at_revision_stream(
        &self,
        collection: LocalCollectionId,
        revision: u64,
    ) -> std::result::Result<EntityScan<E::PrefixScan>, DbError> {
        let prefix = entity_prefix(collection);
        Ok(EntityScan::new(
            self.engine
                .scan_prefix_at_revision_stream(prefix, revision)?,
        ))
    }

    pub fn put_index_entry(
        &mut self,
        index: LocalIndexId,
        value: &Value,
        entity_id: &str,
    ) -> std::result::Result<(), DbError> {
        let key = index_key(index, None, value, entity_id)?;
        self.engine.put(key, Vec::new())
    }

    pub fn delete_index_entry(
        &mut self,
        index: LocalIndexId,
        value: &Value,
        entity_id: &str,
    ) -> std::result::Result<(), DbError> {
        let key = index_key(index, None, value, entity_id)?;
        self.engine.delete(&key)
    }

    pub fn scan_index_value(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> std::result::Result<Vec<String>, DbError> {
        self.scan_index_value_stream(index, path, value)?.collect()
    }

    pub fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> std::result::Result<IndexEntityIdScan<E::PrefixScan>, DbError> {
        let prefix = index_value_prefix(index, path, value)?;
        Ok(IndexEntityIdScan::new(
            self.engine.scan_prefix_stream(prefix)?,
        ))
    }

    pub fn collection_keys(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<Vec<Vec<u8>>, DbError> {
        self.collection_keys_stream(collection)?.collect()
    }

    pub fn collection_keys_stream(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<KvKeyScan<E::PrefixScan>, DbError> {
        let prefix = entity_prefix(collection);
        Ok(KvKeyScan::new(self.engine.scan_prefix_stream(prefix)?))
    }

    pub fn index_keys(&self, index: LocalIndexId) -> std::result::Result<Vec<Vec<u8>>, DbError> {
        self.index_keys_stream(index)?.collect()
    }

    pub fn index_keys_stream(
        &self,
        index: LocalIndexId,
    ) -> std::result::Result<KvKeyScan<E::PrefixScan>, DbError> {
        let prefix = index_prefix(index);
        Ok(KvKeyScan::new(self.engine.scan_prefix_stream(prefix)?))
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
    let Some((version, body)) = split_entity_payload_prefix(payload) else {
        return Err(DbError::Deserialization(
            "entity payload missing format version prefix".to_string(),
        ));
    };
    match version {
        ENTITY_FORMAT_VERSION_V1_MSGPACK => decode_msgpack_entity_v1(body),
        _ => Err(DbError::Deserialization(format!(
            "unsupported entity payload format version: {version}"
        ))),
    }
}

pub(crate) fn encode_entity(entity: &StoredEntity) -> std::result::Result<Vec<u8>, DbError> {
    let wire = StoredEntityWire {
        id: entity.id.clone(),
        collection: entity.collection,
        kind: entity.kind.clone(),
        object: entity
            .object
            .iter()
            .map(|(k, v)| (k.clone(), TypedValue(v.clone())))
            .collect(),
    };
    let body = rmp_serde::to_vec(&wire).map_err(|err| DbError::Serialization(err.to_string()))?;
    let mut out = Vec::with_capacity(ENTITY_FORMAT_VERSION_PREFIX_LEN + body.len());
    out.extend_from_slice(&ENTITY_FORMAT_VERSION_V1_MSGPACK.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

fn entity_prefix(collection: LocalCollectionId) -> Vec<u8> {
    format!("c/{}/e/", collection.0).into_bytes()
}

pub(crate) fn entity_key(collection: LocalCollectionId, id: &str) -> Vec<u8> {
    format!("c/{}/e/{}", collection.0, id).into_bytes()
}

fn index_value_prefix(
    index: LocalIndexId,
    path: Option<&FieldPath>,
    value: &Value,
) -> std::result::Result<Vec<u8>, DbError> {
    let mut key = index_path_prefix(index, path)?;
    key.extend_from_slice(b"v/");
    key.extend_from_slice(encode_index_value_token(value)?.as_bytes());
    key.extend_from_slice(b"/e/");
    Ok(key)
}

pub(crate) fn index_key(
    index: LocalIndexId,
    path: Option<&FieldPath>,
    value: &Value,
    entity_id: &str,
) -> std::result::Result<Vec<u8>, DbError> {
    let mut key = index_value_prefix(index, path, value)?;
    key.extend_from_slice(entity_id.as_bytes());
    Ok(key)
}

pub(crate) fn index_prefix(index: LocalIndexId) -> Vec<u8> {
    format!("i/{}/", index.0).into_bytes()
}

fn index_path_prefix(
    index: LocalIndexId,
    path: Option<&FieldPath>,
) -> std::result::Result<Vec<u8>, DbError> {
    if let Some(path) = path {
        let path_value = field_path_to_value(path);
        let path_token = encode_index_value_token(&path_value)?;
        Ok(format!("i/{}/p/{}/", index.0, path_token).into_bytes())
    } else {
        Ok(format!("i/{}/", index.0).into_bytes())
    }
}

pub(crate) fn index_format_key(index: LocalIndexId) -> Vec<u8> {
    format!("i/{}/fmt", index.0).into_bytes()
}

pub(crate) fn index_format_value() -> Vec<u8> {
    INDEX_FORMAT_VERSION_V1_MSGPACK.to_le_bytes().to_vec()
}

fn encode_index_value_token(value: &Value) -> std::result::Result<String, DbError> {
    let value_bytes = rmp_serde::to_vec(&TypedRef(value))
        .map_err(|err| DbError::Serialization(err.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(value_bytes))
}

fn field_path_to_value(path: &FieldPath) -> Value {
    let mut segments = Vec::with_capacity(path.segments().len());
    for segment in path.segments() {
        match segment {
            PathSegment::Field(field) => segments.push(Value::List(vec![
                Value::String("f".to_string()),
                Value::String(field.clone()),
            ])),
            PathSegment::Index(index) => segments.push(Value::List(vec![
                Value::String("i".to_string()),
                Value::U64(*index as u64),
            ])),
        }
    }
    Value::List(segments)
}

fn extract_index_entity_id(key: &[u8]) -> Option<String> {
    let marker = b"/e/";
    let pos = key.windows(marker.len()).position(|w| w == marker)?;
    let id = &key[(pos + marker.len())..];
    std::str::from_utf8(id).ok().map(|s| s.to_string())
}

pub(crate) fn collect_index_entries(object: &Object) -> Vec<(FieldPath, Value)> {
    let mut out = Vec::new();
    for (field, value) in object {
        let mut path = FieldPath::new();
        path.push_field(field.clone());
        collect_value_entries(value, &mut path, &mut out);
    }
    out
}

fn collect_value_entries(value: &Value, path: &mut FieldPath, out: &mut Vec<(FieldPath, Value)>) {
    out.push((path.clone(), value.clone()));
    match value {
        Value::Object(object) => {
            for (field, nested) in object {
                path.push_field(field.clone());
                collect_value_entries(nested, path, out);
                path.0.pop();
            }
        }
        Value::List(items) => {
            for (idx, nested) in items.iter().enumerate() {
                path.push_index(idx);
                collect_value_entries(nested, path, out);
                path.0.pop();
            }
        }
        _ => {}
    }
}

#[derive(Serialize, Deserialize)]
struct StoredEntityWire {
    id: String,
    collection: usize,
    kind: StoredEntityKind,
    object: BTreeMap<String, TypedValue>,
}

fn decode_msgpack_entity_v1(payload: &[u8]) -> std::result::Result<StoredEntity, DbError> {
    let wire: StoredEntityWire =
        rmp_serde::from_slice(payload).map_err(|err| DbError::Deserialization(err.to_string()))?;
    let mut object = Object::new();
    for (field, value) in wire.object {
        object.insert(field, value.0);
    }
    Ok(StoredEntity {
        id: wire.id,
        collection: wire.collection,
        kind: wire.kind,
        object,
    })
}

fn split_entity_payload_prefix(payload: &[u8]) -> Option<(u16, &[u8])> {
    if payload.len() < ENTITY_FORMAT_VERSION_PREFIX_LEN {
        return None;
    }
    let mut prefix = [0u8; ENTITY_FORMAT_VERSION_PREFIX_LEN];
    prefix.copy_from_slice(&payload[..ENTITY_FORMAT_VERSION_PREFIX_LEN]);
    let version = u16::from_le_bytes(prefix);
    Some((version, &payload[ENTITY_FORMAT_VERSION_PREFIX_LEN..]))
}
