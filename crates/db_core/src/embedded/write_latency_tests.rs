//! Optimistic data writes of [`EmbeddedBackend`]: writes are prepared on a
//! snapshot without the database lock, so readers take snapshots while a
//! write is in flight, and concurrent commits surface as retried conflicts.
//!
//! The tests are deterministic: a storage wrapper blocks the first read of a
//! write's snapshot until the test releases it, so the test acts while the
//! write is being prepared.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use semantic_data::value::{FieldPath, Object, Value};

use crate::catalog::{CollectionKind, LocalCollectionId, LocalIndexId};
use crate::embedded::{
    BoxEntityIdScan, BoxEntityScan, BoxIndexEntryScan, EmbeddedBackend, EmbeddedDb,
    EntityReadSnapshot, EntityStorage, MemoryEntityStorage, StorageCommitOutcome,
    StorageTransactionCapabilities, StorageWriteOp, StoredEntity,
};
use crate::{AsyncRuntime, Backend, Batch, BatchOperation, BatchReply, BatchReturn, DbError};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Blocks the first snapshot read after [`Gate::arm`] until released.
#[derive(Debug)]
struct Gate {
    armed: AtomicBool,
    entered: Mutex<mpsc::Sender<()>>,
    release: Mutex<mpsc::Receiver<()>>,
}

/// The test side of a [`Gate`].
struct GateControl {
    gate: Arc<Gate>,
    entered: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
}

impl GateControl {
    fn new() -> Self {
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        Self {
            gate: Arc::new(Gate {
                armed: AtomicBool::new(false),
                entered: Mutex::new(entered_tx),
                release: Mutex::new(release_rx),
            }),
            entered,
            release,
        }
    }

    fn arm(&self) {
        self.gate.armed.store(true, Ordering::SeqCst);
    }

    /// Wait until a write blocks in its snapshot read.
    fn wait_entered(&self) {
        self.entered
            .recv_timeout(TIMEOUT)
            .expect("the write never read its snapshot");
    }

    fn release(&self) {
        self.release.send(()).unwrap();
    }
}

impl Gate {
    fn pass(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.lock().unwrap().send(()).unwrap();
            let _ = self.release.lock().unwrap().recv_timeout(TIMEOUT);
        }
    }
}

/// Memory storage whose owned snapshots (the snapshots writes are prepared
/// on) pass the gate before every row read.
#[derive(Debug)]
struct GatedStorage {
    inner: MemoryEntityStorage,
    gate: Arc<Gate>,
}

struct GatedSnapshot {
    inner: Arc<dyn EntityReadSnapshot>,
    gate: Arc<Gate>,
}

impl EntityReadSnapshot for GatedSnapshot {
    fn revision(&self) -> Result<Option<u64>, DbError> {
        self.inner.revision()
    }

    fn is_consistent(&self) -> bool {
        self.inner.is_consistent()
    }

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        self.gate.pass();
        self.inner.get_entity(collection, id)
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        self.inner.scan_collection_stream(collection)
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        self.inner.scan_index_value_stream(index, path, value)
    }

    fn scan_index_entries(
        &self,
        index: LocalIndexId,
        scan: &crate::IndexScan,
    ) -> Result<BoxIndexEntryScan, DbError> {
        self.inner.scan_index_entries(index, scan)
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        self.inner.index_needs_rebuild(index)
    }
}

impl EntityStorage for GatedStorage {
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

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        self.inner.index_needs_rebuild(index)
    }

    fn owned_snapshot(&self) -> Result<Option<Arc<dyn EntityReadSnapshot>>, DbError> {
        Ok(self.inner.owned_snapshot()?.map(|inner| {
            Arc::new(GatedSnapshot {
                inner,
                gate: Arc::clone(&self.gate),
            }) as Arc<dyn EntityReadSnapshot>
        }))
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.inner.tx_capabilities()
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

type BlockingOutput = Box<dyn std::any::Any + Send>;

/// Runs blocking work on a fresh thread.
struct ThreadRuntime;

impl AsyncRuntime for ThreadRuntime {
    fn spawn_blocking_erased(
        &self,
        op: Box<dyn FnOnce() -> Result<BlockingOutput, DbError> + Send>,
    ) -> BoxFuture<'static, Result<BlockingOutput, DbError>> {
        let (sender, receiver) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = sender.send(op());
        });
        Box::pin(async move {
            receiver
                .await
                .map_err(|_| DbError::Storage("blocking task dropped".to_string().into()))?
        })
    }
}

fn item(id: &str, code: &str) -> Object {
    Object::from_iter([
        ("id".to_string(), Value::String(id.into())),
        ("code".to_string(), Value::String(code.into())),
    ])
}

fn upsert(id: &str, code: &str) -> Batch {
    Batch::new().with_op(BatchOperation::Upsert {
        collection: "items".into(),
        id: id.into(),
        object: item(id, code),
    })
}

/// A backend over gated storage holding `items` (unique index on `code`)
/// with row `a`.
fn gated_backend() -> (Arc<EmbeddedBackend<GatedStorage>>, GateControl) {
    let control = GateControl::new();
    let mut db = EmbeddedDb::new(GatedStorage {
        inner: MemoryEntityStorage::new(),
        gate: Arc::clone(&control.gate),
    });
    let items = db
        .create_collection("items", CollectionKind::Polymorphic)
        .unwrap();
    db.create_index("items_code", items, "code", true).unwrap();
    db.insert("items", "a", item("a", "code-a")).unwrap();
    let backend = EmbeddedBackend::with_runtime(db, Arc::new(ThreadRuntime));
    (Arc::new(backend), control)
}

/// Run `write` on its own thread, returning its result channel.
fn spawn_write<T: Send + 'static>(
    backend: &Arc<EmbeddedBackend<GatedStorage>>,
    write: impl FnOnce(Arc<EmbeddedBackend<GatedStorage>>) -> BoxFuture<'static, T> + Send + 'static,
) -> mpsc::Receiver<T> {
    let backend = Arc::clone(backend);
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(futures::executor::block_on(write(backend)));
    });
    receiver
}

fn stats_write(
    batch: Batch,
) -> impl FnOnce(Arc<EmbeddedBackend<GatedStorage>>) -> BoxFuture<'static, Result<BatchReply, DbError>>
{
    move |backend| {
        Box::pin(async move {
            backend
                .execute_batch_returning(batch, BatchReturn::Stats)
                .await
        })
    }
}

fn get(backend: &EmbeddedBackend<GatedStorage>, id: &str) -> Option<Object> {
    futures::executor::block_on(backend.get("items".into(), id.into()))
        .unwrap()
        .map(|record| record.object)
}

#[test]
fn readers_take_snapshots_while_a_write_is_prepared() {
    let (backend, gate) = gated_backend();
    gate.arm();
    let write = spawn_write(&backend, stats_write(upsert("b", "code-b")));
    gate.wait_entered();

    // The write is preparing; a reader takes its snapshot and completes
    // without waiting for it (a write holding the lock would block it).
    let read = spawn_write(&backend, |backend| {
        Box::pin(async move {
            (
                backend.get("items".into(), "a".into()).await,
                backend.get("items".into(), "b".into()).await,
            )
        })
    });
    let result = read.recv_timeout(Duration::from_secs(5));
    gate.release();
    let (a, b) = result.expect("a reader waited for an in-flight write");
    assert!(a.unwrap().is_some());
    assert!(b.unwrap().is_none(), "uncommitted writes are invisible");

    let reply = write.recv_timeout(TIMEOUT).unwrap().unwrap();
    assert_eq!(reply.metrics().attempts, 1);
    assert_eq!(reply.metrics().conflicts, 0);
    assert!(get(&backend, "b").is_some());
}

#[test]
fn commits_during_a_prepared_write_are_conflicts_that_retry() {
    let (backend, gate) = gated_backend();
    gate.arm();
    let write = spawn_write(&backend, stats_write(upsert("b", "code-b")));
    gate.wait_entered();
    // Another writer commits while the first one prepares.
    futures::executor::block_on(backend.insert("items".into(), "c".into(), item("c", "code-c")))
        .unwrap();
    gate.release();

    let reply = write.recv_timeout(TIMEOUT).unwrap().unwrap();
    assert_eq!(reply.metrics().attempts, 2, "{:?}", reply.metrics());
    assert_eq!(reply.metrics().conflicts, 1);
    assert!(get(&backend, "b").is_some());
    assert!(get(&backend, "c").is_some());
}

#[test]
fn catalog_changes_during_a_prepared_write_are_conflicts_that_retry() {
    let (backend, gate) = gated_backend();
    gate.arm();
    let write = spawn_write(&backend, stats_write(upsert("b", "code-b")));
    gate.wait_entered();
    // DDL holds the lock for its whole duration and moves the catalog.
    futures::executor::block_on(backend.create_collection("other".into(), CollectionKind::Untyped))
        .unwrap();
    gate.release();

    let reply = write.recv_timeout(TIMEOUT).unwrap().unwrap();
    assert_eq!(reply.metrics().conflicts, 1, "{:?}", reply.metrics());
    assert!(get(&backend, "b").is_some());
}

#[test]
fn uniqueness_checked_on_a_stale_snapshot_is_rechecked_by_the_retry() {
    let (backend, gate) = gated_backend();
    gate.arm();
    let write = spawn_write(&backend, stats_write(upsert("b", "shared")));
    gate.wait_entered();
    // A concurrent commit takes the unique value first. The blocked write
    // validates against its snapshot, where the value is free; its commit
    // conflicts, and the retry sees the value taken.
    futures::executor::block_on(backend.insert("items".into(), "c".into(), item("c", "shared")))
        .unwrap();
    gate.release();

    let error = write.recv_timeout(TIMEOUT).unwrap().unwrap_err();
    assert!(
        matches!(error, DbError::UniqueViolation { .. }),
        "{error:?}"
    );
    assert!(get(&backend, "b").is_none());
    assert_eq!(
        get(&backend, "c").unwrap().get("code"),
        Some(&Value::String("shared".into()))
    );
}
