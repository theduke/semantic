use std::sync::Arc;

use async_trait::async_trait;
use semantic_data::query as public_query;
use semantic_data::schema::{DbOpenMode, Package};
use semantic_data::value::Object;
use semantic_db_core::PackageRegistrationOutcome;
use semantic_db_core::catalog::Catalog;
use semantic_db_core::{
    Batch, BatchOperation, BatchOutcome, Db, DbError, EntityRecord, QueryResult, TextQueryInput,
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

    async fn upsert_package(
        &self,
        package: Package,
    ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
        let _ = package;
        Err(DbError::InvalidQuery(
            "package registration is not exposed by this app Db adapter".to_string(),
        ))
    }
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
        self.execute_batch(public_batch_from_core(batch)?).await
    }

    async fn upsert_package(
        &self,
        package: Package,
    ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
        self.upsert_package(package).await
    }
}

fn public_batch_from_core(batch: Batch) -> std::result::Result<public_query::Batch, DbError> {
    let mut operations = Vec::with_capacity(batch.operations.len());
    for operation in batch.operations {
        let operation = match operation {
            BatchOperation::Upsert {
                collection,
                id,
                object,
            } => public_query::BatchOperation::Upsert {
                collection,
                id,
                object,
            },
            BatchOperation::DeleteById { collection, id } => {
                public_query::BatchOperation::DeleteById { collection, id }
            }
            BatchOperation::DeleteByIds { collection, ids } => {
                public_query::BatchOperation::DeleteByIds { collection, ids }
            }
            BatchOperation::Update { .. } | BatchOperation::Delete { .. } => {
                return Err(DbError::InvalidQuery(
                    "query-based batch operations are not exposed by the app Db adapter yet"
                        .to_string(),
                ));
            }
        };
        operations.push(operation);
    }
    Ok(public_query::Batch { operations })
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

/// A database backend with a typed configuration and scheme-specific URI parser.
///
/// Unlike URLs, database URIs may have opaque payloads such as `log:<blob>`.
/// Implementations receive the complete URI and own its configuration syntax.
/// The blanket `DbProvider` implementation provides dynamic URI dispatch.
#[async_trait]
pub trait DbBackend: Send + Sync + 'static {
    type Config: Send + Sync;

    fn scheme(&self) -> &str;

    fn parse_uri(&self, uri: &str) -> Result<Self::Config, AppError>;

    async fn open_config(
        &self,
        config: Self::Config,
        mode: DbOpenMode,
        principal: &Principal,
    ) -> Result<Arc<dyn SemanticDb>, AppError>;
}

#[async_trait]
impl<B: DbBackend> DbProvider for B {
    fn scheme(&self) -> &str {
        DbBackend::scheme(self)
    }

    async fn open(
        &self,
        request: DbOpenRequest,
        principal: &Principal,
    ) -> Result<Arc<dyn SemanticDb>, AppError> {
        self.open_config(self.parse_uri(&request.uri)?, request.mode, principal)
            .await
    }
}
