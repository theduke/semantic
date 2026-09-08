use std::sync::Arc;

use async_trait::async_trait;
use deadpool_postgres::Pool;
use semantic_data::schema::{Package, RelationType};
use semantic_data::value::Object;
use semantic_db_core::catalog::{Catalog, CollectionKind, LocalCollectionId, SharedCatalog};
use semantic_db_core::embedded::EmbeddedDb;
use semantic_db_core::{
    AccessPath, Backend, Batch, BatchOutcome, DbError, DdlBatch, DdlOutcome, EntityRecord,
    PackageRegistrationOutcome, Query, QueryExplain, QueryPlan, QueryResult, SqlDialectKind,
    TextQueryInput,
};
use semantic_db_kv::EntityStore;
use tokio::sync::Mutex;
use tokio_postgres::IsolationLevel;

use crate::config::{PostgresBackendOptions, PostgresLayout, PostgresSchemaOwnership};
use crate::discovery::discover_catalog_with_identity_policy;
use crate::managed::{
    PostgresSnapshotEngine, bootstrap, lock_and_load, save, storage_error,
    verify_relational_projection,
};
use crate::query::QueryCompiler;

/// Operating mode for the Postgres backend.
#[derive(Debug, Clone)]
pub enum PostgresMode {
    /// Discover existing PostgreSQL tables.
    /// No tables are ever created or modified. The catalog is in-memory only.
    Discovery { schemas: Vec<String> },
    /// Backend-owned lossless semantic storage.
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
/// Managed mode provides the complete semantic backend contract. Discovery
/// mode maps existing relational tables and remains read-only.
pub struct PostgresBackend {
    pool: Pool,
    catalog: SharedCatalog,
    compiler: QueryCompiler,
    options: PostgresBackendOptions,
    write_mutex: Mutex<()>,
}

impl PostgresBackend {
    /// Create a new `PostgresBackend`.
    ///
    /// In `Discovery` mode:
    /// 1. Acquires a client from the pool.
    /// 2. Discovers tables to build an in-memory Catalog.
    /// 3. The catalog lives only in memory — nothing is persisted to Postgres.
    pub async fn new(pool: Pool, mode: PostgresMode) -> Result<Self, DbError> {
        let options = match &mode {
            PostgresMode::Discovery { schemas } => {
                PostgresBackendOptions::relational_discovery(schemas.clone())
                    .with_legacy_discovery_ids(true)
            }
            PostgresMode::Managed => PostgresBackendOptions::semantic_managed(),
        };
        Self::new_with_options_and_mode(pool, options, mode).await
    }

    /// Construct a backend using the explicit layout and ownership options.
    pub async fn new_with_options(
        pool: Pool,
        options: PostgresBackendOptions,
    ) -> Result<Self, DbError> {
        let mode = if options.layout == PostgresLayout::Relational
            && options.ownership == PostgresSchemaOwnership::ReadOnly
        {
            PostgresMode::Discovery {
                schemas: options.schemas.clone(),
            }
        } else {
            PostgresMode::Managed
        };
        Self::new_with_options_and_mode(pool, options, mode).await
    }

    async fn new_with_options_and_mode(
        pool: Pool,
        options: PostgresBackendOptions,
        _mode: PostgresMode,
    ) -> Result<Self, DbError> {
        options.validate()?;
        let catalog = match (options.layout, options.ownership) {
            (PostgresLayout::Relational, PostgresSchemaOwnership::ReadOnly) => {
                let client = pool.get().await.map_err(pool_error)?;
                discover_catalog_with_identity_policy(
                    &client,
                    &options.schemas,
                    options.identity_policy,
                )
                .await?
            }
            (layout, ownership) => {
                let mut client = pool.get().await.map_err(pool_error)?;
                let transaction = if ownership == PostgresSchemaOwnership::Managed {
                    client
                        .build_transaction()
                        .isolation_level(IsolationLevel::Serializable)
                        .start()
                        .await
                        .map_err(storage_error)?
                } else {
                    client
                        .build_transaction()
                        .isolation_level(IsolationLevel::RepeatableRead)
                        .read_only(true)
                        .start()
                        .await
                        .map_err(storage_error)?
                };
                if ownership == PostgresSchemaOwnership::Managed {
                    bootstrap(
                        &transaction,
                        &options.metadata_schema,
                        match layout {
                            PostgresLayout::Semantic => "semantic",
                            PostgresLayout::Relational => "relational",
                        },
                    )
                    .await?;
                }
                let db = lock_and_load(
                    &transaction,
                    &options.metadata_schema,
                    options.db_config,
                    ownership == PostgresSchemaOwnership::Managed,
                )
                .await?;
                let catalog = db.catalog().as_ref().clone();
                if ownership == PostgresSchemaOwnership::Managed {
                    let catalog = save(
                        &transaction,
                        &options.metadata_schema,
                        db,
                        match layout {
                            PostgresLayout::Semantic => "semantic",
                            PostgresLayout::Relational => "relational",
                        },
                    )
                    .await?;
                    transaction.commit().await.map_err(storage_error)?;
                    catalog
                } else {
                    transaction.commit().await.map_err(storage_error)?;
                    catalog
                }
            }
        };

        let shared_catalog = SharedCatalog::new(catalog);
        let compiler =
            QueryCompiler::new_with_legacy_ids(pool.clone(), options.allow_legacy_discovery_ids);

        Ok(Self {
            pool,
            catalog: shared_catalog,
            compiler,
            options,
            write_mutex: Mutex::new(()),
        })
    }

    fn uses_managed_engine(&self) -> bool {
        self.options.layout == PostgresLayout::Semantic
            || self.options.ownership == PostgresSchemaOwnership::Managed
    }

    /// Atomically refresh the catalog in read-only relational discovery mode.
    pub async fn refresh_catalog(&self) -> Result<Arc<Catalog>, DbError> {
        if self.options.layout != PostgresLayout::Relational
            || self.options.ownership != PostgresSchemaOwnership::ReadOnly
        {
            return Err(DbError::InvalidQuery(
                "catalog refresh is only available in relational discovery mode".to_string(),
            ));
        }
        let client = self.pool.get().await.map_err(pool_error)?;
        let catalog = discover_catalog_with_identity_policy(
            &client,
            &self.options.schemas,
            self.options.identity_policy,
        )
        .await?;
        self.catalog.replace(catalog);
        Ok(self.catalog.catalog_arc())
    }

    async fn with_semantic_db<T>(
        &self,
        write: bool,
        mut operation: impl FnMut(
            &mut EmbeddedDb<EntityStore<PostgresSnapshotEngine>>,
        ) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        if write && self.options.ownership == PostgresSchemaOwnership::ReadOnly {
            return Err(DbError::InvalidQuery(Self::WRITE_ERROR.to_string()));
        }
        let _guard = if write {
            Some(self.write_mutex.lock().await)
        } else {
            None
        };
        let max_attempts = if write {
            self.options.transaction_retry_count.saturating_add(1)
        } else {
            1
        };
        for attempt in 1..=max_attempts {
            let outcome = async {
                let mut client = self.pool.get().await.map_err(pool_error)?;
                let transaction = if write {
                    client
                        .build_transaction()
                        .isolation_level(IsolationLevel::Serializable)
                        .start()
                        .await
                        .map_err(storage_error)?
                } else {
                    client
                        .build_transaction()
                        .isolation_level(IsolationLevel::RepeatableRead)
                        .read_only(true)
                        .start()
                        .await
                        .map_err(storage_error)?
                };
                let mut db = lock_and_load(
                    &transaction,
                    &self.options.metadata_schema,
                    self.options.db_config,
                    write,
                )
                .await?;
                if self.options.layout == PostgresLayout::Relational {
                    verify_relational_projection(
                        &transaction,
                        &self.options.metadata_schema,
                        db.catalog().as_ref(),
                    )
                    .await?;
                }
                let result = operation(&mut db)?;
                let catalog = if write {
                    save(
                        &transaction,
                        &self.options.metadata_schema,
                        db,
                        match self.options.layout {
                            PostgresLayout::Semantic => "semantic",
                            PostgresLayout::Relational => "relational",
                        },
                    )
                    .await?
                } else {
                    db.catalog().as_ref().clone()
                };
                transaction.commit().await.map_err(storage_error)?;
                Ok::<_, DbError>((result, catalog))
            }
            .await;
            match outcome {
                Ok((result, catalog)) => {
                    self.catalog.replace(catalog);
                    return Ok(result);
                }
                Err(DbError::TransactionConflict(_)) if attempt < max_attempts => continue,
                Err(DbError::TransactionConflict(_)) => {
                    return Err(DbError::TransactionRetriesExhausted {
                        attempts: max_attempts,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("transaction attempt range is never empty")
    }

    /// Error message for write operations in discovery mode.
    const WRITE_ERROR: &'static str = "this operation is not available in read-only discovery mode";
}

#[async_trait]
impl Backend for PostgresBackend {
    async fn catalog(&self) -> Result<Arc<Catalog>, DbError> {
        if self.uses_managed_engine() {
            return self.with_semantic_db(false, |db| Ok(db.catalog())).await;
        }
        Ok(self.catalog.catalog_arc())
    }

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> Result<LocalCollectionId, DbError> {
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(true, move |db| {
                    db.create_collection(name.clone(), kind.clone())
                })
                .await;
        }
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn execute_ddl(&self, ddl: DdlBatch) -> Result<DdlOutcome, DbError> {
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(true, move |db| db.transact_ddl(ddl.clone()))
                .await;
        }
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn upsert_package(
        &self,
        package: Package,
    ) -> Result<PackageRegistrationOutcome, DbError> {
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(true, move |db| db.upsert_package(package.clone()))
                .await;
        }
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn upsert_relationship(&self, relationship: RelationType) -> Result<(), DbError> {
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(true, move |db| db.upsert_relationship(relationship.clone()))
                .await;
        }
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn delete_relationship(&self, id: String) -> Result<(), DbError> {
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(true, move |db| db.delete_relationship(&id))
                .await;
        }
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn insert(&self, collection: String, id: String, object: Object) -> Result<(), DbError> {
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(true, move |db| {
                    db.insert(&collection, id.clone(), object.clone())
                })
                .await;
        }
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn get(&self, collection: String, id: String) -> Result<Option<EntityRecord>, DbError> {
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(false, move |db| db.get(&collection, &id))
                .await;
        }
        let catalog = self.catalog.catalog_arc();
        self.compiler.execute_get(&collection, &id, catalog).await
    }

    async fn delete(&self, collection: String, id: String) -> Result<(), DbError> {
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(true, move |db| db.delete(&collection, &id))
                .await;
        }
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }

    async fn query(&self, input: TextQueryInput) -> Result<QueryResult, DbError> {
        let query = match input {
            TextQueryInput::Ast(q) => q,
            TextQueryInput::Text { format, query } => self.parse_text_query(format, &query).await?,
        };

        if self.uses_managed_engine() {
            let write = !matches!(query, Query::Select(_));
            return self
                .with_semantic_db(write, move |db| db.query(query.clone()))
                .await;
        }

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

    fn sql_dialect(&self) -> SqlDialectKind {
        SqlDialectKind::PostgreSql
    }

    fn query_to_sql(&self, _query: &Query) -> Result<String, DbError> {
        Err(DbError::InvalidQuery(
            "PostgreSQL SQL rendering requires physical layout mappings and is not exposed as generic logical SQL"
                .to_string(),
        ))
    }

    async fn explain(&self, query: TextQueryInput) -> Result<QueryExplain, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text { format, query } => self.parse_text_query(format, &query).await?,
        };
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(false, move |db| db.explain_query(query.clone()))
                .await;
        }
        let Query::Select(select) = query else {
            return Err(DbError::InvalidQuery(
                "discovery mode can only explain SELECT queries".to_string(),
            ));
        };
        let pair = self
            .compiler
            .plan_select(&select, self.catalog.catalog_arc())?;
        Ok(QueryExplain {
            logical: pair.logical,
            physical: pair.physical,
            access_path: AccessPath::FullScan,
        })
    }

    async fn plan(&self, query: TextQueryInput) -> Result<QueryPlan, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text { format, query } => self.parse_text_query(format, &query).await?,
        };
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(false, move |db| db.plan_query(query.clone()))
                .await;
        }
        let Query::Select(select) = query else {
            return Err(DbError::InvalidQuery(
                "discovery mode can only plan SELECT queries".to_string(),
            ));
        };
        self.compiler
            .plan_select(&select, self.catalog.catalog_arc())?;
        Ok(QueryPlan::FullScan {
            collection: select.collection_or_default().to_string(),
        })
    }

    async fn execute_batch(&self, batch: Batch) -> Result<BatchOutcome, DbError> {
        if self.uses_managed_engine() {
            return self
                .with_semantic_db(true, move |db| db.execute_batch(batch.clone()))
                .await;
        }
        Err(DbError::InvalidQuery(Self::WRITE_ERROR.into()))
    }
}

fn pool_error(error: deadpool_postgres::PoolError) -> DbError {
    DbError::Storage(format!("connection pool error: {error}"))
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
