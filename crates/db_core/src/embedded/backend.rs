use std::sync::{Arc, Mutex, RwLock};

use crate::catalog::{Catalog, CollectionKind, LocalCollectionId, LocalIndexId, SharedCatalog};
use crate::{
    AsyncRuntime, Backend, Batch, BatchOutcome, BatchStats, DbError, DdlBatch, DdlOutcome,
    DeleteQuery, DeleteResult, EntityRecord, MutationStats, PackageRegistrationOutcome, Query,
    QueryExplain, QueryPlan, QueryResult, SavepointId, SelectQuery, StorageErrorKind,
    TextQueryInput, TransactionCommit, TransactionHandle, TransactionOptions, UpdateQuery,
    UpdateResult, spawn_blocking_on,
};
use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::{SinkExt as _, StreamExt as _};
use semantic_data::schema::{Package, RelationType};
use semantic_data::value::{FieldPath, Object, Value};

use crate::embedded::{
    BoxEntityIdScan, BoxEntityScan, BoxIndexEntryScan, DbReader, EmbeddedDb, EmbeddedTransaction,
    EntityReadSnapshot, EntityStorage, StoredEntity,
};

/// [`Backend`] over an [`EmbeddedDb`] that runs blocking work on an
/// [`AsyncRuntime`].
///
/// Writes are serialized by the write side of a lock around the database.
/// Reads hold the read side only while creating a [`DbReader`] over the
/// current committed state and then run without the lock, so long reads and
/// exports neither block writers nor observe their commits. Storages without
/// owned snapshots ([`EntityStorage::owned_snapshot`]) fall back to running
/// the whole read under the lock. Catalog reads use the shared catalog and
/// never take the lock.
pub struct EmbeddedBackend<S: EntityStorage> {
    db: Arc<RwLock<EmbeddedDb<S>>>,
    catalog: SharedCatalog,
    /// Shared with the database, so subscribing never takes the lock.
    change_feed: crate::ChangeFeed,
    runtime: Arc<dyn AsyncRuntime>,
}

impl<S: EntityStorage> EmbeddedBackend<S> {
    pub fn new(db: EmbeddedDb<S>) -> Self {
        Self::with_runtime(db, default_runtime())
    }

    pub fn with_runtime(db: EmbeddedDb<S>, runtime: Arc<dyn AsyncRuntime>) -> Self {
        Self {
            catalog: db.shared_catalog().clone(),
            change_feed: db.change_feed().clone(),
            db: Arc::new(RwLock::new(db)),
            runtime,
        }
    }

    /// The database's change feed.
    pub fn change_feed(&self) -> &crate::ChangeFeed {
        &self.change_feed
    }

    /// Run `op` on a reader over the current committed state.
    fn read<R, F>(&self, op: F) -> BoxFuture<'static, Result<R, DbError>>
    where
        R: Send + 'static,
        F: FnOnce(&DbReader<'_>) -> Result<R, DbError> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let reader = {
                let db = db.read().map_err(|_| lock_poisoned_error())?;
                match db.owned_reader()? {
                    Some(reader) => reader,
                    // Borrowed snapshots keep the lock for the whole read.
                    None => return op(&db.reader()?),
                }
            };
            op(&reader)
        })
    }

    async fn resolve_query(&self, query: TextQueryInput) -> Result<Query, DbError> {
        match query {
            TextQueryInput::Ast(query) => Ok(query),
            TextQueryInput::Text {
                format,
                query,
                params,
            } => {
                self.parse_text_query_with_params(format, &query, &params)
                    .await
            }
        }
    }

    /// Run `op` with exclusive access to the database.
    fn write<R, F>(&self, op: F) -> BoxFuture<'static, Result<R, DbError>>
    where
        R: Send + 'static,
        F: FnOnce(&mut EmbeddedDb<S>) -> Result<R, DbError> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            op(&mut db)
        })
    }
}

#[cfg(feature = "tokio")]
fn default_runtime() -> Arc<dyn AsyncRuntime> {
    Arc::new(crate::TokioAsyncRuntime)
}

#[cfg(not(feature = "tokio"))]
fn default_runtime() -> Arc<dyn AsyncRuntime> {
    Arc::new(crate::InlineAsyncRuntime)
}

/// [`TransactionHandle`] of an [`EmbeddedBackend`].
///
/// Statements run on the transaction's own snapshot without the database
/// lock; only the commit takes the write side.
struct EmbeddedTransactionHandle<S: EntityStorage> {
    db: Arc<RwLock<EmbeddedDb<S>>>,
    runtime: Arc<dyn AsyncRuntime>,
    options: TransactionOptions,
    /// `None` once the transaction was committed.
    tx: Arc<Mutex<Option<EmbeddedTransaction>>>,
}

impl<S: EntityStorage> EmbeddedTransactionHandle<S> {
    /// Run `op` on the transaction.
    fn run<R, F>(&self, op: F) -> BoxFuture<'static, Result<R, DbError>>
    where
        R: Send + 'static,
        F: FnOnce(&mut EmbeddedTransaction) -> Result<R, DbError> + Send + 'static,
    {
        let tx = Arc::clone(&self.tx);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut tx = tx.lock().map_err(|_| lock_poisoned_error())?;
            op(tx.as_mut().ok_or_else(transaction_finished_error)?)
        })
    }
}

#[async_trait]
impl<S: EntityStorage> TransactionHandle for EmbeddedTransactionHandle<S> {
    fn options(&self) -> TransactionOptions {
        self.options
    }

    async fn get(&self, collection: String, id: String) -> Result<Option<EntityRecord>, DbError> {
        self.run(move |tx| tx.get(&collection, &id)).await
    }

    async fn select(&self, query: SelectQuery) -> Result<Vec<Object>, DbError> {
        self.run(move |tx| tx.select(query)).await
    }

    async fn upsert(&self, collection: String, id: String, object: Object) -> Result<(), DbError> {
        self.run(move |tx| tx.upsert(&collection, id, object)).await
    }

    async fn create(&self, collection: String, id: String, object: Object) -> Result<(), DbError> {
        self.run(move |tx| tx.create(&collection, id, object)).await
    }

    async fn delete(&self, collection: String, id: String) -> Result<bool, DbError> {
        self.run(move |tx| tx.delete(&collection, &id)).await
    }

    async fn update_where(&self, query: UpdateQuery) -> Result<UpdateResult, DbError> {
        self.run(move |tx| tx.update_where(query)).await
    }

    async fn delete_where(&self, query: DeleteQuery) -> Result<DeleteResult, DbError> {
        self.run(move |tx| tx.delete_where(query)).await
    }

    async fn execute_batch(&self, batch: Batch) -> Result<BatchStats, DbError> {
        self.run(move |tx| tx.execute_batch(batch)).await
    }

    async fn savepoint(&self) -> Result<SavepointId, DbError> {
        self.run(|tx| Ok(tx.savepoint())).await
    }

    async fn rollback_to(&self, savepoint: SavepointId) -> Result<(), DbError> {
        self.run(move |tx| tx.rollback_to(savepoint)).await
    }

    async fn commit(self: Box<Self>) -> Result<TransactionCommit, DbError> {
        let tx = Arc::clone(&self.tx);
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let tx = tx
                .lock()
                .map_err(|_| lock_poisoned_error())?
                .take()
                .ok_or_else(transaction_finished_error)?;
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.commit_transaction(tx)
        })
        .await
    }

    async fn rollback(self: Box<Self>) -> Result<(), DbError> {
        Ok(())
    }
}

fn transaction_finished_error() -> DbError {
    DbError::InvalidQuery("transaction already finished".into())
}

/// Reads of an interactive transaction on a storage without owned
/// snapshots: every read takes the read side of the database lock and reads
/// the latest committed state.
///
/// Not consistent, so the transaction fences each read by revision and a
/// concurrent commit surfaces as a transaction conflict. Scans are collected
/// while the lock is held.
struct LockedStorageReads<S: EntityStorage> {
    db: Arc<RwLock<EmbeddedDb<S>>>,
}

impl<S: EntityStorage> LockedStorageReads<S> {
    fn read<T>(&self, read: impl FnOnce(&S) -> Result<T, DbError>) -> Result<T, DbError> {
        let db = self.db.read().map_err(|_| lock_poisoned_error())?;
        read(db.storage())
    }
}

impl<S: EntityStorage> EntityReadSnapshot for LockedStorageReads<S> {
    fn revision(&self) -> Result<Option<u64>, DbError> {
        self.read(|storage| storage.current_revision())
    }

    fn is_consistent(&self) -> bool {
        false
    }

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        self.read(|storage| storage.get_entity(collection, id))
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        let rows = self.read(|storage| storage.scan_collection(collection))?;
        Ok(Box::new(rows.into_iter().map(Ok)))
    }

    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        self.read(|storage| storage.collection_row_count(collection))
    }

    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        self.read(|storage| storage.index_entry_count(index))
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        let ids = self.read(|storage| storage.scan_index_value(index, path, value))?;
        Ok(Box::new(ids.into_iter().map(Ok)))
    }

    fn scan_index_entries(
        &self,
        index: LocalIndexId,
        scan: &crate::IndexScan,
    ) -> Result<BoxIndexEntryScan, DbError> {
        let entries = self.read(|storage| {
            storage
                .scan_index_entries(index, scan)?
                .collect::<Result<Vec<_>, _>>()
        })?;
        Ok(Box::new(entries.into_iter().map(Ok)))
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        self.read(|storage| storage.index_needs_rebuild(index))
    }
}

fn lock_poisoned_error() -> DbError {
    DbError::storage(
        StorageErrorKind::InvalidState,
        "embedded backend rwlock poisoned",
    )
}

#[async_trait]
impl<S: EntityStorage> Backend for EmbeddedBackend<S> {
    fn subscribe_changes(
        &self,
        options: crate::ChangeSubscriptionOptions,
    ) -> Result<crate::ChangeStream, DbError> {
        Ok(Box::pin(self.change_feed.subscribe(options)))
    }

    async fn begin_transaction(
        &self,
        options: TransactionOptions,
    ) -> Result<Box<dyn TransactionHandle>, DbError> {
        let db = Arc::clone(&self.db);
        let tx = spawn_blocking_on(self.runtime.as_ref(), move || {
            let guard = db.read().map_err(|_| lock_poisoned_error())?;
            match guard.storage().owned_snapshot()? {
                Some(snapshot) => guard.begin_with_snapshot(options, snapshot),
                // Without owned snapshots, statements read the latest state
                // under the lock, fenced by the transaction's revision.
                None => guard.begin_with_snapshot(
                    options,
                    Arc::new(LockedStorageReads {
                        db: Arc::clone(&db),
                    }),
                ),
            }
        })
        .await?;
        Ok(Box::new(EmbeddedTransactionHandle {
            db: Arc::clone(&self.db),
            runtime: Arc::clone(&self.runtime),
            options,
            tx: Arc::new(Mutex::new(Some(tx))),
        }))
    }

    async fn validation_preflight(&self) -> Result<Vec<crate::ValidationViolation>, DbError> {
        self.read(|reader| reader.validation_preflight()).await
    }

    async fn activate_validation(&self) -> Result<(), DbError> {
        self.write(|db| db.activate_validation()).await
    }

    async fn catalog(&self) -> Result<Arc<Catalog>, DbError> {
        Ok(self.catalog.catalog_arc())
    }

    async fn scan_entities(&self) -> Result<crate::EntityStream, DbError> {
        const CHANNEL_CAPACITY: usize = 16;
        let db = Arc::clone(&self.db);
        let (mut sender, receiver) = futures::channel::mpsc::channel(CHANNEL_CAPACITY);
        let (ready, snapshot_taken) = futures::channel::oneshot::channel::<()>();
        std::thread::Builder::new()
            .name("semantic-entity-export".into())
            .spawn(move || {
                let mut send = |item| futures::executor::block_on(sender.send(item)).is_ok();
                let db = match db.read() {
                    Ok(db) => db,
                    Err(_) => {
                        let _ = ready.send(());
                        send(Err(lock_poisoned_error()));
                        return;
                    }
                };
                let reader = db.owned_reader();
                let _ = ready.send(());
                match reader {
                    // The owned snapshot keeps the export consistent without
                    // holding the lock while the consumer drains it.
                    Ok(Some(reader)) => {
                        drop(db);
                        reader.scan_entities_with(send);
                    }
                    // Borrowed snapshots keep the lock for the whole export.
                    Ok(None) => match db.reader() {
                        Ok(reader) => reader.scan_entities_with(send),
                        Err(error) => {
                            send(Err(error));
                        }
                    },
                    Err(error) => {
                        send(Err(error));
                    }
                }
            })
            .map_err(|error| {
                DbError::Storage(format!("spawn entity export thread: {error}").into())
            })?;
        // The export observes the state at the time of this call.
        let _ = snapshot_taken.await;
        Ok(receiver.boxed())
    }

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> Result<LocalCollectionId, DbError> {
        self.write(move |db| db.create_collection(name, kind)).await
    }

    async fn execute_ddl(&self, ddl: DdlBatch) -> Result<DdlOutcome, DbError> {
        self.write(move |db| db.transact_ddl(ddl)).await
    }

    async fn upsert_package(
        &self,
        package: Package,
    ) -> Result<PackageRegistrationOutcome, DbError> {
        self.write(move |db| db.upsert_package(package)).await
    }

    async fn upsert_relationship(&self, relationship: RelationType) -> Result<(), DbError> {
        self.write(move |db| db.upsert_relationship(relationship))
            .await
    }

    async fn delete_relationship(&self, id: String) -> Result<(), DbError> {
        self.write(move |db| db.delete_relationship(&id)).await
    }

    async fn insert(&self, collection: String, id: String, object: Object) -> Result<(), DbError> {
        self.write(move |db| db.insert(&collection, id, object))
            .await
    }

    async fn get(&self, collection: String, id: String) -> Result<Option<EntityRecord>, DbError> {
        self.read(move |reader| reader.get(&collection, &id)).await
    }

    async fn delete(&self, collection: String, id: String) -> Result<(), DbError> {
        self.write(move |db| db.delete(&collection, &id)).await
    }

    async fn query(&self, query: TextQueryInput) -> Result<QueryResult, DbError> {
        match self.resolve_query(query).await? {
            Query::Select(query) => {
                self.read(move |reader| reader.select(query).map(QueryResult::Select))
                    .await
            }
            query => self.write(move |db| db.query(query)).await,
        }
    }

    async fn explain(&self, query: TextQueryInput) -> Result<QueryExplain, DbError> {
        let query = self.resolve_query(query).await?;
        self.read(move |reader| reader.explain_query(query)).await
    }

    async fn plan(&self, query: TextQueryInput) -> Result<QueryPlan, DbError> {
        let query = self.resolve_query(query).await?;
        self.read(move |reader| reader.plan_query(query)).await
    }

    async fn update_where(&self, query: UpdateQuery) -> Result<MutationStats, DbError> {
        self.write(move |db| db.update_where(query)).await
    }

    async fn delete_where(&self, query: DeleteQuery) -> Result<usize, DbError> {
        self.write(move |db| db.delete_where(query)).await
    }

    async fn execute_batch(&self, batch: Batch) -> Result<BatchOutcome, DbError> {
        self.write(move |db| db.execute_batch(batch)).await
    }

    async fn execute_batch_with_settings(
        &self,
        batch: Batch,
        settings: crate::WriteSettings,
    ) -> Result<BatchOutcome, DbError> {
        self.write(move |db| db.execute_batch_with_settings(batch, settings))
            .await
    }

    async fn execute_batch_returning(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
    ) -> Result<crate::BatchReply, DbError> {
        self.write(move |db| db.execute_batch_returning(batch, returning))
            .await
    }

    async fn execute_batch_returning_with_settings(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        self.write(move |db| db.execute_batch_returning_with_settings(batch, returning, settings))
            .await
    }

    async fn execute_batch_returning_bounded_with_settings(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        self.write(move |db| {
            db.execute_batch_returning_bounded_with_settings(batch, returning, settings)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::StreamExt as _;
    use semantic_data::value::{FieldPath, Object, Value};

    use crate::Backend;

    use super::*;
    use crate::catalog::{LocalCollectionId, LocalIndexId};
    use crate::embedded::{
        BoxEntityIdScan, BoxEntityScan, MemoryEntityStorage, StorageCommitOutcome,
        StorageTransactionCapabilities, StorageWriteOp, StoredEntity,
    };

    #[derive(Debug)]
    struct CountingStorage {
        inner: MemoryEntityStorage,
        yielded: Arc<AtomicUsize>,
    }

    impl EntityStorage for CountingStorage {
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
            let yielded = Arc::clone(&self.yielded);
            let scan = self.inner.scan_collection_stream(collection)?;
            Ok(Box::new(scan.map(move |item| {
                yielded.fetch_add(1, Ordering::Relaxed);
                item
            })))
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

    fn assert_backend_impl<T: Backend>() {}

    #[test]
    fn kv_backend_blanket_impl_compiles() {
        assert_backend_impl::<EmbeddedBackend<MemoryEntityStorage>>();
    }

    #[test]
    fn entity_scan_is_lazy_bounded_and_cancellable() {
        let yielded = Arc::new(AtomicUsize::new(0));
        let storage = CountingStorage {
            inner: MemoryEntityStorage::new(),
            yielded: Arc::clone(&yielded),
        };
        let mut db = EmbeddedDb::new(storage);
        for index in 0..100 {
            let id = format!("entity-{index:03}");
            let mut object = Object::new();
            object.insert("id", Value::String(id.clone()));
            db.insert("entities", id, object).unwrap();
        }
        yielded.store(0, Ordering::Relaxed);
        let backend = EmbeddedBackend::new(db);
        futures::executor::block_on(async {
            let mut stream = backend.scan_entities().await.unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(yielded.load(Ordering::Relaxed) < 100);
            assert!(stream.next().await.unwrap().is_ok());
            drop(stream);
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(yielded.load(Ordering::Relaxed) < 100);
    }

    fn entity(id: &str) -> Object {
        let mut object = Object::new();
        object.insert("id", Value::String(id.to_string()));
        object
    }

    #[test]
    fn db_reader_can_outlive_the_database_borrow() {
        fn assert_send_static<T: Send + 'static>() {}
        assert_send_static::<DbReader<'static>>();
    }

    #[test]
    fn owned_reader_observes_the_state_it_was_created_at() {
        let mut db = EmbeddedDb::new(MemoryEntityStorage::new());
        db.insert("entities", "a", entity("a")).unwrap();
        let reader = db.owned_reader().unwrap().expect("owned snapshots");
        assert!(reader.is_owned());
        db.insert("entities", "b", entity("b")).unwrap();

        let all = || crate::SelectQuery::new().with_collection("entities");
        assert_eq!(reader.select(all()).unwrap().len(), 1);
        assert!(reader.get("entities", "b").unwrap().is_none());
        assert_eq!(db.select(all()).unwrap().len(), 2);
        assert_eq!(db.reader().unwrap().select(all()).unwrap().len(), 2);
    }

    #[test]
    fn storages_without_owned_snapshots_use_borrowed_readers() {
        let storage = CountingStorage {
            inner: MemoryEntityStorage::new(),
            yielded: Arc::new(AtomicUsize::new(0)),
        };
        let mut db = EmbeddedDb::new(storage);
        db.insert("entities", "a", entity("a")).unwrap();
        assert!(db.owned_reader().unwrap().is_none());
        let reader = db.reader().unwrap();
        assert!(!reader.is_owned());
        assert!(reader.get("entities", "a").unwrap().is_some());
        assert_eq!(
            reader
                .select(crate::SelectQuery::new().with_collection("entities"))
                .unwrap()
                .len(),
            1
        );
    }

    type BlockingOutput = Box<dyn std::any::Any + Send>;

    /// Runs blocking work on a fresh thread, so queries (which block on
    /// their own executor) can run under `block_on`.
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

    fn thread_backend<S: EntityStorage>(db: EmbeddedDb<S>) -> EmbeddedBackend<S> {
        EmbeddedBackend::with_runtime(db, Arc::new(ThreadRuntime))
    }

    fn by_id(id: &str) -> crate::SelectQuery {
        crate::SelectQuery::new()
            .with_collection("entities")
            .with_predicate(crate::Expr::Binary {
                op: semantic_data::query::BinaryOp::Eq,
                left: Box::new(crate::Expr::Operand(crate::Operand::Field(
                    FieldPath::from_fields(["id"]),
                ))),
                right: Box::new(crate::Expr::Operand(crate::Operand::Literal(
                    Value::String(id.to_string()),
                ))),
            })
    }

    #[test]
    fn transaction_handles_are_isolated_until_commit() {
        fn assert_handle_send_sync<T: Send + Sync + ?Sized>() {}
        assert_handle_send_sync::<dyn TransactionHandle>();

        let mut db = EmbeddedDb::new(MemoryEntityStorage::new());
        db.insert("entities", "a", entity("a")).unwrap();
        let db = crate::Db::new(thread_backend(db));
        futures::executor::block_on(async {
            let tx = db
                .begin_transaction(TransactionOptions::default())
                .await
                .unwrap();
            tx.upsert("entities".into(), "b".into(), entity("b"))
                .await
                .unwrap();
            assert!(tx.delete("entities".into(), "a".into()).await.unwrap());
            assert!(
                tx.get("entities".into(), "b".into())
                    .await
                    .unwrap()
                    .is_some()
            );
            assert_eq!(tx.select(by_id("b")).await.unwrap().len(), 1);
            assert!(tx.select(by_id("a")).await.unwrap().is_empty());
            // Plain reads and writes proceed while the transaction is open.
            assert!(db.get("entities", "b").await.unwrap().is_none());
            assert!(db.get("entities", "a").await.unwrap().is_some());
            let commit = tx.commit().await.unwrap();
            assert_eq!(commit.stats.upserted, 1);
            assert_eq!(commit.stats.deleted, 1);
            assert!(db.get("entities", "b").await.unwrap().is_some());
            assert!(db.get("entities", "a").await.unwrap().is_none());

            // Dropped handles leave no trace.
            let tx = db
                .begin_transaction(TransactionOptions::default())
                .await
                .unwrap();
            tx.upsert("entities".into(), "dropped".into(), entity("dropped"))
                .await
                .unwrap();
            drop(tx);
            assert!(db.get("entities", "dropped").await.unwrap().is_none());

            // The first of two overlapping transactions wins.
            let first = db
                .begin_transaction(TransactionOptions::default())
                .await
                .unwrap();
            let second = db
                .begin_transaction(TransactionOptions::default())
                .await
                .unwrap();
            first
                .upsert("entities".into(), "c".into(), entity("c"))
                .await
                .unwrap();
            second
                .upsert("entities".into(), "d".into(), entity("d"))
                .await
                .unwrap();
            first.commit().await.unwrap();
            assert!(matches!(
                second.commit().await,
                Err(DbError::TransactionConflict(_))
            ));
            assert!(db.get("entities", "d").await.unwrap().is_none());
        });
    }

    #[test]
    fn transactions_without_owned_snapshots_fence_reads() {
        let storage = CountingStorage {
            inner: MemoryEntityStorage::new(),
            yielded: Arc::new(AtomicUsize::new(0)),
        };
        let mut db = EmbeddedDb::new(storage);
        db.insert("entities", "a", entity("a")).unwrap();
        assert!(matches!(
            db.begin(TransactionOptions::default()),
            Err(error) if error.storage_kind() == Some(StorageErrorKind::Unsupported)
        ));
        let db = crate::Db::new(thread_backend(db));
        futures::executor::block_on(async {
            let snapshot = TransactionOptions {
                isolation: crate::IsolationLevel::Snapshot,
                ..TransactionOptions::default()
            };
            assert!(db.begin_transaction(snapshot).await.is_err());

            let tx = db
                .begin_transaction(TransactionOptions::default())
                .await
                .unwrap();
            tx.upsert("entities".into(), "b".into(), entity("b"))
                .await
                .unwrap();
            assert_eq!(tx.select(by_id("b")).await.unwrap().len(), 1);
            assert!(
                tx.get("entities".into(), "a".into())
                    .await
                    .unwrap()
                    .is_some()
            );
            tx.commit().await.unwrap();
            assert!(db.get("entities", "b").await.unwrap().is_some());

            // A commit after `begin` fails the transaction's later reads
            // and its commit.
            let tx = db
                .begin_transaction(TransactionOptions::default())
                .await
                .unwrap();
            tx.upsert("entities".into(), "d".into(), entity("d"))
                .await
                .unwrap();
            db.insert("entities", "c", entity("c")).await.unwrap();
            assert!(matches!(
                tx.get("entities".into(), "a".into()).await,
                Err(DbError::TransactionConflict(_))
            ));
            assert!(matches!(
                tx.commit().await,
                Err(DbError::TransactionConflict(_))
            ));
            assert!(db.get("entities", "d").await.unwrap().is_none());
        });
    }

    #[cfg(feature = "tokio")]
    #[test]
    fn kv_backend_runtime_constructors_compile() {
        let db = EmbeddedDb::new(MemoryEntityStorage::new());
        let _ = EmbeddedBackend::with_runtime(db, Arc::new(crate::TokioAsyncRuntime));
    }
}
