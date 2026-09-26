//! Engine transaction handles and their use by [`EntityStore`].
use super::*;
use semantic_data::query::BinaryOp;
use semantic_db_core::catalog::CollectionKind;
use semantic_db_core::embedded::EmbeddedDb;
use semantic_db_core::{Batch, BatchOperation, Expr, Operand, SelectQuery};
use std::sync::{Arc, Mutex};

fn put(key: &[u8], value: &[u8]) -> KvWriteOp {
    KvWriteOp::Put {
        key: key.to_vec(),
        value: value.to_vec(),
    }
}

fn keys(scan: BoxKvPrefixScan) -> Vec<Vec<u8>> {
    scan.map(|item| item.unwrap().0).collect()
}

fn seeded_memory() -> MemoryKvEngine {
    let mut engine = MemoryKvEngine::new();
    engine
        .write_batch(&[
            put(b"a", b"1"),
            put(b"b", b"2"),
            put(b"b/1", b"3"),
            put(b"c", b"4"),
            put(&[b'd', u8::MAX], b"5"),
            put(&[b'd', u8::MAX, 0], b"6"),
        ])
        .unwrap();
    engine
}

/// Engine implementing only the required methods, exercising the defaults.
#[derive(Debug, Default)]
struct MinimalEngine(MemoryKvEngine);

impl KvEngine for MinimalEngine {
    type PrefixScan = BoxKvPrefixScan;
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        self.0.get(key)
    }
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), DbError> {
        self.0.put(key, value)
    }
    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.0.delete(key)
    }
    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<Self::PrefixScan, DbError> {
        let items = self
            .0
            .scan_prefix(b"")?
            .into_iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .map(Ok)
            .collect::<Vec<_>>();
        Ok(Box::new(items.into_iter()))
    }
    fn current_revision(&self) -> Result<Option<u64>, DbError> {
        self.0.current_revision()
    }
    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        self.0.write_batch_conditional(ops, expected_revision)
    }
}

fn assert_range_bounds(engine: &impl KvEngine) {
    let read = engine.begin_read().unwrap();
    assert_eq!(
        keys(
            read.scan_range_stream(b"b".to_vec(), Some(b"c".to_vec()))
                .unwrap()
        ),
        vec![b"b".to_vec(), b"b/1".to_vec()],
        "start is inclusive and end exclusive"
    );
    assert_eq!(
        keys(read.scan_range_stream(b"b/".to_vec(), None).unwrap()),
        vec![
            b"b/1".to_vec(),
            b"c".to_vec(),
            vec![b'd', u8::MAX],
            vec![b'd', u8::MAX, 0]
        ],
    );
    assert!(
        keys(
            read.scan_range_stream(b"c".to_vec(), Some(b"b".to_vec()))
                .unwrap()
        )
        .is_empty(),
        "an inverted range is empty"
    );
    assert_eq!(
        keys(read.scan_prefix_stream(b"b".to_vec()).unwrap()),
        vec![b"b".to_vec(), b"b/1".to_vec()]
    );
    assert_eq!(
        keys(read.scan_prefix_stream(vec![b'd', u8::MAX]).unwrap()),
        vec![vec![b'd', u8::MAX], vec![b'd', u8::MAX, 0]],
        "prefixes ending in 0xff have a shorter upper bound"
    );
    assert_eq!(
        keys(
            engine
                .scan_range_stream(b"a/".to_vec(), Some(b"b/2".to_vec()))
                .unwrap()
        ),
        vec![b"b".to_vec(), b"b/1".to_vec()]
    );
}

#[test]
fn prefix_range_end_increments_last_non_max_byte() {
    assert_eq!(prefix_range_end(b"ab"), Some(b"ac".to_vec()));
    assert_eq!(prefix_range_end(&[1, u8::MAX]), Some(vec![2]));
    assert_eq!(prefix_range_end(&[u8::MAX, u8::MAX]), None);
    assert_eq!(prefix_range_end(b""), None);
}

#[test]
fn memory_and_default_range_scans_respect_bounds() {
    let memory = seeded_memory();
    assert_range_bounds(&memory);
    assert_range_bounds(&MinimalEngine(memory));
}

#[test]
fn memory_read_handle_is_isolated_from_later_writes() {
    let mut engine = seeded_memory();
    let revision = engine.current_revision().unwrap();
    let snapshot = engine.read_snapshot();
    engine
        .write_batch(&[put(b"a", b"changed"), put(b"b/2", b"new")])
        .unwrap();
    engine.delete(b"c").unwrap();

    assert!(snapshot.is_snapshot());
    assert_eq!(snapshot.revision(), revision);
    assert_eq!(snapshot.get(b"a").unwrap(), Some(b"1".to_vec()));
    assert_eq!(
        keys(snapshot.scan_prefix_stream(b"b".to_vec()).unwrap()),
        vec![b"b".to_vec(), b"b/1".to_vec()]
    );
    assert_eq!(snapshot.get(b"c").unwrap(), Some(b"4".to_vec()));

    let fresh = engine.begin_read().unwrap();
    assert_eq!(fresh.get(b"a").unwrap(), Some(b"changed".to_vec()));
    assert_eq!(fresh.get(b"c").unwrap(), None);
    assert_eq!(fresh.revision(), engine.current_revision().unwrap());
}

fn assert_write_with_semantics(engine: &mut impl KvEngine) {
    let revision = engine.current_revision().unwrap();
    let outcome = engine
        .write_with(revision, |txn| {
            assert_eq!(txn.get(b"a")?, Some(b"1".to_vec()));
            txn.delete(b"missing")
        })
        .unwrap();
    assert_eq!(outcome, StorageCommitOutcome::Committed { revision });
    assert_eq!(engine.current_revision().unwrap(), revision);

    let failed = engine.write_with(revision, |txn| {
        txn.put(b"a", b"lost")?;
        txn.delete(b"b")?;
        Err(DbError::Storage("abort".into()))
    });
    assert!(failed.is_err());
    assert_eq!(engine.get(b"a").unwrap(), Some(b"1".to_vec()));
    assert_eq!(engine.get(b"b").unwrap(), Some(b"2".to_vec()));
    assert_eq!(engine.current_revision().unwrap(), revision);

    let outcome = engine
        .write_with(revision, |txn| {
            txn.put(b"b/2", b"x")?;
            assert_eq!(txn.get(b"b/2")?, Some(b"x".to_vec()));
            assert_eq!(
                txn.scan_prefix(b"b/")?,
                vec![
                    (b"b/1".to_vec(), b"3".to_vec()),
                    (b"b/2".to_vec(), b"x".to_vec())
                ]
            );
            Ok(())
        })
        .unwrap();
    let next = revision.map(|revision| revision + 1);
    assert_eq!(outcome, StorageCommitOutcome::Committed { revision: next });
    assert_eq!(
        engine.write_with(revision, |_| Ok(())).unwrap(),
        StorageCommitOutcome::Conflict {
            expected_revision: revision,
            actual_revision: next,
        }
    );
}

#[test]
fn memory_and_default_write_transactions_skip_no_ops_and_roll_back() {
    assert_write_with_semantics(&mut seeded_memory());
    assert_write_with_semantics(&mut MinimalEngine(seeded_memory()));
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Counts {
    read_txns: usize,
    write_txns: usize,
    /// Reads outside an explicit handle; each is a transaction on redb.
    implicit_reads: usize,
    revision_reads: usize,
}

#[derive(Debug)]
struct CountingEngine {
    inner: MemoryKvEngine,
    counts: Arc<Mutex<Counts>>,
}

impl CountingEngine {
    fn count(&self, update: impl FnOnce(&mut Counts)) {
        update(&mut self.counts.lock().unwrap());
    }
}

impl KvEngine for CountingEngine {
    type PrefixScan = BoxKvPrefixScan;
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        self.count(|counts| counts.implicit_reads += 1);
        self.inner.get(key)
    }
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), DbError> {
        self.inner.put(key, value)
    }
    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.inner.delete(key)
    }
    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<Self::PrefixScan, DbError> {
        self.count(|counts| counts.implicit_reads += 1);
        self.inner.scan_prefix_stream(prefix)
    }
    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        self.count(|counts| counts.implicit_reads += 1);
        self.inner.scan_range_stream(start, end)
    }
    fn begin_read(&self) -> Result<Box<dyn KvReadTxn + '_>, DbError> {
        self.count(|counts| counts.read_txns += 1);
        self.inner.begin_read()
    }
    fn write_with<F>(
        &mut self,
        expected_revision: Option<u64>,
        f: F,
    ) -> Result<StorageCommitOutcome, DbError>
    where
        F: FnOnce(&mut dyn KvWriteTxn) -> Result<(), DbError>,
    {
        self.count(|counts| counts.write_txns += 1);
        self.inner.write_with(expected_revision, f)
    }
    fn write_batch(&mut self, ops: &[KvWriteOp]) -> Result<(), DbError> {
        self.count(|counts| counts.write_txns += 1);
        self.inner.write_batch(ops)
    }
    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        self.count(|counts| counts.write_txns += 1);
        self.inner.write_batch_conditional(ops, expected_revision)
    }
    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.inner.tx_capabilities()
    }
    fn current_revision(&self) -> Result<Option<u64>, DbError> {
        self.count(|counts| counts.revision_reads += 1);
        self.inner.current_revision()
    }
}

fn row(id: &str, name: &str) -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String(id.into()));
    object.insert("name", Value::String(name.into()));
    object
}

fn counting_db() -> (EmbeddedDb<EntityStore<CountingEngine>>, Arc<Mutex<Counts>>) {
    let counts = Arc::new(Mutex::new(Counts::default()));
    let mut db = EmbeddedDb::open(EntityStore::new(CountingEngine {
        inner: MemoryKvEngine::new(),
        counts: counts.clone(),
    }))
    .unwrap();
    let items = db
        .create_collection("items", CollectionKind::Polymorphic)
        .unwrap();
    db.create_index("by_name", items, "name", true).unwrap();
    db.execute_batch(Batch {
        operations: (0..20)
            .map(|i| BatchOperation::Upsert {
                collection: "items".into(),
                id: format!("row-{i}"),
                object: row(&format!("row-{i}"), &format!("name-{i}")),
            })
            .collect(),
    })
    .unwrap();
    (db, counts)
}

fn take(counts: &Mutex<Counts>) -> Counts {
    std::mem::take(&mut *counts.lock().unwrap())
}

#[test]
fn point_reads_use_one_read_transaction() {
    let (db, counts) = counting_db();
    take(&counts);

    assert!(db.get("items", "row-3").unwrap().is_some());
    assert_eq!(
        take(&counts),
        Counts {
            read_txns: 1,
            ..Counts::default()
        }
    );

    let rows = db
        .select(
            SelectQuery::new()
                .with_collection("items")
                .with_predicate(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "name",
                    ])))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "name-3".into(),
                    )))),
                }),
        )
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        take(&counts),
        Counts {
            read_txns: 1,
            ..Counts::default()
        },
        "statistics, index probe and row materialization share one snapshot"
    );
}

fn upsert(db: &mut EmbeddedDb<EntityStore<CountingEngine>>, id: &str, name: &str) {
    db.execute_batch_returning(
        Batch::new().with_op(BatchOperation::Upsert {
            collection: "items".into(),
            id: id.into(),
            object: row(id, name),
        }),
        semantic_db_core::BatchReturn::Stats,
    )
    .unwrap();
}

#[test]
fn point_writes_read_one_snapshot_and_compare_inside_the_write_transaction() {
    let (mut db, counts) = counting_db();
    take(&counts);

    // Entity plus old/new unique index keys: previously one engine read per
    // touched key before the write transaction.
    upsert(&mut db, "row-3", "changed");
    let write = take(&counts);
    assert_eq!(
        (write.read_txns, write.write_txns, write.implicit_reads),
        (1, 1, 0),
        "{write:?}"
    );
    let (_, store) = db.into_parts();
    let revision = store.current_revision().unwrap();
    let mut db = EmbeddedDb::open(store).unwrap();

    take(&counts);
    upsert(&mut db, "row-3", "changed");
    let noop = take(&counts);
    assert_eq!(noop.implicit_reads, 0, "{noop:?}");
    let (_, store) = db.into_parts();
    assert_eq!(
        store.current_revision().unwrap(),
        revision,
        "an unchanged batch must not advance the revision"
    );
}
