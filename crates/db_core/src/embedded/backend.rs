use std::sync::{Arc, RwLock};

use crate::catalog::{Catalog, CollectionKind, LocalCollectionId, SharedCatalog};
use crate::{
    AsyncRuntime, Backend, Batch, BatchOutcome, DbError, DdlBatch, DdlOutcome, DeleteQuery,
    EntityRecord, MutationStats, PackageRegistrationOutcome, Query, QueryExplain, QueryPlan,
    QueryResult, StorageErrorKind, TextQueryInput, UpdateQuery, spawn_blocking_on,
};
use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::{SinkExt as _, StreamExt as _};
use semantic_data::schema::{Package, RelationType};
use semantic_data::value::Object;

use crate::embedded::{DbReader, EmbeddedDb, EntityStorage};

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
    runtime: Arc<dyn AsyncRuntime>,
}

impl<S: EntityStorage> EmbeddedBackend<S> {
    pub fn new(db: EmbeddedDb<S>) -> Self {
        Self::with_runtime(db, default_runtime())
    }

    pub fn with_runtime(db: EmbeddedDb<S>, runtime: Arc<dyn AsyncRuntime>) -> Self {
        Self {
            catalog: db.shared_catalog().clone(),
            db: Arc::new(RwLock::new(db)),
            runtime,
        }
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

fn lock_poisoned_error() -> DbError {
    DbError::storage(
        StorageErrorKind::InvalidState,
        "embedded backend rwlock poisoned",
    )
}

#[async_trait]
impl<S: EntityStorage> Backend for EmbeddedBackend<S> {
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

    #[cfg(feature = "tokio")]
    #[test]
    fn kv_backend_runtime_constructors_compile() {
        let db = EmbeddedDb::new(MemoryEntityStorage::new());
        let _ = EmbeddedBackend::with_runtime(db, Arc::new(crate::TokioAsyncRuntime));
    }
}
