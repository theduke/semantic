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

pub trait EntityStorage: std::fmt::Debug + Send + Sync + 'static {
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
