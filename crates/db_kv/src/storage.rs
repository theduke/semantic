use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;

use semantic_data::schema::IndexKind;
use semantic_data::value::serde::typed::TypedValue;
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};
use semantic_db_core::embedded::{
    BoxEntityIdScan, BoxEntityScan, EntityReadSnapshot, EntityStorage, StorageCommitOutcome,
    StorageStats, StorageTransactionCapabilities, StorageWriteOp, StoredEntity, StoredEntityKind,
    unsupported_storage_maintenance,
};
use serde::{Deserialize, Serialize};

pub use crate::keys::parse_entity_key;
use crate::keys::{
    collection_rows_key, entity_key, entity_prefix, index_entries_key, index_entry_id, index_key,
    index_marker_key, index_path_prefix, index_prefix, index_range, index_string_prefix,
    index_value_prefix,
};

pub mod layout;
pub mod memory;
pub mod stats;
pub use memory::MemoryKvEngine;
use stats::CounterDeltas;

#[cfg(test)]
mod incremental_tests;
#[cfg(test)]
mod index_range_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod stats_tests;
#[cfg(test)]
mod txn_tests;

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

/// Statistics reported by [`KvMaintenance::stats`].
pub type KvEngineStats = StorageStats;

/// Physical maintenance operations of a key-value engine.
///
/// Every method has a default for engines without the operation, so
/// implementing the trait only requires overriding what the engine supports.
/// [`EntityStore`] forwards these to the `EntityStorage` maintenance methods.
pub trait KvMaintenance {
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
}

pub trait KvEngine: KvMaintenance + std::fmt::Debug + Send + Sync + 'static {
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

    /// Write one key directly through the engine, bypassing the maintained
    /// stats counters.
    pub fn put_raw(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError> {
        self.engine.put(key, value)
    }

    pub fn put_entity(&mut self, entity: &StoredEntity) -> std::result::Result<(), DbError> {
        let key = entity_key(LocalCollectionId(entity.collection), &entity.id);
        let payload = encode_entity(entity)?;
        self.write_keys([(key, Some(payload))])
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

    fn index_needs_rebuild(&self, index: LocalIndexId) -> std::result::Result<bool, DbError> {
        Ok(self.engine.get(&index_marker_key(index))?.as_deref()
            != Some(index_format_value().as_slice()))
    }

    fn prepare_open(&mut self) -> Result<(), DbError> {
        self.migrate_layout()?;
        self.ensure_stats()?;
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

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.engine.tx_capabilities()
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        self.engine.current_revision()
    }

    fn snapshot(&self) -> Result<Box<dyn EntityReadSnapshot + '_>, DbError> {
        Ok(Box::new(KvEntitySnapshot::new(self.engine.begin_read()?)))
    }

    fn owned_snapshot(&self) -> Result<Option<Arc<dyn EntityReadSnapshot>>, DbError> {
        Ok(self
            .engine
            .begin_read_owned()?
            .map(|txn| Arc::new(KvEntitySnapshot::new(txn)) as Arc<dyn EntityReadSnapshot>))
    }

    fn apply_batch(&mut self, ops: &[StorageWriteOp]) -> std::result::Result<(), DbError> {
        match self.commit_ops(ops, None)? {
            StorageCommitOutcome::Committed { .. } => Ok(()),
            StorageCommitOutcome::Conflict { .. } => Err(DbError::Storage(
                "unexpected conflict for unconditional batch".to_string(),
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
        self.engine.write_with(expected_revision, |txn| {
            let values = lower_final_values(operations, &*txn)?;
            write_final_values(txn, values)
        })
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

/// Write the final state of every key, skipping unchanged keys, and update
/// the stats counters by the created and removed entity and index keys.
fn write_final_values(
    txn: &mut dyn KvWriteTxn,
    values: impl IntoIterator<Item = (Vec<u8>, Option<Vec<u8>>)>,
) -> Result<(), DbError> {
    let mut deltas = CounterDeltas::default();
    for (key, value) in values {
        let current = txn.get(&key)?;
        if current == value {
            continue;
        }
        deltas.record(&key, current.is_some(), value.is_some());
        match value {
            Some(value) => txn.put(&key, &value)?,
            None => txn.delete(&key)?,
        }
    }
    deltas.apply(txn)
}

/// Lower storage operations to the final value of every touched key.
///
/// Comparing final states lets old/new index intersections produce no
/// physical writes, including when a DDL reset precedes them.
fn lower_final_values(
    operations: &[StorageWriteOp],
    txn: &dyn KvWriteTxn,
) -> Result<BTreeMap<Vec<u8>, Option<Vec<u8>>>, DbError> {
    let mut lowered = Vec::new();
    for operation in operations {
        match operation {
            StorageWriteOp::PutEntity(entity) => lowered.push(KvWriteOp::Put {
                key: entity_key(LocalCollectionId(entity.collection), &entity.id),
                value: encode_entity(entity)?,
            }),
            StorageWriteOp::DeleteEntity {
                collection,
                entity_id,
            } => {
                lowered.push(KvWriteOp::Delete {
                    key: entity_key(*collection, entity_id),
                });
            }
            StorageWriteOp::ClearCollection(collection) => {
                let prefix = entity_prefix(*collection);
                push_prefix_deletes(&mut lowered, txn.scan_prefix(&prefix)?, &prefix);
            }
            StorageWriteOp::ClearIndex(index) => {
                let prefix = index_prefix(*index);
                push_prefix_deletes(&mut lowered, txn.scan_prefix(&prefix)?, &prefix);
                lowered.push(KvWriteOp::Delete {
                    key: index_marker_key(*index),
                });
            }
            StorageWriteOp::ResetIndex(index) => {
                let prefix = index_prefix(*index);
                push_prefix_deletes(&mut lowered, txn.scan_prefix(&prefix)?, &prefix);
                lowered.push(KvWriteOp::Put {
                    key: index_marker_key(*index),
                    value: index_format_value(),
                });
            }
            StorageWriteOp::IndexEntity {
                index,
                entity_id,
                object,
            } => {
                for key in index_keys(index, entity_id, object)? {
                    lowered.push(KvWriteOp::Put {
                        key,
                        value: Vec::new(),
                    });
                }
            }
            StorageWriteOp::UnindexEntity {
                index,
                entity_id,
                object,
            } => {
                for key in index_keys(index, entity_id, object)? {
                    lowered.push(KvWriteOp::Delete { key });
                }
            }
        }
    }
    let mut final_values = BTreeMap::new();
    for operation in lowered {
        match operation {
            KvWriteOp::Put { key, value } => {
                final_values.insert(key, Some(value));
            }
            KvWriteOp::Delete { key } => {
                final_values.insert(key, None);
            }
        }
    }
    Ok(final_values)
}

/// Entity reads served by one engine read handle.
pub struct KvEntitySnapshot<'a> {
    txn: Box<dyn KvReadTxn + 'a>,
}

impl<'a> KvEntitySnapshot<'a> {
    pub fn new(txn: Box<dyn KvReadTxn + 'a>) -> Self {
        Self { txn }
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
            .map(|payload| decode_entity(&payload))
            .transpose()
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        Ok(Box::new(EntityScan::new(
            self.txn.scan_prefix_stream(entity_prefix(collection))?,
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

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        Ok(self.txn.get(&index_marker_key(index))?.as_deref()
            != Some(index_format_value().as_slice()))
    }
}

/// Count the entries of a key scan without decoding their values.
fn count_keys(mut scan: impl Iterator<Item = KvScanItem>) -> Result<u64, DbError> {
    scan.try_fold(0u64, |count, entry| entry.map(|_| count + 1))
}

fn index_keys(
    index: &semantic_db_core::catalog::IndexSchema,
    entity_id: &str,
    object: &Object,
) -> Result<BTreeSet<Vec<u8>>, DbError> {
    let mut keys = BTreeSet::new();
    match index.schema.kind {
        IndexKind::Equality => {
            if let Some(value) = object.get(&index.canonical_field) {
                keys.insert(index_key(index.lid, None, value, entity_id));
            }
        }
        IndexKind::PathEquality => {
            for (path, value) in collect_index_entries(object) {
                keys.insert(index_key(index.lid, Some(&path), &value, entity_id));
            }
        }
        IndexKind::Range | IndexKind::FullText => {}
    }
    Ok(keys)
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

pub(crate) fn index_format_value() -> Vec<u8> {
    INDEX_FORMAT_VERSION_V2_MSGPACK.to_le_bytes().to_vec()
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
