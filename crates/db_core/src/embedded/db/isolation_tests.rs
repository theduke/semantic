use std::ops::Bound;

use super::*;
use crate::embedded::MemoryEntityStorage;
use crate::embedded::storage::{BoxEntityIdScan, BoxEntityScan, ForwardingReadSnapshot};
use crate::{BatchReturn, Expr, Operand, UpdateQuery};

const ITEMS: &str = "isolation_items";

const LEVELS: [IsolationLevel; 4] = [
    IsolationLevel::ReadCommitted,
    IsolationLevel::RepeatableRead,
    IsolationLevel::Snapshot,
    IsolationLevel::Serializable,
];

/// Memory storage that can hide its consistent snapshots (reads are then
/// forwarded, like a storage without native read transactions) and its
/// commit-time conflict detection.
#[derive(Debug)]
struct LimitedStorage {
    inner: MemoryEntityStorage,
    consistent_snapshots: bool,
    conflict_detection: bool,
}

impl LimitedStorage {
    fn new(consistent_snapshots: bool, conflict_detection: bool) -> Self {
        Self {
            inner: MemoryEntityStorage::new(),
            consistent_snapshots,
            conflict_detection,
        }
    }
}

impl EntityStorage for LimitedStorage {
    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        self.inner.get_entity(collection, id)
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        self.inner.scan_collection_stream(collection)
    }

    fn scan_collection_at_revision_stream(
        &self,
        collection: LocalCollectionId,
        revision: u64,
    ) -> Result<BoxEntityScan, DbError> {
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

    fn snapshot(&self) -> Result<Box<dyn EntityReadSnapshot + '_>, DbError> {
        if self.consistent_snapshots {
            self.inner.snapshot()
        } else {
            Ok(Box::new(ForwardingReadSnapshot::new(self)))
        }
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        StorageTransactionCapabilities {
            conflict_detection: self.conflict_detection,
            ..self.inner.tx_capabilities()
        }
    }

    fn current_revision(&self) -> Result<Option<u64>, DbError> {
        self.inner.current_revision()
    }

    fn apply_batch(&mut self, ops: &[StorageWriteOp]) -> Result<(), DbError> {
        self.inner.apply_batch(ops)
    }

    fn apply_batch_conditional(
        &mut self,
        ops: &[StorageWriteOp],
        expected_revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        self.inner.apply_batch_conditional(ops, expected_revision)
    }
}

fn options(isolation: IsolationLevel) -> TransactionOptions {
    TransactionOptions {
        isolation,
        ..TransactionOptions::default()
    }
}

fn item(id: &str, label: &str) -> BatchOperation {
    BatchOperation::Upsert {
        collection: ITEMS.to_string(),
        id: id.to_string(),
        object: Object::from_iter(
            [("id", id), ("label", label)]
                .map(|(key, value)| (key.to_string(), Value::String(value.to_string()))),
        ),
    }
}

/// `UPDATE items SET label = 'updated' WHERE label = 'old'`.
fn relabel_old() -> BatchOperation {
    let literal = |value: &str| Expr::Operand(Operand::Literal(Value::String(value.into())));
    let label = FieldPath::from_fields(["label"]);
    BatchOperation::Update {
        collection: ITEMS.to_string(),
        query: UpdateQuery::new()
            .with_collection(ITEMS)
            .with_predicate(Expr::Binary {
                op: semantic_data::query::BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(label.clone()))),
                right: Box::new(literal("old")),
            })
            .set(label, literal("updated")),
    }
}

fn setup<S: EntityStorage>(storage: S) -> EmbeddedDb<S> {
    let mut db = EmbeddedDb::new(storage);
    db.create_collection(ITEMS, CollectionKind::Polymorphic)
        .unwrap();
    db
}

fn label<S: EntityStorage>(db: &EmbeddedDb<S>, id: &str) -> Option<Value> {
    db.get(ITEMS, id)
        .unwrap()
        .and_then(|record| record.object.get("label").cloned())
}

#[test]
fn every_isolation_level_commits_on_snapshot_storage() {
    for level in LEVELS {
        for returning in [BatchReturn::Dataset, BatchReturn::Stats] {
            let mut db = setup(MemoryEntityStorage::new());
            db.transact_returning(
                Batch::new()
                    .with_op(item("a", "old"))
                    .with_op(item("b", "new")),
                options(level),
                returning.clone(),
                crate::WriteSettings::default(),
                false,
            )
            .unwrap_or_else(|err| panic!("{level:?}/{returning:?}: {err}"));
            db.transact_returning(
                Batch::new()
                    .with_op(relabel_old())
                    .with_op(item("c", "old")),
                options(level),
                returning.clone(),
                crate::WriteSettings::default(),
                false,
            )
            .unwrap_or_else(|err| panic!("{level:?}/{returning:?}: {err}"));
            assert_eq!(label(&db, "a"), Some(Value::String("updated".into())));
            assert_eq!(label(&db, "b"), Some(Value::String("new".into())));
            assert_eq!(label(&db, "c"), Some(Value::String("old".into())));
        }
    }
}

#[test]
fn snapshot_levels_fail_without_consistent_snapshots() {
    let mut db = setup(LimitedStorage::new(false, true));
    for level in LEVELS {
        let result =
            db.transact_with_options(Batch::new().with_op(item("a", "old")), options(level));
        if level == IsolationLevel::ReadCommitted {
            result.unwrap();
            continue;
        }
        let err = result.unwrap_err();
        assert_eq!(
            err.storage_kind(),
            Some(StorageErrorKind::Unsupported),
            "{err}"
        );
        assert!(
            err.to_string()
                .contains("isolation requires consistent snapshot reads"),
            "{err}"
        );
    }

    // Nothing was written by the rejected transactions.
    let revision = db.storage.current_revision().unwrap();
    let err = db
        .transact_with_options(
            Batch::new().with_op(item("b", "old")),
            options(IsolationLevel::Snapshot),
        )
        .unwrap_err();
    assert_eq!(err.storage_kind(), Some(StorageErrorKind::Unsupported));
    assert_eq!(db.storage.current_revision().unwrap(), revision);
    assert_eq!(label(&db, "b"), None);
}

#[test]
fn serializable_requires_conflict_detection() {
    let mut db = setup(LimitedStorage::new(true, false));
    for level in LEVELS {
        let result =
            db.transact_with_options(Batch::new().with_op(item("a", "old")), options(level));
        if level != IsolationLevel::Serializable {
            result.unwrap_or_else(|err| panic!("{level:?}: {err}"));
            continue;
        }
        let err = result.unwrap_err();
        assert_eq!(
            err.storage_kind(),
            Some(StorageErrorKind::Unsupported),
            "{err}"
        );
        assert!(err.to_string().contains("conflict detection"), "{err}");
    }
}

#[test]
fn snapshot_reads_conflict_once_the_read_revision_moved() {
    let mut db = setup(MemoryEntityStorage::new());
    db.transact(Batch::new().with_op(item("a", "old"))).unwrap();
    let stale = db.storage.current_revision().unwrap();
    db.transact(Batch::new().with_op(item("a", "new"))).unwrap();
    let current = db.storage.current_revision().unwrap();

    for level in LEVELS {
        let reader = RevisionReader::for_isolation(&db.storage, current, level).unwrap();
        assert!(reader.is_snapshot(), "{level:?}");

        let stale_reader = RevisionReader::for_isolation(&db.storage, stale, level);
        if level.requires_snapshot() {
            assert!(
                matches!(stale_reader, Err(DbError::TransactionConflict(_))),
                "{level:?}"
            );
        } else {
            // Read committed falls back to revision-fenced reads.
            assert!(!stale_reader.unwrap().is_snapshot());
        }
    }
}
