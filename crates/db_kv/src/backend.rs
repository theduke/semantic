use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use semantic_data::schema::RelationType;
use semantic_data::value::Object;
use semantic_db_core::catalog::{Catalog, CollectionKind, LocalCollectionId};
use semantic_db_core::{
    Backend, Batch, BatchOutcome, DbError, DdlBatch, DdlOutcome, DeleteQuery, EntityRecord,
    MutationStats, Query, QueryExplain, QueryPlan, QueryResult, TextQueryInput, UpdateQuery,
};

use crate::{DefaultKvBackendSpawner, KvBackendSpawner, KvDb, KvEngine};

pub struct KvBackend<E: KvEngine, S: KvBackendSpawner = DefaultKvBackendSpawner> {
    db: Arc<RwLock<KvDb<E>>>,
    spawner: S,
}

impl<E: KvEngine> KvBackend<E, DefaultKvBackendSpawner> {
    pub fn new(db: KvDb<E>) -> Self {
        Self::with_spawner(db, DefaultKvBackendSpawner::default())
    }
}

impl<E: KvEngine, S: KvBackendSpawner> KvBackend<E, S> {
    pub fn with_spawner(db: KvDb<E>, spawner: S) -> Self {
        Self {
            db: Arc::new(RwLock::new(db)),
            spawner,
        }
    }
}

fn lock_poisoned_error() -> DbError {
    DbError::Storage("kv backend rwlock poisoned".to_string())
}

#[async_trait]
impl<E: KvEngine, S: KvBackendSpawner> Backend for KvBackend<E, S> {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
                let db = db.read().map_err(|_| lock_poisoned_error())?;
                Ok(db.catalog())
            })
            .await
    }

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                db.create_collection(name, kind)
            })
            .await
    }

    async fn execute_ddl(&self, ddl: DdlBatch) -> std::result::Result<DdlOutcome, DbError> {
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                db.transact_ddl(ddl)
            })
            .await
    }

    async fn upsert_relationship(
        &self,
        relationship: RelationType,
    ) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                db.upsert_relationship(relationship)
            })
            .await
    }

    async fn delete_relationship(&self, id: String) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
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
        self.spawner
            .spawn_blocking(move || {
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
        self.spawner
            .spawn_blocking(move || {
                let db = db.read().map_err(|_| lock_poisoned_error())?;
                db.get(&collection, &id)
            })
            .await
    }

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                db.delete(&collection, &id)
            })
            .await
    }

    async fn query(&self, query: TextQueryInput) -> std::result::Result<QueryResult, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text { format, query } => self.parse_text_query(format, &query).await?,
        };
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || match query {
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
            TextQueryInput::Text { format, query } => self.parse_text_query(format, &query).await?,
        };
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
                let db = db.read().map_err(|_| lock_poisoned_error())?;
                db.explain_query(query)
            })
            .await
    }

    async fn plan(&self, query: TextQueryInput) -> std::result::Result<QueryPlan, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text { format, query } => self.parse_text_query(format, &query).await?,
        };
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
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
        self.spawner
            .spawn_blocking(move || {
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                db.update_where(query)
            })
            .await
    }

    async fn delete_where(&self, query: DeleteQuery) -> std::result::Result<usize, DbError> {
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                db.delete_where(query)
            })
            .await
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        let db = Arc::clone(&self.db);
        self.spawner
            .spawn_blocking(move || {
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                db.execute_batch(batch)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use semantic_db_core::Backend;

    use super::*;
    use crate::{InlineSpawner, MemoryKvEngine};

    fn assert_backend_impl<T: Backend>() {}

    #[test]
    fn kv_backend_blanket_impl_compiles() {
        assert_backend_impl::<KvBackend<MemoryKvEngine>>();
        assert_backend_impl::<KvBackend<MemoryKvEngine, InlineSpawner>>();
    }

    #[cfg(feature = "tokio")]
    #[test]
    fn kv_backend_tokio_spawner_impl_compiles() {
        assert_backend_impl::<KvBackend<MemoryKvEngine, crate::TokioSpawner>>();
    }
}
