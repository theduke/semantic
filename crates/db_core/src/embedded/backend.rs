use std::sync::{Arc, RwLock};

use crate::catalog::{Catalog, CollectionKind, LocalCollectionId};
use crate::{
    AsyncRuntime, Backend, Batch, BatchOutcome, DbError, DdlBatch, DdlOutcome, DeleteQuery,
    EntityRecord, MutationStats, PackageRegistrationOutcome, Query, QueryExplain, QueryPlan,
    QueryResult, TextQueryInput, UpdateQuery, spawn_blocking_on,
};
use async_trait::async_trait;
use futures::{SinkExt as _, StreamExt as _};
use semantic_data::schema::{Package, RelationType};
use semantic_data::value::Object;

use crate::embedded::{EmbeddedDb, EntityStorage};

pub struct EmbeddedBackend<S: EntityStorage> {
    db: Arc<RwLock<EmbeddedDb<S>>>,
    runtime: Arc<dyn AsyncRuntime>,
}

impl<S: EntityStorage> EmbeddedBackend<S> {
    pub fn new(db: EmbeddedDb<S>) -> Self {
        Self::with_runtime(db, default_runtime())
    }

    pub fn with_runtime(db: EmbeddedDb<S>, runtime: Arc<dyn AsyncRuntime>) -> Self {
        Self {
            db: Arc::new(RwLock::new(db)),
            runtime,
        }
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
    DbError::Storage("embedded backend rwlock poisoned".to_string())
}

#[async_trait]
impl<S: EntityStorage> Backend for EmbeddedBackend<S> {
    async fn validation_preflight(&self) -> Result<Vec<crate::ValidationViolation>, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            db.read()
                .map_err(|_| lock_poisoned_error())?
                .validation_preflight()
        })
        .await
    }

    async fn activate_validation(&self) -> Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            db.write()
                .map_err(|_| lock_poisoned_error())?
                .activate_validation()
        })
        .await
    }
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            Ok(db.catalog())
        })
        .await
    }

    async fn scan_entities(&self) -> Result<crate::EntityStream, DbError> {
        const CHANNEL_CAPACITY: usize = 16;
        let db = Arc::clone(&self.db);
        let (mut sender, receiver) = futures::channel::mpsc::channel(CHANNEL_CAPACITY);
        std::thread::Builder::new()
            .name("semantic-entity-export".into())
            .spawn(move || {
                let db = match db.read() {
                    Ok(db) => db,
                    Err(_) => {
                        let _ =
                            futures::executor::block_on(sender.send(Err(lock_poisoned_error())));
                        return;
                    }
                };
                db.scan_entities_with(|item| {
                    futures::executor::block_on(sender.send(item)).is_ok()
                });
            })
            .map_err(|error| DbError::Storage(format!("spawn entity export thread: {error}")))?;
        Ok(receiver.boxed())
    }

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.create_collection(name, kind)
        })
        .await
    }

    async fn execute_ddl(&self, ddl: DdlBatch) -> std::result::Result<DdlOutcome, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.transact_ddl(ddl)
        })
        .await
    }

    async fn upsert_package(
        &self,
        package: Package,
    ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.upsert_package(package)
        })
        .await
    }

    async fn upsert_relationship(
        &self,
        relationship: RelationType,
    ) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.upsert_relationship(relationship)
        })
        .await
    }

    async fn delete_relationship(&self, id: String) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.delete_relationship(&id)
        })
        .await
    }

    async fn insert(
        &self,
        collection: String,
        id: String,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.insert(&collection, id, object)
        })
        .await
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            db.get(&collection, &id)
        })
        .await
    }

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.delete(&collection, &id)
        })
        .await
    }

    async fn query(&self, query: TextQueryInput) -> std::result::Result<QueryResult, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text {
                format,
                query,
                params,
            } => {
                self.parse_text_query_with_params(format, &query, &params)
                    .await?
            }
        };
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || match query {
            Query::Select(query) => {
                let db = db.read().map_err(|_| lock_poisoned_error())?;
                db.select(query).map(QueryResult::Select)
            }
            query => {
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                db.query(query)
            }
        })
        .await
    }

    async fn explain(&self, query: TextQueryInput) -> std::result::Result<QueryExplain, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text {
                format,
                query,
                params,
            } => {
                self.parse_text_query_with_params(format, &query, &params)
                    .await?
            }
        };
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            db.explain_query(query)
        })
        .await
    }

    async fn plan(&self, query: TextQueryInput) -> std::result::Result<QueryPlan, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text {
                format,
                query,
                params,
            } => {
                self.parse_text_query_with_params(format, &query, &params)
                    .await?
            }
        };
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            db.plan_query(query)
        })
        .await
    }

    async fn update_where(
        &self,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.update_where(query)
        })
        .await
    }

    async fn delete_where(&self, query: DeleteQuery) -> std::result::Result<usize, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.delete_where(query)
        })
        .await
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.execute_batch(batch)
        })
        .await
    }

    async fn execute_batch_with_settings(
        &self,
        batch: Batch,
        settings: crate::WriteSettings,
    ) -> Result<BatchOutcome, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            db.write()
                .map_err(|_| lock_poisoned_error())?
                .execute_batch_with_settings(batch, settings)
        })
        .await
    }

    async fn execute_batch_returning(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
    ) -> Result<crate::BatchReply, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            db.write()
                .map_err(|_| lock_poisoned_error())?
                .execute_batch_returning(batch, returning)
        })
        .await
    }

    async fn execute_batch_returning_with_settings(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            db.write()
                .map_err(|_| lock_poisoned_error())?
                .execute_batch_returning_with_settings(batch, returning, settings)
        })
        .await
    }

    async fn execute_batch_returning_bounded_with_settings(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            db.write()
                .map_err(|_| lock_poisoned_error())?
                .execute_batch_returning_bounded_with_settings(batch, returning, settings)
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

    #[cfg(feature = "tokio")]
    #[test]
    fn kv_backend_runtime_constructors_compile() {
        let db = EmbeddedDb::new(MemoryEntityStorage::new());
        let _ = EmbeddedBackend::with_runtime(db, Arc::new(crate::TokioAsyncRuntime));
    }
}
