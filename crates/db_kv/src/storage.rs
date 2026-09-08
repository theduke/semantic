use std::collections::{BTreeMap, BTreeSet};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use semantic_data::schema::IndexKind;
use semantic_data::value::serde::typed::{TypedRef, TypedValue};
use semantic_data::value::{FieldPath, Object, PathSegment, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};
use semantic_db_core::embedded::{
    BoxEntityIdScan, BoxEntityScan, EntityStorage, StorageCommitOutcome,
    StorageTransactionCapabilities, StorageWriteOp, StoredEntity, StoredEntityKind,
};
use serde::{Deserialize, Serialize};

pub mod memory;
pub use memory::MemoryKvEngine;

const ENTITY_FORMAT_VERSION_PREFIX_LEN: usize = std::mem::size_of::<u16>();
const ENTITY_FORMAT_VERSION_V1_MSGPACK: u16 = 1;
// Version 2 forces legacy indexes to be rebuilt. Some databases could retain a
// format marker without complete index entries, which made indexed equality
// queries return false empty results.
const INDEX_FORMAT_VERSION_V2_MSGPACK: u16 = 2;

#[derive(facet::Facet, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum KvWriteOp {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
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

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        StorageTransactionCapabilities::default()
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
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        self.write_batch(ops)?;
        Ok(StorageCommitOutcome::Committed {
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

    pub fn tx_capabilities(&self) -> StorageTransactionCapabilities {
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
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        self.engine.write_batch_conditional(ops, expected_revision)
    }
}

impl<E: KvEngine> EntityStorage for EntityStore<E> {
    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> std::result::Result<Option<StoredEntity>, DbError> {
        EntityStore::get_entity(self, collection, id)
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<BoxEntityScan, DbError> {
        Ok(Box::new(EntityStore::scan_collection_stream(
            self, collection,
        )?))
    }

    fn scan_collection_at_revision_stream(
        &self,
        collection: LocalCollectionId,
        revision: u64,
    ) -> std::result::Result<BoxEntityScan, DbError> {
        Ok(Box::new(EntityStore::scan_collection_at_revision_stream(
            self, collection, revision,
        )?))
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> std::result::Result<BoxEntityIdScan, DbError> {
        Ok(Box::new(EntityStore::scan_index_value_stream(
            self, index, path, value,
        )?))
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> std::result::Result<bool, DbError> {
        Ok(self.engine.get(&index_format_key(index))?.as_deref()
            != Some(index_format_value().as_slice()))
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.engine.tx_capabilities()
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        self.engine.current_revision()
    }

    fn apply_batch(&mut self, ops: &[StorageWriteOp]) -> std::result::Result<(), DbError> {
        let lowered = self.lower_write_ops(ops)?;
        self.engine.write_batch(&lowered)
    }

    fn apply_batch_conditional(
        &mut self,
        ops: &[StorageWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        let lowered = self.lower_write_ops(ops)?;
        self.engine
            .write_batch_conditional(&lowered, expected_revision)
    }
}

impl<E: KvEngine> EntityStore<E> {
    fn lower_write_ops(
        &self,
        operations: &[StorageWriteOp],
    ) -> std::result::Result<Vec<KvWriteOp>, DbError> {
        let mut lowered = Vec::new();
        for operation in operations {
            match operation {
                StorageWriteOp::PutEntity(entity) => lowered.push(KvWriteOp::Put {
                    key: entity_key(LocalCollectionId(entity.collection), &entity.id),
                    value: encode_entity(entity)?,
                }),
                StorageWriteOp::ClearCollection(collection) => {
                    push_prefix_deletes(
                        &mut lowered,
                        self.engine.scan_prefix(&entity_prefix(*collection))?,
                        &entity_prefix(*collection),
                    );
                }
                StorageWriteOp::ClearIndex(index) => {
                    push_prefix_deletes(
                        &mut lowered,
                        self.engine.scan_prefix(&index_prefix(*index))?,
                        &index_prefix(*index),
                    );
                }
                StorageWriteOp::ResetIndex(index) => {
                    push_prefix_deletes(
                        &mut lowered,
                        self.engine.scan_prefix(&index_prefix(*index))?,
                        &index_prefix(*index),
                    );
                    lowered.push(KvWriteOp::Put {
                        key: index_format_key(*index),
                        value: index_format_value(),
                    });
                }
                StorageWriteOp::IndexEntity {
                    index,
                    entity_id,
                    object,
                } => match index.schema.kind {
                    IndexKind::Equality => {
                        if let Some(value) = object.get(&index.canonical_field) {
                            lowered.push(KvWriteOp::Put {
                                key: index_key(index.lid, None, value, entity_id)?,
                                value: Vec::new(),
                            });
                        }
                    }
                    IndexKind::PathEquality => {
                        for (path, value) in collect_index_entries(object) {
                            lowered.push(KvWriteOp::Put {
                                key: index_key(index.lid, Some(&path), &value, entity_id)?,
                                value: Vec::new(),
                            });
                        }
                    }
                    IndexKind::Range | IndexKind::FullText => {}
                },
            }
        }
        Ok(lowered)
    }
}

fn push_prefix_deletes(
    operations: &mut Vec<KvWriteOp>,
    existing: Vec<(Vec<u8>, Vec<u8>)>,
    prefix: &[u8],
) {
    let mut keys = existing
        .into_iter()
        .map(|(key, _)| key)
        .collect::<BTreeSet<_>>();
    keys.extend(operations.iter().filter_map(|operation| match operation {
        KvWriteOp::Put { key, .. } if key.starts_with(prefix) => Some(key.clone()),
        _ => None,
    }));
    operations.extend(keys.into_iter().map(|key| KvWriteOp::Delete { key }));
}

pub fn decode_entity(payload: &[u8]) -> std::result::Result<StoredEntity, DbError> {
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

pub fn encode_entity(entity: &StoredEntity) -> std::result::Result<Vec<u8>, DbError> {
    let wire = StoredEntityWire {
        id: entity.id.clone(),
        collection: entity.collection,
        kind: (&entity.kind).into(),
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

/// Decode an entity key into its collection and entity identifier.
pub fn parse_entity_key(key: &[u8]) -> Option<(LocalCollectionId, &str)> {
    let key = std::str::from_utf8(key).ok()?;
    let key = key.strip_prefix("c/")?;
    let (collection, id) = key.split_once("/e/")?;
    Some((LocalCollectionId(collection.parse().ok()?), id))
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
    INDEX_FORMAT_VERSION_V2_MSGPACK.to_le_bytes().to_vec()
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
    kind: StoredEntityKindWire,
    object: BTreeMap<String, TypedValue>,
}

#[derive(Serialize, Deserialize)]
enum StoredEntityKindWire {
    Untyped,
    Record,
    Class,
}

impl From<&StoredEntityKind> for StoredEntityKindWire {
    fn from(value: &StoredEntityKind) -> Self {
        match value {
            StoredEntityKind::Untyped => Self::Untyped,
            StoredEntityKind::Record => Self::Record,
            StoredEntityKind::Class => Self::Class,
        }
    }
}

impl From<StoredEntityKindWire> for StoredEntityKind {
    fn from(value: StoredEntityKindWire) -> Self {
        match value {
            StoredEntityKindWire::Untyped => Self::Untyped,
            StoredEntityKindWire::Record => Self::Record,
            StoredEntityKindWire::Class => Self::Class,
        }
    }
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
        kind: wire.kind.into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::schema::{IndexSchema as DataIndexSchema, KeyPath};
    use semantic_db_core::catalog::IndexSchema;

    #[derive(Serialize)]
    struct LegacyStoredEntityWire {
        id: String,
        collection: usize,
        kind: LegacyStoredEntityKind,
        object: BTreeMap<String, TypedValue>,
    }

    #[derive(Serialize)]
    enum LegacyStoredEntityKind {
        Untyped,
    }

    #[test]
    fn parses_entity_key_with_delimiters_in_id() {
        let key = entity_key(LocalCollectionId(12), "path/with/e/delimiters");
        assert_eq!(
            parse_entity_key(&key),
            Some((LocalCollectionId(12), "path/with/e/delimiters"))
        );
        assert_eq!(parse_entity_key(b"i/12/v/token/e/id"), None);
    }

    #[test]
    fn decodes_original_v1_entity_kind_variant_names() {
        let legacy = LegacyStoredEntityWire {
            id: "one".to_string(),
            collection: 7,
            kind: LegacyStoredEntityKind::Untyped,
            object: BTreeMap::new(),
        };
        let mut encoded = ENTITY_FORMAT_VERSION_V1_MSGPACK.to_le_bytes().to_vec();
        encoded.extend(rmp_serde::to_vec_named(&legacy).unwrap());

        assert_eq!(
            decode_entity(&encoded).unwrap(),
            StoredEntity {
                id: "one".to_string(),
                collection: 7,
                kind: StoredEntityKind::Untyped,
                object: Object::new(),
            }
        );
    }

    #[test]
    fn lowering_preserves_clear_after_staged_entity_put() {
        let collection = LocalCollectionId(7);
        let mut store = EntityStore::new(MemoryKvEngine::new());
        let entity = StoredEntity {
            id: "one".to_string(),
            collection: collection.0,
            kind: StoredEntityKind::Untyped,
            object: Object::new(),
        };

        store
            .apply_batch(&[
                StorageWriteOp::PutEntity(entity),
                StorageWriteOp::ClearCollection(collection),
            ])
            .unwrap();

        assert_eq!(store.get_entity(collection, "one").unwrap(), None);
    }

    #[test]
    fn lowering_preserves_index_reset_and_clear_order() {
        let index = IndexSchema {
            lid: LocalIndexId(3),
            schema: DataIndexSchema {
                id: "items.by_kind".to_string(),
                name: "by_kind".to_string(),
                kind: IndexKind::Equality,
                collection: "items".to_string(),
                key_path: KeyPath {
                    segments: vec!["kind".to_string()],
                },
                unique: false,
            },
            collection: LocalCollectionId(7),
            canonical_field: "kind".to_string(),
            field_id: None,
            attr_id: None,
        };
        let mut object = Object::new();
        object.insert("kind", Value::String("music".to_string()));
        let mut store = EntityStore::new(MemoryKvEngine::new());

        store
            .apply_batch(&[
                StorageWriteOp::ResetIndex(index.lid),
                StorageWriteOp::IndexEntity {
                    index: index.clone(),
                    entity_id: "one".to_string(),
                    object: object.clone(),
                },
                StorageWriteOp::ClearIndex(index.lid),
            ])
            .unwrap();
        assert!(store.index_needs_rebuild(index.lid).unwrap());
        assert!(
            store
                .scan_index_value(index.lid, None, &Value::String("music".to_string()))
                .unwrap()
                .is_empty()
        );

        store
            .apply_batch(&[
                StorageWriteOp::ClearIndex(index.lid),
                StorageWriteOp::ResetIndex(index.lid),
                StorageWriteOp::IndexEntity {
                    index: index.clone(),
                    entity_id: "one".to_string(),
                    object,
                },
            ])
            .unwrap();
        assert!(!store.index_needs_rebuild(index.lid).unwrap());
        assert_eq!(
            store
                .scan_index_value(index.lid, None, &Value::String("music".to_string()))
                .unwrap(),
            vec!["one".to_string()]
        );
    }

    #[test]
    fn index_entries_are_additive_idempotent_and_resettable() {
        let index = IndexSchema {
            lid: LocalIndexId(3),
            schema: DataIndexSchema {
                id: "items.by_kind".to_string(),
                name: "by_kind".to_string(),
                kind: IndexKind::Equality,
                collection: "items".to_string(),
                key_path: KeyPath {
                    segments: vec!["kind".to_string()],
                },
                unique: false,
            },
            collection: LocalCollectionId(7),
            canonical_field: "kind".to_string(),
            field_id: None,
            attr_id: None,
        };
        let mut old = Object::new();
        old.insert("kind", Value::String("old".to_string()));
        let mut new = Object::new();
        new.insert("kind", Value::String("new".to_string()));
        let mut store = EntityStore::new(MemoryKvEngine::new());

        store
            .apply_batch(&[
                StorageWriteOp::ResetIndex(index.lid),
                StorageWriteOp::IndexEntity {
                    index: index.clone(),
                    entity_id: "one".to_string(),
                    object: old.clone(),
                },
                StorageWriteOp::IndexEntity {
                    index: index.clone(),
                    entity_id: "one".to_string(),
                    object: new.clone(),
                },
                StorageWriteOp::IndexEntity {
                    index: index.clone(),
                    entity_id: "one".to_string(),
                    object: new,
                },
            ])
            .unwrap();
        assert_eq!(
            store
                .scan_index_value(index.lid, None, &Value::String("old".to_string()))
                .unwrap(),
            vec!["one".to_string()]
        );
        assert_eq!(
            store
                .scan_index_value(index.lid, None, &Value::String("new".to_string()))
                .unwrap(),
            vec!["one".to_string()]
        );

        store
            .apply_batch(&[
                StorageWriteOp::ResetIndex(index.lid),
                StorageWriteOp::IndexEntity {
                    index: index.clone(),
                    entity_id: "one".to_string(),
                    object: old,
                },
            ])
            .unwrap();
        assert!(
            store
                .scan_index_value(index.lid, None, &Value::String("new".to_string()))
                .unwrap()
                .is_empty()
        );
    }
}
