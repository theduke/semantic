//! Interactive transactions: statements executed one at a time against one
//! snapshot, committed atomically.
//!
//! [`EmbeddedDb::begin`] pins the catalog and an owned storage snapshot at
//! the current revision. Every statement reads that snapshot through the
//! transaction's overlay ([`TxState`]), so it observes the transaction's own
//! uncommitted writes and nothing committed after the snapshot; queries run
//! through an [`OverlaySnapshot`](super::overlay::OverlaySnapshot). Writes
//! only stage rows in the overlay (cascade deletes are expanded as each
//! statement runs); nothing reaches the storage before
//! [`EmbeddedDb::commit_transaction`], which runs the point write path's
//! validation and index maintenance over the overlay and commits it
//! conditionally on the snapshot revision. A transaction that is dropped
//! without committing has no effect.
//!
//! Every statement is atomic: a failing statement leaves the transaction as
//! it was before the statement.

use super::compact::{
    ExecutionCounts, PreparedCommit, TxRead, TxState, TxView, apply_tx_operation,
    cascade_deletes_from, prepare_commit,
};
use super::overlay::OverlaySnapshot;
use super::*;
use crate::catalog::CatalogSnapshot;
use crate::{BatchStats, SavepointId, TransactionCommit, TransactionResult};

/// Reads of an interactive transaction, served from an owned storage
/// snapshot at the transaction's revision.
///
/// Snapshots that are not consistent (see
/// [`EntityReadSnapshot::is_consistent`]) are fenced: a read that observes a
/// different revision fails with a transaction conflict.
pub(super) struct SnapshotReader {
    snapshot: Arc<dyn EntityReadSnapshot>,
    revision: Option<u64>,
}

impl SnapshotReader {
    fn fence(&self) -> Result<(), DbError> {
        if self.snapshot.is_consistent() {
            return Ok(());
        }
        let actual = self.snapshot.revision()?;
        if actual != self.revision {
            return Err(DbError::TransactionConflict(format!(
                "database changed during transaction: expected revision {:?}, found {actual:?}",
                self.revision
            )));
        }
        Ok(())
    }

    fn read<T>(
        &self,
        read: impl FnOnce(&dyn EntityReadSnapshot) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        self.fence()?;
        let value = read(&*self.snapshot)?;
        self.fence()?;
        Ok(value)
    }
}

impl TxRead for SnapshotReader {
    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        self.read(|snapshot| snapshot.get_entity(collection, id))
    }

    fn scan_index_value(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<Vec<String>, DbError> {
        self.read(|snapshot| snapshot.scan_index_value(index, path, value))
    }

    fn scan_index_range_ids(
        &self,
        index: LocalIndexId,
        range: &crate::IndexScanRange,
    ) -> Result<Vec<String>, DbError> {
        let scan = crate::IndexScan {
            range: range.clone(),
            reverse: false,
            with_keys: false,
        };
        self.read(|snapshot| {
            snapshot
                .scan_index_entries(index, &scan)?
                .map(|entry| entry.map(|entry| entry.id))
                .collect()
        })
    }

    fn scan_collection(
        &self,
        collection: LocalCollectionId,
        visit: &mut dyn FnMut(StoredEntity) -> Result<(), DbError>,
    ) -> Result<(), DbError> {
        self.read(|snapshot| {
            for entity in snapshot.scan_collection_stream(collection)? {
                visit(entity?)?;
            }
            Ok(())
        })
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        self.snapshot.index_needs_rebuild(index)
    }

    fn planning_snapshot(&self) -> Result<QueryReader<'_>, DbError> {
        Ok(QueryReader::Shared(self.snapshot.clone()))
    }
}

struct Savepoint {
    id: SavepointId,
    /// Undo log position of the savepoint.
    undo: usize,
    stats: BatchStats,
}

/// An interactive transaction over an [`EmbeddedDb`].
///
/// Created by [`EmbeddedDb::begin`]. The transaction owns its snapshot and
/// overlay and does not borrow the database: statements run while other
/// readers and writers use the database, and neither see the other's
/// uncommitted state. Commit with [`EmbeddedDb::commit_transaction`] (or
/// [`Self::commit`]); dropping the transaction rolls it back.
pub struct EmbeddedTransaction {
    options: TransactionOptions,
    catalog: CatalogSnapshot,
    reader: SnapshotReader,
    state: TxState,
    stats: BatchStats,
    savepoints: Vec<Savepoint>,
    next_savepoint: u64,
    recursive_validation: bool,
}

impl std::fmt::Debug for EmbeddedTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedTransaction")
            .field("options", &self.options)
            .field("catalog_version", &self.catalog.version)
            .field("revision", &self.reader.revision)
            .field("written_rows", &self.state.overlay().len())
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl EmbeddedTransaction {
    pub fn options(&self) -> TransactionOptions {
        self.options
    }

    /// Storage revision the transaction reads at; the commit requires the
    /// storage to still be at this revision.
    pub fn revision(&self) -> Option<u64> {
        self.reader.revision
    }

    /// The catalog the transaction reads and writes under.
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog.catalog
    }

    /// Rows written so far, including cascade deletes.
    pub fn stats(&self) -> &BatchStats {
        &self.stats
    }

    fn view(&mut self) -> TxView<'_> {
        TxView::new(&self.catalog.catalog, &self.reader, &mut self.state)
    }

    fn collection(&self, name: &str) -> Result<&CollectionSchema, DbError> {
        self.catalog
            .catalog
            .collection_by_name(name)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: name.to_string(),
            })
    }

    /// The row `id` of `collection` as seen by the transaction.
    pub fn get(&mut self, collection: &str, id: &str) -> Result<Option<EntityRecord>, DbError> {
        let collection = self.collection(collection)?.name.clone();
        let key = (collection.clone(), id.to_string());
        Ok(self.view().get(&key)?.map(|object| EntityRecord {
            id: key.1,
            collection,
            object,
        }))
    }

    /// Run `query` against the snapshot with the transaction's writes
    /// applied (see [`super::overlay`] for the exact semantics).
    pub fn select(&self, query: SelectQuery) -> Result<Vec<Object>, DbError> {
        let snapshot = OverlaySnapshot::new(
            self.reader.snapshot.clone(),
            self.catalog.catalog.clone(),
            self.state.overlay(),
        );
        let reader = DbReader::new(
            self.catalog.clone(),
            QueryReader::Shared(Arc::new(snapshot)),
        );
        let rows = reader.select(query)?;
        self.reader.fence()?;
        Ok(rows)
    }

    /// Insert or replace a row.
    pub fn upsert(
        &mut self,
        collection: &str,
        id: impl Into<String>,
        object: Object,
    ) -> Result<(), DbError> {
        self.execute_batch(Batch::new().with_op(BatchOperation::Upsert {
            collection: collection.to_string(),
            id: id.into(),
            object,
        }))?;
        Ok(())
    }

    /// Insert a row; fails with [`DbError::EntityExists`] when the row
    /// exists (including rows written by the transaction).
    pub fn create(
        &mut self,
        collection: &str,
        id: impl Into<String>,
        object: Object,
    ) -> Result<(), DbError> {
        self.execute_batch(Batch::new().with_op(BatchOperation::Create {
            collection: collection.to_string(),
            id: id.into(),
            object,
        }))?;
        Ok(())
    }

    /// Delete a row (and the rows whose cascading references point at it).
    /// Returns whether the row existed.
    pub fn delete(&mut self, collection: &str, id: &str) -> Result<bool, DbError> {
        let stats = self.execute_batch(Batch::new().with_op(BatchOperation::DeleteById {
            collection: collection.to_string(),
            id: id.to_string(),
        }))?;
        // Cascades only follow deletes of existing rows.
        Ok(stats.deleted > 0)
    }

    pub fn update_where(&mut self, query: UpdateQuery) -> Result<crate::UpdateResult, DbError> {
        evaluate_mutation_limit(query.limit.as_ref())
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
        let catalog = self.catalog.catalog.clone();
        let collection = self.collection(query.collection_or_default())?.clone();
        ensure_collection_mutable(&collection)?;
        let query = canonicalize_update_query(&query, &catalog, &collection)?;
        let field_format = query.field_format;
        let (mut result, _) = self.statement(|tx, stats| {
            let context = DefaultExpressionContext::now();
            let query_context = QueryContext::new(catalog.clone());
            let recursive_validation = tx.recursive_validation;
            let result = mutation::tx_update(
                &mut tx.view(),
                &query_context,
                &collection.name,
                &query,
                &context,
                recursive_validation,
            )?;
            stats.updated += result.stats.affected;
            Ok(result)
        })?;
        result.returning = crate::format_output_rows(&catalog, result.returning, field_format);
        Ok(result)
    }

    pub fn delete_where(&mut self, query: DeleteQuery) -> Result<crate::DeleteResult, DbError> {
        evaluate_mutation_limit(query.limit.as_ref())
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
        let catalog = self.catalog.catalog.clone();
        let collection = self.collection(query.collection_or_default())?.clone();
        ensure_collection_mutable(&collection)?;
        let query = canonicalize_delete_query(&query, &catalog, &collection)?;
        let field_format = query.field_format;
        let (mut result, _) = self.statement(|tx, stats| {
            let query_context = QueryContext::new(catalog.clone());
            let result =
                mutation::tx_delete(&mut tx.view(), &query_context, &collection.name, &query)?;
            stats.deleted += result.deleted;
            Ok(result)
        })?;
        result.returning = crate::format_output_rows(&catalog, result.returning, field_format);
        Ok(result)
    }

    /// Apply the operations of `batch` in order, as one statement. Returns
    /// the rows the batch wrote, including cascade deletes.
    pub fn execute_batch(&mut self, batch: Batch) -> Result<BatchStats, DbError> {
        validate_batch_mutation_limits(&batch)?;
        let catalog = self.catalog.catalog.clone();
        let batch = canonicalize_batch(batch, &catalog)?;
        let ((), stats) = self.statement(|tx, stats| {
            let context = DefaultExpressionContext::now();
            let query_context = QueryContext::new(catalog.clone());
            let recursive_validation = tx.recursive_validation;
            let mut view = tx.view();
            for operation in batch.operations {
                let operation = with_canonical_collection(&catalog, operation);
                apply_tx_operation(
                    &mut view,
                    &query_context,
                    operation,
                    stats,
                    &context,
                    recursive_validation,
                )?;
            }
            Ok(())
        })?;
        Ok(stats)
    }

    /// Run one writing statement atomically: on error, every write of the
    /// statement is undone. Cascade deletes of the statement's deletes are
    /// expanded before it completes. Returns the statement's result and the
    /// rows it wrote.
    fn statement<T>(
        &mut self,
        run: impl FnOnce(&mut Self, &mut BatchStats) -> Result<T, DbError>,
    ) -> Result<(T, BatchStats), DbError> {
        if self.options.read_only {
            return Err(DbError::InvalidQuery(
                "read-only transaction cannot contain mutation operations".to_string(),
            ));
        }
        let position = self.state.undo_position();
        let mut stats = BatchStats {
            upserted: 0,
            updated: 0,
            deleted: 0,
        };
        let result = run(self, &mut stats).and_then(|value| {
            let deleted = self.state.take_deleted();
            cascade_deletes_from(&mut self.view(), deleted, &mut stats)?;
            Ok(value)
        });
        match &result {
            Ok(_) => {
                self.stats.upserted += stats.upserted;
                self.stats.updated += stats.updated;
                self.stats.deleted += stats.deleted;
            }
            Err(_) => self.state.rollback_to(position),
        }
        if self.savepoints.is_empty() {
            self.state.stop_undo();
        }
        result.map(|value| (value, stats))
    }

    /// Mark the current state; [`Self::rollback_to`] returns to it.
    pub fn savepoint(&mut self) -> SavepointId {
        let id = SavepointId(self.next_savepoint);
        self.next_savepoint += 1;
        self.savepoints.push(Savepoint {
            id,
            undo: self.state.undo_position(),
            stats: self.stats.clone(),
        });
        id
    }

    /// Undo every write made after `savepoint`. The savepoint stays valid;
    /// savepoints created after it are released.
    pub fn rollback_to(&mut self, savepoint: SavepointId) -> Result<(), DbError> {
        let index = self
            .savepoints
            .iter()
            .position(|candidate| candidate.id == savepoint)
            .ok_or_else(|| DbError::InvalidQuery(format!("unknown savepoint {}", savepoint.0)))?;
        self.savepoints.truncate(index + 1);
        let savepoint = &self.savepoints[index];
        self.state.rollback_to(savepoint.undo);
        self.stats = savepoint.stats.clone();
        Ok(())
    }

    /// End the transaction without writing anything (like dropping it).
    pub fn rollback(self) {}

    /// Commit through `db`; see [`EmbeddedDb::commit_transaction`].
    pub fn commit<S: EntityStorage>(
        self,
        db: &mut EmbeddedDb<S>,
    ) -> Result<TransactionCommit, DbError> {
        db.commit_transaction(self)
    }
}

impl EmbeddedTransaction {
    /// Whether [`Self::prepare`] may run without exclusive access to the
    /// database: the transaction reads a consistent snapshot that does not
    /// call back into a lock around the database.
    pub(crate) fn prepares_without_lock(&self) -> bool {
        self.reader.snapshot.is_consistent()
    }

    /// Validate the transaction and derive its storage writes against its
    /// own snapshot, without touching the database (see
    /// [`compact::prepare_commit`] for why this needs no lock). `reply` builds
    /// the result from the validated changes. Commit the result with
    /// [`EmbeddedDb::commit_prepared`].
    fn prepare<R>(
        self,
        settings: crate::WriteSettings,
        require_bounded: bool,
        reply: impl FnOnce(
            &TxView<'_>,
            &crate::batch_return::ChangeSet,
            &BatchStats,
        ) -> Result<R, DbError>,
    ) -> Result<PreparedTransaction<R>, DbError> {
        let EmbeddedTransaction {
            catalog,
            reader,
            mut state,
            mut stats,
            ..
        } = self;
        let commit = prepare_commit(
            &mut TxView::new(&catalog.catalog, &reader, &mut state),
            &mut stats,
            settings,
            require_bounded,
            reply,
        )?;
        Ok(PreparedTransaction {
            revision: reader.revision,
            catalog_version: catalog.version,
            commit,
            stats,
            counts: state.counts,
        })
    }

    /// Prepare the transaction's commit (see [`EmbeddedDb::commit_transaction`]).
    pub(crate) fn prepare_commit(self) -> Result<PreparedTransaction<()>, DbError> {
        self.prepare(crate::WriteSettings::default(), false, |_, _, _| Ok(()))
    }

    /// Apply `batch` and prepare the transaction, replying in `returning`
    /// mode (see [`EmbeddedDb::execute_batch_returning`]).
    pub(crate) fn prepare_batch(
        mut self,
        batch: Batch,
        returning: &crate::BatchReturn,
        settings: crate::WriteSettings,
        require_bounded: bool,
    ) -> Result<PreparedTransaction<crate::BatchReply>, DbError> {
        self.execute_batch(batch)?;
        self.prepare(settings, require_bounded, |view, changes, stats| {
            compact::batch_reply(view, changes, stats, returning)
        })
    }

    /// Run `query` and prepare the transaction (see
    /// [`EmbeddedDb::insert_query`]).
    pub(crate) fn prepare_insert(
        mut self,
        query: InsertQuery,
    ) -> Result<PreparedTransaction<crate::InsertResult>, DbError> {
        let catalog = self.catalog.catalog.clone();
        let plan = InsertPlan::new(&catalog, query, |select| self.select(select))?;
        self.execute_batch(plan.batch.clone())?;
        let returning = plan.reply_mode();
        self.prepare(
            crate::WriteSettings::default(),
            false,
            |view, changes, stats| match compact::batch_reply(view, changes, stats, &returning)? {
                crate::BatchReply::Dataset(outcome) => plan.result(&catalog, &outcome.dataset),
                _ => plan.result(&catalog, &crate::Dataset::new()),
            },
        )
    }

    /// Run `query` and prepare the transaction (see
    /// [`EmbeddedDb::update_where_returning`]).
    pub(crate) fn prepare_update(
        mut self,
        query: UpdateQuery,
    ) -> Result<PreparedTransaction<crate::UpdateResult>, DbError> {
        let result = self.update_where(query)?;
        self.prepare(crate::WriteSettings::default(), false, |_, _, _| Ok(result))
    }

    /// Run `query` and prepare the transaction (see
    /// [`EmbeddedDb::delete_where_returning`]).
    pub(crate) fn prepare_delete(
        mut self,
        query: DeleteQuery,
    ) -> Result<PreparedTransaction<crate::DeleteResult>, DbError> {
        let result = self.delete_where(query)?;
        self.prepare(crate::WriteSettings::default(), false, |_, _, _| Ok(result))
    }
}

/// A validated transaction with its storage writes, ready to commit.
pub(crate) struct PreparedTransaction<R> {
    revision: Option<u64>,
    catalog_version: u64,
    commit: PreparedCommit<R>,
    stats: BatchStats,
    counts: ExecutionCounts,
}

/// Outcome of [`EmbeddedDb::commit_prepared`].
pub(crate) struct CommittedTransaction<R> {
    pub reply: R,
    /// Revision the commit created (the read revision when nothing was
    /// written).
    pub revision: Option<u64>,
    pub stats: BatchStats,
    pub metrics: crate::WriteMetrics,
}

/// `operation` addressing its collection by the catalog's collection name,
/// so every statement keys rows alike.
fn with_canonical_collection(catalog: &Catalog, mut operation: BatchOperation) -> BatchOperation {
    let (BatchOperation::Create { collection, .. }
    | BatchOperation::Upsert { collection, .. }
    | BatchOperation::DeleteById { collection, .. }
    | BatchOperation::DeleteByIds { collection, .. }
    | BatchOperation::Update { collection, .. }
    | BatchOperation::Delete { collection, .. }) = &mut operation;
    if let Some(schema) = catalog.collection_by_name(collection) {
        *collection = schema.name.clone();
    }
    operation
}

impl<S: EntityStorage> EmbeddedDb<S> {
    /// Begin an interactive transaction at the current committed state.
    ///
    /// Requires a storage with owned snapshots
    /// ([`EntityStorage::owned_snapshot`]) and commit-time conflict
    /// detection; otherwise fails with an `Unsupported` storage error.
    /// Snapshot isolation levels additionally require the snapshot to be
    /// consistent. Every level reads one snapshot and commits only if no
    /// other write landed since (see [`Self::commit_transaction`]).
    pub fn begin(&self, options: TransactionOptions) -> Result<EmbeddedTransaction, DbError> {
        let snapshot = self.storage.owned_snapshot()?.ok_or_else(|| {
            DbError::storage(
                StorageErrorKind::Unsupported,
                "interactive transactions require owned storage snapshots",
            )
        })?;
        self.begin_with_snapshot(options, snapshot)
    }

    /// Begin an interactive transaction reading `snapshot`, which must
    /// observe the current committed state.
    pub(crate) fn begin_with_snapshot(
        &self,
        options: TransactionOptions,
        snapshot: Arc<dyn EntityReadSnapshot>,
    ) -> Result<EmbeddedTransaction, DbError> {
        if options.concurrency == TransactionConcurrency::Mvcc
            && !self.storage.tx_capabilities().mvcc
        {
            return Err(DbError::InvalidQuery(
                "mvcc transaction requested but backend does not support mvcc".to_string(),
            ));
        }
        if options.isolation.requires_snapshot() && !snapshot.is_consistent() {
            return Err(snapshot_isolation_unsupported(options.isolation));
        }
        let catalog = self.catalog.snapshot();
        // Nothing commits while `self` is borrowed, so the storage is at the
        // snapshot's state; reading it directly keeps begin-time reads off
        // snapshots that call back into a lock around the database.
        let revision = if snapshot.is_consistent() {
            snapshot.revision()?
        } else {
            self.storage.current_revision()?
        };
        if let Some(reason) = compact::optimistic_unsupported(&self.storage, revision) {
            return Err(interactive_unsupported(reason));
        }
        if compact::reverse_references_unavailable(&catalog.catalog) {
            return Err(interactive_unsupported("reverse_references_unavailable"));
        }
        let storage = RevisionReader::new(&self.storage, revision)?;
        if compact::reverse_references_incomplete(&catalog.catalog, &storage)? {
            return Err(interactive_unsupported(
                "reverse_reference_backfill_incomplete",
            ));
        }
        let recursive_validation = match catalog.catalog.collection_by_name(validation::STATE) {
            Some(collection) => storage
                .get_entity(collection.lid, validation::ACTIVE)?
                .is_some(),
            None => false,
        };
        Ok(EmbeddedTransaction {
            options,
            catalog,
            reader: SnapshotReader { snapshot, revision },
            state: TxState::default(),
            stats: BatchStats {
                upserted: 0,
                updated: 0,
                deleted: 0,
            },
            savepoints: Vec::new(),
            next_savepoint: 0,
            recursive_validation,
        })
    }

    /// Validate and commit an interactive transaction.
    ///
    /// Runs the point write path's final-state validation (primary ids,
    /// unique indexes, references and the incoming references of deleted or
    /// retyped rows, cascades), derives index, reverse reference and
    /// relationship edge writes, and commits them only if the storage is
    /// still at the transaction's revision and the catalog unchanged.
    /// Otherwise fails with [`DbError::TransactionConflict`]: the
    /// transaction cannot be replayed automatically, so the caller retries
    /// (see [`Self::run_transaction`]).
    pub fn commit_transaction(
        &mut self,
        tx: EmbeddedTransaction,
    ) -> Result<TransactionCommit, DbError> {
        let EmbeddedTransaction {
            options,
            catalog,
            reader,
            mut state,
            mut stats,
            ..
        } = tx;
        let revision = reader.revision;
        drop(reader);
        if state.overlay().is_empty() {
            return Ok(TransactionCommit {
                revision,
                stats,
                metrics: crate::WriteMetrics::default(),
            });
        }
        self.execution_counts = compact::ExecutionCounts::default();
        if self.catalog.snapshot().version != catalog.version {
            return Err(DbError::TransactionConflict(
                "catalog changed during transaction".into(),
            ));
        }
        let reader = RevisionReader::for_isolation(&self.storage, revision, options.isolation)?;
        let prepared = prepare_commit(
            &mut TxView::new(&catalog.catalog, &reader, &mut state),
            &mut stats,
            crate::WriteSettings::default(),
            false,
            |_, _, _| Ok(()),
        );
        drop(reader);
        self.execution_counts = state.counts;
        let prepared = prepared?;
        let revision = self.commit_compact_ops(
            revision,
            catalog.version,
            &prepared.ops,
            crate::ChangeSource::Transaction,
            prepared.changes,
        )?;
        Ok(TransactionCommit {
            revision,
            stats,
            metrics: (&self.execution_counts).into(),
        })
    }

    /// Commit a transaction prepared by [`EmbeddedTransaction::prepare`]:
    /// the only step of an optimistic write that needs exclusive access.
    ///
    /// Commits only if the storage is still at the transaction's revision
    /// and the catalog unchanged; otherwise fails with a (retryable)
    /// [`DbError::TransactionConflict`]. Transactions that write nothing
    /// commit nothing.
    pub(crate) fn commit_prepared<R>(
        &mut self,
        prepared: PreparedTransaction<R>,
        source: crate::ChangeSource,
    ) -> Result<CommittedTransaction<R>, DbError> {
        let PreparedTransaction {
            revision,
            catalog_version,
            commit,
            stats,
            counts,
        } = prepared;
        self.execution_counts = counts;
        let revision = if commit.ops.is_empty() && commit.changes.is_empty() {
            revision
        } else {
            self.commit_compact_ops(
                revision,
                catalog_version,
                &commit.ops,
                source,
                commit.changes,
            )?
        };
        Ok(CommittedTransaction {
            reply: commit.reply,
            revision,
            stats,
            metrics: (&self.execution_counts).into(),
        })
    }

    /// Run `body` in an interactive transaction and commit it, re-running
    /// the whole transaction on conflicts per `options.conflict_policy` and
    /// `options.max_retries`.
    pub fn run_transaction<T>(
        &mut self,
        options: TransactionOptions,
        mut body: impl FnMut(&mut EmbeddedTransaction) -> Result<T, DbError>,
    ) -> Result<TransactionResult<T>, DbError> {
        run_with_transaction_retries(options, |_| {
            let mut tx = self.begin(options)?;
            let value = body(&mut tx)?;
            self.commit_transaction(tx)?;
            Ok(value)
        })
    }
}

fn interactive_unsupported(reason: &str) -> DbError {
    DbError::storage(
        StorageErrorKind::Unsupported,
        format!("interactive transactions are not supported by this storage ({reason})"),
    )
}
