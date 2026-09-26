use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

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
    Batch, BatchOutcome, BatchStats, DbError, DdlBatch, DdlOutcome, DeleteQuery, DeleteResult,
    LogicalPlan, MutationStats, PackageRegistrationOutcome, PhysicalPlan, Query, QueryAnalysis,
    QueryMetrics, QueryResult, SavepointId, SelectQuery, SqlDialectKind, StorageErrorKind,
    TextQueryFormat, TextQueryInput, TransactionCommit, TransactionOptions, UpdateQuery,
    UpdateResult, WriteMetrics, prql, sql,
};
use futures::Stream;

pub use semantic_data::builtin::DEFAULT_COLLECTION;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_collection_input_uses_data_default_collection() {
        assert_eq!(
            CollectionInput::Default.into_collection(),
            semantic_data::builtin::DEFAULT_COLLECTION
        );
        assert_eq!(
            CollectionInput::from(None::<String>).into_collection(),
            semantic_data::builtin::DEFAULT_COLLECTION
        );
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct EntityRecord {
    pub id: String,
    pub collection: String,
    pub object: Object,
}

pub type EntityStream = Pin<Box<dyn Stream<Item = Result<EntityRecord, DbError>> + Send>>;

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
    /// Ordered scans of an equality or range index: ranges, prefixes, `IN`
    /// probes, composite and partial indexes.
    IndexRange {
        collection: String,
        index_name: String,
        /// Key columns of the index, in key order.
        fields: Vec<String>,
        /// Number of key ranges (`IN` probes) scanned.
        ranges: usize,
        /// Whether the scan serves the query's ordering (descending when
        /// `descending`).
        ordered: bool,
        descending: bool,
        /// Whether rows are served from index keys without row reads.
        index_only: bool,
    },
    /// Token probes of a full-text index answering a text match.
    TextSearch {
        collection: String,
        index_name: String,
        /// Indexed columns.
        fields: Vec<String>,
        /// Distinct query tokens, one index probe each.
        tokens: Vec<String>,
        mode: semantic_data::query::TextMatchMode,
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
    /// See [`QueryPlan::IndexRange`].
    IndexRange {
        index_name: String,
        fields: Vec<String>,
        ranges: usize,
        ordered: bool,
        descending: bool,
        index_only: bool,
    },
    /// See [`QueryPlan::TextSearch`].
    TextSearch {
        index_name: String,
        fields: Vec<String>,
        tokens: Vec<String>,
        mode: semantic_data::query::TextMatchMode,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryExplain {
    pub logical: LogicalPlan,
    pub physical: PhysicalPlan,
    pub access_path: AccessPath,
    /// Measured execution (`EXPLAIN ANALYZE`), absent for plain explains.
    pub analyze: Option<QueryAnalysis>,
}

impl QueryExplain {
    /// One-line summary of the physical plan (see
    /// [`PhysicalPlan::summary`]).
    pub fn summary(&self) -> String {
        self.physical.summary()
    }
}

impl std::fmt::Display for QueryExplain {
    /// The plan as an indented operator tree; with an analysis, each
    /// operator is annotated with its rows and time, followed by the query
    /// metrics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let operators = self
            .analyze
            .iter()
            .flat_map(|analysis| &analysis.operator_stats)
            .map(|stats| (stats.path.as_str(), stats))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut lines = Vec::new();
        self.physical.walk(&mut |path, plan| {
            let mut line = format!("{}{}", "  ".repeat(path.len()), plan.node_label());
            if let Some(stats) = operators.get(crate::metrics::operator_path_string(path).as_str())
            {
                line.push_str(&format!(
                    "  (rows={} time={:?}",
                    stats.rows_out, stats.elapsed
                ));
                for (key, value) in &stats.extra {
                    match value {
                        Value::String(value) => line.push_str(&format!(" {key}={value}")),
                        Value::U64(value) => line.push_str(&format!(" {key}={value}")),
                        other => line.push_str(&format!(" {key}={other:?}")),
                    }
                }
                line.push(')');
            }
            lines.push(line);
        });
        f.write_str(&lines.join("\n"))?;
        if let Some(analysis) = &self.analyze {
            let metrics = &analysis.metrics;
            write!(
                f,
                "\nrows_emitted={} rows_scanned={} rows_decoded={} point_reads={} \
                 index_probes={} index_entries_read={} sort_rows_retained={} hash_groups={} \
                 join_build_rows={} join_probe_rows={}\nplanning={:?} execution={:?} total={:?}",
                metrics.rows_emitted,
                metrics.rows_scanned,
                metrics.rows_decoded,
                metrics.point_reads,
                metrics.index_probes,
                metrics.index_entries_read,
                metrics.sort_rows_retained,
                metrics.hash_groups,
                metrics.joins.build_rows,
                metrics.joins.probe_rows,
                metrics.plan_elapsed,
                metrics.exec_elapsed,
                metrics.elapsed,
            )?;
        }
        Ok(())
    }
}

/// An interactive transaction of a [`Backend`] (see
/// [`Backend::begin_transaction`]).
///
/// Statements run one at a time against a snapshot of the committed state
/// taken when the transaction began, and observe the transaction's own
/// writes. Nothing is visible to others before [`Self::commit`]; a handle
/// dropped without committing is rolled back. Each statement is atomic: a
/// failing statement leaves the transaction as it was.
#[async_trait]
pub trait TransactionHandle: Send + Sync {
    fn options(&self) -> TransactionOptions;

    async fn get(&self, collection: String, id: String) -> Result<Option<EntityRecord>, DbError>;

    async fn select(&self, query: SelectQuery) -> Result<Vec<Object>, DbError>;

    async fn upsert(&self, collection: String, id: String, object: Object) -> Result<(), DbError>;

    /// Insert a row; fails with [`DbError::EntityExists`] when it exists.
    async fn create(&self, collection: String, id: String, object: Object) -> Result<(), DbError>;

    /// Delete a row (and its cascading dependents); returns whether it
    /// existed.
    async fn delete(&self, collection: String, id: String) -> Result<bool, DbError>;

    async fn update_where(&self, query: UpdateQuery) -> Result<UpdateResult, DbError>;

    async fn delete_where(&self, query: DeleteQuery) -> Result<DeleteResult, DbError>;

    /// Apply a batch as one statement; returns the rows it wrote.
    async fn execute_batch(&self, batch: Batch) -> Result<BatchStats, DbError>;

    async fn savepoint(&self) -> Result<SavepointId, DbError>;

    /// Undo the writes made after `savepoint`.
    async fn rollback_to(&self, savepoint: SavepointId) -> Result<(), DbError>;

    /// Validate and commit the transaction. Fails with
    /// [`DbError::TransactionConflict`] when another write committed since
    /// the transaction began; the caller may retry the whole transaction.
    async fn commit(self: Box<Self>) -> Result<TransactionCommit, DbError>;

    /// End the transaction without writing anything.
    async fn rollback(self: Box<Self>) -> Result<(), DbError>;
}

#[async_trait]
pub trait Backend: Send + Sync {
    /// Begin an interactive transaction. The default reports that the
    /// backend does not support interactive transactions.
    async fn begin_transaction(
        &self,
        options: TransactionOptions,
    ) -> Result<Box<dyn TransactionHandle>, DbError> {
        let _ = options;
        Err(DbError::storage(
            StorageErrorKind::Unsupported,
            "backend does not support interactive transactions",
        ))
    }

    /// Subscribe to the changes committed from now on (see
    /// [`crate::ChangeFeed`]). The default reports that the backend has no
    /// change feed.
    fn subscribe_changes(
        &self,
        options: crate::ChangeSubscriptionOptions,
    ) -> Result<crate::ChangeStream, DbError> {
        let _ = options;
        Err(DbError::storage(
            StorageErrorKind::Unsupported,
            "backend does not support change feeds",
        ))
    }

    async fn validation_preflight(&self) -> Result<Vec<crate::ValidationViolation>, DbError> {
        Err(DbError::InvalidQuery(
            "validation preflight requires a managed backend".into(),
        ))
    }

    async fn activate_validation(&self) -> Result<(), DbError> {
        Err(DbError::InvalidQuery(
            "validation activation requires a managed backend".into(),
        ))
    }

    // Maintenance (see `crate::maintenance`). The defaults report the
    // operations as unsupported.

    /// Rebuild the indexes selected by `target` from the rows.
    async fn reindex(&self, target: crate::ReindexTarget) -> Result<crate::ReindexReport, DbError> {
        let _ = target;
        Err(crate::unsupported_maintenance("reindex"))
    }

    /// Check the consistency of the stored data without writing.
    async fn verify(&self, options: crate::VerifyOptions) -> Result<crate::VerifyReport, DbError> {
        let _ = options;
        Err(crate::unsupported_maintenance("verify"))
    }

    /// Verify, rebuild what the problems call for, and verify again.
    async fn repair(&self, options: crate::VerifyOptions) -> Result<crate::RepairReport, DbError> {
        let _ = options;
        Err(crate::unsupported_maintenance("repair"))
    }

    /// Compact the physical storage.
    async fn compact_storage(&self) -> Result<crate::CompactReport, DbError> {
        Err(crate::unsupported_maintenance("storage compaction"))
    }

    /// Physical storage statistics.
    async fn storage_stats(&self) -> Result<crate::embedded::StorageStats, DbError> {
        Err(crate::unsupported_maintenance("storage statistics"))
    }

    /// Rewrite stored payloads in the current format, `batch_size` rows per
    /// write transaction.
    async fn rewrite_payloads(&self, batch_size: usize) -> Result<crate::RewriteReport, DbError> {
        let _ = batch_size;
        Err(crate::unsupported_maintenance("payload rewrites"))
    }

    /// Write a consistent copy of the database to a new file at `path`.
    async fn backup(&self, path: std::path::PathBuf) -> Result<crate::BackupReport, DbError> {
        let _ = path;
        Err(crate::unsupported_maintenance("backups"))
    }

    /// Stream all portable entities at one revision, with the catalog they
    /// were written under.
    async fn export_snapshot(&self) -> Result<crate::ExportSnapshot, DbError> {
        Err(crate::unsupported_maintenance("snapshot exports"))
    }
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError>;

    /// Scan all portable (non-internal) entities without collecting them.
    async fn scan_entities(&self) -> Result<EntityStream, DbError> {
        Err(DbError::InvalidQuery(
            "backend does not support streaming entity scans".into(),
        ))
    }

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

    /// Run `query` and report its execution metrics.
    ///
    /// The default runs [`Self::query`] and reports only the elapsed time;
    /// backends that collect metrics override it.
    async fn query_with_metrics(
        &self,
        query: TextQueryInput,
    ) -> Result<(QueryResult, QueryMetrics), DbError> {
        let started = Instant::now();
        let result = self.query(query).await?;
        let metrics = QueryMetrics {
            elapsed: started.elapsed(),
            ..QueryMetrics::default()
        };
        Ok((result, metrics))
    }

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
        sql::parse_sql_query(sql_query, self.sql_dialect()).map_err(DbError::from)
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

    /// `EXPLAIN ANALYZE`: execute a SELECT and return its explain with the
    /// measured execution in [`QueryExplain::analyze`].
    ///
    /// The default explains the query and then runs it through
    /// [`Self::query_with_metrics`], so its analysis has no per-operator
    /// statistics; backends that instrument operators override it.
    async fn explain_analyze(&self, query: TextQueryInput) -> Result<QueryExplain, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text {
                format,
                query,
                params,
            } => {
                self.parse_text_query_with_params(format, &query, &params)
                    .await?
            }
        };
        if !matches!(query, Query::Select(_)) {
            return Err(DbError::InvalidQuery(
                "EXPLAIN ANALYZE is only supported for SELECT".to_string(),
            ));
        }
        let mut explain = self.explain(TextQueryInput::Ast(query.clone())).await?;
        let (_, metrics) = self.query_with_metrics(TextQueryInput::Ast(query)).await?;
        explain.analyze = Some(QueryAnalysis {
            metrics,
            operator_stats: Vec::new(),
        });
        Ok(explain)
    }

    async fn parse_text_query_with_params(
        &self,
        format: TextQueryFormat,
        query: &str,
        params: &std::collections::BTreeMap<String, Value>,
    ) -> Result<Query, DbError> {
        if params.is_empty() {
            return self.parse_text_query(format, query).await;
        }
        if format != TextQueryFormat::Sql {
            return Err(DbError::QueryParameter {
                reason: "unsupported_format".to_string(),
                name: None,
            });
        }
        sql::parse_sql_query_with_params(query, self.sql_dialect(), params)
            .map(|parsed| parsed.query)
            .map_err(DbError::from)
    }

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

    async fn execute_batch_with_settings(
        &self,
        batch: Batch,
        settings: crate::WriteSettings,
    ) -> Result<BatchOutcome, DbError> {
        if settings != crate::WriteSettings::default() {
            return Err(DbError::InvalidQuery(
                "backend does not support non-default write settings".into(),
            ));
        }
        self.execute_batch(batch).await
    }

    async fn execute_batch_returning(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
    ) -> Result<crate::BatchReply, DbError> {
        match returning {
            crate::BatchReturn::Dataset => self
                .execute_batch(batch)
                .await
                .map(crate::BatchReply::Dataset),
            _ => Err(DbError::InvalidQuery(
                "backend does not support compact batch returning".into(),
            )),
        }
    }

    async fn execute_batch_returning_with_settings(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        if settings != crate::WriteSettings::default() {
            return Err(DbError::InvalidQuery(
                "backend does not support non-default write settings".into(),
            ));
        }
        self.execute_batch_returning(batch, returning).await
    }

    /// Execute a compact batch without falling back to collection materialization.
    async fn execute_batch_returning_bounded_with_settings(
        &self,
        _batch: Batch,
        _returning: crate::BatchReturn,
        _settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        Err(DbError::InvalidQuery(
            "backend does not support bounded batch execution".into(),
        ))
    }
}

pub struct Db {
    backend: Box<dyn Backend>,
}

impl Db {
    pub async fn validation_preflight(&self) -> Result<Vec<crate::ValidationViolation>, DbError> {
        self.backend.validation_preflight().await
    }

    pub async fn activate_validation(&self) -> Result<(), DbError> {
        self.backend.activate_validation().await
    }

    /// See [`Backend::reindex`].
    pub async fn reindex(
        &self,
        target: crate::ReindexTarget,
    ) -> Result<crate::ReindexReport, DbError> {
        self.backend.reindex(target).await
    }

    /// See [`Backend::verify`].
    pub async fn verify(
        &self,
        options: crate::VerifyOptions,
    ) -> Result<crate::VerifyReport, DbError> {
        self.backend.verify(options).await
    }

    /// See [`Backend::repair`].
    pub async fn repair(
        &self,
        options: crate::VerifyOptions,
    ) -> Result<crate::RepairReport, DbError> {
        self.backend.repair(options).await
    }

    /// See [`Backend::compact_storage`].
    pub async fn compact_storage(&self) -> Result<crate::CompactReport, DbError> {
        self.backend.compact_storage().await
    }

    /// See [`Backend::storage_stats`].
    pub async fn storage_stats(&self) -> Result<crate::embedded::StorageStats, DbError> {
        self.backend.storage_stats().await
    }

    /// See [`Backend::rewrite_payloads`].
    pub async fn rewrite_payloads(
        &self,
        batch_size: usize,
    ) -> Result<crate::RewriteReport, DbError> {
        self.backend.rewrite_payloads(batch_size).await
    }

    /// See [`Backend::backup`].
    pub async fn backup(
        &self,
        path: impl Into<std::path::PathBuf>,
    ) -> Result<crate::BackupReport, DbError> {
        self.backend.backup(path.into()).await
    }

    /// See [`Backend::export_snapshot`].
    pub async fn export_snapshot(&self) -> Result<crate::ExportSnapshot, DbError> {
        self.backend.export_snapshot().await
    }
    pub fn new(backend: impl Backend + 'static) -> Self {
        Self {
            backend: Box::new(backend),
        }
    }

    pub async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        self.backend.catalog().await
    }

    /// Begin an interactive transaction (see [`TransactionHandle`]).
    pub async fn begin_transaction(
        &self,
        options: TransactionOptions,
    ) -> Result<Box<dyn TransactionHandle>, DbError> {
        self.backend.begin_transaction(options).await
    }

    /// Subscribe to committed changes (see [`Backend::subscribe_changes`]).
    pub fn subscribe_changes(
        &self,
        options: crate::ChangeSubscriptionOptions,
    ) -> Result<crate::ChangeStream, DbError> {
        self.backend.subscribe_changes(options)
    }

    pub async fn scan_entities(&self) -> Result<EntityStream, DbError> {
        self.backend.scan_entities().await
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
        let query: TextQueryInput = query.into().into();
        tracing::trace!(query = ?query, "Executing query");
        if tracing::enabled!(tracing::Level::DEBUG) {
            // Collect metrics only when they are logged.
            return self
                .query_with_metrics_logged(query)
                .await
                .map(|(result, _)| result);
        }
        let started = Instant::now();
        let result = self.backend.query(query).await;
        tracing::trace!(
            elapsed = ?started.elapsed(),
            success = result.is_ok(),
            "Query executed"
        );
        result
    }

    /// Run `query` and return its execution metrics (all counters zero
    /// for backends that do not collect them).
    pub async fn query_with_metrics(
        &self,
        query: impl Into<PublicQueryInput>,
    ) -> Result<(QueryResult, QueryMetrics), DbError> {
        let query: TextQueryInput = query.into().into();
        tracing::trace!(query = ?query, "Executing query");
        self.query_with_metrics_logged(query).await
    }

    async fn query_with_metrics_logged(
        &self,
        query: TextQueryInput,
    ) -> Result<(QueryResult, QueryMetrics), DbError> {
        let started = Instant::now();
        let result = self.backend.query_with_metrics(query).await;
        match &result {
            Ok((_, metrics)) => tracing::debug!(
                elapsed = ?started.elapsed(),
                rows_emitted = metrics.rows_emitted,
                rows_scanned = metrics.rows_scanned,
                point_reads = metrics.point_reads,
                index_probes = metrics.index_probes,
                "Query executed"
            ),
            Err(error) => tracing::debug!(
                elapsed = ?started.elapsed(),
                error = %error,
                "Query failed"
            ),
        }
        result
    }

    pub async fn select(
        &self,
        query: PublicSelectQuery,
    ) -> std::result::Result<Vec<Object>, DbError> {
        match self.query(PublicQuery::Select(query)).await? {
            QueryResult::Select(rows) => Ok(rows),
            _ => Err(DbError::InvalidQuery(
                "backend returned non-select result for select query".to_string(),
            )),
        }
    }

    /// Explain a query.
    ///
    /// SQL text may be prefixed with `EXPLAIN` or `EXPLAIN ANALYZE`; the
    /// latter runs [`Self::explain_analyze`].
    pub async fn explain(
        &self,
        query: impl Into<PublicQueryInput>,
    ) -> std::result::Result<QueryExplain, DbError> {
        let (analyze, query) = self.strip_sql_explain(query.into().into())?;
        if analyze {
            return self.backend.explain_analyze(query).await;
        }
        self.backend.explain(query).await
    }

    /// `EXPLAIN ANALYZE`: execute a SELECT and explain it with its measured
    /// execution metrics and per-operator statistics (when the backend
    /// instruments operators). SQL text may be prefixed with `EXPLAIN
    /// [ANALYZE]`.
    pub async fn explain_analyze(
        &self,
        query: impl Into<PublicQueryInput>,
    ) -> Result<QueryExplain, DbError> {
        let (_, query) = self.strip_sql_explain(query.into().into())?;
        self.backend.explain_analyze(query).await
    }

    /// Resolve SQL text of the form `EXPLAIN [ANALYZE] <query>` into the
    /// explained query and whether `ANALYZE` was given. Other input is
    /// returned unchanged.
    fn strip_sql_explain(&self, query: TextQueryInput) -> Result<(bool, TextQueryInput), DbError> {
        let TextQueryInput::Text {
            format: TextQueryFormat::Sql,
            query: text,
            params,
        } = &query
        else {
            return Ok((false, query));
        };
        match sql::parse_sql_explain_with_params(text, self.backend.sql_dialect(), params)? {
            Some(explain) => Ok((explain.analyze, TextQueryInput::Ast(explain.query))),
            None => Ok((false, query)),
        }
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
        let started = Instant::now();
        let result = self.backend.execute_batch(batch.into()).await;
        log_batch(started, result.as_ref().map(|outcome| &outcome.metrics));
        result
    }

    pub async fn execute_batch_with_settings(
        &self,
        batch: PublicBatch,
        settings: crate::WriteSettings,
    ) -> Result<BatchOutcome, DbError> {
        let started = Instant::now();
        let result = self
            .backend
            .execute_batch_with_settings(batch.into(), settings)
            .await;
        log_batch(started, result.as_ref().map(|outcome| &outcome.metrics));
        result
    }

    pub async fn execute_batch_returning(
        &self,
        batch: PublicBatch,
        returning: crate::BatchReturn,
    ) -> Result<crate::BatchReply, DbError> {
        let started = Instant::now();
        let result = self
            .backend
            .execute_batch_returning(batch.into(), returning)
            .await;
        log_batch(started, result.as_ref().map(crate::BatchReply::metrics));
        result
    }

    pub async fn execute_batch_returning_with_settings(
        &self,
        batch: PublicBatch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        let started = Instant::now();
        let result = self
            .backend
            .execute_batch_returning_with_settings(batch.into(), returning, settings)
            .await;
        log_batch(started, result.as_ref().map(crate::BatchReply::metrics));
        result
    }

    pub async fn execute_batch_returning_bounded_with_settings(
        &self,
        batch: PublicBatch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        let started = Instant::now();
        let result = self
            .backend
            .execute_batch_returning_bounded_with_settings(batch.into(), returning, settings)
            .await;
        log_batch(started, result.as_ref().map(crate::BatchReply::metrics));
        result
    }
}

/// Log the outcome of a batch with its write metrics at debug level.
fn log_batch(started: Instant, result: Result<&WriteMetrics, &DbError>) {
    match result {
        Ok(metrics) => tracing::debug!(
            elapsed = ?started.elapsed(),
            point_reads = metrics.point_reads,
            index_reads = metrics.index_reads,
            collection_scans = metrics.collection_scans,
            fallback_scans = metrics.fallback_scans,
            visited_rows = metrics.visited_rows,
            storage_writes = metrics.storage_writes,
            "Batch executed"
        ),
        Err(error) => tracing::debug!(
            elapsed = ?started.elapsed(),
            error = %error,
            "Batch failed"
        ),
    }
}
