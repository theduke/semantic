//! Read-only access to one committed database state.

use super::*;
use crate::catalog::CatalogSnapshot;
use crate::embedded::storage::{BoxEntityIdScan, BoxEntityScan, BoxIndexEntryScan};

/// Read-only view of the database at one committed state.
///
/// A reader pairs a catalog snapshot with a storage read snapshot taken
/// together, and serves queries, point reads, plans, validation preflights
/// and exports without access to the [`EmbeddedDb`] it was created from.
///
/// Readers from [`EmbeddedDb::owned_reader`] are `'static`: they own their
/// storage snapshot ([`EntityStorage::owned_snapshot`]), so they can outlive
/// a lock around the database and run concurrently with writers, which
/// neither block them nor become visible to them. [`EmbeddedDb::reader`]
/// falls back to a snapshot borrowing the storage when the storage has no
/// owned snapshots; such a reader keeps the database borrowed while used.
pub struct DbReader<'a> {
    catalog: CatalogSnapshot,
    snapshot: QueryReader<'a>,
}

impl std::fmt::Debug for DbReader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbReader")
            .field("catalog_version", &self.catalog.version)
            .field("owned", &self.is_owned())
            .finish_non_exhaustive()
    }
}

impl<'a> DbReader<'a> {
    pub(super) fn new(catalog: CatalogSnapshot, snapshot: QueryReader<'a>) -> Self {
        Self { catalog, snapshot }
    }

    /// The catalog this reader observes.
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog.catalog
    }

    /// Version of [`Self::catalog`] in the database's shared catalog.
    pub fn catalog_version(&self) -> u64 {
        self.catalog.version
    }

    /// Storage revision observed by this reader.
    pub fn revision(&self) -> std::result::Result<Option<u64>, DbError> {
        self.snapshot.revision()
    }

    /// Whether the reader owns its storage snapshot (and is not tied to the
    /// database's lifetime).
    pub fn is_owned(&self) -> bool {
        matches!(self.snapshot, QueryReader::Shared(_))
    }

    /// The storage snapshot serving this reader.
    pub(super) fn storage_snapshot(&self) -> &dyn EntityReadSnapshot {
        &*self.snapshot
    }

    fn query_context(&self) -> QueryContext {
        QueryContext::new(self.catalog.catalog.clone())
    }

    pub fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        let collection_schema = self
            .catalog()
            .collection_by_name(collection)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: collection.to_string(),
            })?;
        let Some(entity) = self.snapshot.get_entity(collection_schema.lid, id)? else {
            return Ok(None);
        };
        Ok(Some(EntityRecord {
            id: entity.id,
            collection: collection_schema.name.clone(),
            object: entity.object,
        }))
    }

    pub fn select(&self, query: SelectQuery) -> std::result::Result<Vec<Object>, DbError> {
        let prepared = self.prepare_select(query)?;
        self.run_select(&prepared, None)
    }

    /// Run a SELECT while collecting execution metrics.
    ///
    /// Returns the rows and the explain of the executed plan, whose
    /// [`QueryExplain::analyze`] holds the metrics (and per-operator
    /// statistics with `operator_stats`). The physical plan is the one
    /// executed, including rewrites applied after optimization.
    pub fn select_analyzed(
        &self,
        query: SelectQuery,
        operator_stats: bool,
    ) -> std::result::Result<(Vec<Object>, QueryExplain), DbError> {
        let started = std::time::Instant::now();
        let prepared = self.prepare_select(query)?;
        let plan_elapsed = started.elapsed();
        let collector = Arc::new(if operator_stats {
            crate::MetricsCollector::with_operator_stats()
        } else {
            crate::MetricsCollector::new()
        });
        let exec_started = std::time::Instant::now();
        let rows = self.run_select(&prepared, Some(&collector))?;
        let mut metrics = collector.snapshot();
        metrics.exec_elapsed = exec_started.elapsed();
        metrics.plan_elapsed = plan_elapsed;
        metrics.elapsed = started.elapsed();
        let catalog = self.catalog();
        let access_path = prepared
            .collection
            .and_then(|lid| catalog.collection_by_lid(lid))
            .map_or(AccessPath::FullScan, |collection| {
                access_path_from_physical(catalog, collection, &prepared.physical)
            });
        let explain = QueryExplain {
            logical: prepared.logical,
            physical: prepared.physical,
            access_path,
            analyze: Some(crate::QueryAnalysis {
                metrics,
                operator_stats: collector.operator_stats(),
            }),
        };
        Ok((rows, explain))
    }

    /// `EXPLAIN ANALYZE`: execute a SELECT with per-operator statistics and
    /// return its explain (see [`Self::select_analyzed`]).
    pub fn explain_analyze_query(
        &self,
        query: Query,
    ) -> std::result::Result<QueryExplain, DbError> {
        let Query::Select(query) = query else {
            return Err(DbError::InvalidQuery(
                "EXPLAIN ANALYZE is only supported for SELECT".to_string(),
            ));
        };
        Ok(self.select_analyzed(query, true)?.1)
    }

    /// Canonicalize and plan a SELECT.
    fn prepare_select(&self, query: SelectQuery) -> std::result::Result<PreparedSelect, DbError> {
        let catalog = self.catalog().clone();
        let collection_name = query.collection_or_default().to_string();
        let (query, stats, source, collection) = if is_all_collection_alias(&collection_name) {
            (query, None, ALL_COLLECTION_ALIAS.to_string(), None)
        } else {
            let collection = catalog
                .collection_by_name(&collection_name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: collection_name.clone(),
                })?;
            let query = canonicalize_select_query(&query, catalog.as_ref(), collection)?;
            let stats = stats_for_query(&catalog, &*self.snapshot, &query, collection)?;
            (
                query,
                Some(stats),
                collection.name.clone(),
                Some(collection.lid),
            )
        };
        let optimizer = crate::Optimizer::core();
        let context = self.query_context();
        let stats_provider = stats
            .as_ref()
            .map(|value| value as &dyn crate::StatsProvider);
        let pair = optimizer.optimize_query(&query, Some(source.clone()), stats_provider, &context);
        let mut physical = pair.physical;
        self.rewrite_count_fast_path(&mut physical)?;
        Ok(PreparedSelect {
            field_format: query.field_format,
            logical: pair.logical,
            physical,
            source,
            collection,
        })
    }

    /// Execute a prepared SELECT and format its rows.
    fn run_select(
        &self,
        prepared: &PreparedSelect,
        metrics: Option<&Arc<crate::MetricsCollector>>,
    ) -> std::result::Result<Vec<Object>, DbError> {
        let catalog = self.catalog();
        let mut rows = self.execute_physical_plan_with_metrics(
            &prepared.physical,
            Some(prepared.source.as_str()),
            metrics,
        )?;
        // Inject computed attributes.
        for row in &mut rows {
            let _ = crate::inject_computed_attributes(catalog.as_ref(), row);
        }
        Ok(crate::format_output_rows(
            catalog.as_ref(),
            rows,
            prepared.field_format,
        ))
    }

    pub fn plan_query(&self, query: Query) -> std::result::Result<QueryPlan, DbError> {
        if matches!(query, Query::Ddl(_)) {
            return Err(DbError::InvalidQuery(
                "query planning/explain is not supported for DDL".to_string(),
            ));
        }
        let collection = query.collection_or_default().to_string();
        let explain = self.explain_query(query)?;
        match explain.access_path {
            AccessPath::FullScan => Ok(QueryPlan::FullScan { collection }),
            AccessPath::IndexLookup {
                index_name,
                field,
                value,
            } => Ok(QueryPlan::IndexLookup {
                collection,
                index_name,
                field,
                value,
            }),
            AccessPath::IndexRange {
                index_name,
                fields,
                ranges,
                ordered,
                descending,
                index_only,
            } => Ok(QueryPlan::IndexRange {
                collection,
                index_name,
                fields,
                ranges,
                ordered,
                descending,
                index_only,
            }),
            AccessPath::TextSearch {
                index_name,
                fields,
                tokens,
                mode,
            } => Ok(QueryPlan::TextSearch {
                collection,
                index_name,
                fields,
                tokens,
                mode,
            }),
        }
    }

    pub fn explain_query(&self, query: Query) -> std::result::Result<QueryExplain, DbError> {
        if matches!(query, Query::Ddl(_)) {
            return Err(DbError::InvalidQuery(
                "query planning/explain is not supported for DDL".to_string(),
            ));
        }
        let catalog = self.catalog().clone();
        let collection_name = query.collection_or_default().to_string();
        let (select, stats, source, collection_for_access_path) =
            if is_all_collection_alias(&collection_name) {
                let Query::Select(select) = query else {
                    return Err(DbError::InvalidQuery(format!(
                        "collection alias '{ALL_COLLECTION_ALIAS}' is only supported for SELECT"
                    )));
                };
                (select, None, ALL_COLLECTION_ALIAS.to_string(), None)
            } else {
                let collection = catalog
                    .collection_by_name(&collection_name)
                    .ok_or_else(|| DbError::UnknownCollectionByName {
                        name: collection_name.clone(),
                    })?;

                let select = match canonicalize_query(&query, catalog.as_ref(), collection)? {
                    Query::Select(query) => query,
                    Query::Insert(_) => {
                        return Err(DbError::InvalidQuery(
                            "query planning/explain is not supported for INSERT".to_string(),
                        ));
                    }
                    Query::Update(query) => SelectQuery {
                        collection: query.collection,
                        source_alias: None,
                        joins: Vec::new(),
                        predicate: query.predicate,
                        projection: query.returning,
                        distinct: false,
                        group_by: Vec::new(),
                        having: None,
                        order_by: Vec::new(),
                        offset: crate::Expr::from(0usize),
                        limit: query.limit,
                        field_format: query.field_format,
                    },
                    Query::Delete(query) => SelectQuery {
                        collection: query.collection,
                        source_alias: None,
                        joins: Vec::new(),
                        predicate: query.predicate,
                        projection: query.returning,
                        distinct: false,
                        group_by: Vec::new(),
                        having: None,
                        order_by: Vec::new(),
                        offset: crate::Expr::from(0usize),
                        limit: query.limit,
                        field_format: query.field_format,
                    },
                    Query::Ddl(_) => {
                        return Err(DbError::InvalidQuery(
                            "query planning/explain is not supported for DDL".to_string(),
                        ));
                    }
                };
                let stats = stats_for_query(&catalog, &*self.snapshot, &select, collection)?;
                (
                    select,
                    Some(stats),
                    collection.name.clone(),
                    Some(collection.lid),
                )
            };

        let optimizer = crate::Optimizer::core();
        let context = self.query_context();
        let stats_provider = stats
            .as_ref()
            .map(|value| value as &dyn crate::StatsProvider);
        let pair = optimizer.optimize_query(&select, Some(source), stats_provider, &context);
        let access_path = collection_for_access_path
            .and_then(|lid| catalog.collection_by_lid(lid))
            .map_or(AccessPath::FullScan, |collection| {
                access_path_from_physical(&catalog, collection, &pair.physical)
            });

        Ok(QueryExplain {
            logical: pair.logical,
            physical: pair.physical,
            access_path,
            analyze: None,
        })
    }

    /// Execute a plan with all reads served by this reader's snapshot.
    pub fn execute_physical_plan(
        &self,
        plan: &crate::PhysicalPlan,
        default_collection: Option<&str>,
    ) -> std::result::Result<Vec<Object>, DbError> {
        self.execute_physical_plan_with_metrics(plan, default_collection, None)
    }

    /// Execute a plan, collecting metrics into `metrics` when given: reads
    /// then go through a [`MeteredReadSnapshot`].
    fn execute_physical_plan_with_metrics(
        &self,
        plan: &crate::PhysicalPlan,
        default_collection: Option<&str>,
        metrics: Option<&Arc<crate::MetricsCollector>>,
    ) -> std::result::Result<Vec<Object>, DbError> {
        let (reader, context) = match metrics {
            None => (self.snapshot.borrowed(), self.query_context()),
            Some(metrics) => (
                self.snapshot.metered(metrics.clone()),
                self.query_context().with_metrics(metrics.clone()),
            ),
        };
        let local_refs = LocalRefResolver::for_plan(plan, reader.shared());
        let source = EmbeddedPhysicalDataSource {
            reader,
            catalog: self.catalog().clone(),
            default_collection: default_collection.map(ToOwned::to_owned),
            local_refs,
        };
        crate::execute_physical_plan_with_source(plan, &source, &context)
            .map_err(|err| DbError::InvalidQuery(err.to_string()))
    }

    pub fn collection_rows(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<Vec<EntityRecord>, DbError> {
        let collection_schema = self
            .catalog()
            .collection_by_lid(collection)
            .ok_or(DbError::UnknownCollection(collection))?;
        self.snapshot
            .scan_collection_stream(collection)?
            .map(|item| {
                item.map(|item| EntityRecord {
                    id: item.id,
                    collection: collection_schema.name.clone(),
                    object: item.object,
                })
            })
            .collect()
    }

    /// Visit every entity in non-internal collections. Returning `false`
    /// from `visit` cancels the scan.
    pub(crate) fn scan_entities_with(
        &self,
        mut visit: impl FnMut(std::result::Result<EntityRecord, DbError>) -> bool,
    ) {
        for (_, collection) in self.catalog().collections() {
            if collection.internal {
                continue;
            }
            let scan = match self.snapshot.scan_collection_stream(collection.lid) {
                Ok(scan) => scan,
                Err(error) => {
                    visit(Err(error));
                    return;
                }
            };
            for item in scan {
                let record = item.map(|stored| EntityRecord {
                    id: stored.id,
                    collection: collection.name.clone(),
                    object: stored.object,
                });
                if !visit(record) {
                    return;
                }
            }
        }
    }

    /// Inspect stored data without normalizing, repairing, or writing
    /// anything. Reports the first violation per row; unsupported
    /// constraints are explicit errors.
    ///
    /// Snapshots that are not consistent are fenced by revision, so a
    /// concurrent commit surfaces as a conflict.
    pub fn validation_preflight(
        &self,
    ) -> std::result::Result<Vec<crate::ValidationViolation>, DbError> {
        let catalog = self.catalog();
        let fence = (!self.snapshot.is_consistent())
            .then(|| self.snapshot.revision())
            .transpose()?;
        crate::validation::validate_enforcement_support(catalog)?;
        let mut violations = Vec::new();
        for (_, collection) in catalog.collections().filter(|(_, c)| !c.internal) {
            for row in self.snapshot.scan_collection_stream(collection.lid)? {
                let row = row?;
                let key = (collection.name.clone(), row.id.clone());
                match crate::validate_stored_object(catalog, &key, &row.object, |key| {
                    let collection = catalog.collection_by_name(&key.0).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: key.0.clone(),
                        }
                    })?;
                    Ok(self
                        .snapshot
                        .get_entity(collection.lid, &key.1)?
                        .map(|row| row.object))
                }) {
                    Ok(_) => {}
                    Err(DbError::Validation(error)) => {
                        violations.push(crate::ValidationViolation {
                            collection: key.0,
                            id: key.1,
                            error,
                        })
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        if let Some(revision) = fence
            && self.snapshot.revision()? != revision
        {
            return Err(DbError::TransactionConflict(
                "database changed during validation preflight".into(),
            ));
        }
        Ok(violations)
    }

    /// Answer `SELECT count(*) FROM collection` shapes from the maintained
    /// row count instead of scanning.
    ///
    /// Rewrites an ungrouped aggregate without `HAVING` whose projection is
    /// only `count(*)` over an unfiltered scan of one user collection (also
    /// below single-input operators such as `LIMIT`) into a constant row.
    /// Internal collections and the `all` alias keep scanning.
    fn rewrite_count_fast_path(
        &self,
        plan: &mut crate::PhysicalPlan,
    ) -> std::result::Result<bool, DbError> {
        use crate::PhysicalPlan as P;

        match plan {
            P::Aggregate {
                input,
                group_by,
                projection,
                having,
            } => {
                let P::Source(crate::PhysicalSource::Scan { source }) = input.as_ref() else {
                    return Ok(false);
                };
                let count_only = !projection.is_empty()
                    && projection.iter().all(|field| {
                        field.wildcard.is_none()
                            && matches!(
                                &field.expr,
                                crate::Expr::Aggregate {
                                    op: semantic_data::query::AggregateOp::Count,
                                    distinct: false,
                                    arg,
                                } if matches!(arg.as_ref(), crate::FunctionArg::Wildcard)
                            )
                    });
                if !group_by.is_empty() || having.is_some() || !count_only {
                    return Ok(false);
                }
                let catalog = self.catalog();
                let collection = match (&source.collection_id, source.source_name.as_deref()) {
                    (Some(lid), _) => catalog.collection_by_lid(*lid),
                    (None, Some(name)) if !is_all_collection_alias(name) => {
                        catalog.collection_by_name(name)
                    }
                    _ => None,
                };
                let Some(collection) = collection.filter(|collection| !collection.internal) else {
                    return Ok(false);
                };
                let count =
                    Value::I64(collection_row_count(&*self.snapshot, collection.lid)? as i64);
                let row = projection
                    .iter()
                    .map(|field| (crate::aggregate_output_key(field), count.clone()))
                    .collect::<Object>();
                *plan = P::Values { values: vec![row] };
                Ok(true)
            }
            P::Limit { input, .. }
            | P::Sort { input, .. }
            | P::TopN { input, .. }
            | P::Project { input, .. }
            | P::Distinct { input }
            | P::Materialize { input }
            | P::Exchange { input, .. } => self.rewrite_count_fast_path(input),
            _ => Ok(false),
        }
    }
}

/// A planned SELECT, ready to execute.
struct PreparedSelect {
    field_format: FieldFormat,
    logical: crate::LogicalPlan,
    physical: crate::PhysicalPlan,
    /// Name of the queried source.
    source: String,
    /// The queried collection (`None` for the `all` alias).
    collection: Option<LocalCollectionId>,
}

/// The storage snapshot of one reader or query.
pub(crate) enum QueryReader<'a> {
    /// Owned snapshot that row views may keep for on-demand reads.
    Shared(Arc<dyn EntityReadSnapshot>),
    /// Snapshot borrowing the storage.
    Borrowed(Box<dyn EntityReadSnapshot + 'a>),
    /// Snapshot borrowed from a longer-lived [`QueryReader::Borrowed`].
    Ref(&'a (dyn EntityReadSnapshot + 'a)),
}

impl QueryReader<'_> {
    pub(super) fn shared(&self) -> Option<Arc<dyn EntityReadSnapshot>> {
        match self {
            Self::Shared(snapshot) => Some(snapshot.clone()),
            Self::Borrowed(_) | Self::Ref(_) => None,
        }
    }

    /// A reader over the same snapshot for one query.
    fn borrowed(&self) -> QueryReader<'_> {
        match self {
            Self::Shared(snapshot) => QueryReader::Shared(snapshot.clone()),
            Self::Borrowed(snapshot) => QueryReader::Ref(snapshot.as_ref()),
            Self::Ref(snapshot) => QueryReader::Ref(*snapshot),
        }
    }

    /// A reader over the same snapshot for one query that counts its reads
    /// into `metrics`.
    fn metered(&self, metrics: Arc<crate::MetricsCollector>) -> QueryReader<'_> {
        match self {
            Self::Shared(snapshot) => QueryReader::Shared(Arc::new(MeteredReadSnapshot::new(
                snapshot.clone(),
                metrics,
            ))),
            Self::Borrowed(snapshot) => QueryReader::Borrowed(Box::new(MeteredReadSnapshot::new(
                snapshot.as_ref(),
                metrics,
            ))),
            Self::Ref(snapshot) => {
                QueryReader::Borrowed(Box::new(MeteredReadSnapshot::new(*snapshot, metrics)))
            }
        }
    }
}

/// Read snapshot that counts rows scanned and decoded, point reads, index
/// probes and index entries into a [`crate::MetricsCollector`].
struct MeteredReadSnapshot<S> {
    inner: S,
    metrics: Arc<crate::MetricsCollector>,
}

impl<S> MeteredReadSnapshot<S> {
    fn new(inner: S, metrics: Arc<crate::MetricsCollector>) -> Self {
        Self { inner, metrics }
    }

    fn count_index_scan(&self, scan: BoxEntityIdScan) -> BoxEntityIdScan {
        self.metrics.add_index_probes(1);
        let metrics = self.metrics.clone();
        Box::new(scan.inspect(move |_| metrics.add_index_entries_read(1)))
    }
}

impl<S> EntityReadSnapshot for MeteredReadSnapshot<S>
where
    S: std::ops::Deref + Send + Sync,
    S::Target: EntityReadSnapshot,
{
    fn revision(&self) -> std::result::Result<Option<u64>, DbError> {
        self.inner.revision()
    }

    fn is_consistent(&self) -> bool {
        self.inner.is_consistent()
    }

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> std::result::Result<Option<StoredEntity>, DbError> {
        self.metrics.add_point_reads(1);
        let entity = self.inner.get_entity(collection, id)?;
        if entity.is_some() {
            self.metrics.add_rows_decoded(1);
        }
        Ok(entity)
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<BoxEntityScan, DbError> {
        let metrics = self.metrics.clone();
        let scan = self.inner.scan_collection_stream(collection)?;
        Ok(Box::new(scan.inspect(move |row| {
            if row.is_ok() {
                metrics.add_rows_scanned(1);
                metrics.add_rows_decoded(1);
            }
        })))
    }

    fn scan_collection(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<Vec<StoredEntity>, DbError> {
        let rows = self.inner.scan_collection(collection)?;
        self.metrics.add_rows_scanned(rows.len() as u64);
        self.metrics.add_rows_decoded(rows.len() as u64);
        Ok(rows)
    }

    fn count_collection_entities(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<u64, DbError> {
        self.inner.count_collection_entities(collection)
    }

    fn collection_row_count(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<Option<u64>, DbError> {
        self.inner.collection_row_count(collection)
    }

    fn index_entry_count(
        &self,
        index: crate::catalog::LocalIndexId,
    ) -> std::result::Result<Option<u64>, DbError> {
        self.inner.index_entry_count(index)
    }

    fn scan_index_value_stream(
        &self,
        index: crate::catalog::LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> std::result::Result<BoxEntityIdScan, DbError> {
        let scan = self.inner.scan_index_value_stream(index, path, value)?;
        Ok(self.count_index_scan(scan))
    }

    fn scan_index_value(
        &self,
        index: crate::catalog::LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> std::result::Result<Vec<String>, DbError> {
        let ids = self.inner.scan_index_value(index, path, value)?;
        self.metrics.add_index_probes(1);
        self.metrics.add_index_entries_read(ids.len() as u64);
        Ok(ids)
    }

    fn scan_index_range_stream(
        &self,
        index: crate::catalog::LocalIndexId,
        path: Option<&FieldPath>,
        lower: std::ops::Bound<&Value>,
        upper: std::ops::Bound<&Value>,
    ) -> std::result::Result<BoxEntityIdScan, DbError> {
        let scan = self
            .inner
            .scan_index_range_stream(index, path, lower, upper)?;
        Ok(self.count_index_scan(scan))
    }

    fn scan_index_prefix_stream(
        &self,
        index: crate::catalog::LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> std::result::Result<BoxEntityIdScan, DbError> {
        let scan = self.inner.scan_index_prefix_stream(index, path, prefix)?;
        Ok(self.count_index_scan(scan))
    }

    fn scan_index_entries(
        &self,
        index: crate::catalog::LocalIndexId,
        scan: &crate::IndexScan,
    ) -> std::result::Result<BoxIndexEntryScan, DbError> {
        let entries = self.inner.scan_index_entries(index, scan)?;
        self.metrics.add_index_probes(1);
        let metrics = self.metrics.clone();
        Ok(Box::new(
            entries.inspect(move |_| metrics.add_index_entries_read(1)),
        ))
    }

    fn index_needs_rebuild(
        &self,
        index: crate::catalog::LocalIndexId,
    ) -> std::result::Result<bool, DbError> {
        self.inner.index_needs_rebuild(index)
    }
}

impl<'a> std::ops::Deref for QueryReader<'a> {
    type Target = dyn EntityReadSnapshot + 'a;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Shared(snapshot) => snapshot.as_ref(),
            Self::Borrowed(snapshot) => snapshot.as_ref(),
            Self::Ref(snapshot) => *snapshot,
        }
    }
}

/// Planner statistics for `query`, read from `reader`.
pub(super) fn stats_for_query(
    catalog: &Catalog,
    reader: &dyn EntityReadSnapshot,
    query: &SelectQuery,
    base_collection: &CollectionSchema,
) -> std::result::Result<QueryStatsSnapshot, DbError> {
    let mut collection_ids = BTreeSet::from([base_collection.lid]);
    for join in &query.joins {
        let collection = match join.source.collection.as_deref() {
            Some(name) => catalog.collection_by_name(name).ok_or_else(|| {
                DbError::UnknownCollectionByName {
                    name: name.to_string(),
                }
            })?,
            None => base_collection,
        };
        collection_ids.insert(collection.lid);
    }
    let mut collections = Vec::with_capacity(collection_ids.len());
    for collection_id in collection_ids {
        let collection = catalog
            .collection_by_lid(collection_id)
            .ok_or(DbError::UnknownCollection(collection_id))?;
        collections.push(stats_for_collection(catalog, reader, collection)?);
    }
    Ok(QueryStatsSnapshot { collections })
}

fn stats_for_collection(
    catalog: &Catalog,
    reader: &dyn EntityReadSnapshot,
    collection: &CollectionSchema,
) -> std::result::Result<CollectionStatsEntry, DbError> {
    let row_count = collection_row_count(reader, collection.lid)? as f64;
    let mut index_entries = Vec::new();
    let mut indexed_fields = BTreeSet::new();
    let mut unique_fields = BTreeSet::new();
    let mut indexed_field_ids = BTreeSet::new();
    let mut unique_field_ids = BTreeSet::new();
    let mut indexed_attr_ids = BTreeSet::new();
    let mut unique_attr_ids = BTreeSet::new();

    let mut index_entry_counts = BTreeMap::new();
    for index in catalog.indexes_for_collection(collection.lid) {
        let entries = if index.schema.kind.is_value_index() || index.schema.kind.is_full_text() {
            reader.index_entry_count(index.lid)?
        } else {
            None
        };
        if let Some(entries) = entries {
            index_entry_counts.insert(index.lid, entries as f64);
        }
        // Full-text entries are tokens: they neither answer value lookups
        // nor bound the column's distinct values.
        if index.schema.kind.is_full_text() {
            continue;
        }
        // Composite and partial indexes neither answer lookups of their
        // first column nor bound its distinct values.
        if !index.is_simple() {
            continue;
        }
        if let Some(entries) = entries {
            index_entries.push(IndexEntryStats {
                canonical_field: index.canonical_field.clone(),
                field_id: index.field_id,
                attr_id: index.attr_id,
                entries: entries as f64,
            });
        }
        indexed_fields.insert(index.canonical_field.clone());
        if index.schema.unique {
            unique_fields.insert(index.canonical_field.clone());
        }
        if let Some(field_id) = index.field_id {
            indexed_field_ids.insert(field_id);
            if index.schema.unique {
                unique_field_ids.insert(field_id);
            }
        }
        if let Some(attr_id) = index.attr_id {
            indexed_attr_ids.insert(attr_id);
            if index.schema.unique {
                unique_attr_ids.insert(attr_id);
            }
        }
    }

    Ok(CollectionStatsEntry {
        source: collection.name.clone(),
        row_count,
        indexed_fields,
        unique_fields,
        indexed_field_ids,
        unique_field_ids,
        indexed_attr_ids,
        unique_attr_ids,
        collection_id: collection.lid,
        has_path_equality_index: catalog.find_path_equality_index(collection.lid).is_some(),
        index_entries,
        index_entry_counts,
    })
}

/// The full-text search a physical plan reads its rows through, if any.
fn find_text_search(plan: &crate::PhysicalPlan) -> Option<&crate::PhysicalTextSearch> {
    match plan {
        crate::PhysicalPlan::Source(crate::PhysicalSource::TextSearch(search)) => Some(search),
        crate::PhysicalPlan::Filter { input, .. }
        | crate::PhysicalPlan::Sort { input, .. }
        | crate::PhysicalPlan::TopN { input, .. }
        | crate::PhysicalPlan::Project { input, .. }
        | crate::PhysicalPlan::Aggregate { input, .. }
        | crate::PhysicalPlan::Limit { input, .. }
        | crate::PhysicalPlan::Distinct { input, .. }
        | crate::PhysicalPlan::Materialize { input, .. }
        | crate::PhysicalPlan::Exchange { input, .. }
        | crate::PhysicalPlan::RepartitionHash { input, .. } => find_text_search(input),
        _ => None,
    }
}

fn access_path_from_physical(
    catalog: &Catalog,
    collection: &CollectionSchema,
    physical: &crate::PhysicalPlan,
) -> AccessPath {
    if let Some(search) = find_text_search(physical) {
        return AccessPath::TextSearch {
            index_name: search.index_name.clone(),
            fields: search.columns.iter().map(format_field_path).collect(),
            tokens: search.tokens.clone(),
            mode: search.mode,
        };
    }
    if let Some(scan) = find_index_range(physical) {
        return AccessPath::IndexRange {
            index_name: scan.index_name.clone(),
            fields: scan.columns.iter().map(format_field_path).collect(),
            ranges: scan.ranges.len(),
            ordered: scan.ordered,
            descending: scan.direction == semantic_data::query::SortDirection::Desc,
            index_only: scan.index_only,
        };
    }
    let Some((field_ref, value)) = find_index_lookup(physical) else {
        return AccessPath::FullScan;
    };
    let Some(field_path) = lookup_field_path(collection, field_ref) else {
        return AccessPath::FullScan;
    };

    let top_level = field_path
        .segments()
        .first()
        .and_then(|segment| match segment {
            PathSegment::Field(field) => Some(field.as_str()),
            _ => None,
        })
        .unwrap_or_default();
    if !top_level.is_empty()
        && field_path.segments().len() == 1
        && let Some(index) = catalog.find_lookup_index(collection.lid, top_level)
    {
        return AccessPath::IndexLookup {
            index_name: index.schema.name.clone(),
            field: format_field_path(&field_path),
            value: value.clone(),
        };
    }
    if let Some(index) = catalog.find_path_equality_index(collection.lid) {
        return AccessPath::IndexLookup {
            index_name: index.schema.name.clone(),
            field: format_field_path(&field_path),
            value: value.clone(),
        };
    }
    AccessPath::FullScan
}

#[cfg(all(test, feature = "sql"))]
mod tests;
