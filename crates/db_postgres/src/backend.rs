use std::sync::Arc;

use async_trait::async_trait;
use deadpool_postgres::Pool;
use semantic_data::schema::{Package, RelationType};
use semantic_data::value::Object;
use semantic_db_core::catalog::{Catalog, CollectionKind, LocalCollectionId, SharedCatalog};
use semantic_db_core::{
    AccessPath, Backend, Batch, BatchOutcome, DbError, DdlBatch, DdlOutcome, EntityRecord,
    LogicalPlan, PackageRegistrationOutcome, PhysicalPlan, PhysicalSource, Query, QueryExplain,
    QueryPlan, QueryResult, SourceRef, TextQueryInput,
};

use crate::discovery::discover_catalog;
use crate::query::QueryCompiler;

/// Operating mode for the Postgres backend.
#[derive(Debug, Clone)]
pub enum PostgresMode {
    /// Discover existing tables from `information_schema`.
    /// No tables are ever created or modified. The catalog is in-memory only.
    Discovery { schemas: Vec<String> },
    /// Not yet implemented.
    Managed,
}

/// Configuration for creating a `PostgresBackend`.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// Connection URI for the PostgreSQL database.
    pub uri: String,
    /// Active Postgres mode.
    pub mode: PostgresMode,
    /// Maximum pool size.
    pub max_pool_size: Option<usize>,
}

/// A PostgreSQL-backed implementation of the `Backend` trait.
///
/// Currently supports **read-only discovery mode** only.
/// See `PostgresMode::Discovery` for details.
pub struct PostgresBackend {
    #[allow(dead_code)]
    pool: Pool,
    catalog: SharedCatalog,
    compiler: QueryCompiler,
    #[allow(dead_code)]
    mode: PostgresMode,
}

impl PostgresBackend {
    /// Create a new `PostgresBackend`.
    ///
    /// In `Discovery` mode:
    /// 1. Acquires a client from the pool.
    /// 2. Discovers tables from `information_schema` to build an in-memory Catalog.
    /// 3. The catalog lives only in memory — nothing is persisted to Postgres.
    ///
    /// In `Managed` mode: returns `Err(DbError::InvalidQuery(...))`.
    pub async fn new(pool: Pool, mode: PostgresMode) -> Result<Self, DbError> {
        let catalog = match &mode {
            PostgresMode::Discovery { schemas } => {
                let client = pool
                    .get()
                    .await
                    .map_err(|e| DbError::Storage(format!("connection pool error: {}", e)))?;
                discover_catalog(&client, schemas).await?
            }
            PostgresMode::Managed => {
                return Err(DbError::InvalidQuery(
                    "managed mode not yet implemented".into(),
                ));
            }
        };

        let shared_catalog = SharedCatalog::new(catalog);
        let compiler = QueryCompiler::new(pool.clone());

        Ok(Self {
            pool,
            catalog: shared_catalog,
            compiler,
            mode,
        })
    }

    /// Error message for write operations in discovery mode.
    const WRITE_ERROR: &'static str = "this operation is not available in read-only discovery mode";
}

#[async_trait]
impl Backend for PostgresBackend {
    async fn catalog(&self) -> Result<Arc<Catalog>, DbError> {
        Ok(self.catalog.catalog_arc())
    }

    async fn create_collection(
        &self,
        _name: String,
        _kind: CollectionKind,
    ) -> Result<LocalCollectionId, DbError> {
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn execute_ddl(&self, _ddl: DdlBatch) -> Result<DdlOutcome, DbError> {
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn upsert_package(
        &self,
        _package: Package,
    ) -> Result<PackageRegistrationOutcome, DbError> {
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn upsert_relationship(&self, _relationship: RelationType) -> Result<(), DbError> {
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn delete_relationship(&self, _id: String) -> Result<(), DbError> {
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn insert(
        &self,
        _collection: String,
        _id: String,
        _object: Object,
    ) -> Result<(), DbError> {
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn get(&self, collection: String, id: String) -> Result<Option<EntityRecord>, DbError> {
        let catalog = self.catalog.catalog_arc();
        self.compiler.execute_get(&collection, &id, catalog).await
    }

    async fn delete(&self, _collection: String, _id: String) -> Result<(), DbError> {
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn query(&self, input: TextQueryInput) -> Result<QueryResult, DbError> {
        let query = match input {
            TextQueryInput::Ast(q) => q,
            TextQueryInput::Text { format, query } => self.parse_text_query(format, &query).await?,
        };

        match query {
            Query::Select(select) => {
                let catalog = self.catalog.catalog_arc();
                let objects = self.compiler.execute_select(&select, catalog).await?;
                Ok(QueryResult::Select(objects))
            }
            Query::Insert(_)
            | Query::Update(_)
            | Query::Delete(_)
            | Query::Ddl(_) => Err(DbError::InvalidQuery(
                "INSERT/UPDATE/DELETE/DDL queries are not available in read-only discovery mode"
                    .into(),
            )),
        }
    }

    async fn explain(&self, _query: TextQueryInput) -> Result<QueryExplain, DbError> {
        // In discovery mode, always report FullScan since we don't have
        // insight into Postgres's internal index selection.
        Ok(QueryExplain {
            logical: LogicalPlan::Source {
                source: SourceRef::unnamed(),
                pushed_predicate: None,
            },
            physical: PhysicalPlan::Source(PhysicalSource::Scan {
                source: SourceRef::unnamed(),
            }),
            access_path: AccessPath::FullScan,
        })
    }

    async fn plan(&self, _query: TextQueryInput) -> Result<QueryPlan, DbError> {
        Err(DbError::InvalidQuery(
            "plan() is not yet implemented for the Postgres backend".into(),
        ))
    }

    async fn execute_batch(&self, _batch: Batch) -> Result<BatchOutcome, DbError> {
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }
}

/// Create a connection pool from a PostgresConfig.
pub fn create_pool(config: &PostgresConfig) -> Result<Pool, DbError> {
    let uri_config: tokio_postgres::Config =
        config.uri.parse().map_err(|e: tokio_postgres::Error| {
            DbError::InvalidQuery(format!("invalid POSTGRES_URI: {}", e))
        })?;

    let mgr = deadpool_postgres::Manager::new(uri_config, tokio_postgres::NoTls);
    let max_size = config.max_pool_size.unwrap_or(4);
    Ok(Pool::builder(mgr)
        .max_size(max_size)
        .build()
        .map_err(|e| DbError::Storage(format!("failed to create connection pool: {}", e)))?)
}
