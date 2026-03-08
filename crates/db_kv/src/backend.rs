use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use semantic_data::value::Object;
use semantic_db_core::catalog::{Catalog, CollectionKind, LocalCollectionId};
use semantic_db_core::{
    Backend, Batch, BatchOutcome, DbError, DeleteQuery, EntityRecord, MutationStats, Query,
    QueryExplain, QueryPlan, QueryResult, UpdateQuery,
};

use crate::{KvDb, KvEngine};

pub struct KvBackend<E: KvEngine> {
    db: Mutex<KvDb<E>>,
}

impl<E: KvEngine> KvBackend<E> {
    pub fn new(db: KvDb<E>) -> Self {
        Self { db: Mutex::new(db) }
    }

    fn lock_db(&self) -> std::result::Result<std::sync::MutexGuard<'_, KvDb<E>>, DbError> {
        self.db
            .lock()
            .map_err(|_| DbError::Storage("kv backend mutex poisoned".to_string()))
    }
}

#[async_trait]
impl<E: KvEngine> Backend for KvBackend<E> {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        let db = self.lock_db()?;
        Ok(db.catalog())
    }

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        let mut db = self.lock_db()?;
        db.create_collection(name, kind)
    }

    async fn insert(
        &self,
        collection: String,
        id: String,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        let mut db = self.lock_db()?;
        db.insert(&collection, id, object)
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        let db = self.lock_db()?;
        db.get(&collection, &id)
    }

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError> {
        let mut db = self.lock_db()?;
        db.delete(&collection, &id)
    }

    async fn query(&self, query: Query) -> std::result::Result<QueryResult, DbError> {
        let mut db = self.lock_db()?;
        db.query(query)
    }

    async fn explain_query(&self, query: Query) -> std::result::Result<QueryExplain, DbError> {
        let db = self.lock_db()?;
        db.explain_query(query)
    }

    async fn plan_query(&self, query: Query) -> std::result::Result<QueryPlan, DbError> {
        let db = self.lock_db()?;
        db.plan_query(query)
    }

    async fn update_where(
        &self,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        let mut db = self.lock_db()?;
        db.update_where(query)
    }

    async fn delete_where(&self, query: DeleteQuery) -> std::result::Result<usize, DbError> {
        let mut db = self.lock_db()?;
        db.delete_where(query)
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        let mut db = self.lock_db()?;
        db.execute_batch(batch)
    }
}
