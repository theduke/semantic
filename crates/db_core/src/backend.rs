use std::sync::Arc;

use async_trait::async_trait;
use semantic_data::value::{Object, Value};

use crate::catalog::{Catalog, CollectionKind, LocalCollectionId};
use crate::{
    Batch, BatchOutcome, DbError, DeleteQuery, LogicalPlan, MutationStats, PhysicalPlan,
    SelectQuery, UpdateQuery,
};

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct EntityRecord {
    pub id: String,
    pub collection: String,
    pub object: Object,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum QueryPlan {
    FullScan {
        collection: String,
    },
    IndexLookup {
        collection: String,
        index_name: String,
        field: String,
        value: Value,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccessPath {
    FullScan,
    IndexLookup {
        index_name: String,
        field: String,
        value: Value,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryExplain {
    pub logical: LogicalPlan,
    pub physical: PhysicalPlan,
    pub access_path: AccessPath,
}

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
