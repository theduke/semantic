use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;
use std::time::Instant;

use semantic_data::schema::IndexKind;
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};
use semantic_db_core::embedded::{
    BackupSource, BoxCheckedEntityScan, BoxEntityIdScan, BoxEntityScan, BoxIndexKeyScan,
    EntityReadSnapshot, EntityStorage, IndexKeyEntry, PayloadRewriteBatch, StorageCommitOutcome,
    StorageStats, StorageTransactionCapabilities, StorageWriteOp, StoredEntity,
    unsupported_storage_maintenance,
};
use semantic_db_core::{DbError, StorageErrorKind};
use serde::{Deserialize, Serialize};

pub use crate::keys::parse_entity_key;
use crate::keys::{
    collection_rows_key, entity_key, entity_prefix, index_entries_key, index_entry_id, index_key,
    index_key_entity_id, index_marker_key, index_path_prefix, index_prefix, index_range,
    index_string_prefix, index_value_prefix, try_index_key,
};

pub mod entity_codec;
pub(crate) mod field_dict;
mod index_scan;
pub mod layout;
pub mod memory;
pub mod stats;
mod value_codec;
pub use entity_codec::{EntityPayloadFormat, decode_entity, encode_entity};
use field_dict::{DictStaging, FieldDictCache, HandleDicts, ScanDecoder};
pub use memory::MemoryKvEngine;
use stats::CounterDeltas;

#[cfg(test)]
mod entity_format_tests;
#[cfg(test)]
mod full_text_tests;
#[cfg(test)]
mod incremental_tests;
#[cfg(test)]
mod index_range_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod maintenance_tests;
#[cfg(test)]
mod stats_tests;
#[cfg(test)]
mod txn_tests;
#[cfg(test)]
mod write_amplification_tests;

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

/// A read handle over a key-value engine.
///
/// Native implementations serve every read from one engine transaction, so
/// all reads observe the state at [`Self::revision`] (see
/// [`Self::is_snapshot`]). Scans are returned as owned iterators.
pub trait KvReadTxn: Send + Sync {
    /// Revision of the state observed by this handle.
    fn revision(&self) -> Option<u64>;

    /// Whether all reads observe one engine state, independent of later
    /// commits.
    fn is_snapshot(&self) -> bool;

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError>;

    /// Scan keys in `start..end` (or `start..` when `end` is `None`) in key
    /// order.
    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError>;

    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<BoxKvPrefixScan, DbError> {
        let end = prefix_range_end(&prefix);
        self.scan_range_stream(prefix, end)
    }

    /// Scan keys in `start..end` (or `start..`) in descending key order.
    ///
    /// The default buffers the ascending scan; ordered engines should
    /// iterate backwards lazily.
    fn scan_range_rev_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        let mut items = self
            .scan_range_stream(start, end)?
            .collect::<Result<Vec<_>, _>>()?;
        items.reverse();
        Ok(Box::new(items.into_iter().map(Ok)))
    }
}

/// Mutable access to one engine write transaction.
///
/// Reads observe the committed state plus the writes already made through
/// this handle.
pub trait KvWriteTxn {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError>;
    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError>;
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError>;
    fn delete(&mut self, key: &[u8]) -> Result<(), DbError>;
}

/// Read handle for engines without native read transactions: every read is
/// forwarded to the engine.
struct ForwardingReadTxn<'a, E: ?Sized> {
    engine: &'a E,
    revision: Option<u64>,
}

impl<E: KvEngine + ?Sized> KvReadTxn for ForwardingReadTxn<'_, E> {
    fn revision(&self) -> Option<u64> {
        self.revision
    }

    fn is_snapshot(&self) -> bool {
        false
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        self.engine.get(key)
    }

    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        self.engine.scan_range_stream(start, end)
    }

    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<BoxKvPrefixScan, DbError> {
        Ok(Box::new(self.engine.scan_prefix_stream(prefix)?))
    }
}

/// Write handle for engines without native write transactions.
///
/// Buffers writes and commits them as one conditional batch.
struct BufferedWriteTxn<'a, E: ?Sized> {
    engine: &'a E,
    writes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
}

impl<E: KvEngine + ?Sized> BufferedWriteTxn<'_, E> {
    fn into_ops(self) -> Vec<KvWriteOp> {
        self.writes
            .into_iter()
            .map(|(key, value)| match value {
                Some(value) => KvWriteOp::Put { key, value },
                None => KvWriteOp::Delete { key },
            })
            .collect()
    }
}

impl<E: KvEngine + ?Sized> KvWriteTxn for BufferedWriteTxn<'_, E> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        match self.writes.get(key) {
            Some(value) => Ok(value.clone()),
            None => self.engine.get(key),
        }
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        let mut rows = self
            .engine
            .scan_prefix(prefix)?
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        for (key, value) in self
            .writes
            .range::<[u8], _>((
                std::ops::Bound::Included(prefix),
                std::ops::Bound::Unbounded,
            ))
            .take_while(|(key, _)| key.starts_with(prefix))
        {
            match value {
                Some(value) => rows.insert(key.clone(), value.clone()),
                None => rows.remove(key),
            };
        }
        Ok(rows.into_iter().collect())
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError> {
        self.writes.insert(key.to_vec(), Some(value.to_vec()));
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        // Deleting an absent key is not a change.
        if self.get(key)?.is_some() {
            self.writes.insert(key.to_vec(), None);
        }
        Ok(())
    }
}

/// Exclusive upper bound of all keys starting with `prefix`, or `None` when
/// the prefix has no finite upper bound.
pub fn prefix_range_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last != u8::MAX {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

/// Statistics reported by [`KvEngine::stats`].
pub type KvEngineStats = StorageStats;

pub trait KvEngine: std::fmt::Debug + Send + Sync + 'static {
    type PrefixScan: Iterator<Item = KvScanItem> + Send + 'static;

    /// Open a read handle for one logical operation.
    ///
    /// The default forwards every read to the engine and is not a snapshot.
    fn begin_read(&self) -> Result<Box<dyn KvReadTxn + '_>, DbError> {
        Ok(Box::new(ForwardingReadTxn {
            engine: self,
            revision: self.current_revision()?,
        }))
    }

    /// Open a read handle that does not borrow the engine, so it can be
    /// shared with `'static` consumers such as query row views.
    ///
    /// Returns `None` when the engine only has borrowed read handles (the
    /// default).
    fn begin_read_owned(&self) -> Result<Option<Box<dyn KvReadTxn>>, DbError> {
        Ok(None)
    }

    /// Run `f` in one write transaction and commit its writes.
    ///
    /// Returns a conflict without running `f` when `expected_revision` is set
    /// and differs from the current revision. A transaction that writes
    /// nothing must leave the revision unchanged. When `f` fails, nothing is
    /// committed. The default buffers writes and commits them with
    /// [`Self::write_batch_conditional`] (or [`Self::write_batch`] without an
    /// expected revision), relying on the engine to ignore empty batches.
    fn write_with<F>(
        &mut self,
        expected_revision: Option<u64>,
        f: F,
    ) -> Result<StorageCommitOutcome, DbError>
    where
        F: FnOnce(&mut dyn KvWriteTxn) -> Result<(), DbError>,
        Self: Sized,
    {
        if let Some(expected) = expected_revision {
            let actual = self.current_revision()?;
            if actual != Some(expected) {
                return Ok(StorageCommitOutcome::Conflict {
                    expected_revision: Some(expected),
                    actual_revision: actual,
                });
            }
        }
        let mut txn = BufferedWriteTxn {
            engine: &*self,
            writes: BTreeMap::new(),
        };
        f(&mut txn)?;
        let ops = txn.into_ops();
        if expected_revision.is_some() {
            return self.write_batch_conditional(&ops, expected_revision);
        }
        self.write_batch(&ops)?;
        Ok(StorageCommitOutcome::Committed {
            revision: self.current_revision()?,
        })
    }

    /// Scan keys in `start..end` (or `start..` when `end` is `None`).
    ///
    /// The default scans the longest common prefix of the bounds and filters
    /// it; engines with ordered storage should override it.
    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        let prefix_len = end.as_ref().map_or(0, |end| {
            start
                .iter()
                .zip(end.iter())
                .take_while(|(a, b)| a == b)
                .count()
        });
        let scan = self.scan_prefix_stream(start[..prefix_len].to_vec())?;
        Ok(Box::new(scan.filter(move |item| {
            match item {
                Ok((key, _)) => {
                    key.as_slice() >= start.as_slice()
                        && end
                            .as_ref()
                            .is_none_or(|end| key.as_slice() < end.as_slice())
                }
                Err(_) => true,
            }
        })))
    }

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

    // Physical maintenance. The defaults suit engines without the operation;
    // [`EntityStore`] forwards these to the `EntityStorage` maintenance
    // methods.

    /// Compact the engine's storage, reclaiming unused space.
    ///
    /// Returns whether any compaction was performed.
    fn compact(&mut self) -> Result<bool, DbError> {
        Err(unsupported_storage_maintenance("compaction"))
    }

    /// Verify the integrity of the engine's storage, repairing it if possible.
    ///
    /// Returns `true` when the storage was intact and `false` when it was
    /// repaired.
    fn check_integrity(&mut self) -> Result<bool, DbError> {
        Err(unsupported_storage_maintenance("integrity checks"))
    }

    /// Physical storage statistics; unknown values are `None`.
    fn stats(&self) -> Result<KvEngineStats, DbError> {
        Ok(KvEngineStats::default())
    }

    /// Capture the current committed state for a backup: a consistent copy
    /// of every stored key (including engine-private entries such as the
    /// revision) written to a new database file.
    fn backup_source(&self) -> Result<Box<dyn BackupSource>, DbError> {
        Err(unsupported_storage_maintenance("backups"))
    }
}

pub struct EntityScan<I> {
    inner: I,
    decoder: ScanDecoder,
}

impl<I> EntityScan<I> {
    fn new(inner: I, decoder: ScanDecoder) -> Self {
        Self { inner, decoder }
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
            .map(|item| item.and_then(|(key, payload)| self.decoder.decode(&key, &payload)))
    }
}

/// Entity ids of index entries, in key order and without duplicates.
pub struct IndexEntityIdScan<I> {
    inner: I,
    id_position: IndexIdPosition,
    seen: BTreeSet<String>,
}

/// Where the entity id starts in the index keys of a scan.
#[derive(Debug, Clone, Copy)]
enum IndexIdPosition {
    /// All keys share the value token; the id starts at this offset.
    At(usize),
    /// Keys have different value tokens starting at this offset; the id
    /// follows the value token.
    AfterValueAt(usize),
}

impl<I> IndexEntityIdScan<I> {
    /// Scan of entries sharing one value prefix of `value_prefix_len` bytes.
    fn for_value(inner: I, value_prefix_len: usize) -> Self {
        Self::new(inner, IndexIdPosition::At(value_prefix_len))
    }

    /// Scan of entries with values starting after `path_prefix_len` bytes.
    fn for_values(inner: I, path_prefix_len: usize) -> Self {
        Self::new(inner, IndexIdPosition::AfterValueAt(path_prefix_len))
    }

    fn new(inner: I, id_position: IndexIdPosition) -> Self {
        Self {
            inner,
            id_position,
            seen: BTreeSet::new(),
        }
    }
}

impl IndexIdPosition {
    fn entity_id(self, key: &[u8]) -> Option<&str> {
        match self {
            Self::At(offset) => std::str::from_utf8(key.get(offset..)?).ok(),
            Self::AfterValueAt(offset) => index_entry_id(key, offset),
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
                    let Some(id) = self.id_position.entity_id(&key) else {
                        return Some(Err(DbError::Deserialization(
                            "malformed index entry key".to_string(),
                        )));
                    };
                    if !self.seen.contains(id) {
                        let id = id.to_string();
                        self.seen.insert(id.clone());
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
    payload_format: EntityPayloadFormat,
    /// Committed field-name dictionaries, shared with read handles.
    dictionaries: Arc<FieldDictCache>,
}

impl<E: KvEngine> EntityStore<E> {
    pub fn new(engine: E) -> Self {
        Self::with_payload_format(engine, EntityPayloadFormat::default())
    }

    /// Create a store writing entity payloads in `payload_format`.
    ///
    /// Payloads of every format are always readable; use
    /// [`EntityPayloadFormat::SelfContained`] when payloads are decoded
    /// outside the store with [`decode_entity`].
    pub fn with_payload_format(engine: E, payload_format: EntityPayloadFormat) -> Self {
        Self {
            engine,
            payload_format,
            dictionaries: Arc::default(),
        }
    }

    pub fn payload_format(&self) -> EntityPayloadFormat {
        self.payload_format
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

    /// Write one key directly through the engine, bypassing the maintained
    /// stats counters.
    pub fn put_raw(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError> {
        self.engine.put(key, value)
    }

    pub fn put_entity(&mut self, entity: &StoredEntity) -> std::result::Result<(), DbError> {
        self.commit_ops(&[StorageWriteOp::PutEntity(entity.clone())], None)
            .map(|_| ())
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
        self.decode_entity_at(collection, id, &payload).map(Some)
    }

    pub fn delete_entity(
        &mut self,
        collection: LocalCollectionId,
        id: &str,
    ) -> std::result::Result<(), DbError> {
        let key = entity_key(collection, id);
        self.write_keys([(key, None)])
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
        let scan = self.engine.scan_prefix_stream(prefix)?;
        Ok(EntityScan::new(scan, self.scan_decoder(collection)?))
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
        let scan = self
            .engine
            .scan_prefix_at_revision_stream(prefix, revision)?;
        Ok(EntityScan::new(scan, self.scan_decoder(collection)?))
    }

    pub fn put_index_entry(
        &mut self,
        index: LocalIndexId,
        value: &Value,
        entity_id: &str,
    ) -> std::result::Result<(), DbError> {
        let key = index_key(index, None, value, entity_id);
        self.write_keys([(key, Some(Vec::new()))])
    }

    pub fn delete_index_entry(
        &mut self,
        index: LocalIndexId,
        value: &Value,
        entity_id: &str,
    ) -> std::result::Result<(), DbError> {
        let key = index_key(index, None, value, entity_id);
        self.write_keys([(key, None)])
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
        let prefix = index_value_prefix(index, path, value);
        let len = prefix.len();
        Ok(IndexEntityIdScan::for_value(
            self.engine.scan_prefix_stream(prefix)?,
            len,
        ))
    }

    /// Ids of the entries of `index` (at `path`) whose value lies within
    /// `lower..upper`, ordered by value and then id.
    pub fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<IndexEntityIdScan<BoxKvPrefixScan>, DbError> {
        let path_prefix_len = index_path_prefix(index, path).len();
        let scan = match index_range(index, path, lower, upper) {
            Some((start, end)) => self.engine.scan_range_stream(start, Some(end))?,
            None => Box::new(std::iter::empty()),
        };
        Ok(IndexEntityIdScan::for_values(scan, path_prefix_len))
    }

    /// Ids of the entries of `index` (at `path`) whose value is a string
    /// starting with `prefix`, ordered by value and then id.
    pub fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<IndexEntityIdScan<E::PrefixScan>, DbError> {
        let path_prefix_len = index_path_prefix(index, path).len();
        Ok(IndexEntityIdScan::for_values(
            self.engine
                .scan_prefix_stream(index_string_prefix(index, path, prefix))?,
            path_prefix_len,
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

    fn count_collection_entities(&self, collection: LocalCollectionId) -> Result<u64, DbError> {
        count_keys(self.engine.scan_prefix_stream(entity_prefix(collection))?)
    }

    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        stats::read_counter(
            self.engine.begin_read()?.as_ref(),
            &collection_rows_key(collection),
        )
    }

    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        stats::read_counter(
            self.engine.begin_read()?.as_ref(),
            &index_entries_key(index),
        )
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

    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        Ok(Box::new(EntityStore::scan_index_range_stream(
            self, index, path, lower, upper,
        )?))
    }

    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        Ok(Box::new(EntityStore::scan_index_prefix_stream(
            self, index, path, prefix,
        )?))
    }

    fn scan_index_entries(
        &self,
        index: LocalIndexId,
        scan: &semantic_db_core::IndexScan,
    ) -> Result<semantic_db_core::embedded::BoxIndexEntryScan, DbError> {
        index_scan::scan_index_entries(&*self.engine.begin_read()?, index, scan)
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> std::result::Result<bool, DbError> {
        Ok(self.engine.get(&index_marker_key(index))?.as_deref()
            != Some(index_format_value().as_slice()))
    }

    fn prepare_open(&mut self) -> Result<(), DbError> {
        let started = Instant::now();
        tracing::debug!(
            operation = "database_open",
            phase = "kv_layout_migration",
            "Database startup phase started"
        );
        let outcome = self.migrate_layout()?;
        tracing::debug!(
            operation = "database_open",
            phase = "kv_layout_migration",
            elapsed = ?started.elapsed(),
            outcome = ?outcome,
            "Database startup phase completed"
        );

        let started = Instant::now();
        tracing::debug!(
            operation = "database_open",
            phase = "kv_statistics",
            "Database startup phase started"
        );
        let outcome = self.ensure_stats()?;
        tracing::debug!(
            operation = "database_open",
            phase = "kv_statistics",
            elapsed = ?started.elapsed(),
            outcome = ?outcome,
            "Database startup phase completed"
        );
        Ok(())
    }

    fn compact_storage(&mut self) -> Result<bool, DbError> {
        self.engine.compact()
    }

    fn check_storage_integrity(&mut self) -> Result<bool, DbError> {
        self.engine.check_integrity()
    }

    fn storage_stats(&self) -> Result<StorageStats, DbError> {
        self.engine.stats()
    }

    fn rebuild_storage_stats(&mut self) -> Result<bool, DbError> {
        self.rebuild_stats()
    }

    fn rewrite_payload_batch(
        &mut self,
        resume_after: Option<&[u8]>,
        limit: usize,
    ) -> Result<PayloadRewriteBatch, DbError> {
        self.rewrite_entities_batch(resume_after, limit)
    }

    fn backup_source(&self) -> Result<Box<dyn BackupSource>, DbError> {
        self.engine.backup_source()
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.engine.tx_capabilities()
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        self.engine.current_revision()
    }

    fn snapshot(&self) -> Result<Box<dyn EntityReadSnapshot + '_>, DbError> {
        Ok(Box::new(KvEntitySnapshot::with_dictionaries(
            self.engine.begin_read()?,
            self.dictionaries.clone(),
        )))
    }

    fn owned_snapshot(&self) -> Result<Option<Arc<dyn EntityReadSnapshot>>, DbError> {
        Ok(self.engine.begin_read_owned()?.map(|txn| {
            Arc::new(KvEntitySnapshot::with_dictionaries(
                txn,
                self.dictionaries.clone(),
            )) as Arc<dyn EntityReadSnapshot>
        }))
    }

    fn apply_batch(&mut self, ops: &[StorageWriteOp]) -> std::result::Result<(), DbError> {
        match self.commit_ops(ops, None)? {
            StorageCommitOutcome::Committed { .. } => Ok(()),
            StorageCommitOutcome::Conflict { .. } => Err(DbError::storage(
                StorageErrorKind::Conflict,
                "unexpected conflict for unconditional batch",
            )),
        }
    }

    fn apply_batch_conditional(
        &mut self,
        ops: &[StorageWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        self.commit_ops(ops, expected_revision)
    }
}

impl<E: KvEngine> EntityStore<E> {
    /// Commit storage operations in one engine write transaction.
    ///
    /// Final key states are compared against the write transaction's own
    /// view, so unchanged keys are skipped without separate read
    /// transactions, and a batch without effective changes writes nothing.
    fn commit_ops(
        &mut self,
        operations: &[StorageWriteOp],
        expected_revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        let format = self.payload_format;
        let mut staging = DictStaging::new(&self.dictionaries);
        let outcome = self.engine.write_with(expected_revision, |txn| {
            // A conditional commit runs against the revision the caller read
            // the old objects of `ReindexEntity` at, so the previous states
            // of their entries are known.
            let known_base = expected_revision.is_some();
            let (values, bases) =
                lower_final_values(operations, &*txn, &mut staging, format, known_base)?;
            staging.write_additions(txn)?;
            write_final_states(txn, values, CounterDeltas::with_bases(bases))
        })?;
        if matches!(outcome, StorageCommitOutcome::Committed { .. }) {
            staging.publish();
        }
        Ok(outcome)
    }

    /// Write final key states in one unconditional write transaction.
    fn write_keys(
        &mut self,
        values: impl IntoIterator<Item = (Vec<u8>, Option<Vec<u8>>)>,
    ) -> Result<(), DbError> {
        self.engine
            .write_with(None, |txn| write_final_values(txn, values))
            .map(|_| ())
    }
}

/// Final state of one key written by a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FinalState {
    value: Option<Vec<u8>>,
    /// Whether the key exists before the batch, when the operation that
    /// wrote it knows; `None` makes the write compare against a read.
    existed: Option<bool>,
}

impl FinalState {
    fn unknown(value: Option<Vec<u8>>) -> Self {
        Self {
            value,
            existed: None,
        }
    }
}

/// Write the final state of every key, skipping unchanged keys, and update
/// the stats counters by the created and removed entity and index keys.
fn write_final_values(
    txn: &mut dyn KvWriteTxn,
    values: impl IntoIterator<Item = (Vec<u8>, Option<Vec<u8>>)>,
) -> Result<(), DbError> {
    write_final_states(
        txn,
        values
            .into_iter()
            .map(|(key, value)| (key, FinalState::unknown(value))),
        CounterDeltas::default(),
    )
}

/// [`write_final_values`] for states that may know their previous
/// existence.
///
/// Keys whose previous existence is known skip the comparison read. Only
/// index entries (empty values) carry that knowledge, so existence is their
/// whole previous state: a key known to exist with a final value, or known
/// to be absent without one, is unchanged.
fn write_final_states(
    txn: &mut dyn KvWriteTxn,
    values: impl IntoIterator<Item = (Vec<u8>, FinalState)>,
    mut deltas: CounterDeltas,
) -> Result<(), DbError> {
    for (key, FinalState { value, existed }) in values {
        let existed = match existed {
            Some(existed) => {
                debug_assert!(
                    value.as_ref().is_none_or(Vec::is_empty),
                    "known previous states are limited to index entries"
                );
                if existed == value.is_some() {
                    continue;
                }
                existed
            }
            None => {
                let current = txn.get(&key)?;
                if current == value {
                    continue;
                }
                current.is_some()
            }
        };
        deltas.record(&key, existed, value.is_some());
        match value {
            Some(value) => txn.put(&key, &value)?,
            None => txn.delete(&key)?,
        }
    }
    deltas.apply(txn)
}

/// Lower storage operations to the final state of every touched key.
///
/// Comparing final states lets old/new index intersections produce no
/// physical writes, including when a DDL reset precedes them.
///
/// With `known_base`, the batch commits against the revision the old
/// objects of [`StorageWriteOp::ReindexEntity`] were read at, so the entries
/// it adds are known to be absent and those it removes known to exist; such
/// keys skip the comparison read. That knowledge is dropped for keys another
/// operation of the batch also writes and for indexes the batch clears or
/// resets.
///
/// Also returns the counter bases of the key spaces the batch clears (see
/// [`CounterDeltas`]): the number of keys each held before the batch.
fn lower_final_values(
    operations: &[StorageWriteOp],
    txn: &dyn KvWriteTxn,
    dictionaries: &mut DictStaging<'_>,
    format: EntityPayloadFormat,
    known_base: bool,
) -> Result<LoweredBatch, DbError> {
    let reset_indexes = operations
        .iter()
        .filter_map(|operation| match operation {
            StorageWriteOp::ClearIndex(index) | StorageWriteOp::ResetIndex(index) => Some(*index),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut lowered = LoweredWrites::default();
    let mut bases = BTreeMap::<Vec<u8>, u64>::new();
    let mut clear_prefix = |lowered: &mut LoweredWrites, prefix: Vec<u8>, counter: Vec<u8>| {
        let existing = txn.scan_prefix(&prefix)?;
        bases.insert(counter, existing.len() as u64);
        lowered.delete_prefix(existing, &prefix);
        Ok::<_, DbError>(())
    };
    for operation in operations {
        match operation {
            StorageWriteOp::PutEntity(entity) => lowered.put(
                entity_key(LocalCollectionId(entity.collection), &entity.id),
                dictionaries.encode(txn, entity, format)?,
                None,
            ),
            StorageWriteOp::DeleteEntity {
                collection,
                entity_id,
            } => lowered.delete(entity_key(*collection, entity_id), None),
            StorageWriteOp::ClearCollection(collection) => {
                clear_prefix(
                    &mut lowered,
                    entity_prefix(*collection),
                    collection_rows_key(*collection),
                )?;
            }
            StorageWriteOp::ClearIndex(index) => {
                clear_prefix(
                    &mut lowered,
                    index_prefix(*index),
                    index_entries_key(*index),
                )?;
                lowered.delete(index_marker_key(*index), None);
            }
            StorageWriteOp::ResetIndex(index) => {
                clear_prefix(
                    &mut lowered,
                    index_prefix(*index),
                    index_entries_key(*index),
                )?;
                lowered.put(index_marker_key(*index), index_format_value(), None);
            }
            StorageWriteOp::IndexEntity {
                index,
                entity_id,
                object,
            } => {
                for key in index_keys(index, entity_id, object)? {
                    lowered.put(key, Vec::new(), None);
                }
            }
            StorageWriteOp::UnindexEntity {
                index,
                entity_id,
                object,
            } => {
                for key in index_keys(index, entity_id, object)? {
                    lowered.delete(key, None);
                }
            }
            StorageWriteOp::ReindexEntity {
                index,
                entity_id,
                old,
                new,
            } => {
                let keys = |object: &Option<Arc<Object>>| match object {
                    Some(object) => index_keys(index, entity_id, object),
                    None => Ok(BTreeSet::new()),
                };
                let (mut old_keys, mut new_keys) = (keys(old)?, keys(new)?);
                // Entries in both states stay as they are.
                if !old_keys.is_empty() && !new_keys.is_empty() {
                    let common = old_keys
                        .intersection(&new_keys)
                        .cloned()
                        .collect::<Vec<_>>();
                    for key in &common {
                        old_keys.remove(key);
                        new_keys.remove(key);
                    }
                }
                let known_keys = known_base && !reset_indexes.contains(&index.lid);
                for key in old_keys {
                    lowered.delete(key, known_keys.then_some(true));
                }
                for key in new_keys {
                    lowered.put(key, Vec::new(), known_keys.then_some(false));
                }
            }
        }
    }
    Ok((lowered.finish(), bases))
}

/// Key writes of a batch in operation order, each with the previous
/// existence of its key when the operation writing it knows it.
#[derive(Default)]
struct LoweredWrites {
    writes: Vec<(Vec<u8>, Option<Vec<u8>>, Option<bool>)>,
}

impl LoweredWrites {
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>, existed: Option<bool>) {
        self.writes.push((key, Some(value), existed));
    }

    fn delete(&mut self, key: Vec<u8>, existed: Option<bool>) {
        self.writes.push((key, None, existed));
    }

    /// Delete every key under `prefix`: the `existing` ones and those the
    /// batch wrote so far.
    fn delete_prefix(&mut self, existing: Vec<(Vec<u8>, Vec<u8>)>, prefix: &[u8]) {
        let mut keys = existing
            .into_iter()
            .map(|(key, _)| key)
            .collect::<BTreeSet<_>>();
        keys.extend(
            self.writes
                .iter()
                .filter(|(key, value, _)| value.is_some() && key.starts_with(prefix))
                .map(|(key, _, _)| key.clone()),
        );
        for key in keys {
            self.delete(key, None);
        }
    }

    /// The final state of every written key, in key order: the last write
    /// wins, and no single operation vouches for the previous state of a
    /// key written more than once.
    fn finish(mut self) -> Vec<(Vec<u8>, FinalState)> {
        // Stable: writes of one key keep their operation order.
        self.writes.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out: Vec<(Vec<u8>, FinalState)> = Vec::with_capacity(self.writes.len());
        for (key, value, existed) in self.writes {
            match out.last_mut() {
                Some((last, state)) if *last == key => *state = FinalState::unknown(value),
                _ => out.push((key, FinalState { value, existed })),
            }
        }
        out
    }
}

/// Final key states of a lowered batch, in key order, with its counter
/// bases.
type LoweredBatch = (Vec<(Vec<u8>, FinalState)>, BTreeMap<Vec<u8>, u64>);

/// Entity reads served by one engine read handle.
pub struct KvEntitySnapshot<'a> {
    txn: Box<dyn KvReadTxn + 'a>,
    /// The store's committed field-name dictionaries.
    dictionaries: Arc<FieldDictCache>,
    /// Field-name dictionaries loaded through `txn`.
    loaded: HandleDicts,
}

impl<'a> KvEntitySnapshot<'a> {
    pub fn new(txn: Box<dyn KvReadTxn + 'a>) -> Self {
        Self::with_dictionaries(txn, Arc::default())
    }

    fn with_dictionaries(txn: Box<dyn KvReadTxn + 'a>, dictionaries: Arc<FieldDictCache>) -> Self {
        Self {
            txn,
            dictionaries,
            loaded: HandleDicts::default(),
        }
    }
}

impl EntityReadSnapshot for KvEntitySnapshot<'_> {
    fn revision(&self) -> Result<Option<u64>, DbError> {
        Ok(self.txn.revision())
    }

    fn is_consistent(&self) -> bool {
        self.txn.is_snapshot()
    }

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        self.txn
            .get(&entity_key(collection, id))?
            .map(|payload| self.decode_entity_at(collection, id, &payload))
            .transpose()
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        let scan = self.txn.scan_prefix_stream(entity_prefix(collection))?;
        Ok(Box::new(EntityScan::new(
            scan,
            self.scan_decoder(collection)?,
        )))
    }

    fn count_collection_entities(&self, collection: LocalCollectionId) -> Result<u64, DbError> {
        count_keys(self.txn.scan_prefix_stream(entity_prefix(collection))?)
    }

    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        stats::read_counter(self.txn.as_ref(), &collection_rows_key(collection))
    }

    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        stats::read_counter(self.txn.as_ref(), &index_entries_key(index))
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        let prefix = index_value_prefix(index, path, value);
        let len = prefix.len();
        Ok(Box::new(IndexEntityIdScan::for_value(
            self.txn.scan_prefix_stream(prefix)?,
            len,
        )))
    }

    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        let path_prefix_len = index_path_prefix(index, path).len();
        let scan = match index_range(index, path, lower, upper) {
            Some((start, end)) => self.txn.scan_range_stream(start, Some(end))?,
            None => Box::new(std::iter::empty()),
        };
        Ok(Box::new(IndexEntityIdScan::for_values(
            scan,
            path_prefix_len,
        )))
    }

    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        let path_prefix_len = index_path_prefix(index, path).len();
        Ok(Box::new(IndexEntityIdScan::for_values(
            self.txn
                .scan_prefix_stream(index_string_prefix(index, path, prefix))?,
            path_prefix_len,
        )))
    }

    fn scan_index_entries(
        &self,
        index: LocalIndexId,
        scan: &semantic_db_core::IndexScan,
    ) -> Result<semantic_db_core::embedded::BoxIndexEntryScan, DbError> {
        index_scan::scan_index_entries(&*self.txn, index, scan)
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        Ok(self.txn.get(&index_marker_key(index))?.as_deref()
            != Some(index_format_value().as_slice()))
    }

    fn scan_index_keys(
        &self,
        index: &semantic_db_core::catalog::IndexSchema,
    ) -> Result<BoxIndexKeyScan, DbError> {
        let lid = index.lid;
        let path_token = index.schema.kind == IndexKind::PathEquality;
        Ok(Box::new(
            self.txn
                .scan_prefix_stream(index_prefix(lid))?
                .map(move |entry| {
                    entry.map(|(key, _)| IndexKeyEntry {
                        entity_id: index_key_entity_id(&key, lid, path_token).map(str::to_string),
                        key,
                    })
                }),
        ))
    }

    fn index_keys_for(
        &self,
        index: &semantic_db_core::catalog::IndexSchema,
        entity_id: &str,
        object: &Object,
    ) -> Result<Vec<Vec<u8>>, DbError> {
        Ok(index_keys(index, entity_id, object)?.into_iter().collect())
    }

    fn contains_index_key(&self, index: LocalIndexId, key: &[u8]) -> Result<bool, DbError> {
        if !key.starts_with(&index_prefix(index)) {
            return Ok(false);
        }
        Ok(self.txn.get(key)?.is_some())
    }

    fn scan_collection_checked(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxCheckedEntityScan, DbError> {
        let prefix_len = entity_prefix(collection).len();
        let mut decoder = self.scan_decoder(collection)?;
        Ok(Box::new(
            self.txn
                .scan_prefix_stream(entity_prefix(collection))?
                .map(move |entry| {
                    let (key, payload) = entry?;
                    let id = String::from_utf8_lossy(&key[prefix_len..]).into_owned();
                    let entity = decoder.decode(&key, &payload);
                    Ok((id, entity))
                }),
        ))
    }
}

/// Count the entries of a key scan without decoding their values.
fn count_keys(mut scan: impl Iterator<Item = KvScanItem>) -> Result<u64, DbError> {
    scan.try_fold(0u64, |count, entry| entry.map(|_| count + 1))
}

/// The index keys `object` derives in `index`.
///
/// Fails when a key value nests deeper than the memcomparable encoding
/// allows ([`crate::keys::memcmp::MAX_DEPTH`]).
fn index_keys(
    index: &semantic_db_core::catalog::IndexSchema,
    entity_id: &str,
    object: &Object,
) -> Result<BTreeSet<Vec<u8>>, DbError> {
    let mut keys = BTreeSet::new();
    match index.schema.kind {
        IndexKind::Equality | IndexKind::Range => {
            if let Some(value) = index.key_value(object) {
                keys.insert(try_index_key(index.lid, None, &value, entity_id)?);
            }
        }
        IndexKind::PathEquality => {
            for (path, value) in collect_index_entries(object)? {
                keys.insert(try_index_key(index.lid, Some(&path), &value, entity_id)?);
            }
        }
        IndexKind::FullText => {
            for token in index.text_tokens(object).unwrap_or_default() {
                keys.insert(index_key(index.lid, None, &Value::String(token), entity_id));
            }
        }
    }
    Ok(keys)
}

pub(crate) fn index_format_value() -> Vec<u8> {
    INDEX_FORMAT_VERSION_V2_MSGPACK.to_le_bytes().to_vec()
}

/// Every `(path, value)` pair of `object`, including the nested values of
/// objects and lists (path-equality index entries).
///
/// Fails for values nested deeper than
/// [`MAX_VALUE_DEPTH`](semantic_data::value::MAX_VALUE_DEPTH).
pub(crate) fn collect_index_entries(object: &Object) -> Result<Vec<(FieldPath, Value)>, DbError> {
    let mut out = Vec::new();
    for (field, value) in object {
        let mut path = FieldPath::new();
        path.push_field(field.clone());
        collect_value_entries(
            value,
            &mut path,
            &mut out,
            semantic_data::value::MAX_VALUE_DEPTH,
        )?;
    }
    Ok(out)
}

fn collect_value_entries(
    value: &Value,
    path: &mut FieldPath,
    out: &mut Vec<(FieldPath, Value)>,
    depth: usize,
) -> Result<(), DbError> {
    out.push((path.clone(), value.clone()));
    let inner = || {
        depth.checked_sub(1).ok_or_else(|| {
            DbError::Serialization(format!(
                "value nesting exceeds the maximum depth of {}",
                semantic_data::value::MAX_VALUE_DEPTH
            ))
        })
    };
    match value {
        Value::Object(object) => {
            let depth = inner()?;
            for (field, nested) in object {
                path.push_field(field.clone());
                collect_value_entries(nested, path, out, depth)?;
                path.0.pop();
            }
        }
        Value::List(items) => {
            let depth = inner()?;
            for (idx, nested) in items.iter().enumerate() {
                path.push_index(idx);
                collect_value_entries(nested, path, out, depth)?;
                path.0.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::entity_codec::ENTITY_FORMAT_VERSION_V1_MSGPACK;
    use super::*;
    use semantic_data::schema::{IndexSchema as DataIndexSchema, KeyPath};
    use semantic_data::value::serde::typed::TypedValue;
    use semantic_db_core::catalog::IndexSchema;
    use semantic_db_core::embedded::StoredEntityKind;

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
    fn counts_collection_entities_by_key() {
        let mut store = EntityStore::new(MemoryKvEngine::new());
        let entity = |collection: usize, id: &str| {
            StorageWriteOp::PutEntity(StoredEntity {
                id: id.to_string(),
                collection,
                kind: StoredEntityKind::Untyped,
                object: Object::new(),
            })
        };
        store
            .apply_batch(&[entity(7, "a"), entity(7, "b"), entity(8, "c")])
            .unwrap();

        let count = |store: &EntityStore<MemoryKvEngine>, collection| {
            let snapshot = EntityStorage::snapshot(store).unwrap();
            let counted = snapshot
                .count_collection_entities(LocalCollectionId(collection))
                .unwrap();
            assert_eq!(
                EntityStorage::count_collection_entities(store, LocalCollectionId(collection))
                    .unwrap(),
                counted
            );
            counted
        };
        assert_eq!(count(&store, 7), 2);
        assert_eq!(count(&store, 8), 1);
        assert_eq!(count(&store, 9), 0);

        // The memory engine provides owned snapshots isolated from writes.
        let owned = EntityStorage::owned_snapshot(&store).unwrap().unwrap();
        store.apply_batch(&[entity(7, "c")]).unwrap();
        assert_eq!(
            owned
                .count_collection_entities(LocalCollectionId(7))
                .unwrap(),
            2
        );
        assert_eq!(count(&store, 7), 3);
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
                extra_key_paths: Vec::new(),
                predicate: None,
                analyzer: Default::default(),
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
                extra_key_paths: Vec::new(),
                predicate: None,
                analyzer: Default::default(),
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
