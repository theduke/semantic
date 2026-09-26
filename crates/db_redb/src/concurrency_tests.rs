//! Readers run on snapshots outside the backend's write lock: long selects
//! and exports neither block writers nor observe their commits.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use futures::StreamExt as _;
use semantic_data::schema::DbOpenMode;
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::CollectionKind;
use semantic_db_core::embedded::{EmbeddedBackend, EmbeddedDb, StorageCommitOutcome};
use semantic_db_core::{
    Backend, Batch, BatchOperation, DbError, Query, SelectQuery, TextQueryInput, TokioAsyncRuntime,
};
use semantic_db_kv::{
    BoxKvPrefixScan, EntityStore, KvEngine, KvReadTxn, KvWriteOp, KvWriteTxn, MemoryKvEngine,
};

use crate::RedbKvEngine;

const TIMEOUT: Duration = Duration::from_secs(5);

/// Per-row delay and yielded-row counter for scans of owned read handles
/// (the handles readers use).
#[derive(Debug, Default)]
struct ScanControl {
    delay_micros: AtomicU64,
    /// Delay of point reads, which optimistic writes prepare with.
    get_delay_micros: AtomicU64,
    rows: AtomicUsize,
}

impl ScanControl {
    fn set_delay(&self, delay: Duration) {
        self.delay_micros
            .store(delay.as_micros() as u64, Ordering::SeqCst);
    }

    fn set_get_delay(&self, delay: Duration) {
        self.get_delay_micros
            .store(delay.as_micros() as u64, Ordering::SeqCst);
    }

    fn rows(&self) -> usize {
        self.rows.load(Ordering::SeqCst)
    }

    fn slow_scan(self: &Arc<Self>, scan: BoxKvPrefixScan) -> BoxKvPrefixScan {
        let control = Arc::clone(self);
        Box::new(scan.inspect(move |_| {
            control.rows.fetch_add(1, Ordering::SeqCst);
            let delay = control.delay_micros.load(Ordering::SeqCst);
            if delay > 0 {
                std::thread::sleep(Duration::from_micros(delay));
            }
        }))
    }
}

/// Engine wrapper whose owned read handles scan slowly.
#[derive(Debug)]
struct SlowScans<E> {
    inner: E,
    control: Arc<ScanControl>,
}

struct SlowReadTxn {
    inner: Box<dyn KvReadTxn>,
    control: Arc<ScanControl>,
}

impl KvReadTxn for SlowReadTxn {
    fn revision(&self) -> Option<u64> {
        self.inner.revision()
    }

    fn is_snapshot(&self) -> bool {
        self.inner.is_snapshot()
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        let delay = self.control.get_delay_micros.load(Ordering::SeqCst);
        if delay > 0 {
            std::thread::sleep(Duration::from_micros(delay));
        }
        self.inner.get(key)
    }

    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        Ok(self
            .control
            .slow_scan(self.inner.scan_range_stream(start, end)?))
    }

    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<BoxKvPrefixScan, DbError> {
        Ok(self
            .control
            .slow_scan(self.inner.scan_prefix_stream(prefix)?))
    }
}

impl<E: KvEngine> KvEngine for SlowScans<E> {
    type PrefixScan = E::PrefixScan;

    fn begin_read(&self) -> Result<Box<dyn KvReadTxn + '_>, DbError> {
        self.inner.begin_read()
    }

    fn begin_read_owned(&self) -> Result<Option<Box<dyn KvReadTxn>>, DbError> {
        Ok(self.inner.begin_read_owned()?.map(|inner| {
            Box::new(SlowReadTxn {
                inner,
                control: Arc::clone(&self.control),
            }) as Box<dyn KvReadTxn>
        }))
    }

    fn write_with<F>(
        &mut self,
        expected_revision: Option<u64>,
        f: F,
    ) -> Result<StorageCommitOutcome, DbError>
    where
        F: FnOnce(&mut dyn KvWriteTxn) -> Result<(), DbError>,
    {
        self.inner.write_with(expected_revision, f)
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
        self.inner.put(key, value)
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.inner.delete(key)
    }

    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<Self::PrefixScan, DbError> {
        self.inner.scan_prefix_stream(prefix)
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.inner.scan_prefix(prefix)
    }

    fn tx_capabilities(&self) -> semantic_db_core::embedded::StorageTransactionCapabilities {
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
        self.inner.write_batch_conditional(ops, expected_revision)
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> Result<(), DbError> {
        self.inner.write_batch(ops)
    }
}

type TestBackend<E> = EmbeddedBackend<EntityStore<SlowScans<E>>>;

/// Keeps the backing directory of a redb engine alive.
struct Fixture<E: KvEngine> {
    backend: Arc<TestBackend<E>>,
    control: Arc<ScanControl>,
    _dir: Option<tempfile::TempDir>,
}

fn fixture<E: KvEngine>(engine: E, dir: Option<tempfile::TempDir>) -> Fixture<E> {
    let control = Arc::new(ScanControl::default());
    let engine = SlowScans {
        inner: engine,
        control: Arc::clone(&control),
    };
    let mut db = EmbeddedDb::open(EntityStore::new(engine)).unwrap();
    db.create_collection("items", CollectionKind::Untyped)
        .unwrap();
    Fixture {
        backend: Arc::new(EmbeddedBackend::with_runtime(
            db,
            Arc::new(TokioAsyncRuntime),
        )),
        control,
        _dir: dir,
    }
}

fn memory() -> Fixture<MemoryKvEngine> {
    fixture(MemoryKvEngine::new(), None)
}

fn redb() -> Fixture<RedbKvEngine> {
    let dir = tempfile::tempdir().unwrap();
    let engine = RedbKvEngine::open(dir.path().join("db"), DbOpenMode::AutoCreate).unwrap();
    fixture(engine, Some(dir))
}

fn item(id: &str) -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String(id.to_string()));
    object.insert("value", Value::String(format!("value of {id}")));
    object
}

fn item_id(index: usize) -> String {
    format!("item-{index:05}")
}

async fn seed(backend: &impl Backend, count: usize) {
    let mut batch = Batch::new();
    for index in 0..count {
        let id = item_id(index);
        batch = batch.with_op(BatchOperation::Upsert {
            collection: "items".to_string(),
            object: item(&id),
            id,
        });
    }
    backend.execute_batch(batch).await.unwrap();
}

async fn select_items(backend: &impl Backend) -> Result<Vec<Object>, DbError> {
    let query = Query::Select(SelectQuery::new().with_collection("items"));
    match backend.query(TextQueryInput::Ast(query)).await? {
        semantic_db_core::QueryResult::Select(rows) => Ok(rows),
        other => panic!("unexpected query result {other:?}"),
    }
}

async fn writes_do_not_wait_for_slow_selects<E: KvEngine>(fixture: Fixture<E>) {
    const ROWS: usize = 5_000;
    let Fixture {
        backend, control, ..
    } = fixture;
    seed(&*backend, ROWS).await;
    control.set_delay(Duration::from_millis(2));
    let rows_before = control.rows();

    let select = tokio::spawn({
        let backend = Arc::clone(&backend);
        async move { select_items(&*backend).await }
    });
    tokio::time::timeout(TIMEOUT, async {
        while control.rows() == rows_before {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("select started scanning");

    tokio::time::timeout(
        TIMEOUT,
        backend.insert("items".into(), "late".into(), item("late")),
    )
    .await
    .expect("insert completed while the select was scanning")
    .unwrap();
    assert!(
        !select.is_finished(),
        "the select finished before the concurrent insert"
    );

    control.set_delay(Duration::ZERO);
    let rows = select.await.unwrap().unwrap();
    assert_eq!(rows.len(), ROWS, "the select observes its snapshot only");
    assert_eq!(select_items(&*backend).await.unwrap().len(), ROWS + 1);
}

async fn exports_neither_block_nor_observe_writes<E: KvEngine>(fixture: Fixture<E>) {
    const ROWS: usize = 50;
    let backend = fixture.backend;
    seed(&*backend, ROWS).await;

    let mut export = backend.scan_entities().await.unwrap();
    let first = export.next().await.unwrap().unwrap();
    // The export thread is now parked on its bounded channel.
    tokio::time::timeout(
        TIMEOUT,
        backend.insert("items".into(), "late".into(), item("late")),
    )
    .await
    .expect("insert completed while the export was paused")
    .unwrap();

    let mut exported = vec![(first.collection, first.id)];
    while let Some(record) = export.next().await {
        let record = record.unwrap();
        exported.push((record.collection, record.id));
    }
    let expected = (0..ROWS)
        .map(|index| ("items".to_string(), item_id(index)))
        .collect::<BTreeSet<_>>();
    assert_eq!(exported.len(), ROWS, "no row exported twice");
    assert_eq!(exported.into_iter().collect::<BTreeSet<_>>(), expected);
    assert!(
        backend
            .get("items".into(), "late".into())
            .await
            .unwrap()
            .is_some()
    );
}

async fn reads_started_after_a_commit_observe_it<E: KvEngine>(fixture: Fixture<E>) {
    let backend = fixture.backend;
    seed(&*backend, 3).await;
    backend
        .insert("items".into(), "late".into(), item("late"))
        .await
        .unwrap();
    let rows = select_items(&*backend).await.unwrap();
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter()
            .any(|row| row.get("id") == Some(&Value::String("late".into())))
    );
    assert!(
        backend
            .get("items".into(), "late".into())
            .await
            .unwrap()
            .is_some()
    );
}

async fn concurrent_writers_all_commit<E: KvEngine>(fixture: Fixture<E>) {
    const PER_WRITER: usize = 200;
    let backend = fixture.backend;
    seed(&*backend, 1).await;
    // Writes prepare on snapshots without the lock; slow point reads make
    // the two writers' preparations overlap, so their commits conflict.
    fixture.control.set_get_delay(Duration::from_micros(200));
    let writers = (0..2)
        .map(|writer| {
            let backend = Arc::clone(&backend);
            tokio::spawn(async move {
                let mut metrics = semantic_db_core::WriteMetrics::default();
                for index in 0..PER_WRITER {
                    let id = format!("writer-{writer}-{index:03}");
                    let reply = backend
                        .execute_batch_returning(
                            Batch::new().with_op(BatchOperation::Upsert {
                                collection: "items".into(),
                                object: item(&id),
                                id,
                            }),
                            semantic_db_core::BatchReturn::Stats,
                        )
                        .await
                        .unwrap();
                    metrics.attempts += reply.metrics().attempts;
                    metrics.conflicts += reply.metrics().conflicts;
                }
                metrics
            })
        })
        .collect::<Vec<_>>();
    let reader = tokio::spawn({
        let backend = Arc::clone(&backend);
        async move {
            // Concurrent reads observe monotonically growing snapshots.
            let mut last = 0;
            for _ in 0..20 {
                let rows = select_items(&*backend).await.unwrap().len();
                assert!(rows >= last);
                last = rows;
            }
        }
    });
    for writer in writers {
        let metrics = writer.await.unwrap();
        // Every conflict was retried: one more attempt per conflict.
        assert_eq!(
            metrics.attempts,
            PER_WRITER as u64 + metrics.conflicts,
            "{metrics:?}"
        );
        assert!(
            metrics.conflicts > 0,
            "overlapping writers never conflicted"
        );
    }
    fixture.control.set_get_delay(Duration::ZERO);
    reader.await.unwrap();
    assert_eq!(
        select_items(&*backend).await.unwrap().len(),
        1 + 2 * PER_WRITER
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writes_do_not_wait_for_slow_selects_memory() {
    writes_do_not_wait_for_slow_selects(memory()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writes_do_not_wait_for_slow_selects_redb() {
    writes_do_not_wait_for_slow_selects(redb()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exports_neither_block_nor_observe_writes_memory() {
    exports_neither_block_nor_observe_writes(memory()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exports_neither_block_nor_observe_writes_redb() {
    exports_neither_block_nor_observe_writes(redb()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reads_started_after_a_commit_observe_it_memory() {
    reads_started_after_a_commit_observe_it(memory()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reads_started_after_a_commit_observe_it_redb() {
    reads_started_after_a_commit_observe_it(redb()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_all_commit_memory() {
    concurrent_writers_all_commit(memory()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_all_commit_redb() {
    concurrent_writers_all_commit(redb()).await;
}
