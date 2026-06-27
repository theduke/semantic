use std::sync::Arc;

use async_trait::async_trait;
use semantic_data::schema::DbOpenMode;
use semantic_data::value::Object;
use semantic_db_core::catalog::Catalog;
use semantic_db_core::{
    Batch, BatchOutcome, Db, DbError, EntityRecord, QueryResult, TextQueryInput,
};

use crate::{AppError, Principal};

#[async_trait]
pub trait SemanticDb: Send + Sync + 'static {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError>;

    async fn query(&self, query: TextQueryInput) -> std::result::Result<QueryResult, DbError>;

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError>;

    async fn insert(
        &self,
        collection: String,
        id: String,
        object: Object,
    ) -> std::result::Result<(), DbError>;

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError>;

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError>;
}

#[async_trait]
impl SemanticDb for Db {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        self.catalog().await
    }

    async fn query(&self, query: TextQueryInput) -> std::result::Result<QueryResult, DbError> {
        match query {
            TextQueryInput::Text { format, query } => {
                let format = match format {
                    semantic_db_core::TextQueryFormat::Sql => {
                        semantic_data::query::TextQueryFormat::Sql
                    }
                    semantic_db_core::TextQueryFormat::Prql => {
                        semantic_data::query::TextQueryFormat::Prql
                    }
                };
                self.query(semantic_data::query::QueryInput::Text { format, query })
                    .await
            }
            TextQueryInput::Ast(_) => Err(DbError::InvalidQuery(
                "core AST query execution is not exposed by the app Db adapter yet".to_string(),
            )),
        }
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        self.get(collection, id).await
    }

    async fn insert(
        &self,
        collection: String,
        id: String,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        self.insert(collection, id, object).await
    }

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError> {
        self.delete(collection, id).await
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        let _ = batch;
        Err(DbError::InvalidQuery(
            "core batch execution is not exposed by the app Db adapter yet".to_string(),
        ))
    }
}

#[derive(Clone, Debug)]
pub struct DbOpenRequest {
    pub uri: String,
    pub mode: DbOpenMode,
}

#[async_trait]
pub trait DbProvider: Send + Sync + 'static {
    fn scheme(&self) -> &str;

    async fn open(
        &self,
        request: DbOpenRequest,
        principal: &Principal,
    ) -> std::result::Result<Arc<dyn SemanticDb>, AppError>;
}
