use std::sync::Arc;

use async_trait::async_trait;
use semantic_data::value::{Object, Value};

use crate::catalog::{Catalog, CollectionKind, LocalCollectionId};
use crate::{
    Batch, BatchOutcome, DbError, DeleteQuery, LogicalPlan, MutationStats, PhysicalPlan, Query,
    QueryResult, SelectQuery, SqlDialectKind, TextQueryFormat, TextQueryInput, UpdateQuery, prql,
    sql,
};

pub const DEFAULT_COLLECTION: &str = "entities";

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

    async fn query(&self, query: Query) -> std::result::Result<QueryResult, DbError>;

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

    async fn query_input(
        &self,
        query: TextQueryInput,
    ) -> std::result::Result<QueryResult, DbError> {
        match query {
            TextQueryInput::Ast(query) => self.query(query).await,
            TextQueryInput::Text { format, query } => {
                let parsed = self.parse_text_query(format, &query).await?;
                self.query(parsed).await
            }
        }
    }

    async fn query_sql(&self, sql_query: String) -> std::result::Result<QueryResult, DbError> {
        self.query_text(TextQueryFormat::Sql, sql_query).await
    }

    async fn query_prql(&self, prql_query: String) -> std::result::Result<QueryResult, DbError> {
        self.query_text(TextQueryFormat::Prql, prql_query).await
    }

    async fn query_text(
        &self,
        format: TextQueryFormat,
        query: String,
    ) -> std::result::Result<QueryResult, DbError> {
        let parsed = self.parse_text_query(format, &query).await?;
        self.query(parsed).await
    }

    async fn explain_query(&self, query: Query) -> std::result::Result<QueryExplain, DbError>;

    async fn explain_query_input(
        &self,
        query: TextQueryInput,
    ) -> std::result::Result<QueryExplain, DbError> {
        match query {
            TextQueryInput::Ast(query) => self.explain_query(query).await,
            TextQueryInput::Text { format, query } => {
                let parsed = self.parse_text_query(format, &query).await?;
                self.explain_query(parsed).await
            }
        }
    }

    async fn explain_query_sql(
        &self,
        sql_query: String,
    ) -> std::result::Result<QueryExplain, DbError> {
        self.explain_query_text(TextQueryFormat::Sql, sql_query)
            .await
    }

    async fn explain_query_prql(
        &self,
        prql_query: String,
    ) -> std::result::Result<QueryExplain, DbError> {
        self.explain_query_text(TextQueryFormat::Prql, prql_query)
            .await
    }

    async fn explain_query_text(
        &self,
        format: TextQueryFormat,
        query: String,
    ) -> std::result::Result<QueryExplain, DbError> {
        let parsed = self.parse_text_query(format, &query).await?;
        self.explain_query(parsed).await
    }

    async fn plan_query(&self, query: Query) -> std::result::Result<QueryPlan, DbError>;

    async fn plan_query_input(
        &self,
        query: TextQueryInput,
    ) -> std::result::Result<QueryPlan, DbError> {
        match query {
            TextQueryInput::Ast(query) => self.plan_query(query).await,
            TextQueryInput::Text { format, query } => {
                let parsed = self.parse_text_query(format, &query).await?;
                self.plan_query(parsed).await
            }
        }
    }

    async fn plan_query_sql(&self, sql_query: String) -> std::result::Result<QueryPlan, DbError> {
        self.plan_query_text(TextQueryFormat::Sql, sql_query).await
    }

    async fn plan_query_prql(&self, prql_query: String) -> std::result::Result<QueryPlan, DbError> {
        self.plan_query_text(TextQueryFormat::Prql, prql_query)
            .await
    }

    async fn plan_query_text(
        &self,
        format: TextQueryFormat,
        query: String,
    ) -> std::result::Result<QueryPlan, DbError> {
        let parsed = self.parse_text_query(format, &query).await?;
        self.plan_query(parsed).await
    }

    fn query_to_sql(&self, query: &Query) -> std::result::Result<String, DbError> {
        sql::query_to_sql(query).map_err(|err| DbError::InvalidQuery(err.to_string()))
    }

    async fn update_where(
        &self,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        match self.query(Query::Update(query)).await? {
            QueryResult::Update(result) => Ok(result.stats),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-update result for update query".to_string(),
            )),
        }
    }

    async fn delete_where(&self, query: DeleteQuery) -> std::result::Result<usize, DbError> {
        match self.query(Query::Delete(query)).await? {
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

    pub fn supported_text_query_formats(&self) -> Vec<TextQueryFormat> {
        self.backend.supported_text_query_formats()
    }

    pub async fn query(&self, query: Query) -> std::result::Result<QueryResult, DbError> {
        self.backend.query_input(TextQueryInput::Ast(query)).await
    }

    pub async fn query_input(
        &self,
        query: impl Into<TextQueryInput>,
    ) -> std::result::Result<QueryResult, DbError> {
        self.backend.query_input(query.into()).await
    }

    pub async fn query_sql(
        &self,
        sql_query: impl Into<String>,
    ) -> std::result::Result<QueryResult, DbError> {
        self.backend.query_sql(sql_query.into()).await
    }

    pub async fn query_prql(
        &self,
        prql_query: impl Into<String>,
    ) -> std::result::Result<QueryResult, DbError> {
        self.backend.query_prql(prql_query.into()).await
    }

    pub async fn query_text(
        &self,
        format: TextQueryFormat,
        query: impl Into<String>,
    ) -> std::result::Result<QueryResult, DbError> {
        self.backend.query_text(format, query.into()).await
    }

    pub async fn select(&self, query: SelectQuery) -> std::result::Result<Vec<Object>, DbError> {
        match self.backend.query(Query::Select(query)).await? {
            QueryResult::Select(rows) => Ok(rows),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-select result for select query".to_string(),
            )),
        }
    }

    pub async fn explain_query(&self, query: Query) -> std::result::Result<QueryExplain, DbError> {
        self.backend
            .explain_query_input(TextQueryInput::Ast(query))
            .await
    }

    pub async fn explain_query_input(
        &self,
        query: impl Into<TextQueryInput>,
    ) -> std::result::Result<QueryExplain, DbError> {
        self.backend.explain_query_input(query.into()).await
    }

    pub async fn explain_query_sql(
        &self,
        sql_query: impl Into<String>,
    ) -> std::result::Result<QueryExplain, DbError> {
        self.backend.explain_query_sql(sql_query.into()).await
    }

    pub async fn explain_query_prql(
        &self,
        prql_query: impl Into<String>,
    ) -> std::result::Result<QueryExplain, DbError> {
        self.backend.explain_query_prql(prql_query.into()).await
    }

    pub async fn explain_query_text(
        &self,
        format: TextQueryFormat,
        query: impl Into<String>,
    ) -> std::result::Result<QueryExplain, DbError> {
        self.backend.explain_query_text(format, query.into()).await
    }

    pub async fn plan_query(&self, query: Query) -> std::result::Result<QueryPlan, DbError> {
        self.backend
            .plan_query_input(TextQueryInput::Ast(query))
            .await
    }

    pub async fn plan_query_input(
        &self,
        query: impl Into<TextQueryInput>,
    ) -> std::result::Result<QueryPlan, DbError> {
        self.backend.plan_query_input(query.into()).await
    }

    pub async fn plan_query_sql(
        &self,
        sql_query: impl Into<String>,
    ) -> std::result::Result<QueryPlan, DbError> {
        self.backend.plan_query_sql(sql_query.into()).await
    }

    pub async fn plan_query_prql(
        &self,
        prql_query: impl Into<String>,
    ) -> std::result::Result<QueryPlan, DbError> {
        self.backend.plan_query_prql(prql_query.into()).await
    }

    pub async fn plan_query_text(
        &self,
        format: TextQueryFormat,
        query: impl Into<String>,
    ) -> std::result::Result<QueryPlan, DbError> {
        self.backend.plan_query_text(format, query.into()).await
    }

    pub fn query_to_sql(&self, query: &Query) -> std::result::Result<String, DbError> {
        self.backend.query_to_sql(query)
    }

    pub async fn update_where(
        &self,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        match self.backend.query(Query::Update(query)).await? {
            QueryResult::Update(result) => Ok(result.stats),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-update result for update query".to_string(),
            )),
        }
    }

    pub async fn delete_where(&self, query: DeleteQuery) -> std::result::Result<usize, DbError> {
        match self.backend.query(Query::Delete(query)).await? {
            QueryResult::Delete(result) => Ok(result.deleted),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-delete result for delete query".to_string(),
            )),
        }
    }

    pub async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        self.backend.execute_batch(batch).await
    }
}
