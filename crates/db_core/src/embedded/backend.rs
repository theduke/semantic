use std::sync::{Arc, Mutex, RwLock};

use crate::catalog::{Catalog, CollectionKind, LocalCollectionId, LocalIndexId, SharedCatalog};
use crate::{
    AsyncRuntime, Backend, Batch, BatchOutcome, BatchStats, DbError, DdlBatch, DdlOutcome,
    DeleteQuery, DeleteResult, EntityRecord, MutationStats, PackageRegistrationOutcome, Query,
    QueryExplain, QueryMetrics, QueryPlan, QueryResult, SavepointId, SelectQuery, StorageErrorKind,
    TextQueryInput, TransactionCommit, TransactionHandle, TransactionOptions, UpdateQuery,
    UpdateResult, spawn_blocking_on,
};
use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::{SinkExt as _, StreamExt as _};
use semantic_data::schema::{Package, RelationType};
use semantic_data::value::{FieldPath, Object, Value};

use crate::TransactionMetrics;
use crate::embedded::db::{CommittedTransaction, PreparedTransaction};
use crate::embedded::{
    BoxEntityIdScan, BoxEntityScan, BoxIndexEntryScan, DbReader, EmbeddedDb, EmbeddedTransaction,
    EntityReadSnapshot, EntityStorage, StoredEntity,
};

/// [`Backend`] over an [`EmbeddedDb`] that runs blocking work on an
/// [`AsyncRuntime`].
///
/// Reads hold the read side of a lock around the database only while
/// creating a [`DbReader`] over the current committed state and then run
/// without the lock, so long reads and exports neither block writers nor
/// observe their commits. Storages without owned snapshots
/// ([`EntityStorage::owned_snapshot`]) fall back to running the whole read
/// under the lock. Catalog reads use the shared catalog and never take the
/// lock.
///
/// Data writes (inserts, deletes, batches and `INSERT`/`UPDATE`/`DELETE`
/// queries) are optimistic, see [`Self::data_write`]: they are prepared on a
/// snapshot without the lock and take its write side only for the
/// revision-conditional commit, so readers can take snapshots while a write
/// is being prepared. DDL, package migrations and maintenance mutate the
/// catalog or whole collections and hold the write side for their whole
/// duration.
pub struct EmbeddedBackend<S: EntityStorage> {
    db: Arc<RwLock<EmbeddedDb<S>>>,
    catalog: SharedCatalog,
    /// Shared with the database, so subscribing never takes the lock.
    change_feed: crate::ChangeFeed,
    runtime: Arc<dyn AsyncRuntime>,
    /// See [`crate::DbConfig::slow_query_threshold`].
    slow_query_threshold: Option<std::time::Duration>,
}

impl<S: EntityStorage> EmbeddedBackend<S> {
    pub fn new(db: EmbeddedDb<S>) -> Self {
        Self::with_runtime(db, default_runtime())
    }

    pub fn with_runtime(db: EmbeddedDb<S>, runtime: Arc<dyn AsyncRuntime>) -> Self {
        Self {
            catalog: db.shared_catalog().clone(),
            change_feed: db.change_feed().clone(),
            slow_query_threshold: db.config().slow_query_threshold,
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
            TextQueryInput::Ast(query) => query.into_bound(&std::collections::BTreeMap::new()),
            TextQueryInput::AstWithParams { query, params } => query.into_bound(&params),
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

    /// Run `op` with shared access to the database, holding the read side
    /// of the lock.
    fn shared<R, F>(&self, op: F) -> BoxFuture<'static, Result<R, DbError>>
    where
        R: Send + 'static,
        F: FnOnce(&EmbeddedDb<S>) -> Result<R, DbError> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            op(&db)
        })
    }

    /// Run a data write optimistically.
    ///
    /// Each attempt begins a transaction at the current state (holding the
    /// read side of the lock only to take the snapshot), applies the write
    /// and validates it with `prepare` without any lock, then takes the write
    /// side only to commit (see [`EmbeddedDb::commit_prepared`]). The commit
    /// is conditional on the storage revision and catalog version the
    /// attempt read: every concurrent commit moves the revision, so an
    /// attempt whose reads (including its uniqueness and reference checks)
    /// could be stale fails with a conflict and is retried from the start,
    /// per [`TransactionOptions::default`]. Concurrent writers therefore
    /// still serialize at the commit.
    ///
    /// `locked` runs the write with exclusive access (the pre-existing path)
    /// when the storage cannot prepare writes without the lock (no owned
    /// consistent snapshots or no revision-conditional commits), and as the
    /// final attempt once the optimistic retries are exhausted, so in-process
    /// contention never fails a write. It receives the metrics of the
    /// optimistic attempts made so far.
    fn data_write<T, R, P, F, L>(
        &self,
        prepare: P,
        finish: F,
        locked: L,
    ) -> BoxFuture<'static, Result<R, DbError>>
    where
        T: Send + 'static,
        R: Send + 'static,
        P: FnMut(EmbeddedTransaction) -> Result<PreparedTransaction<T>, DbError> + Send + 'static,
        F: FnOnce(CommittedTransaction<T>, TransactionMetrics) -> Result<R, DbError>
            + Send
            + 'static,
        L: FnOnce(&mut EmbeddedDb<S>, TransactionMetrics) -> Result<R, DbError> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            optimistic_write(&db, prepare, finish, locked)
        })
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

/// See [`EmbeddedBackend::data_write`].
fn optimistic_write<S, T, R>(
    db: &RwLock<EmbeddedDb<S>>,
    mut prepare: impl FnMut(EmbeddedTransaction) -> Result<PreparedTransaction<T>, DbError>,
    finish: impl FnOnce(CommittedTransaction<T>, TransactionMetrics) -> Result<R, DbError>,
    locked: impl FnOnce(&mut EmbeddedDb<S>, TransactionMetrics) -> Result<R, DbError>,
) -> Result<R, DbError>
where
    S: EntityStorage,
{
    let options = TransactionOptions::default();
    let mut metrics = TransactionMetrics {
        attempts: 0,
        conflicts: 0,
    };
    let optimistic_attempts = match options.conflict_policy {
        crate::ConflictPolicy::Retry => options.max_retries,
        crate::ConflictPolicy::Fail => 0,
    };
    while metrics.attempts < optimistic_attempts {
        let tx = {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            match db.begin(options) {
                Ok(tx) => tx,
                Err(error) if error.storage_kind() == Some(StorageErrorKind::Unsupported) => break,
                Err(error) => return Err(error),
            }
        };
        if !tx.prepares_without_lock() {
            break;
        }
        metrics.attempts += 1;
        let committed = prepare(tx).and_then(|prepared| {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.commit_prepared(prepared, crate::ChangeSource::Batch)
        });
        match committed {
            Ok(committed) => return finish(committed, metrics),
            Err(DbError::TransactionConflict(reason)) => {
                tracing::debug!(%reason, attempt = metrics.attempts, "optimistic write conflicted");
                metrics.conflicts += 1;
            }
            Err(error) => return Err(error),
        }
    }
    // Release what the optimistic attempts hold before the final attempt.
    drop(prepare);
    let mut db = db.write().map_err(|_| lock_poisoned_error())?;
    locked(&mut db, metrics)
}

/// `reply` with the attempts of `prior` optimistic attempts added.
fn with_prior_attempts(reply: crate::BatchReply, prior: TransactionMetrics) -> crate::BatchReply {
    let mut metrics = *reply.metrics();
    metrics.attempts += u64::from(prior.attempts);
    metrics.conflicts += u64::from(prior.conflicts);
    reply.with_metrics(metrics)
}

/// What an entity export observes, announced before streaming.
struct ExportMeta {
    revision: Option<u64>,
    catalog_version: u64,
    catalog: Arc<Catalog>,
}

impl ExportMeta {
    /// Send the metadata of `reader` to the caller; `false` when the export
    /// cannot proceed.
    fn announce(
        reader: &DbReader<'_>,
        ready: futures::channel::oneshot::Sender<Result<ExportMeta, DbError>>,
    ) -> bool {
        let meta = reader.revision().map(|revision| ExportMeta {
            revision,
            catalog_version: reader.catalog_version(),
            catalog: Arc::clone(reader.catalog()),
        });
        let ok = meta.is_ok();
        ready.send(meta).is_ok() && ok
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
            if !tx.prepares_without_lock() {
                // Reads call back into the database lock: validate under it.
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                return db.commit_transaction(tx);
            }
            // Validate on the transaction's snapshot; only the conditional
            // commit takes the lock.
            let prepared = tx.prepare_commit()?;
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            let committed = db.commit_prepared(prepared, crate::ChangeSource::Transaction)?;
            Ok(TransactionCommit {
                revision: committed.revision,
                stats: committed.stats,
                metrics: committed.metrics,
            })
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
        Ok(self.export_snapshot().await?.stream)
    }

    async fn export_snapshot(&self) -> Result<crate::ExportSnapshot, DbError> {
        const CHANNEL_CAPACITY: usize = 16;
        let db = Arc::clone(&self.db);
        let (mut sender, receiver) = futures::channel::mpsc::channel(CHANNEL_CAPACITY);
        let (ready, snapshot_taken) =
            futures::channel::oneshot::channel::<Result<ExportMeta, DbError>>();
        std::thread::Builder::new()
            .name("semantic-entity-export".into())
            .spawn(move || {
                let send = |item| futures::executor::block_on(sender.send(item)).is_ok();
                let db = match db.read() {
                    Ok(db) => db,
                    Err(_) => {
                        let _ = ready.send(Err(lock_poisoned_error()));
                        return;
                    }
                };
                match db.owned_reader() {
                    // The owned snapshot keeps the export consistent without
                    // holding the lock while the consumer drains it.
                    Ok(Some(reader)) => {
                        drop(db);
                        if ExportMeta::announce(&reader, ready) {
                            reader.scan_entities_with(send);
                        }
                    }
                    // Borrowed snapshots keep the lock for the whole export.
                    Ok(None) => match db.reader() {
                        Ok(reader) => {
                            if ExportMeta::announce(&reader, ready) {
                                reader.scan_entities_with(send);
                            }
                        }
                        Err(error) => {
                            let _ = ready.send(Err(error));
                        }
                    },
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                }
            })
            .map_err(|error| {
                DbError::Storage(format!("spawn entity export thread: {error}").into())
            })?;
        // The export observes the state at the time of this call.
        let meta = snapshot_taken.await.map_err(|_| {
            DbError::storage(
                StorageErrorKind::InvalidState,
                "entity export thread stopped before taking its snapshot",
            )
        })??;
        Ok(crate::ExportSnapshot {
            revision: meta.revision,
            catalog_version: meta.catalog_version,
            catalog: meta.catalog,
            stream: receiver.boxed(),
        })
    }

    async fn reindex(&self, target: crate::ReindexTarget) -> Result<crate::ReindexReport, DbError> {
        self.write(move |db| db.reindex(&target)).await
    }

    async fn verify(&self, options: crate::VerifyOptions) -> Result<crate::VerifyReport, DbError> {
        // The physical check needs exclusive access; the logical checks run
        // on a snapshot without the lock.
        let integrity = if options.check_storage_integrity {
            Some(
                self.write(|db| db.check_storage_integrity_for_verify())
                    .await?,
            )
        } else {
            None
        };
        self.read(move |reader| EmbeddedDb::<S>::verify_reader(reader, &options, integrity))
            .await
    }

    async fn repair(&self, options: crate::VerifyOptions) -> Result<crate::RepairReport, DbError> {
        self.write(move |db| db.repair(&options)).await
    }

    async fn compact_storage(&self) -> Result<crate::CompactReport, DbError> {
        self.write(|db| db.compact_storage()).await
    }

    async fn storage_stats(&self) -> Result<crate::embedded::StorageStats, DbError> {
        self.shared(|db| db.storage_stats()).await
    }

    async fn rewrite_payloads(&self, batch_size: usize) -> Result<crate::RewriteReport, DbError> {
        // One lock acquisition per batch, so writers interleave with the
        // rewrite.
        let started = std::time::Instant::now();
        let mut report = crate::RewriteReport::default();
        let mut resume_after: Option<Vec<u8>> = None;
        loop {
            let resume = resume_after.take();
            let batch = self
                .write(move |db| db.rewrite_payload_batch(resume.as_deref(), batch_size))
                .await?;
            report.scanned += batch.scanned;
            report.rewritten += batch.rewritten;
            report.batches += u64::from(batch.rewritten > 0);
            resume_after = batch.resume_after;
            if resume_after.is_none() {
                break;
            }
        }
        report.duration = started.elapsed();
        Ok(report)
    }

    async fn backup(&self, path: std::path::PathBuf) -> Result<crate::BackupReport, DbError> {
        // Capture the state under the lock, copy it without.
        let source = self.shared(|db| db.backup_source()).await?;
        spawn_blocking_on(self.runtime.as_ref(), move || {
            crate::embedded::db::write_backup(source, &path)
        })
        .await
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
        let batch = Batch::new().with_op(crate::BatchOperation::Upsert {
            collection,
            id,
            object,
        });
        self.batch_write(
            batch,
            crate::BatchReturn::Stats,
            crate::WriteSettings::default(),
            false,
        )
        .await?;
        Ok(())
    }

    async fn get(&self, collection: String, id: String) -> Result<Option<EntityRecord>, DbError> {
        self.read(move |reader| reader.get(&collection, &id)).await
    }

    async fn delete(&self, collection: String, id: String) -> Result<(), DbError> {
        let batch = Batch::new().with_op(crate::BatchOperation::DeleteById { collection, id });
        self.batch_write(
            batch,
            crate::BatchReturn::Stats,
            crate::WriteSettings::default(),
            false,
        )
        .await?;
        Ok(())
    }

    async fn query(&self, query: TextQueryInput) -> Result<QueryResult, DbError> {
        if self.slow_query_threshold.is_some() {
            // The slow query log reports metrics, so collect them.
            return self
                .query_with_metrics(query)
                .await
                .map(|(result, _)| result);
        }
        match self.resolve_query(query).await? {
            Query::Select(query) => {
                self.read(move |reader| reader.select(query).map(QueryResult::Select))
                    .await
            }
            query => self.write_query(query).await,
        }
    }

    async fn query_with_metrics(
        &self,
        query: TextQueryInput,
    ) -> Result<(QueryResult, QueryMetrics), DbError> {
        let started = std::time::Instant::now();
        let (result, mut metrics, physical) = match self.resolve_query(query).await? {
            Query::Select(query) => {
                let (rows, explain) = self
                    .read(move |reader| reader.select_analyzed(query, false))
                    .await?;
                let metrics = explain
                    .analyze
                    .map(|analysis| analysis.metrics)
                    .unwrap_or_default();
                (QueryResult::Select(rows), metrics, Some(explain.physical))
            }
            query => (
                self.write_query(query).await?,
                QueryMetrics::default(),
                None,
            ),
        };
        metrics.elapsed = started.elapsed();
        crate::metrics::log_slow_query(self.slow_query_threshold, &metrics, physical.as_ref());
        Ok((result, metrics))
    }

    async fn explain(&self, query: TextQueryInput) -> Result<QueryExplain, DbError> {
        let query = self.resolve_query(query).await?;
        self.read(move |reader| reader.explain_query(query)).await
    }

    async fn explain_analyze(&self, query: TextQueryInput) -> Result<QueryExplain, DbError> {
        let started = std::time::Instant::now();
        let query = self.resolve_query(query).await?;
        let mut explain = self
            .read(move |reader| reader.explain_analyze_query(query))
            .await?;
        if let Some(analysis) = &mut explain.analyze {
            analysis.metrics.elapsed = started.elapsed();
        }
        Ok(explain)
    }

    async fn plan(&self, query: TextQueryInput) -> Result<QueryPlan, DbError> {
        let query = self.resolve_query(query).await?;
        self.read(move |reader| reader.plan_query(query)).await
    }

    async fn update_where(&self, query: UpdateQuery) -> Result<MutationStats, DbError> {
        Ok(self.update_query(query).await?.stats)
    }

    async fn delete_where(&self, query: DeleteQuery) -> Result<usize, DbError> {
        Ok(self.delete_query(query).await?.deleted)
    }

    async fn execute_batch(&self, batch: Batch) -> Result<BatchOutcome, DbError> {
        self.execute_batch_with_settings(batch, crate::WriteSettings::default())
            .await
    }

    async fn execute_batch_with_settings(
        &self,
        batch: Batch,
        settings: crate::WriteSettings,
    ) -> Result<BatchOutcome, DbError> {
        match self
            .batch_write(batch, crate::BatchReturn::Dataset, settings, false)
            .await?
        {
            crate::BatchReply::Dataset(outcome) => Ok(outcome),
            _ => unreachable!("dataset returning requested"),
        }
    }

    async fn execute_batch_returning(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
    ) -> Result<crate::BatchReply, DbError> {
        self.batch_write(batch, returning, crate::WriteSettings::default(), false)
            .await
    }

    async fn execute_batch_returning_with_settings(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        self.batch_write(batch, returning, settings, false).await
    }

    async fn execute_batch_returning_bounded_with_settings(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        self.batch_write(batch, returning, settings, true).await
    }
}

impl<S: EntityStorage> EmbeddedBackend<S> {
    /// Execute `batch` as an optimistic data write.
    fn batch_write(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
        require_bounded: bool,
    ) -> BoxFuture<'static, Result<crate::BatchReply, DbError>> {
        // Attempts copy the batch; the final locked attempt takes it.
        let batch = Arc::new(batch);
        let locked_batch = Arc::clone(&batch);
        let locked_returning = returning.clone();
        self.data_write(
            move |tx| tx.prepare_batch((*batch).clone(), &returning, settings, require_bounded),
            |committed, metrics| {
                Ok(committed
                    .reply
                    .with_metrics(committed.metrics.with_transaction(metrics)))
            },
            move |db, prior| {
                let locked_batch =
                    Arc::try_unwrap(locked_batch).unwrap_or_else(|batch| (*batch).clone());
                let reply = if require_bounded {
                    db.execute_batch_returning_bounded_with_settings(
                        locked_batch,
                        locked_returning,
                        settings,
                    )
                } else {
                    db.execute_batch_returning_with_settings(
                        locked_batch,
                        locked_returning,
                        settings,
                    )
                }?;
                Ok(with_prior_attempts(reply, prior))
            },
        )
    }

    fn update_query(
        &self,
        query: UpdateQuery,
    ) -> BoxFuture<'static, Result<crate::UpdateResult, DbError>> {
        let locked = query.clone();
        self.data_write(
            move |tx| tx.prepare_update(query.clone()),
            |committed, _| Ok(committed.reply),
            move |db, _| db.update_where_returning(locked),
        )
    }

    fn delete_query(
        &self,
        query: DeleteQuery,
    ) -> BoxFuture<'static, Result<crate::DeleteResult, DbError>> {
        let locked = query.clone();
        self.data_write(
            move |tx| tx.prepare_delete(query.clone()),
            |committed, _| Ok(committed.reply),
            move |db, _| db.delete_where_returning(locked),
        )
    }

    /// Run a mutating query: data writes are optimistic, DDL holds the lock.
    async fn write_query(&self, query: Query) -> Result<QueryResult, DbError> {
        match query {
            Query::Insert(query) => {
                let locked = query.clone();
                self.data_write(
                    move |tx| tx.prepare_insert(query.clone()),
                    |committed, _| Ok(committed.reply),
                    move |db, _| db.insert_query(locked),
                )
                .await
                .map(QueryResult::Insert)
            }
            Query::Update(query) => self.update_query(query).await.map(QueryResult::Update),
            Query::Delete(query) => self.delete_query(query).await.map(QueryResult::Delete),
            query => self.write(move |db| db.query(query)).await,
        }
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

    fn binding_test_db() -> EmbeddedDb<MemoryEntityStorage> {
        let mut db = EmbeddedDb::new(MemoryEntityStorage::new());
        db.transact_ddl(
            DdlBatch::new().with_op(crate::DdlOperation::UpsertCollection {
                name: "items".into(),
                kind: crate::DdlCollectionKind::Untyped,
                integrity_mode: crate::catalog::IntegrityMode::Permissive,
            }),
        )
        .unwrap();
        db
    }

    #[test]
    fn db_sql_parse_preserves_parameters_or_reports_disabled_frontend() {
        let db = crate::Db::new(thread_backend(binding_test_db()));
        let result = db.parse_sql("SELECT :value AS value FROM items");
        #[cfg(feature = "sql")]
        {
            use semantic_data::query::{Expr, Operand, Query};
            let Query::Select(select) = result.unwrap() else {
                panic!("select")
            };
            assert!(
                matches!(&*select.projection[0].expr, Expr::Operand(Operand::Parameter(name)) if name == "value")
            );
        }
        #[cfg(not(feature = "sql"))]
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("sql support is disabled")
        );
    }

    #[test]
    fn client_constructed_ast_is_reusable_and_binds_for_query_plan_explain_and_metrics() {
        use semantic_data::query as public;
        use std::collections::BTreeMap;
        let mut db = binding_test_db();
        for score in [1, 2, 3] {
            let id = format!("row-{score}");
            let mut row = Object::new();
            row.insert("id", Value::String(id.clone()));
            row.insert("score", Value::I64(score));
            db.insert("items", id, row).unwrap();
        }
        let db = crate::Db::new(thread_backend(db));
        let ast: public::Query = public::SelectQuery::new()
            .with_collection("items")
            .with_predicate(public::Expr::Binary {
                op: public::BinaryOp::Gt,
                left: Box::new(public::Expr::Operand(public::Operand::Field(
                    FieldPath::from_fields(["score"]),
                ))),
                right: Box::new(public::Expr::parameter("minimum")),
            })
            .into();
        futures::executor::block_on(async {
            for (minimum, count) in [(1, 2), (2, 1)] {
                let input = public::QueryInput::ast_with_params(
                    ast.clone(),
                    BTreeMap::from([("minimum".into(), Value::I64(minimum))]),
                );
                let QueryResult::Select(rows) = db.query(input.clone()).await.unwrap() else {
                    panic!("select");
                };
                assert_eq!(rows.len(), count);
                db.plan(input.clone()).await.unwrap();
                db.explain(input.clone()).await.unwrap();
                let (QueryResult::Select(measured), _) =
                    db.query_with_metrics(input).await.unwrap()
                else {
                    panic!("select");
                };
                assert_eq!(measured, rows);
            }
            for result in [
                db.query(ast.clone()).await.map(|_| ()),
                db.plan(ast.clone()).await.map(|_| ()),
                db.explain(ast.clone()).await.map(|_| ()),
                db.query_with_metrics(ast).await.map(|_| ()),
            ] {
                assert!(
                    matches!(result, Err(DbError::QueryParameter { reason, .. }) if reason == "missing")
                );
            }
        });
    }

    #[test]
    fn client_constructed_projection_references_execute_without_sql_frontend() {
        use semantic_data::query as public;
        use std::collections::BTreeMap;
        let mut embedded = binding_test_db();
        for (index, score) in [2, 1, 2].into_iter().enumerate() {
            let id = format!("row-{index}");
            let mut row = Object::new();
            row.insert("id", Value::String(id.clone()));
            row.insert("score", Value::I64(score));
            embedded.insert("items", id, row).unwrap();
        }
        let db = crate::Db::new(thread_backend(embedded));
        let ast = public::SelectQuery::new()
            .with_collection("items")
            .with_projection(vec![
                public::QueryField {
                    expr: Box::new(public::Expr::Operand(public::Operand::Field(
                        FieldPath::from_fields(["score"]),
                    ))),
                    alias: Some("score".into()),
                    wildcard: None,
                },
                public::QueryField {
                    expr: Box::new(public::Expr::Aggregate {
                        op: public::AggregateOp::Count,
                        distinct: false,
                        arg: Box::new(public::FunctionArg::Wildcard),
                    }),
                    alias: Some("count".into()),
                    wildcard: None,
                },
            ])
            .with_group_by(vec![public::Expr::ProjectionRef(Box::new(
                public::Expr::parameter("group"),
            ))])
            .with_order_by(vec![public::OrderBy {
                expr: public::Expr::ProjectionRef(Box::new(public::Expr::parameter("order"))),
                direction: public::SortDirection::Desc,
            }]);
        futures::executor::block_on(async {
            let input = public::QueryInput::ast_with_params(
                ast,
                BTreeMap::from([
                    ("group".into(), Value::U64(1)),
                    ("order".into(), Value::U64(2)),
                ]),
            );
            let QueryResult::Select(rows) = db.query(input).await.unwrap() else {
                panic!("select");
            };
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].get("score"), Some(&Value::I64(2)));
            assert_eq!(rows[1].get("score"), Some(&Value::I64(1)));
        });
    }

    #[cfg(feature = "sql")]
    #[test]
    fn parsed_ast_execution_matches_bound_sql_for_projection_ordinals_and_aggregates() {
        use semantic_data::query as public;
        use std::collections::BTreeMap;
        let mut embedded = binding_test_db();
        for (index, score) in [2, 1, 2].into_iter().enumerate() {
            let id = format!("row-{index}");
            let mut row = Object::new();
            row.insert("id", Value::String(id.clone()));
            row.insert("score", Value::I64(score));
            embedded.insert("items", id, row).unwrap();
        }
        let db = crate::Db::new(thread_backend(embedded));
        for sql in [
            "SELECT score AS score FROM items ORDER BY :order LIMIT :limit",
            "SELECT score AS score, COUNT(*) AS count FROM items GROUP BY :group ORDER BY :order LIMIT :limit",
            "SELECT :value AS value, COUNT(*) AS count FROM items GROUP BY :group ORDER BY :order LIMIT :limit",
        ] {
            let ast = crate::sql::parse_sql_query_unbound(sql, crate::sql::SqlDialectKind::Generic)
                .unwrap();
            for order in [1, 2] {
                if order == 2 && !sql.contains("COUNT") {
                    continue;
                }
                let params: BTreeMap<_, _> = [
                    ("order", Value::U64(order)),
                    ("limit", Value::U64(2)),
                    ("group", Value::U64(1)),
                    ("value", Value::I64(7)),
                ]
                .into_iter()
                .filter(|(name, _)| sql.contains(&format!(":{name}")))
                .map(|(name, value)| (name.to_string(), value))
                .collect();
                futures::executor::block_on(async {
                    let expected = db
                        .query(public::QueryInput::sql_with_params(sql, params.clone()))
                        .await
                        .unwrap();
                    let actual = db
                        .query(public::QueryInput::ast_with_params(ast.clone(), params))
                        .await
                        .unwrap();
                    assert_eq!(actual, expected, "{sql}");
                });
            }
        }
    }

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
