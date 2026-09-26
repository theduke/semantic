use semantic_data::value::{FieldPath, Object, Value};

use crate::DbError;
use crate::catalog::{IndexSchema, LocalCollectionId, LocalIndexId};

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

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError>;
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

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.storage.scan_index_value_stream(index, path, value)
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

    fn index_needs_rebuild(&self, index: LocalIndexId) -> std::result::Result<bool, DbError>;

    /// Open a read handle for one logical operation.
    ///
    /// The default forwards every read to this storage and is not consistent;
    /// backends with read transactions should return a handle that serves all
    /// reads from one transaction.
    fn snapshot(&self) -> Result<Box<dyn EntityReadSnapshot + '_>, DbError> {
        Ok(Box::new(ForwardingReadSnapshot::new(self)))
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
            .ok_or_else(|| DbError::Storage(format!("snapshot {revision} not available")))?;
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

    fn index_needs_rebuild(&self, index: LocalIndexId) -> std::result::Result<bool, DbError> {
        Ok(!self.initialized_indexes.contains(&index))
    }

    fn snapshot(&self) -> Result<Box<dyn EntityReadSnapshot + '_>, DbError> {
        Ok(Box::new(MemoryEntityReadSnapshot { storage: self }))
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

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        EntityStorage::scan_index_value_stream(self.storage, index, path, value)
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
