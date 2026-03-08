use std::sync::Arc;

use async_trait::async_trait;
use semantic_data::query::{
    Batch as PublicBatch, DeleteQuery as PublicDeleteQuery, Query as PublicQuery,
    QueryInput as PublicQueryInput, SelectQuery as PublicSelectQuery,
    TextQueryFormat as PublicTextQueryFormat, UpdateQuery as PublicUpdateQuery,
};
use semantic_data::schema::{Package, RelationType};
use semantic_data::value::{Object, Value};

use crate::catalog::{Catalog, CollectionKind, LocalCollectionId};
use crate::{
    Batch, BatchOutcome, DbError, DdlBatch, DdlOutcome, DeleteQuery, LogicalPlan, MutationStats,
    PackageRegistrationOutcome, PhysicalPlan, Query, QueryResult, SqlDialectKind, TextQueryFormat,
    TextQueryInput, UpdateQuery, prql, sql,
};

pub const DEFAULT_COLLECTION: &str = "entities";
pub const ALL_COLLECTION_ALIAS: &str = "all";

pub fn is_all_collection_alias(name: &str) -> bool {
    name.eq_ignore_ascii_case(ALL_COLLECTION_ALIAS)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionInput {
    Named(String),
    Default,
}

impl CollectionInput {
    pub fn into_collection(self) -> String {
        match self {
            Self::Named(name) => name,
            Self::Default => DEFAULT_COLLECTION.to_string(),
        }
    }
}

impl From<String> for CollectionInput {
    fn from(value: String) -> Self {
        Self::Named(value)
    }
}

impl From<&str> for CollectionInput {
    fn from(value: &str) -> Self {
        Self::Named(value.to_string())
    }
}

impl From<Option<String>> for CollectionInput {
    fn from(value: Option<String>) -> Self {
        match value {
            Some(value) => Self::Named(value),
            None => Self::Default,
        }
    }
}

impl From<Option<&str>> for CollectionInput {
    fn from(value: Option<&str>) -> Self {
        match value {
            Some(value) => Self::Named(value.to_string()),
            None => Self::Default,
        }
    }
}

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

    async fn execute_ddl(&self, ddl: DdlBatch) -> std::result::Result<DdlOutcome, DbError>;

    async fn upsert_package(
        &self,
        package: Package,
    ) -> std::result::Result<PackageRegistrationOutcome, DbError>;

    async fn upsert_relationship(
        &self,
        relationship: RelationType,
    ) -> std::result::Result<(), DbError>;

    async fn delete_relationship(&self, id: String) -> std::result::Result<(), DbError>;

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

    async fn query(&self, query: TextQueryInput) -> std::result::Result<QueryResult, DbError>;

    fn sql_dialect(&self) -> SqlDialectKind {
        SqlDialectKind::Generic
    }

    fn supported_text_query_formats(&self) -> Vec<TextQueryFormat> {
        Vec::from([
            #[cfg(feature = "sql")]
            TextQueryFormat::Sql,
            #[cfg(feature = "prql")]
            TextQueryFormat::Prql,
        ])
    }

    async fn parse_sql_query(
        &self,
        sql_query: &str,
    ) -> std::result::Result<sql::ParsedSqlQuery, DbError> {
        sql::parse_sql_query(sql_query, self.sql_dialect())
            .map_err(|err| DbError::InvalidQuery(err.to_string()))
    }

    async fn parse_prql_query(
        &self,
        prql_query: &str,
    ) -> std::result::Result<prql::ParsedPrqlQuery, DbError> {
        prql::parse_prql_query(prql_query, self.sql_dialect())
            .map_err(|err| DbError::InvalidQuery(err.to_string()))
    }

    async fn parse_text_query(
        &self,
        format: TextQueryFormat,
        query: &str,
    ) -> std::result::Result<Query, DbError> {
        match format {
            TextQueryFormat::Sql => Ok(self.parse_sql_query(query).await?.query),
            TextQueryFormat::Prql => Ok(self.parse_prql_query(query).await?.query),
        }
    }

    async fn explain(&self, query: TextQueryInput) -> std::result::Result<QueryExplain, DbError>;

    async fn plan(&self, query: TextQueryInput) -> std::result::Result<QueryPlan, DbError>;

    fn query_to_sql(&self, query: &Query) -> std::result::Result<String, DbError> {
        sql::query_to_sql(query).map_err(|err| DbError::InvalidQuery(err.to_string()))
    }

    async fn update_where(
        &self,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        match self
            .query(TextQueryInput::Ast(Query::Update(query)))
            .await?
        {
            QueryResult::Update(result) => Ok(result.stats),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-update result for update query".to_string(),
            )),
        }
    }

    async fn delete_where(&self, query: DeleteQuery) -> std::result::Result<usize, DbError> {
        match self
            .query(TextQueryInput::Ast(Query::Delete(query)))
            .await?
        {
            QueryResult::Delete(result) => Ok(result.deleted),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-delete result for delete query".to_string(),
            )),
        }
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError>;
}

pub struct Db {
    backend: Box<dyn Backend>,
}

impl Db {
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

    pub async fn execute_ddl(&self, ddl: DdlBatch) -> std::result::Result<DdlOutcome, DbError> {
        self.backend.execute_ddl(ddl).await
    }

    pub async fn upsert_package(
        &self,
        package: Package,
    ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
        self.backend.upsert_package(package).await
    }

    pub async fn upsert_relationship(
        &self,
        relationship: RelationType,
    ) -> std::result::Result<(), DbError> {
        self.backend.upsert_relationship(relationship).await
    }

    pub async fn delete_relationship(
        &self,
        id: impl Into<String>,
    ) -> std::result::Result<(), DbError> {
        self.backend.delete_relationship(id.into()).await
    }

    pub async fn insert(
        &self,
        collection: impl Into<CollectionInput>,
        id: impl Into<String>,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        let collection = collection.into().into_collection();
        self.backend.insert(collection, id.into(), object).await
    }

    pub async fn get(
        &self,
        collection: impl Into<CollectionInput>,
        id: impl Into<String>,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        self.backend
            .get(collection.into().into_collection(), id.into())
            .await
    }

    pub async fn delete(
        &self,
        collection: impl Into<CollectionInput>,
        id: impl Into<String>,
    ) -> std::result::Result<(), DbError> {
        self.backend
            .delete(collection.into().into_collection(), id.into())
            .await
    }

    pub fn supported_text_query_formats(&self) -> Vec<PublicTextQueryFormat> {
        self.backend
            .supported_text_query_formats()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    pub async fn query(
        &self,
        query: impl Into<PublicQueryInput>,
    ) -> std::result::Result<QueryResult, DbError> {
        self.backend.query(query.into().into()).await
    }

    pub async fn select(
        &self,
        query: PublicSelectQuery,
    ) -> std::result::Result<Vec<Object>, DbError> {
        match self
            .backend
            .query(TextQueryInput::Ast(Query::Select(query.into())))
            .await?
        {
            QueryResult::Select(rows) => Ok(rows),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-select result for select query".to_string(),
            )),
        }
    }

    pub async fn explain(
        &self,
        query: impl Into<PublicQueryInput>,
    ) -> std::result::Result<QueryExplain, DbError> {
        self.backend.explain(query.into().into()).await
    }

    pub async fn plan(
        &self,
        query: impl Into<PublicQueryInput>,
    ) -> std::result::Result<QueryPlan, DbError> {
        self.backend.plan(query.into().into()).await
    }

    pub fn query_to_sql(&self, query: &PublicQuery) -> std::result::Result<String, DbError> {
        self.backend.query_to_sql(&query.clone().into())
    }

    pub async fn update_where(
        &self,
        query: PublicUpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        match self
            .backend
            .query(TextQueryInput::Ast(Query::Update(query.into())))
            .await?
        {
            QueryResult::Update(result) => Ok(result.stats),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-update result for update query".to_string(),
            )),
        }
    }

    pub async fn delete_where(
        &self,
        query: PublicDeleteQuery,
    ) -> std::result::Result<usize, DbError> {
        match self
            .backend
            .query(TextQueryInput::Ast(Query::Delete(query.into())))
            .await?
        {
            QueryResult::Delete(result) => Ok(result.deleted),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-delete result for delete query".to_string(),
            )),
        }
    }

    pub async fn execute_batch(
        &self,
        batch: PublicBatch,
    ) -> std::result::Result<BatchOutcome, DbError> {
        self.backend.execute_batch(batch.into()).await
    }
}
