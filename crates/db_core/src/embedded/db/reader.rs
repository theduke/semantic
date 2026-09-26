//! Read-only access to one committed database state.

use super::*;
use crate::catalog::CatalogSnapshot;

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
        let catalog = self.catalog().clone();
        let collection_name = query.collection_or_default().to_string();
        let (query, stats, source) = if is_all_collection_alias(&collection_name) {
            (query, None, ALL_COLLECTION_ALIAS.to_string())
        } else {
            let collection = catalog
                .collection_by_name(&collection_name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: collection_name.clone(),
                })?;
            let query = canonicalize_select_query(&query, catalog.as_ref(), collection)?;
            let stats = stats_for_query(&catalog, &*self.snapshot, &query, collection)?;
            (query, Some(stats), collection.name.clone())
        };
        let optimizer = crate::Optimizer::core();
        let context = self.query_context();
        let stats_provider = stats
            .as_ref()
            .map(|value| value as &dyn crate::StatsProvider);
        let mut physical = optimizer
            .optimize_query(&query, Some(source.clone()), stats_provider, &context)
            .physical;
        self.rewrite_count_fast_path(&mut physical)?;
        let mut rows = self.execute_physical_plan(&physical, Some(source.as_str()))?;
        // Inject computed attributes.
        for row in &mut rows {
            let _ = crate::inject_computed_attributes(catalog.as_ref(), row);
        }
        Ok(crate::format_output_rows(
            catalog.as_ref(),
            rows,
            query.field_format,
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
        })
    }

    /// Execute a plan with all reads served by this reader's snapshot.
    pub fn execute_physical_plan(
        &self,
        plan: &crate::PhysicalPlan,
        default_collection: Option<&str>,
    ) -> std::result::Result<Vec<Object>, DbError> {
        let reader = self.snapshot.borrowed();
        let local_refs = LocalRefResolver::for_plan(plan, reader.shared());
        let source = EmbeddedPhysicalDataSource {
            reader,
            catalog: self.catalog().clone(),
            default_collection: default_collection.map(ToOwned::to_owned),
            local_refs,
        };
        crate::execute_physical_plan_with_source(plan, &source, &self.query_context())
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
            | P::Project { input, .. }
            | P::Distinct { input }
            | P::Materialize { input }
            | P::Exchange { input, .. } => self.rewrite_count_fast_path(input),
            _ => Ok(false),
        }
    }
}

/// The storage snapshot of one reader or query.
pub(super) enum QueryReader<'a> {
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

    for index in catalog.indexes_for_collection(collection.lid) {
        if index.schema.kind == IndexKind::Equality
            && let Some(entries) = reader.index_entry_count(index.lid)?
        {
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
    })
}

fn access_path_from_physical(
    catalog: &Catalog,
    collection: &CollectionSchema,
    physical: &crate::PhysicalPlan,
) -> AccessPath {
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
        && let Some(index) = catalog.find_equality_index(collection.lid, top_level)
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
