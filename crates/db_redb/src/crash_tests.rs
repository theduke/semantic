//! Crash consistency of the redb engine.
//!
//! - [`FaultyEngine`] wraps a [`RedbKvEngine`] and fails (with an error or a
//!   panic) at a chosen point of a chosen engine write: before the commit,
//!   after some puts or deletes inside `write_with`, or after the commit but
//!   before returning. After the failure the database file is reopened and
//!   must show the whole write or nothing, with indexes, reverse references
//!   and stats counters consistent (`verify` is clean).
//! - The same injection interrupts the migrations run on open (layout
//!   migration, stats backfill, index rebuilds of a legacy database); the
//!   next open must complete them with the data intact.
//! - An ignored test kills a child process writing batches with `kill -9`
//!   at random moments and checks every reported commit survived.
//!
//! See `docs/testing.md`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write as _};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use semantic_data::query::{BinaryOp, SortDirection};
use semantic_data::schema::{DbOpenMode, IndexKind};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::catalog::IntegrityMode;
use semantic_db_core::embedded::{
    BackupSource, EmbeddedDb, StorageCommitOutcome, StorageTransactionCapabilities,
};
use semantic_db_core::{
    Batch, BatchOperation, DbError, DdlBatch, DdlCollectionKind, DdlOperation, Expr, Operand,
    OrderBy, SelectQuery, VerifyOptions,
};
use semantic_db_kv::{
    BoxKvPrefixScan, EntityStore, KvEngine, KvEngineStats, KvReadTxn, KvWriteOp, KvWriteTxn,
};
use semantic_db_test::rng::{SeedGuard, TestRng, env_usize, seeds_from_env};

use crate::{RedbKvEngine, RedbOptions};

/// Where an armed fault strikes inside a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaultPoint {
    /// After the write body ran, before the commit.
    BeforeCommit,
    /// When the body issues its `n`-th put or delete (counting from 1).
    AfterWrites(usize),
    /// After the commit, before returning to the caller.
    AfterCommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fault {
    /// Engine write (`write_with` call) to fail, counting from 0 after
    /// arming.
    write: usize,
    point: FaultPoint,
    panic: bool,
}

/// Shared control of a [`FaultyEngine`].
#[derive(Debug, Default)]
struct FaultPlan {
    armed: Mutex<Option<Fault>>,
    writes: AtomicUsize,
    triggered: AtomicBool,
}

impl FaultPlan {
    fn arm(&self, fault: Fault) {
        self.writes.store(0, Ordering::SeqCst);
        self.triggered.store(false, Ordering::SeqCst);
        *self.armed.lock().unwrap() = Some(fault);
    }

    fn disarm(&self) -> bool {
        *self.armed.lock().unwrap() = None;
        self.triggered.load(Ordering::SeqCst)
    }

    /// The fault of the write starting now, if it is the armed one.
    fn fault_for_next_write(&self) -> Option<Fault> {
        let write = self.writes.fetch_add(1, Ordering::SeqCst);
        let fault = (*self.armed.lock().unwrap())?;
        (fault.write == write).then_some(fault)
    }

    fn strike(&self, fault: Fault) -> DbError {
        self.triggered.store(true, Ordering::SeqCst);
        // Strike once; later writes (for example of a reopen) run normally.
        *self.armed.lock().unwrap() = None;
        if fault.panic {
            panic!("injected panic at {:?}", fault.point);
        }
        DbError::storage(
            semantic_db_core::StorageErrorKind::Backend,
            format!("injected failure at {:?}", fault.point),
        )
    }
}

/// A [`RedbKvEngine`] failing at the point armed in its [`FaultPlan`].
#[derive(Debug)]
struct FaultyEngine {
    inner: RedbKvEngine,
    plan: Arc<FaultPlan>,
}

/// Write transaction counting puts and deletes to fail at the `n`-th.
struct FaultyTxn<'a> {
    inner: &'a mut dyn KvWriteTxn,
    plan: &'a FaultPlan,
    fault: Option<Fault>,
    writes: usize,
}

impl FaultyTxn<'_> {
    fn count_write(&mut self) -> Result<(), DbError> {
        self.writes += 1;
        match self.fault {
            Some(fault) if fault.point == FaultPoint::AfterWrites(self.writes) => {
                Err(self.plan.strike(fault))
            }
            _ => Ok(()),
        }
    }
}

impl KvWriteTxn for FaultyTxn<'_> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        self.inner.get(key)
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.inner.scan_prefix(prefix)
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError> {
        self.inner.put(key, value)?;
        self.count_write()
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.inner.delete(key)?;
        self.count_write()
    }
}

fn apply_ops(txn: &mut dyn KvWriteTxn, ops: &[KvWriteOp]) -> Result<(), DbError> {
    for op in ops {
        match op {
            KvWriteOp::Put { key, value } => txn.put(key, value)?,
            KvWriteOp::Delete { key } => txn.delete(key)?,
        }
    }
    Ok(())
}

impl KvEngine for FaultyEngine {
    type PrefixScan = <RedbKvEngine as KvEngine>::PrefixScan;

    fn begin_read(&self) -> Result<Box<dyn KvReadTxn + '_>, DbError> {
        self.inner.begin_read()
    }

    fn begin_read_owned(&self) -> Result<Option<Box<dyn KvReadTxn>>, DbError> {
        self.inner.begin_read_owned()
    }

    fn write_with<F>(
        &mut self,
        expected_revision: Option<u64>,
        f: F,
    ) -> Result<StorageCommitOutcome, DbError>
    where
        F: FnOnce(&mut dyn KvWriteTxn) -> Result<(), DbError>,
    {
        let plan = Arc::clone(&self.plan);
        let fault = plan.fault_for_next_write();
        let outcome = self.inner.write_with(expected_revision, |txn| {
            let mut txn = FaultyTxn {
                inner: txn,
                plan: &plan,
                fault,
                writes: 0,
            };
            f(&mut txn)?;
            match fault {
                Some(fault) if fault.point == FaultPoint::BeforeCommit => Err(plan.strike(fault)),
                _ => Ok(()),
            }
        })?;
        match fault {
            Some(fault) if fault.point == FaultPoint::AfterCommit => Err(plan.strike(fault)),
            _ => Ok(outcome),
        }
    }

    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        self.inner.scan_range_stream(start, end)
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        self.inner.get(key)
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), DbError> {
        self.write_batch(&[KvWriteOp::Put { key, value }])
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.write_batch(&[KvWriteOp::Delete { key: key.to_vec() }])
    }

    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<Self::PrefixScan, DbError> {
        self.inner.scan_prefix_stream(prefix)
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.inner.scan_prefix(prefix)
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.inner.tx_capabilities()
    }

    fn current_revision(&self) -> Result<Option<u64>, DbError> {
        self.inner.current_revision()
    }

    fn scan_prefix_at_revision_stream(
        &self,
        prefix: Vec<u8>,
        revision: u64,
    ) -> Result<Self::PrefixScan, DbError> {
        self.inner.scan_prefix_at_revision_stream(prefix, revision)
    }

    fn scan_prefix_at_revision(
        &self,
        prefix: &[u8],
        revision: u64,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.inner.scan_prefix_at_revision(prefix, revision)
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        self.write_with(expected_revision, |txn| apply_ops(txn, ops))
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> Result<(), DbError> {
        self.write_with(None, |txn| apply_ops(txn, ops)).map(|_| ())
    }

    fn compact(&mut self) -> Result<bool, DbError> {
        self.inner.compact()
    }

    fn check_integrity(&mut self) -> Result<bool, DbError> {
        self.inner.check_integrity()
    }

    fn stats(&self) -> Result<KvEngineStats, DbError> {
        self.inner.stats()
    }

    fn backup_source(&self) -> Result<Box<dyn BackupSource>, DbError> {
        self.inner.backup_source()
    }
}

type FaultyDb = EmbeddedDb<EntityStore<FaultyEngine>>;
type PlainDb = EmbeddedDb<EntityStore<RedbKvEngine>>;

const ITEMS: &str = "crash_items";
const ROWS: u64 = 16;

fn open_faulty(path: &Path, plan: &Arc<FaultPlan>) -> Result<FaultyDb, DbError> {
    let inner = RedbKvEngine::open(path, DbOpenMode::AutoCreate)?;
    EmbeddedDb::open(EntityStore::new(FaultyEngine {
        inner,
        plan: Arc::clone(plan),
    }))
}

fn open_plain(path: &Path) -> PlainDb {
    let engine = RedbKvEngine::open(path, DbOpenMode::OpenExisting).unwrap();
    EmbeddedDb::open(EntityStore::new(engine)).unwrap()
}

fn index(name: &str, field: &str, kind: IndexKind, unique: bool) -> DdlOperation {
    DdlOperation::UpsertIndex {
        name: name.into(),
        collection: ITEMS.into(),
        field: field.into(),
        unique,
        kind,
        extra_fields: Vec::new(),
        predicate: None,
        analyzer: Default::default(),
    }
}

fn schema() -> DdlBatch {
    DdlBatch::new()
        .with_op(DdlOperation::UpsertCollection {
            name: ITEMS.into(),
            kind: DdlCollectionKind::Polymorphic,
            integrity_mode: IntegrityMode::Permissive,
        })
        .with_op(index("crash_nick", "nick", IndexKind::Equality, true))
        .with_op(index("crash_n", "n", IndexKind::Range, false))
        .with_op(index("crash_body", "body", IndexKind::FullText, false))
}

/// A row whose unique `nick` is derived from its id and `version`, so any
/// batch of these rows is valid.
fn row(id: &str, version: u64) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(id.to_string()));
    row.insert("nick", Value::String(format!("{id}@{version}")));
    row.insert("n", Value::I64((version % 17) as i64));
    row.insert(
        "body",
        Value::String(["quick fox", "lazy dog", "red fox jumps"][(version % 3) as usize].into()),
    );
    row
}

type Rows = BTreeMap<String, Object>;

fn rows<S: semantic_db_core::embedded::EntityStorage>(db: &EmbeddedDb<S>) -> Rows {
    let lid = db.catalog().collection_by_name(ITEMS).unwrap().lid;
    db.collection_rows(lid)
        .unwrap()
        .into_iter()
        .map(|record| (record.id, record.object))
        .collect()
}

fn assert_verified<S: semantic_db_core::embedded::EntityStorage>(
    db: &mut EmbeddedDb<S>,
    context: &str,
) {
    let report = db
        .verify(&VerifyOptions::all())
        .unwrap_or_else(|err| panic!("{context}: verify: {err}"));
    assert!(
        report.is_ok(),
        "{context}: verify found problems:\n{report}"
    );
    assert!(
        report.checked.counters > 0,
        "{context}: stats are maintained"
    );
}

/// Like [`assert_verified`] after a crash: redb repairs its file after a
/// write transaction was dropped by a panic or a killed process, and the
/// first integrity check reports that repair. Every logical check must pass
/// right away and the storage must be intact once repaired.
fn assert_verified_after_crash<S: semantic_db_core::embedded::EntityStorage>(
    db: &mut EmbeddedDb<S>,
    context: &str,
) {
    let report = db
        .verify(&VerifyOptions::all())
        .unwrap_or_else(|err| panic!("{context}: verify: {err}"));
    let only_repaired = report.problems.iter().all(|problem| {
        problem.kind == semantic_db_core::VerifyProblemKind::StorageIntegrity
            && problem.detail.contains("repaired")
    });
    assert!(
        only_repaired && report.problem_count == report.problems.len() as u64,
        "{context}: verify found problems:\n{report}"
    );
    assert_verified(db, &format!("{context}, after the repair"));
}

/// Indexed queries agree with the rows.
fn assert_queries_match<S: semantic_db_core::embedded::EntityStorage>(
    db: &EmbeddedDb<S>,
    expected: &Rows,
    context: &str,
) {
    for (id, object) in expected {
        let query = SelectQuery::new()
            .with_collection(ITEMS)
            .with_predicate(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "nick",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(
                    object.get("nick").unwrap().clone(),
                ))),
            });
        let found = db.select(query).unwrap();
        assert_eq!(found.len(), 1, "{context}: lookup of {id}");
    }
    let by_n = db
        .select(
            SelectQuery::new()
                .with_collection(ITEMS)
                .with_predicate(Expr::Binary {
                    op: BinaryOp::Gte,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["n"])))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::I64(0)))),
                })
                .with_order_by(vec![OrderBy {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                    direction: SortDirection::Asc,
                }]),
        )
        .unwrap();
    assert_eq!(by_n.len(), expected.len(), "{context}: range scan");
}

/// A random batch over the rows and its effect on `before`.
fn random_batch(rng: &mut TestRng, before: &Rows, version: u64) -> (Batch, Rows) {
    let mut after = before.clone();
    let mut batch = Batch::new();
    for _ in 0..1 + rng.below(6) {
        let id = format!("r{}", rng.below(ROWS));
        if rng.one_in(3) {
            after.remove(&id);
            batch = batch.with_op(BatchOperation::DeleteById {
                collection: ITEMS.into(),
                id,
            });
        } else {
            let object = row(&id, version + rng.below(1_000) * 100);
            after.insert(id.clone(), object.clone());
            batch = batch.with_op(BatchOperation::Upsert {
                collection: ITEMS.into(),
                id,
                object,
            });
        }
    }
    (batch, after)
}

fn random_fault(rng: &mut TestRng) -> Fault {
    Fault {
        write: 0,
        point: match rng.below(3) {
            0 => FaultPoint::BeforeCommit,
            1 => FaultPoint::AfterWrites(1 + rng.below(12) as usize),
            _ => FaultPoint::AfterCommit,
        },
        panic: rng.one_in(2),
    }
}

/// Run `write` with `fault` armed; returns whether the fault struck.
fn with_fault<T>(plan: &FaultPlan, fault: Fault, write: impl FnOnce() -> T) -> bool {
    plan.arm(fault);
    let outcome = catch_unwind(AssertUnwindSafe(write));
    let triggered = plan.disarm();
    if outcome.is_err() {
        assert!(triggered && fault.panic, "unexpected panic");
    }
    triggered
}

#[test]
fn injected_write_failures_leave_all_or_nothing() {
    for seed in seeds_from_env(&[0xC4A5]) {
        let _guard = SeedGuard::new("injected write failures", seed);
        let mut rng = TestRng::new(seed);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("faults.redb");
        let plan = Arc::new(FaultPlan::default());
        let mut db = open_faulty(&path, &plan).unwrap();
        db.transact_ddl(schema()).unwrap();
        let mut expected = Rows::new();

        let mut struck = 0;
        for case in 0..25 {
            let context = format!("seed {seed} case {case}");
            let (batch, after) = random_batch(&mut rng, &expected, case);
            let fault = random_fault(&mut rng);
            let result = std::cell::RefCell::new(None);
            let triggered = with_fault(&plan, fault, || {
                *result.borrow_mut() = Some(db.execute_batch(batch.clone()).map(|_| ()));
            });
            let context = format!("{context} ({fault:?}, triggered: {triggered})");
            struck += usize::from(triggered);
            let committed = !triggered || fault.point == FaultPoint::AfterCommit;
            if !fault.panic {
                // The handle stays usable after a failed write.
                assert_eq!(
                    result.into_inner().unwrap().is_ok(),
                    !triggered,
                    "{context}: result"
                );
                let seen = rows(&db);
                assert!(
                    seen == if committed {
                        after.clone()
                    } else {
                        expected.clone()
                    },
                    "{context}: rows after the failure"
                );
                assert_verified(&mut db, &context);
            }
            // Reopen the file like after a crash.
            drop(db);
            let mut reopened = open_plain(&path);
            let seen = rows(&reopened);
            assert!(
                seen == after || seen == expected,
                "{context}: partial batch visible after reopen"
            );
            assert_eq!(seen == after, committed || after == expected, "{context}");
            if fault.panic {
                assert_verified_after_crash(&mut reopened, &format!("{context} after reopen"));
            } else {
                assert_verified(&mut reopened, &format!("{context} after reopen"));
            }
            assert_queries_match(&reopened, &seen, &context);
            expected = seen;
            drop(reopened);
            db = open_faulty(&path, &plan).unwrap();
        }
        assert!(struck > 12, "most faults must strike ({struck})");
    }
}

#[test]
fn injected_ddl_failures_leave_all_or_nothing() {
    let mut rng = TestRng::new(0xDD1);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ddl.redb");
    let plan = Arc::new(FaultPlan::default());
    let mut db = open_faulty(&path, &plan).unwrap();
    db.transact_ddl(schema()).unwrap();
    let mut batch = Batch::new();
    for index in 0..ROWS {
        let id = format!("r{index}");
        batch = batch.with_op(BatchOperation::Upsert {
            collection: ITEMS.into(),
            object: row(&id, index),
            id,
        });
    }
    db.execute_batch(batch).unwrap();
    let expected = rows(&db);

    for case in 0..12 {
        // Alternately add and drop an index whose build writes an entry
        // per row.
        let name = "crash_extra";
        let exists = db
            .catalog()
            .indexes()
            .any(|(_, index)| index.schema.name == name);
        let ddl = if exists {
            DdlBatch::new().with_op(DdlOperation::DeleteIndex {
                name: name.into(),
                collection: ITEMS.into(),
            })
        } else {
            DdlBatch::new().with_op(index(name, "body", IndexKind::Range, false))
        };
        let fault = random_fault(&mut rng);
        let context = format!("case {case} ({fault:?})");
        with_fault(&plan, fault, || {
            let _ = db.transact_ddl(ddl.clone());
        });
        drop(db);
        let mut reopened = open_plain(&path);
        assert_eq!(rows(&reopened), expected, "{context}");
        assert_verified_after_crash(&mut reopened, &context);
        assert_queries_match(&reopened, &expected, &context);
        drop(reopened);
        db = open_faulty(&path, &plan).unwrap();
    }
}

/// Write the legacy fixture (textual layout, no stats) into a new file.
fn write_legacy_database(path: &Path) {
    let mut engine = RedbKvEngine::open(path, DbOpenMode::AutoCreate).unwrap();
    let ops = crate::engine_tests::legacy_layout_entries()
        .into_iter()
        .map(|(key, value)| KvWriteOp::Put { key, value })
        .collect::<Vec<_>>();
    engine.write_batch(&ops).unwrap();
}

/// The legacy fixture's rows and index are intact and every migration is
/// complete.
fn assert_migrated(path: &Path, context: &str, crashed: bool) {
    let mut db = open_plain(path);
    let items = db.catalog().collection_by_name("items").unwrap().lid;
    let kinds = db
        .collection_rows(items)
        .unwrap()
        .into_iter()
        .map(|record| (record.id, record.object.get("kind").cloned()))
        .collect::<BTreeMap<_, _>>();
    let kind = |kind: &str| Some(Value::String(kind.into()));
    assert_eq!(
        kinds,
        BTreeMap::from([
            ("one".to_string(), kind("music")),
            ("three".to_string(), kind("music")),
            ("two".to_string(), kind("video")),
        ]),
        "{context}"
    );
    let music = db
        .select(
            SelectQuery::new()
                .with_collection("items")
                .with_predicate(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "kind",
                    ])))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "music".into(),
                    )))),
                }),
        )
        .unwrap();
    assert_eq!(music.len(), 2, "{context}");
    if crashed {
        assert_verified_after_crash(&mut db, context);
    } else {
        assert_verified(&mut db, context);
    }
    let (_, store) = db.into_parts();
    assert_eq!(
        store.layout_version().unwrap(),
        Some(semantic_db_kv::LAYOUT_VERSION_CURRENT),
        "{context}"
    );
    assert!(
        store.scan_raw_prefix(b"c/").unwrap().is_empty(),
        "{context}"
    );
    assert!(
        store.scan_raw_prefix(b"i/").unwrap().is_empty(),
        "{context}"
    );
}

#[test]
fn interrupted_open_migrations_complete_on_the_next_open() {
    let mut rng = TestRng::new(0x316);
    let plan = Arc::new(FaultPlan::default());
    // Fail every write of the first open in turn, at every point.
    let mut write = 0;
    loop {
        let mut any_struck = false;
        for (point, panic) in [
            (FaultPoint::BeforeCommit, false),
            (FaultPoint::AfterWrites(1 + rng.below(3) as usize), true),
            (FaultPoint::AfterCommit, rng.one_in(2)),
        ] {
            let fault = Fault {
                write,
                point,
                panic,
            };
            let context = format!("{fault:?}");
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("legacy.redb");
            write_legacy_database(&path);
            let triggered = with_fault(&plan, fault, || {
                if let Ok(db) = open_faulty(&path, &plan) {
                    drop(db);
                }
            });
            any_struck |= triggered;
            assert_migrated(&path, &context, panic);
            // Once migrated, opening only reads.
            let untouched = Fault {
                write: 0,
                point: FaultPoint::BeforeCommit,
                panic: false,
            };
            let wrote = with_fault(&plan, untouched, || {
                drop(open_faulty(&path, &plan).unwrap());
            });
            assert!(!wrote, "{context}: opening a migrated database wrote");
        }
        if !any_struck {
            break;
        }
        write += 1;
        assert!(write < 64, "the open keeps writing");
    }
    assert!(
        write >= 2,
        "the layout migration and the stats backfill write"
    );
}

const CHILD_PATH_ENV: &str = "SEMANTIC_CRASH_CHILD_DB";
const BATCH_ROWS: usize = 8;

/// Rows of batch `k` of the crash child.
fn batch_rows(k: u64) -> Vec<(String, Object)> {
    (0..BATCH_ROWS)
        .map(|i| {
            let id = format!("b{k}-{i}");
            let mut object = row(&id, k);
            object.insert("batch", Value::I64(k as i64));
            (id, object)
        })
        .collect()
}

/// Child half of [`killed_writer_keeps_every_reported_commit`]: writes
/// batches to the database at `$SEMANTIC_CRASH_CHILD_DB` until killed and
/// reports each commit on stdout after it returned.
#[test]
fn crash_child_writer() {
    let Ok(path) = std::env::var(CHILD_PATH_ENV) else {
        return;
    };
    let engine = RedbKvEngine::open_with_options(
        &path,
        DbOpenMode::OpenExisting,
        RedbOptions::default().with_durability(crate::RedbDurability::Immediate),
    )
    .unwrap();
    let mut db = EmbeddedDb::open(EntityStore::new(engine)).unwrap();
    let mut stdout = std::io::stdout();
    for k in 0..100_000u64 {
        let mut batch = Batch::new();
        for (id, object) in batch_rows(k) {
            batch = batch.with_op(BatchOperation::Upsert {
                collection: ITEMS.into(),
                id,
                object,
            });
        }
        if k >= 3 {
            for (id, _) in batch_rows(k - 3) {
                batch = batch.with_op(BatchOperation::DeleteById {
                    collection: ITEMS.into(),
                    id,
                });
            }
        }
        let mut head = Object::new();
        head.insert("id", Value::String("head".into()));
        head.insert("batch", Value::I64(k as i64));
        batch = batch.with_op(BatchOperation::Upsert {
            collection: ITEMS.into(),
            id: "head".into(),
            object: head,
        });
        db.execute_batch(batch).unwrap();
        writeln!(stdout, "committed {k}").unwrap();
        stdout.flush().unwrap();
    }
}

/// Spawn the crash child on `path`, kill it once it reported `commits`
/// commits (plus a random delay) and return the last reported commit.
fn run_and_kill(path: &Path, commits: u64, delay: Duration) -> Option<u64> {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_tests::crash_child_writer",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_PATH_ENV, path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(k) = line.strip_prefix("committed ") {
                let _ = sender.send(k.trim().parse::<u64>().unwrap());
            }
        }
    });
    let mut last = None;
    while last.is_none_or(|last| last + 1 < commits) {
        match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok(k) => last = Some(k),
            Err(err) => panic!("crash child stopped reporting: {err}"),
        }
    }
    std::thread::sleep(delay);
    // SIGKILL: no destructors, no flush.
    child.kill().unwrap();
    child.wait().unwrap();
    reader.join().unwrap();
    last.into_iter().chain(receiver.try_iter()).max()
}

#[test]
#[ignore = "expensive: spawns and kills child processes"]
fn killed_writer_keeps_every_reported_commit() {
    let iterations = env_usize("SEMANTIC_CRASH_ITERATIONS", 5);
    for seed in seeds_from_env(&[0x6B11]) {
        let _guard = SeedGuard::new("kill -9 crash test", seed);
        let mut rng = TestRng::new(seed);
        for iteration in 0..iterations {
            let dir = tempfile::tempdir().unwrap();
            let path: PathBuf = dir.path().join("killed.redb");
            let mut db = open_faulty(&path, &Arc::new(FaultPlan::default())).unwrap();
            db.transact_ddl(schema()).unwrap();
            drop(db);

            let commits = 1 + rng.below(40);
            let delay = Duration::from_micros(rng.below(20_000));
            let reported = run_and_kill(&path, commits, delay).expect("child committed");
            let context = format!("iteration {iteration}: reported {reported}");

            let mut db = open_plain(&path);
            let rows = rows(&db);
            let head = rows["head"].get("batch").cloned();
            let Some(Value::I64(head)) = head else {
                panic!("{context}: head row missing: {head:?}");
            };
            let head = head as u64;
            eprintln!("kill -9 {context}: durable head batch {head}");
            // The commit in flight when the child died may or may not have
            // become durable; everything reported did.
            assert!(
                head == reported || head == reported + 1,
                "{context}: head is at batch {head}"
            );
            let mut batches = BTreeMap::<u64, usize>::new();
            for (id, object) in &rows {
                if id == "head" {
                    continue;
                }
                let Some(Value::I64(batch)) = object.get("batch") else {
                    panic!("{context}: row {id} without batch");
                };
                *batches.entry(*batch as u64).or_default() += 1;
            }
            let expected = (head.saturating_sub(2)..=head)
                .map(|k| (k, BATCH_ROWS))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(batches, expected, "{context}: partial or lost batches");
            assert_verified_after_crash(&mut db, &context);
            assert_queries_match(
                &db,
                &rows.into_iter().filter(|(id, _)| id != "head").collect(),
                &context,
            );
        }
    }
}
