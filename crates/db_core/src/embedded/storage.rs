use std::ops::Bound;

use semantic_data::value::{FieldPath, Object, Value};

use crate::catalog::{IndexSchema, LocalCollectionId, LocalIndexId};
use crate::{DbError, IsolationLevel, StorageErrorKind};

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum StoredEntityKind {
    Class,
    Record,
    Untyped,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct StoredEntity {
    pub collection: usize,
    pub kind: StoredEntityKind,
    pub id: String,
    pub object: Object,
}

#[derive(Debug, Clone)]
pub enum StorageWriteOp {
    PutEntity(StoredEntity),
    DeleteEntity {
        collection: LocalCollectionId,
        entity_id: String,
    },
    ClearCollection(LocalCollectionId),
    /// Remove all entries and the initialization marker for an index.
    ///
    /// Subsequent reads report that the index needs rebuilding.
    ClearIndex(LocalIndexId),
    /// Remove all entries for an index and write its initialization marker.
    ///
    /// Later [`StorageWriteOp::IndexEntity`] operations in the same batch add
    /// entries to the newly initialized index.
    ResetIndex(LocalIndexId),
    /// Add the entries derived from an entity to an index.
    ///
    /// This operation is additive: it does not remove entries previously
    /// derived from the same entity. Repeating the same logical entry is
    /// idempotent. Callers rebuilding an index must issue [`Self::ResetIndex`]
    /// first.
    IndexEntity {
        index: IndexSchema,
        entity_id: String,
        object: Object,
    },
    /// Remove entries derived from the old object, preserving the index marker.
    UnindexEntity {
        index: IndexSchema,
        entity_id: String,
        object: Object,
    },
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub struct StorageTransactionCapabilities {
    pub conflict_detection: bool,
    pub mvcc: bool,
    pub snapshot_reads: bool,
}

impl Default for StorageTransactionCapabilities {
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
pub enum StorageCommitOutcome {
    Committed {
        revision: Option<u64>,
    },
    Conflict {
        expected_revision: Option<u64>,
        actual_revision: Option<u64>,
    },
}

pub type EntityScanItem = std::result::Result<StoredEntity, DbError>;
pub type BoxEntityScan = Box<dyn Iterator<Item = EntityScanItem> + Send>;
pub type EntityIdScanItem = std::result::Result<String, DbError>;
pub type BoxEntityIdScan = Box<dyn Iterator<Item = EntityIdScanItem> + Send>;

/// A read handle over entity storage.
///
/// Handles returned by [`EntityStorage::snapshot`] let one logical operation
/// (a query, a point read, or the read phase of a transaction attempt) perform
/// all of its reads through a single storage transaction. When
/// [`Self::is_consistent`] is true, every read observes the state at
/// [`Self::revision`], independent of concurrent commits.
pub trait EntityReadSnapshot: Send + Sync {
    /// Revision of the state observed by this handle.
    fn revision(&self) -> Result<Option<u64>, DbError>;

    /// Whether all reads through this handle observe one storage state.
    ///
    /// Fallback handles that forward each read to the storage return `false`;
    /// callers requiring a stable view must then fence reads by revision.
    fn is_consistent(&self) -> bool;

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError>;

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError>;

    fn scan_collection(&self, collection: LocalCollectionId) -> Result<Vec<StoredEntity>, DbError> {
        self.scan_collection_stream(collection)?.collect()
    }

    /// Number of entities in `collection`.
    ///
    /// The default counts a collection scan; backends should count stored
    /// keys without decoding entity payloads.
    fn count_collection_entities(&self, collection: LocalCollectionId) -> Result<u64, DbError> {
        count_entity_scan(self.scan_collection_stream(collection)?)
    }

    /// Maintained number of entities in `collection`, or `None` when the
    /// storage does not maintain row counts (callers then fall back to
    /// [`Self::count_collection_entities`]).
    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        let _ = collection;
        Ok(None)
    }

    /// Maintained number of entries of `index`, or `None` when the storage
    /// does not maintain index entry counts.
    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        let _ = index;
        Ok(None)
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError>;

    fn scan_index_value(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<Vec<String>, DbError> {
        self.scan_index_value_stream(index, path, value)?.collect()
    }

    /// Ids of the entries of `index` (at `path` for path-equality indexes)
    /// whose value lies within `lower..upper` by `Value` order, ordered by
    /// value and then id.
    ///
    /// The default reports that ordered index scans are unsupported.
    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        let _ = (index, path, lower, upper);
        Err(unsupported_ordered_index_scan())
    }

    /// Ids of the entries of `index` (at `path`) whose value is a string
    /// starting with `prefix`, ordered by value and then id.
    ///
    /// The default reports that ordered index scans are unsupported.
    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        let _ = (index, path, prefix);
        Err(unsupported_ordered_index_scan())
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError>;
}

/// Count the entities of a scan, stopping at the first error.
pub fn count_entity_scan(mut scan: BoxEntityScan) -> Result<u64, DbError> {
    scan.try_fold(0u64, |count, entity| entity.map(|_| count + 1))
}

/// Error returned by storages without ordered index scans.
pub fn unsupported_ordered_index_scan() -> DbError {
    DbError::storage(
        StorageErrorKind::Unsupported,
        "ordered index scans are not supported by this storage",
    )
}

/// Error returned by storages without the maintenance `operation`.
pub fn unsupported_storage_maintenance(operation: &str) -> DbError {
    DbError::storage(
        StorageErrorKind::Unsupported,
        format!("{operation} is not supported by this storage"),
    )
}

/// Error returned when `isolation` needs consistent snapshots the storage
/// cannot provide.
pub(crate) fn snapshot_isolation_unsupported(isolation: IsolationLevel) -> DbError {
    DbError::storage(
        StorageErrorKind::Unsupported,
        format!(
            "{isolation:?} isolation requires consistent snapshot reads, which this storage \
             does not provide"
        ),
    )
}

/// Physical storage statistics for observability.
///
/// Values a storage cannot report are `None` (or empty).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageStats {
    /// Size of the database file in bytes.
    pub file_size_bytes: Option<u64>,
    /// Number of stored key-value entries across all tables.
    pub entries: Option<u64>,
    /// Entry counts of the physical tables.
    pub tables: Vec<StorageTableStats>,
    /// Bytes allocated by the storage engine.
    pub allocated_bytes: Option<u64>,
    /// Bytes of stored keys and values.
    pub stored_bytes: Option<u64>,
    /// Allocated bytes not holding live data (reclaimable by compaction).
    pub fragmented_bytes: Option<u64>,
}

/// Statistics of one physical table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageTableStats {
    pub name: String,
    pub entries: u64,
}

/// Snapshot fallback for storages without native read transactions.
///
/// Every read is forwarded to the storage, so the handle is not consistent.
#[derive(Debug)]
pub struct ForwardingReadSnapshot<'a, S: ?Sized> {
    storage: &'a S,
}

impl<'a, S: EntityStorage + ?Sized> ForwardingReadSnapshot<'a, S> {
    pub fn new(storage: &'a S) -> Self {
        Self { storage }
    }
}

impl<S: EntityStorage + ?Sized> EntityReadSnapshot for ForwardingReadSnapshot<'_, S> {
    fn revision(&self) -> Result<Option<u64>, DbError> {
        self.storage.current_revision()
    }

    fn is_consistent(&self) -> bool {
        false
    }

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        self.storage.get_entity(collection, id)
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        self.storage.scan_collection_stream(collection)
    }

    fn count_collection_entities(&self, collection: LocalCollectionId) -> Result<u64, DbError> {
        self.storage.count_collection_entities(collection)
    }

    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        self.storage.collection_row_count(collection)
    }

    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        self.storage.index_entry_count(index)
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.storage.scan_index_value_stream(index, path, value)
    }

    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.storage
            .scan_index_range_stream(index, path, lower, upper)
    }

    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.storage.scan_index_prefix_stream(index, path, prefix)
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        self.storage.index_needs_rebuild(index)
    }
}

/// Reads bound to the revision of one transaction attempt.
///
/// Uses a single consistent snapshot when the storage provides one at the
/// requested revision. Otherwise falls back to revision fencing, so a
/// concurrent commit causes a conflict instead of mixing states.
pub(crate) struct RevisionReader<'a, S: EntityStorage> {
    storage: &'a S,
    revision: Option<u64>,
    snapshot: Option<Box<dyn EntityReadSnapshot + 'a>>,
}

impl<'a, S: EntityStorage> RevisionReader<'a, S> {
    pub(crate) fn new(storage: &'a S, revision: Option<u64>) -> Result<Self, DbError> {
        let snapshot = storage.snapshot()?;
        let snapshot =
            (snapshot.is_consistent() && snapshot.revision()? == revision).then_some(snapshot);
        Ok(Self {
            storage,
            revision,
            snapshot,
        })
    }

    /// Open a reader for a transaction attempt at `isolation`.
    ///
    /// [`IsolationLevel::ReadCommitted`] behaves like [`Self::new`]. Every
    /// other level requires one consistent snapshot at `revision`: storages
    /// without consistent snapshots fail with an `Unsupported` storage error,
    /// and a snapshot that moved past `revision` is a transaction conflict.
    pub(crate) fn for_isolation(
        storage: &'a S,
        revision: Option<u64>,
        isolation: IsolationLevel,
    ) -> Result<Self, DbError> {
        if !isolation.requires_snapshot() {
            return Self::new(storage, revision);
        }
        let snapshot = storage.snapshot()?;
        if !snapshot.is_consistent() {
            return Err(snapshot_isolation_unsupported(isolation));
        }
        let actual = snapshot.revision()?;
        if actual != revision {
            return Err(DbError::TransactionConflict(format!(
                "snapshot revision moved: expected {revision:?}, found {actual:?}"
            )));
        }
        Ok(Self {
            storage,
            revision,
            snapshot: Some(snapshot),
        })
    }

    /// Whether reads are served from one consistent snapshot.
    #[cfg(test)]
    pub(crate) fn is_snapshot(&self) -> bool {
        self.snapshot.is_some()
    }

    pub(crate) fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        match &self.snapshot {
            Some(snapshot) => snapshot.get_entity(collection, id),
            None => self
                .storage
                .get_entity_at_revision(collection, id, self.revision),
        }
    }

    pub(crate) fn scan_index_value(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<Vec<String>, DbError> {
        match &self.snapshot {
            Some(snapshot) => snapshot.scan_index_value(index, path, value),
            None => self
                .storage
                .scan_index_value_at_revision(index, path, value, self.revision),
        }
    }

    /// Stream every entity of `collection` into `visit`, one row at a time.
    ///
    /// Without a consistent snapshot the scan is fenced by the revision
    /// before and after, so a concurrent commit surfaces as a conflict.
    pub(crate) fn scan_collection(
        &self,
        collection: LocalCollectionId,
        mut visit: impl FnMut(StoredEntity) -> Result<(), DbError>,
    ) -> Result<(), DbError> {
        let scan = match &self.snapshot {
            Some(snapshot) => snapshot.scan_collection_stream(collection)?,
            None => {
                self.storage.ensure_revision(self.revision)?;
                self.storage.scan_collection_stream(collection)?
            }
        };
        for entity in scan {
            visit(entity?)?;
        }
        if self.snapshot.is_none() {
            self.storage.ensure_revision(self.revision)?;
        }
        Ok(())
    }

    /// The consistent snapshot serving reads, when there is one.
    pub(crate) fn snapshot(&self) -> Option<&(dyn EntityReadSnapshot + 'a)> {
        self.snapshot.as_deref()
    }

    pub(crate) fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        match &self.snapshot {
            Some(snapshot) => snapshot.index_needs_rebuild(index),
            None => self.storage.index_needs_rebuild(index),
        }
    }
}

pub trait EntityStorage: std::fmt::Debug + Send + Sync + 'static {
    /// A point read bound to the transaction revision. Backends without a native
    /// historical point lookup can use revision fencing: a concurrent commit
    /// causes a retry instead of mixing snapshots.
    fn get_entity_at_revision(
        &self,
        collection: LocalCollectionId,
        id: &str,
        revision: Option<u64>,
    ) -> Result<Option<StoredEntity>, DbError> {
        self.ensure_revision(revision)?;
        let row = self.get_entity(collection, id)?;
        self.ensure_revision(revision)?;
        Ok(row)
    }

    fn scan_index_value_at_revision(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
        revision: Option<u64>,
    ) -> Result<Vec<String>, DbError> {
        self.ensure_revision(revision)?;
        let ids = self.scan_index_value(index, path, value)?;
        self.ensure_revision(revision)?;
        Ok(ids)
    }

    fn ensure_revision(&self, expected: Option<u64>) -> Result<(), DbError> {
        let actual = self.current_revision()?;
        if actual != expected {
            return Err(DbError::TransactionConflict(format!(
                "read revision changed: expected {expected:?}, found {actual:?}"
            )));
        }
        Ok(())
    }

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> std::result::Result<Option<StoredEntity>, DbError>;

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<BoxEntityScan, DbError>;

    fn scan_collection(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<Vec<StoredEntity>, DbError> {
        self.scan_collection_stream(collection)?.collect()
    }

    /// Number of entities in `collection`.
    ///
    /// The default counts a collection scan; backends should count stored
    /// keys without decoding entity payloads.
    fn count_collection_entities(&self, collection: LocalCollectionId) -> Result<u64, DbError> {
        count_entity_scan(self.scan_collection_stream(collection)?)
    }

    /// Maintained number of entities in `collection`, or `None` when the
    /// storage does not maintain row counts (callers then fall back to
    /// [`Self::count_collection_entities`]).
    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        let _ = collection;
        Ok(None)
    }

    /// Maintained number of entries of `index`, or `None` when the storage
    /// does not maintain index entry counts.
    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        let _ = index;
        Ok(None)
    }

    fn scan_collection_at_revision_stream(
        &self,
        collection: LocalCollectionId,
        revision: u64,
    ) -> std::result::Result<BoxEntityScan, DbError>;

    fn scan_collection_at_revision(
        &self,
        collection: LocalCollectionId,
        revision: u64,
    ) -> std::result::Result<Vec<StoredEntity>, DbError> {
        self.scan_collection_at_revision_stream(collection, revision)?
            .collect()
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> std::result::Result<BoxEntityIdScan, DbError>;

    fn scan_index_value(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> std::result::Result<Vec<String>, DbError> {
        self.scan_index_value_stream(index, path, value)?.collect()
    }

    /// Ids of the entries of `index` (at `path` for path-equality indexes)
    /// whose value lies within `lower..upper` by `Value` order, ordered by
    /// value and then id.
    ///
    /// The default reports that ordered index scans are unsupported.
    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        let _ = (index, path, lower, upper);
        Err(unsupported_ordered_index_scan())
    }

    /// Ids of the entries of `index` (at `path`) whose value is a string
    /// starting with `prefix`, ordered by value and then id.
    ///
    /// The default reports that ordered index scans are unsupported.
    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        let _ = (index, path, prefix);
        Err(unsupported_ordered_index_scan())
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> std::result::Result<bool, DbError>;

    /// Prepare the storage before the database loads its catalog.
    ///
    /// Called once by `EmbeddedDb::open`. Backends use it to verify or
    /// upgrade their physical layout (for example, to migrate a legacy key
    /// layout). It must be idempotent and should not write when nothing
    /// needs to change.
    fn prepare_open(&mut self) -> Result<(), DbError> {
        Ok(())
    }

    /// Open a read handle for one logical operation.
    ///
    /// The default forwards every read to this storage and is not consistent;
    /// backends with read transactions should return a handle that serves all
    /// reads from one transaction.
    fn snapshot(&self) -> Result<Box<dyn EntityReadSnapshot + '_>, DbError> {
        Ok(Box::new(ForwardingReadSnapshot::new(self)))
    }

    /// Open a consistent read handle that does not borrow the storage.
    ///
    /// Owned handles can be shared with `'static` consumers, for example
    /// query row views that read referenced rows on demand. Returns `None`
    /// (the default) when the storage only has borrowed handles; callers
    /// then use [`Self::snapshot`].
    fn owned_snapshot(&self) -> Result<Option<std::sync::Arc<dyn EntityReadSnapshot>>, DbError> {
        Ok(None)
    }

    /// Compact the physical storage, reclaiming unused space.
    ///
    /// Returns whether any compaction was performed. The default reports the
    /// operation as unsupported.
    fn compact_storage(&mut self) -> Result<bool, DbError> {
        Err(unsupported_storage_maintenance("compaction"))
    }

    /// Verify the integrity of the physical storage, repairing it if possible.
    ///
    /// Returns `true` when the storage was intact and `false` when it was
    /// repaired; unrecoverable corruption is an error. The default reports
    /// the operation as unsupported.
    fn check_storage_integrity(&mut self) -> Result<bool, DbError> {
        Err(unsupported_storage_maintenance("integrity checks"))
    }

    /// Physical storage statistics. The default reports nothing.
    fn storage_stats(&self) -> Result<StorageStats, DbError> {
        Ok(StorageStats::default())
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities;
    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError>;
    fn apply_batch(&mut self, ops: &[StorageWriteOp]) -> std::result::Result<(), DbError>;
    fn apply_batch_conditional(
        &mut self,
        ops: &[StorageWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<StorageCommitOutcome, DbError>;
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct MemoryEntityStorage {
    entities: std::collections::BTreeMap<(usize, String), StoredEntity>,
    indexes: Vec<IndexedEntity>,
    initialized_indexes: std::collections::BTreeSet<LocalIndexId>,
    revision: u64,
    snapshots: Option<std::collections::BTreeMap<u64, MemorySnapshot>>,
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
struct MemorySnapshot {
    entities: std::collections::BTreeMap<(usize, String), StoredEntity>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct IndexedEntity {
    index: IndexSchema,
    entity_id: String,
    object: Object,
}

#[cfg(test)]
impl MemoryEntityStorage {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Copy of the current state without historical snapshots.
    fn frozen_copy(&self) -> Self {
        Self {
            entities: self.entities.clone(),
            indexes: self.indexes.clone(),
            initialized_indexes: self.initialized_indexes.clone(),
            revision: self.revision,
            snapshots: None,
        }
    }

    pub(crate) fn corrupt_index(&mut self, index: LocalIndexId) {
        self.indexes.retain(|entry| entry.index.lid != index);
        self.initialized_indexes.remove(&index);
    }

    fn capture_snapshot(&mut self) {
        if let Some(snapshots) = &mut self.snapshots {
            snapshots.insert(
                self.revision,
                MemorySnapshot {
                    entities: self.entities.clone(),
                },
            );
        }
    }

    fn apply(&mut self, operations: &[StorageWriteOp]) {
        for operation in operations {
            match operation {
                StorageWriteOp::PutEntity(entity) => {
                    self.entities
                        .insert((entity.collection, entity.id.clone()), entity.clone());
                }
                StorageWriteOp::DeleteEntity {
                    collection,
                    entity_id,
                } => {
                    self.entities.remove(&(collection.0, entity_id.clone()));
                }
                StorageWriteOp::UnindexEntity {
                    index,
                    entity_id,
                    object,
                } => {
                    self.indexes.retain(|entry| {
                        entry.index.lid != index.lid
                            || entry.entity_id != *entity_id
                            || entry.object != *object
                    });
                }
                StorageWriteOp::ClearCollection(collection) => {
                    self.entities
                        .retain(|(stored_collection, _), _| *stored_collection != collection.0);
                }
                StorageWriteOp::ClearIndex(index) => {
                    self.indexes.retain(|entry| entry.index.lid != *index);
                    self.initialized_indexes.remove(index);
                }
                StorageWriteOp::ResetIndex(index) => {
                    self.indexes.retain(|entry| entry.index.lid != *index);
                    self.initialized_indexes.insert(*index);
                }
                StorageWriteOp::IndexEntity {
                    index,
                    entity_id,
                    object,
                } => {
                    let entry = IndexedEntity {
                        index: index.clone(),
                        entity_id: entity_id.clone(),
                        object: object.clone(),
                    };
                    if !self.indexes.iter().any(|indexed| {
                        indexed.index.lid == entry.index.lid
                            && indexed.entity_id == entry.entity_id
                            && indexed.object == entry.object
                    }) {
                        self.indexes.push(entry);
                    }
                }
            }
        }
        if !operations.is_empty() {
            self.revision = self.revision.saturating_add(1);
            self.capture_snapshot();
        }
    }

    fn scan_index(
        indexes: &[IndexedEntity],
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Vec<String> {
        use semantic_data::schema::IndexKind;
        use semantic_data::value::ObjectAccess as _;

        indexes
            .iter()
            .filter(|entry| entry.index.lid == index)
            .filter(|entry| match entry.index.schema.kind {
                IndexKind::Equality => {
                    entry.object.get(&entry.index.canonical_field) == Some(value)
                }
                IndexKind::PathEquality => {
                    path.and_then(|path| entry.object.value_at_path(path)) == Some(value)
                }
                IndexKind::Range | IndexKind::FullText => false,
            })
            .map(|entry| entry.entity_id.clone())
            .collect()
    }

    /// Ids of the entries whose indexed value matches `filter`, ordered by
    /// value and then id.
    fn scan_index_ordered(
        indexes: &[IndexedEntity],
        index: LocalIndexId,
        path: Option<&FieldPath>,
        filter: impl Fn(&Value) -> bool,
    ) -> BoxEntityIdScan {
        use semantic_data::schema::IndexKind;
        use semantic_data::value::ObjectAccess as _;

        let entries = indexes
            .iter()
            .filter(|entry| entry.index.lid == index)
            .filter_map(|entry| {
                let value = match entry.index.schema.kind {
                    IndexKind::Equality => entry.object.get(&entry.index.canonical_field),
                    IndexKind::PathEquality => {
                        path.and_then(|path| entry.object.value_at_path(path))
                    }
                    IndexKind::Range | IndexKind::FullText => None,
                }?;
                filter(value).then(|| (value.clone(), entry.entity_id.clone()))
            })
            .collect::<std::collections::BTreeSet<_>>();
        let mut seen = std::collections::BTreeSet::new();
        let ids = entries
            .into_iter()
            .filter_map(|(_, id)| seen.insert(id.clone()).then_some(Ok(id)))
            .collect::<Vec<_>>();
        Box::new(ids.into_iter())
    }
}

#[cfg(test)]
impl EntityStorage for MemoryEntityStorage {
    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> std::result::Result<Option<StoredEntity>, DbError> {
        Ok(self.entities.get(&(collection.0, id.to_string())).cloned())
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<BoxEntityScan, DbError> {
        Ok(Box::new(
            self.entities
                .iter()
                .filter(|((stored_collection, _), _)| *stored_collection == collection.0)
                .map(|(_, entity)| Ok(entity.clone()))
                .collect::<Vec<_>>()
                .into_iter(),
        ))
    }

    fn count_collection_entities(&self, collection: LocalCollectionId) -> Result<u64, DbError> {
        Ok(self
            .entities
            .keys()
            .filter(|(stored_collection, _)| *stored_collection == collection.0)
            .count() as u64)
    }

    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        self.count_collection_entities(collection).map(Some)
    }

    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        Ok(Some(
            self.indexes
                .iter()
                .filter(|entry| entry.index.lid == index)
                .count() as u64,
        ))
    }

    fn scan_collection_at_revision_stream(
        &self,
        collection: LocalCollectionId,
        revision: u64,
    ) -> std::result::Result<BoxEntityScan, DbError> {
        if revision == self.revision {
            return self.scan_collection_stream(collection);
        }
        let snapshot = self
            .snapshots
            .as_ref()
            .and_then(|snapshots| snapshots.get(&revision))
            .ok_or_else(|| DbError::Storage(format!("snapshot {revision} not available").into()))?;
        Ok(Box::new(
            snapshot
                .entities
                .iter()
                .filter(|((stored_collection, _), _)| *stored_collection == collection.0)
                .map(|(_, entity)| Ok(entity.clone()))
                .collect::<Vec<_>>()
                .into_iter(),
        ))
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> std::result::Result<BoxEntityIdScan, DbError> {
        Ok(Box::new(
            Self::scan_index(&self.indexes, index, path, value)
                .into_iter()
                .map(Ok),
        ))
    }

    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        use std::ops::RangeBounds as _;

        Ok(Self::scan_index_ordered(
            &self.indexes,
            index,
            path,
            |value| (lower, upper).contains(value),
        ))
    }

    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        Ok(Self::scan_index_ordered(
            &self.indexes,
            index,
            path,
            |value| matches!(value, Value::String(value) if value.starts_with(prefix)),
        ))
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> std::result::Result<bool, DbError> {
        Ok(!self.initialized_indexes.contains(&index))
    }

    fn snapshot(&self) -> Result<Box<dyn EntityReadSnapshot + '_>, DbError> {
        Ok(Box::new(MemoryEntityReadSnapshot { storage: self }))
    }

    fn owned_snapshot(&self) -> Result<Option<std::sync::Arc<dyn EntityReadSnapshot>>, DbError> {
        Ok(Some(std::sync::Arc::new(OwnedStorageSnapshot(
            self.frozen_copy(),
        ))))
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        StorageTransactionCapabilities {
            conflict_detection: true,
            mvcc: self.snapshots.is_some(),
            snapshot_reads: self.snapshots.is_some(),
        }
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        Ok(Some(self.revision))
    }

    fn apply_batch(&mut self, ops: &[StorageWriteOp]) -> std::result::Result<(), DbError> {
        self.apply(ops);
        Ok(())
    }

    fn apply_batch_conditional(
        &mut self,
        ops: &[StorageWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        if let Some(expected) = expected_revision
            && expected != self.revision
        {
            return Ok(StorageCommitOutcome::Conflict {
                expected_revision: Some(expected),
                actual_revision: Some(self.revision),
            });
        }
        self.apply(ops);
        Ok(StorageCommitOutcome::Committed {
            revision: Some(self.revision),
        })
    }
}

/// Read call counters of a [`CountingEntityStorage`].
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct StorageReadCounts {
    pub(crate) collection_scans: std::sync::atomic::AtomicUsize,
    pub(crate) entity_gets: std::sync::atomic::AtomicUsize,
    pub(crate) collection_counts: std::sync::atomic::AtomicUsize,
    /// Entities yielded by collection scans.
    pub(crate) rows_yielded: std::sync::atomic::AtomicUsize,
    /// Report maintained row counts as unknown, forcing the key-count
    /// fallback.
    pub(crate) hide_row_counts: std::sync::atomic::AtomicBool,
    /// Entities put or deleted by committed batches, as (collection, id).
    pub(crate) entity_writes: std::sync::Mutex<Vec<(usize, String)>>,
}

#[cfg(test)]
impl StorageReadCounts {
    pub(crate) fn reset(&self) {
        use std::sync::atomic::Ordering;

        self.collection_scans.store(0, Ordering::Relaxed);
        self.entity_gets.store(0, Ordering::Relaxed);
        self.collection_counts.store(0, Ordering::Relaxed);
        self.rows_yielded.store(0, Ordering::Relaxed);
        self.entity_writes.lock().unwrap().clear();
    }

    /// Ids of the entities of `collection` written since the last reset.
    pub(crate) fn written_ids(&self, collection: LocalCollectionId) -> Vec<String> {
        self.entity_writes
            .lock()
            .unwrap()
            .iter()
            .filter(|(written, _)| *written == collection.0)
            .map(|(_, id)| id.clone())
            .collect()
    }

    pub(crate) fn rows_yielded(&self) -> usize {
        self.rows_yielded.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn collection_scans(&self) -> usize {
        self.collection_scans
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn entity_gets(&self) -> usize {
        self.entity_gets.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn collection_counts(&self) -> usize {
        self.collection_counts
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// [`MemoryEntityStorage`] wrapper counting collection scans, point reads
/// and collection counts, including those made through snapshots.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct CountingEntityStorage {
    inner: MemoryEntityStorage,
    counts: std::sync::Arc<StorageReadCounts>,
}

#[cfg(test)]
impl CountingEntityStorage {
    pub(crate) fn new() -> (Self, std::sync::Arc<StorageReadCounts>) {
        let storage = Self::default();
        let counts = storage.counts.clone();
        (storage, counts)
    }

    fn count(counter: &std::sync::atomic::AtomicUsize) {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn record_writes(&self, ops: &[StorageWriteOp]) {
        let mut writes = self.counts.entity_writes.lock().unwrap();
        for op in ops {
            match op {
                StorageWriteOp::PutEntity(entity) => {
                    writes.push((entity.collection, entity.id.clone()));
                }
                StorageWriteOp::DeleteEntity {
                    collection,
                    entity_id,
                } => writes.push((collection.0, entity_id.clone())),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
impl EntityStorage for CountingEntityStorage {
    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        Self::count(&self.counts.entity_gets);
        self.inner.get_entity(collection, id)
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        Self::count(&self.counts.collection_scans);
        let counts = self.counts.clone();
        Ok(Box::new(
            self.inner
                .scan_collection_stream(collection)?
                .map(move |entity| {
                    Self::count(&counts.rows_yielded);
                    entity
                }),
        ))
    }

    fn count_collection_entities(&self, collection: LocalCollectionId) -> Result<u64, DbError> {
        Self::count(&self.counts.collection_counts);
        self.inner.count_collection_entities(collection)
    }

    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        if self
            .counts
            .hide_row_counts
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Ok(None);
        }
        self.inner.collection_row_count(collection)
    }

    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        self.inner.index_entry_count(index)
    }

    fn owned_snapshot(&self) -> Result<Option<std::sync::Arc<dyn EntityReadSnapshot>>, DbError> {
        Ok(Some(std::sync::Arc::new(OwnedStorageSnapshot(Self {
            inner: self.inner.frozen_copy(),
            counts: self.counts.clone(),
        }))))
    }

    fn scan_collection_at_revision_stream(
        &self,
        collection: LocalCollectionId,
        revision: u64,
    ) -> Result<BoxEntityScan, DbError> {
        Self::count(&self.counts.collection_scans);
        self.inner
            .scan_collection_at_revision_stream(collection, revision)
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.inner.scan_index_value_stream(index, path, value)
    }

    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.inner
            .scan_index_range_stream(index, path, lower, upper)
    }

    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.inner.scan_index_prefix_stream(index, path, prefix)
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        self.inner.index_needs_rebuild(index)
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.inner.tx_capabilities()
    }

    fn current_revision(&self) -> Result<Option<u64>, DbError> {
        self.inner.current_revision()
    }

    fn apply_batch(&mut self, ops: &[StorageWriteOp]) -> Result<(), DbError> {
        self.inner.apply_batch(ops)?;
        self.record_writes(ops);
        Ok(())
    }

    fn apply_batch_conditional(
        &mut self,
        ops: &[StorageWriteOp],
        expected_revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        let outcome = self.inner.apply_batch_conditional(ops, expected_revision)?;
        if matches!(outcome, StorageCommitOutcome::Committed { .. }) {
            self.record_writes(ops);
        }
        Ok(outcome)
    }
}

/// Owned read handle over a frozen test storage copy.
#[cfg(test)]
struct OwnedStorageSnapshot<S>(S);

#[cfg(test)]
impl<S: EntityStorage> EntityReadSnapshot for OwnedStorageSnapshot<S> {
    fn revision(&self) -> Result<Option<u64>, DbError> {
        self.0.current_revision()
    }

    fn is_consistent(&self) -> bool {
        true
    }

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        self.0.get_entity(collection, id)
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        self.0.scan_collection_stream(collection)
    }

    fn count_collection_entities(&self, collection: LocalCollectionId) -> Result<u64, DbError> {
        self.0.count_collection_entities(collection)
    }

    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        self.0.collection_row_count(collection)
    }

    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        self.0.index_entry_count(index)
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.0.scan_index_value_stream(index, path, value)
    }

    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.0.scan_index_range_stream(index, path, lower, upper)
    }

    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.0.scan_index_prefix_stream(index, path, prefix)
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        self.0.index_needs_rebuild(index)
    }
}

/// Borrowed view of [`MemoryEntityStorage`].
///
/// Writes require exclusive access, so the borrowed state cannot change while
/// the handle is alive.
#[cfg(test)]
struct MemoryEntityReadSnapshot<'a> {
    storage: &'a MemoryEntityStorage,
}

#[cfg(test)]
impl EntityReadSnapshot for MemoryEntityReadSnapshot<'_> {
    fn revision(&self) -> Result<Option<u64>, DbError> {
        Ok(Some(self.storage.revision))
    }

    fn is_consistent(&self) -> bool {
        true
    }

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        EntityStorage::get_entity(self.storage, collection, id)
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        EntityStorage::scan_collection_stream(self.storage, collection)
    }

    fn count_collection_entities(&self, collection: LocalCollectionId) -> Result<u64, DbError> {
        EntityStorage::count_collection_entities(self.storage, collection)
    }

    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        EntityStorage::collection_row_count(self.storage, collection)
    }

    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        EntityStorage::index_entry_count(self.storage, index)
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        EntityStorage::scan_index_value_stream(self.storage, index, path, value)
    }

    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        EntityStorage::scan_index_range_stream(self.storage, index, path, lower, upper)
    }

    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        EntityStorage::scan_index_prefix_stream(self.storage, index, path, prefix)
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        EntityStorage::index_needs_rebuild(self.storage, index)
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::schema::{IndexKind, IndexSchema as DataIndexSchema, KeyPath};

    use super::*;

    fn test_index() -> IndexSchema {
        IndexSchema {
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
        }
    }

    fn object(kind: &str) -> Object {
        let mut object = Object::new();
        object.insert("kind", Value::String(kind.to_string()));
        object
    }

    #[test]
    fn memory_ordered_index_scans_follow_value_then_id_order() {
        let index = test_index();
        let mut storage = MemoryEntityStorage::new();
        let mut ops = vec![StorageWriteOp::ResetIndex(index.lid)];
        for (id, kind) in [("d", "b"), ("a", "c"), ("c", "ab"), ("b", "ab"), ("e", "a")] {
            ops.push(StorageWriteOp::IndexEntity {
                index: index.clone(),
                entity_id: id.to_string(),
                object: object(kind),
            });
        }
        storage.apply_batch(&ops).unwrap();
        let ids = |scan: BoxEntityIdScan| scan.collect::<Result<Vec<_>, _>>().unwrap();

        let ab = Value::String("ab".to_string());
        let c = Value::String("c".to_string());
        let range = storage
            .scan_index_range_stream(index.lid, None, Bound::Included(&ab), Bound::Excluded(&c))
            .unwrap();
        assert_eq!(ids(range), ["b", "c", "d"]);
        let range = storage
            .snapshot()
            .unwrap()
            .scan_index_range_stream(index.lid, None, Bound::Excluded(&ab), Bound::Unbounded)
            .unwrap();
        assert_eq!(ids(range), ["d", "a"]);
        let prefix = storage
            .scan_index_prefix_stream(index.lid, None, "a")
            .unwrap();
        assert_eq!(ids(prefix), ["e", "b", "c"]);
    }

    #[test]
    fn memory_index_operations_are_additive_idempotent_and_resettable() {
        let index = test_index();
        let mut storage = MemoryEntityStorage::new();
        let old = object("old");
        let new = object("new");
        storage
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
            storage
                .scan_index_value(index.lid, None, &Value::String("old".to_string()))
                .unwrap(),
            vec!["one".to_string()]
        );
        assert_eq!(
            storage
                .scan_index_value(index.lid, None, &Value::String("new".to_string()))
                .unwrap(),
            vec!["one".to_string()]
        );

        storage
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
            storage
                .scan_index_value(index.lid, None, &Value::String("new".to_string()))
                .unwrap()
                .is_empty()
        );
    }
}
