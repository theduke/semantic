use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use semantic_data::value::Object;
use semantic_db_core::catalog::{Catalog, CollectionKind, LocalCollectionId};
use semantic_db_core::{
    Backend, Batch, BatchOutcome, DbError, DeleteQuery, EntityRecord, MutationStats, Query,
    QueryExplain, QueryPlan, QueryResult, UpdateQuery,
};

use crate::{KvDb, KvEngine};

pub struct KvBackend<E: KvEngine> {
    db: RwLock<KvDb<E>>,
}

impl<E: KvEngine> KvBackend<E> {
    pub fn new(db: KvDb<E>) -> Self {
        Self {
            db: RwLock::new(db),
        }
    }
}

#[async_trait]
impl<E: KvEngine> Backend for KvBackend<E> {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        let db = self
            .db
            .read()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        Ok(db.catalog())
    }

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        let mut db = self
            .db
            .write()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        db.create_collection(name, kind)
    }

    async fn insert(
        &self,
        collection: String,
        id: String,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        let mut db = self
            .db
            .write()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        db.insert(&collection, id, object)
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        let db = self
            .db
            .read()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        db.get(&collection, &id)
    }

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError> {
        let mut db = self
            .db
            .write()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        db.delete(&collection, &id)
    }

    async fn query(&self, query: Query) -> std::result::Result<QueryResult, DbError> {
        match query {
            Query::Select(query) => {
                let db = self
                    .db
                    .read()
                    .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
                db.select(query).map(QueryResult::Select)
            }
            query => {
                let mut db = self
                    .db
                    .write()
                    .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
                db.query(query)
            }
        }
    }

    async fn explain_query(&self, query: Query) -> std::result::Result<QueryExplain, DbError> {
        let db = self
            .db
            .read()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        db.explain_query(query)
    }

    async fn plan_query(&self, query: Query) -> std::result::Result<QueryPlan, DbError> {
        let db = self
            .db
            .read()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        db.plan_query(query)
    }

    async fn update_where(
        &self,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        let mut db = self
            .db
            .write()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        db.update_where(query)
    }

    async fn delete_where(&self, query: DeleteQuery) -> std::result::Result<usize, DbError> {
        let mut db = self
            .db
            .write()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        db.delete_where(query)
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        let mut db = self
            .db
            .write()
            .map_err(|_| DbError::Storage("kv backend rwlock poisoned".to_string()))?;
        db.execute_batch(batch)
    }
}

#[cfg(test)]
mod tests {
    use semantic_db_core::Backend;

    use super::*;
    use crate::MemoryKvEngine;

    fn assert_backend_impl<T: Backend>() {}

    #[test]
    fn kv_backend_blanket_impl_compiles() {
        assert_backend_impl::<KvBackend<MemoryKvEngine>>();
    }
}
