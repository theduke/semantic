use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use semantic_data::value::Object;
use semantic_db_core::BatchOutcome;
use semantic_db_core::catalog::{Catalog, CollectionKind, LocalCollectionId};
use semantic_db_kv::{
    Batch, DbError, DeleteQuery, EntityRecord, KvEngine, MutationStats, QueryExplain, QueryPlan,
    SelectQuery, UpdateQuery,
};

#[async_trait]
pub trait Backend: Send + Sync {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError>;

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError>;

    async fn insert(
        &self,
        collection: String,
        id: String,
        object: Object,
    ) -> std::result::Result<(), DbError>;

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError>;

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError>;

    async fn query(
        &self,
        collection: String,
        query: SelectQuery,
    ) -> std::result::Result<Vec<Object>, DbError>;

    async fn explain_query(
        &self,
        collection: String,
        query: SelectQuery,
    ) -> std::result::Result<QueryExplain, DbError>;

    async fn plan_query(
        &self,
        collection: String,
        query: SelectQuery,
    ) -> std::result::Result<QueryPlan, DbError>;

    async fn update_where(
        &self,
        collection: String,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError>;

    async fn delete_where(
        &self,
        collection: String,
        query: DeleteQuery,
    ) -> std::result::Result<usize, DbError>;

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError>;
}

pub struct Database {
    backend: Box<dyn Backend>,
}

impl Database {
    pub fn new(backend: impl Backend + 'static) -> Self {
        Self {
            backend: Box::new(backend),
        }
    }

    pub async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        self.backend.catalog().await
    }

    pub async fn create_collection(
        &self,
        name: impl Into<String>,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        self.backend.create_collection(name.into(), kind).await
    }

    pub async fn insert(
        &self,
        collection: impl Into<String>,
        id: impl Into<String>,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        self.backend
            .insert(collection.into(), id.into(), object)
            .await
    }

    pub async fn get(
        &self,
        collection: impl Into<String>,
        id: impl Into<String>,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        self.backend.get(collection.into(), id.into()).await
    }

    pub async fn delete(
        &self,
        collection: impl Into<String>,
        id: impl Into<String>,
    ) -> std::result::Result<(), DbError> {
        self.backend.delete(collection.into(), id.into()).await
    }

    pub async fn query(
        &self,
        collection: impl Into<String>,
        query: SelectQuery,
    ) -> std::result::Result<Vec<Object>, DbError> {
        self.backend.query(collection.into(), query).await
    }

    pub async fn explain_query(
        &self,
        collection: impl Into<String>,
        query: SelectQuery,
    ) -> std::result::Result<QueryExplain, DbError> {
        self.backend.explain_query(collection.into(), query).await
    }

    pub async fn plan_query(
        &self,
        collection: impl Into<String>,
        query: SelectQuery,
    ) -> std::result::Result<QueryPlan, DbError> {
        self.backend.plan_query(collection.into(), query).await
    }

    pub async fn update_where(
        &self,
        collection: impl Into<String>,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        self.backend.update_where(collection.into(), query).await
    }

    pub async fn delete_where(
        &self,
        collection: impl Into<String>,
        query: DeleteQuery,
    ) -> std::result::Result<usize, DbError> {
        self.backend.delete_where(collection.into(), query).await
    }

    pub async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        self.backend.execute_batch(batch).await
    }
}

pub struct KvBackend<E: KvEngine> {
    db: Mutex<semantic_db_kv::Database<E>>,
}

impl<E: KvEngine> KvBackend<E> {
    pub fn new(db: semantic_db_kv::Database<E>) -> Self {
        Self { db: Mutex::new(db) }
    }
}

#[async_trait]
impl<E: KvEngine> Backend for KvBackend<E> {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        let db = self.db.lock().await;
        Ok(db.catalog())
    }

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        let mut db = self.db.lock().await;
        db.create_collection(name, kind)
    }

    async fn insert(
        &self,
        collection: String,
        id: String,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        let mut db = self.db.lock().await;
        db.insert(&collection, id, object)
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        let db = self.db.lock().await;
        db.get(&collection, &id)
    }

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError> {
        let mut db = self.db.lock().await;
        db.delete(&collection, &id)
    }

    async fn query(
        &self,
        collection: String,
        query: SelectQuery,
    ) -> std::result::Result<Vec<Object>, DbError> {
        let db = self.db.lock().await;
        db.query(&collection, query)
    }

    async fn explain_query(
        &self,
        collection: String,
        query: SelectQuery,
    ) -> std::result::Result<QueryExplain, DbError> {
        let db = self.db.lock().await;
        db.explain_query(&collection, query)
    }

    async fn plan_query(
        &self,
        collection: String,
        query: SelectQuery,
    ) -> std::result::Result<QueryPlan, DbError> {
        let db = self.db.lock().await;
        db.plan_query(&collection, query)
    }

    async fn update_where(
        &self,
        collection: String,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        let mut db = self.db.lock().await;
        db.update_where(&collection, query)
    }

    async fn delete_where(
        &self,
        collection: String,
        query: DeleteQuery,
    ) -> std::result::Result<usize, DbError> {
        let mut db = self.db.lock().await;
        db.delete_where(&collection, query)
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        let mut db = self.db.lock().await;
        db.execute_batch(batch)
    }
}
