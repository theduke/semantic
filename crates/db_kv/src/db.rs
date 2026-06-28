use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use futures::{StreamExt, stream};
use semantic_data::query::FieldFormat;
use semantic_data::schema::{IndexKind, core::type_kind::TypeKind, core::type_node::Type};
use semantic_data::schema::{
    Migration, MigrationOperation, Package, RelationIndexingMode, RelationMode, RelationType,
};
use semantic_data::value::{FieldPath, Object, PathSegment, Value, ValueRef};
use semantic_db_core::{
    ALL_COLLECTION_ALIAS, AccessPath, AppliedMigration, Batch, BatchOperation, BatchOutcome,
    CORE_CATALOG_SCHEMA_COLLECTION, DEFAULT_COLLECTION, DeleteQuery, EntityRecord, InsertQuery,
    InsertSource, MutationStats, PackageRegistrationOutcome, Query, QueryExplain, QueryPlan,
    QueryResult, SelectQuery, UpdateQuery, apply_core_schema_migrations, apply_migration_ddl_batch,
    canonicalize_delete_query, canonicalize_insert_query, canonicalize_query,
    canonicalize_select_query, canonicalize_update_query, execute_batch, is_all_collection_alias,
    normalize_object_for_collection, normalize_package_definition, ref_target_class_ids,
    resolved_field_types_for_object, touched_collections, validate_package_migrations,
};
use semantic_db_core::{DbConfig, DbError, MigrationMismatchPolicy};

use crate::{
    schema_store::{catalog_write_ops, load_catalog},
    storage::{
        EntityStore, KvCommitOutcome, KvEngine, KvTransactionCapabilities, KvWriteOp,
        MemoryKvEngine, StoredEntity, StoredEntityKind,
    },
};
use semantic_db_core::catalog::{
    ATTR_RELATION_FROM, ATTR_RELATION_TO, Catalog, CollectionKind, CollectionSchema, IntegrityMode,
    LocalAttrId, LocalClassId, LocalCollectionId, LocalFieldId, OBJECT_TYPE_FIELD, SharedCatalog,
};
use semantic_db_core::{
    DdlBatch, DdlCollectionKind, DdlOperation, DdlOutcome, QueryContext, TransactionConcurrency,
    TransactionOptions, apply_ddl_batch, fresh_catalog_with_core_schema,
    run_with_transaction_retries,
};

const RELATION_EDGES_COLLECTION: &str = "__semantic.relationship_edges";
const REL_EDGE_RELATION_FIELD: &str = "relation";
const REL_EDGE_SOURCE_FIELD: &str = "source";
const REL_EDGE_TARGET_FIELD: &str = "target";
const REL_EDGE_DEPTH_FIELD: &str = "depth";
const REL_EDGE_SOURCE_KEY_FIELD: &str = "relation_source";
const REL_EDGE_TARGET_KEY_FIELD: &str = "relation_target";
const REL_EDGE_SOURCE_INDEX_NAME: &str = "__rel_source_idx";
const REL_EDGE_TARGET_INDEX_NAME: &str = "__rel_target_idx";

#[derive(Debug)]
pub struct KvDb<E: KvEngine> {
    catalog: SharedCatalog,
    store: EntityStore<E>,
    config: DbConfig,
}

impl KvDb<MemoryKvEngine> {
    pub fn in_memory() -> Self {
        Self::new(MemoryKvEngine::new())
    }

    pub fn in_memory_with_config(config: DbConfig) -> Self {
        Self::new_with_config(MemoryKvEngine::new(), config)
    }

    pub fn in_memory_mvcc() -> Self {
        Self::new(MemoryKvEngine::with_mvcc(true))
    }
}

impl<E: KvEngine> KvDb<E> {
    fn query_context(&self) -> QueryContext {
        QueryContext::from_shared(&self.catalog)
    }

    pub fn new(engine: E) -> Self {
        Self::open(engine).expect("database initialization failed")
    }

    pub fn new_with_config(engine: E, config: DbConfig) -> Self {
        Self::open_with_config(engine, config).expect("database initialization failed")
    }

    pub fn open(engine: E) -> std::result::Result<Self, DbError> {
        Self::open_with_config(engine, DbConfig::default())
    }

    pub fn open_with_config(engine: E, config: DbConfig) -> std::result::Result<Self, DbError> {
        let mut store = EntityStore::new(engine);
        let bootstrap_catalog = fresh_catalog_with_core_schema()
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
        let loaded_catalog = if let Some(catalog) = load_catalog(&store, &bootstrap_catalog)? {
            catalog
        } else {
            let ops = catalog_write_ops(&store, &bootstrap_catalog)?;
            store.write_batch(&ops)?;
            bootstrap_catalog
        };
        let core_schema_was_internal = loaded_catalog
            .collection_by_name(CORE_CATALOG_SCHEMA_COLLECTION)
            .is_some_and(|collection| collection.internal);
        let (mut catalog, executed_core_migrations) = apply_core_schema_migrations(&loaded_catalog)
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
        let mut catalog_changed = !executed_core_migrations.is_empty();
        catalog_changed |= !core_schema_was_internal
            && catalog
                .collection_by_name(CORE_CATALOG_SCHEMA_COLLECTION)
                .is_some_and(|collection| collection.internal);
        catalog_changed |= mark_collection_internal(&mut catalog, CORE_CATALOG_SCHEMA_COLLECTION)?;
        if catalog_changed {
            let ops = catalog_write_ops(&store, &catalog)?;
            store.write_batch(&ops)?;
        }
        let mut db = Self {
            catalog: SharedCatalog::new(catalog),
            store,
            config,
        };
        if db
            .catalog()
            .collection_by_name(DEFAULT_COLLECTION)
            .is_none()
        {
            db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertCollection {
                name: DEFAULT_COLLECTION.to_string(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::StrictRegisteredSchema,
            }))?;
        }
        if db
            .catalog()
            .collection_by_name(RELATION_EDGES_COLLECTION)
            .is_none()
        {
            db.create_collection(RELATION_EDGES_COLLECTION, CollectionKind::Polymorphic)?;
        }
        db.mark_collection_internal(RELATION_EDGES_COLLECTION)?;
        if let Some(collection) = db.catalog().collection_by_name(RELATION_EDGES_COLLECTION) {
            if db
                .catalog()
                .find_equality_index(collection.lid, REL_EDGE_SOURCE_KEY_FIELD)
                .is_none()
            {
                db.create_index(
                    REL_EDGE_SOURCE_INDEX_NAME,
                    collection.lid,
                    REL_EDGE_SOURCE_KEY_FIELD,
                    false,
                )?;
            }
            if db
                .catalog()
                .find_equality_index(collection.lid, REL_EDGE_TARGET_KEY_FIELD)
                .is_none()
            {
                db.create_index(
                    REL_EDGE_TARGET_INDEX_NAME,
                    collection.lid,
                    REL_EDGE_TARGET_KEY_FIELD,
                    false,
                )?;
            }
        }
        Ok(db)
    }

    fn mark_collection_internal(&mut self, name: &str) -> std::result::Result<(), DbError> {
        let mut catalog = self.catalog().as_ref().clone();
        if !mark_collection_internal(&mut catalog, name)? {
            return Ok(());
        }
        let ops = catalog_write_ops(&self.store, &catalog)?;
        self.store.write_batch(&ops)?;
        self.catalog.replace(catalog);
        Ok(())
    }

    pub fn catalog(&self) -> std::sync::Arc<Catalog> {
        self.catalog.catalog_arc()
    }

    pub fn shared_catalog(&self) -> &SharedCatalog {
        &self.catalog
    }

    pub fn create_collection(
        &mut self,
        name: impl Into<String>,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        let name = name.into();
        let ddl_kind = match kind {
            CollectionKind::Untyped => semantic_db_core::DdlCollectionKind::Untyped,
            CollectionKind::Schema => semantic_db_core::DdlCollectionKind::Schema,
            CollectionKind::Polymorphic => semantic_db_core::DdlCollectionKind::Polymorphic,
        };
        let ddl = DdlBatch::new().with_op(DdlOperation::UpsertCollection {
            name: name.clone(),
            kind: ddl_kind,
            integrity_mode: IntegrityMode::Permissive,
        });
        self.transact_ddl(ddl)?;
        self.catalog()
            .collection_by_name(&name)
            .map(|c| c.lid)
            .ok_or_else(|| DbError::UnknownCollectionByName { name })
    }

    pub fn create_index(
        &mut self,
        name: impl Into<String>,
        collection: LocalCollectionId,
        field: impl Into<String>,
        unique: bool,
    ) -> std::result::Result<(), DbError> {
        let snapshot = self.catalog();
        let collection_name = snapshot
            .collection_by_lid(collection)
            .ok_or(DbError::UnknownCollection(collection))?
            .name
            .clone();
        let ddl = DdlBatch::new().with_op(DdlOperation::UpsertIndex {
            name: name.into(),
            collection: collection_name,
            field: field.into(),
            unique,
        });
        self.transact_ddl(ddl)?;
        Ok(())
    }

    pub fn set_auto_index_enabled(&mut self, enabled: bool) -> std::result::Result<(), DbError> {
        let ddl = DdlBatch::new().with_op(DdlOperation::SetAutoIndex { enabled });
        self.transact_ddl(ddl)?;
        Ok(())
    }

    pub fn upsert_relationship(
        &mut self,
        relationship: RelationType,
    ) -> std::result::Result<(), DbError> {
        let ddl = DdlBatch::new().with_op(DdlOperation::UpsertRelationship { relationship });
        self.transact_ddl(ddl)?;
        Ok(())
    }

    pub fn delete_relationship(&mut self, id: &str) -> std::result::Result<(), DbError> {
        let ddl = DdlBatch::new().with_op(DdlOperation::DeleteRelationship { id: id.to_string() });
        self.transact_ddl(ddl)?;
        Ok(())
    }

    pub fn upsert_package(
        &mut self,
        package: Package,
    ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
        validate_package_migrations(&package)
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
        let package = normalize_package_definition(&package)
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;

        let txn_result = run_with_transaction_retries(TransactionOptions::default(), |_| {
            let catalog_snapshot = self.catalog.snapshot();
            let read_revision = self.store.current_revision()?;
            let (next_catalog, before, after, executed_migrations) = self.apply_package_update(
                catalog_snapshot.catalog.as_ref(),
                read_revision,
                &package,
            )?;
            let mut extra_ops =
                self.ddl_cleanup_ops(catalog_snapshot.catalog.as_ref(), &next_catalog)?;
            extra_ops.extend(catalog_write_ops(&self.store, &next_catalog)?);

            match self.persist_dataset_delta(
                &next_catalog,
                &before,
                &after,
                read_revision,
                &extra_ops,
            )? {
                KvCommitOutcome::Committed { .. } => {
                    self.catalog
                        .compare_and_swap(catalog_snapshot.version, next_catalog)
                        .map_err(|mismatch| {
                            DbError::TransactionConflict(format!(
                                "catalog version changed: expected {}, actual {}",
                                mismatch.expected, mismatch.actual
                            ))
                        })?;
                    Ok(PackageRegistrationOutcome {
                        executed_migrations,
                    })
                }
                KvCommitOutcome::Conflict {
                    expected_revision,
                    actual_revision,
                } => Err(DbError::TransactionConflict(format!(
                    "expected revision {:?}, found {:?}",
                    expected_revision, actual_revision
                ))),
            }
        })?;

        Ok(txn_result.value)
    }

    pub fn auto_index_enabled(&self) -> bool {
        self.catalog().auto_index_enabled()
    }

    pub fn into_parts(self) -> (SharedCatalog, E) {
        (self.catalog, self.store.into_inner())
    }

    pub fn insert(
        &mut self,
        collection: &str,
        id: impl Into<String>,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        let id = id.into();
        self.execute_batch(Batch::new().with_op(BatchOperation::Upsert {
            collection: collection.to_string(),
            id,
            object,
        }))?;
        Ok(())
    }

    pub fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        let catalog = self.catalog();
        let collection_schema = catalog.collection_by_name(collection).ok_or_else(|| {
            DbError::UnknownCollectionByName {
                name: collection.to_string(),
            }
        })?;

        let Some(entity) = self.store.get_entity(collection_schema.lid, id)? else {
            return Ok(None);
        };

        Ok(Some(EntityRecord {
            id: entity.id,
            collection: collection_schema.name.clone(),
            object: entity.object,
        }))
    }

    pub fn delete(&mut self, collection: &str, id: &str) -> std::result::Result<(), DbError> {
        self.execute_batch(Batch::new().with_op(BatchOperation::DeleteById {
            collection: collection.to_string(),
            id: id.to_string(),
        }))?;
        Ok(())
    }

    pub fn query(&mut self, query: Query) -> std::result::Result<QueryResult, DbError> {
        match query {
            Query::Select(query) => self.select(query).map(QueryResult::Select),
            Query::Insert(query) => self.insert_query(query).map(QueryResult::Insert),
            Query::Update(query) => self.update_where_returning(query).map(QueryResult::Update),
            Query::Delete(query) => self.delete_where_returning(query).map(QueryResult::Delete),
            Query::Ddl(query) => self.transact_ddl(query.batch).map(|_| QueryResult::Ddl(())),
        }
    }

    pub fn select(&self, query: SelectQuery) -> std::result::Result<Vec<Object>, DbError> {
        let collection_name = query.collection_or_default().to_string();
        let (query, stats, source) = if is_all_collection_alias(&collection_name) {
            (query, None, ALL_COLLECTION_ALIAS.to_string())
        } else {
            let catalog = self.catalog();
            let collection = catalog
                .collection_by_name(&collection_name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: collection_name.clone(),
                })?;
            (
                canonicalize_select_query(&query, catalog.as_ref(), collection)?,
                Some(self.stats_for_collection(collection)?),
                collection.name.clone(),
            )
        };
        let optimizer = semantic_db_core::Optimizer::core();
        let context = self.query_context();
        let stats_provider = stats
            .as_ref()
            .map(|value| value as &dyn semantic_db_core::StatsProvider);
        let pair = optimizer.optimize_query(&query, Some(source.clone()), stats_provider, &context);
        let mut rows = self.execute_physical_plan(&pair.physical, Some(source.as_str()))?;
        let catalog = self.catalog();
        // Inject computed attributes.
        for row in &mut rows {
            let _ = semantic_db_core::inject_computed_attributes(catalog.as_ref(), row);
        }
        Ok(self.format_output_rows(catalog.as_ref(), rows, query.field_format))
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
                let catalog = self.catalog();
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
                        offset: semantic_db_core::Expr::from(0usize),
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
                        offset: semantic_db_core::Expr::from(0usize),
                        limit: query.limit,
                        field_format: query.field_format,
                    },
                    Query::Ddl(_) => {
                        return Err(DbError::InvalidQuery(
                            "query planning/explain is not supported for DDL".to_string(),
                        ));
                    }
                };
                (
                    select,
                    Some(self.stats_for_collection(collection)?),
                    collection.name.clone(),
                    Some(collection.lid),
                )
            };

        let optimizer = semantic_db_core::Optimizer::core();
        let context = self.query_context();
        let stats_provider = stats
            .as_ref()
            .map(|value| value as &dyn semantic_db_core::StatsProvider);
        let pair = optimizer.optimize_query(&select, Some(source), stats_provider, &context);
        let access_path = if let Some(collection_lid) = collection_for_access_path {
            if let Some(collection) = self.catalog().collection_by_lid(collection_lid) {
                self.access_path_from_physical(collection, &pair.physical)
            } else {
                AccessPath::FullScan
            }
        } else {
            AccessPath::FullScan
        };

        Ok(QueryExplain {
            logical: pair.logical,
            physical: pair.physical,
            access_path,
        })
    }

    pub fn insert_query(
        &mut self,
        query: InsertQuery,
    ) -> std::result::Result<semantic_db_core::InsertResult, DbError> {
        let collection_name = query.collection_or_default().to_string();
        let catalog = self.catalog();
        let collection = catalog
            .collection_by_name(&collection_name)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: collection_name.clone(),
            })?
            .clone();
        let query = canonicalize_insert_query(&query, catalog.as_ref(), &collection)?;
        let target_columns = query
            .columns
            .iter()
            .map(|column| collection.canonical_field_name(column).to_string())
            .collect::<Vec<_>>();
        let InsertQuery {
            source, returning, ..
        } = query;
        let rows = self.materialize_insert_rows(&collection, &target_columns, source)?;
        if rows.is_empty() {
            return Err(DbError::InvalidQuery(
                "INSERT requires at least one row".to_string(),
            ));
        }
        let inserted = rows.len();

        let mut batch = Batch::new();
        let mut returning_rows = Vec::new();
        for object in rows {
            let id = self.extract_insert_id(&collection, &object)?;
            if !returning.is_empty() {
                returning_rows.push(semantic_db_core::project_object(&object, &returning));
            }
            batch = batch.with_op(BatchOperation::Upsert {
                collection: collection.name.clone(),
                id,
                object,
            });
        }

        self.execute_batch(batch)?;
        Ok(semantic_db_core::InsertResult {
            inserted,
            returning: self.format_output_rows(
                catalog.as_ref(),
                returning_rows,
                query.field_format,
            ),
        })
    }

    fn materialize_insert_rows(
        &self,
        collection: &CollectionSchema,
        target_columns: &[String],
        source: InsertSource,
    ) -> std::result::Result<Vec<Object>, DbError> {
        match source {
            InsertSource::Objects(rows) => {
                if !target_columns.is_empty() {
                    return Err(DbError::InvalidQuery(
                        "column list is not supported with object insert source".to_string(),
                    ));
                }
                Ok(rows)
            }
            InsertSource::Values(rows) => {
                self.materialize_insert_value_rows(collection, target_columns, rows)
            }
            InsertSource::Select(select) => {
                self.materialize_insert_select_rows(target_columns, select)
            }
        }
    }

    fn materialize_insert_value_rows(
        &self,
        collection: &CollectionSchema,
        target_columns: &[String],
        rows: Vec<Vec<semantic_db_core::Expr>>,
    ) -> std::result::Result<Vec<Object>, DbError> {
        if target_columns.is_empty() {
            return Err(DbError::InvalidQuery(format!(
                "INSERT into '{}' with VALUES requires explicit target columns",
                collection.name
            )));
        }
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            if row.len() != target_columns.len() {
                return Err(DbError::InvalidQuery(format!(
                    "VALUES row has {} expressions but INSERT specifies {} columns",
                    row.len(),
                    target_columns.len()
                )));
            }
            let mut object = Object::new();
            for (idx, expr) in row.iter().enumerate() {
                let value =
                    semantic_db_core::evaluate_expr(&Object::new(), expr).ok_or_else(|| {
                        DbError::InvalidQuery(format!(
                            "failed to evaluate INSERT value expression for column '{}'",
                            target_columns[idx]
                        ))
                    })?;
                object.insert(target_columns[idx].clone(), value);
            }
            out.push(object);
        }
        Ok(out)
    }

    fn materialize_insert_select_rows(
        &self,
        target_columns: &[String],
        select: SelectQuery,
    ) -> std::result::Result<Vec<Object>, DbError> {
        let source_projection = select.projection.clone();
        let source_rows = self.select(select)?;

        if target_columns.is_empty() {
            return Ok(source_rows);
        }
        if source_projection.is_empty() {
            return Err(DbError::InvalidQuery(
                "INSERT ... SELECT with target columns requires explicit SELECT projection"
                    .to_string(),
            ));
        }
        if source_projection.len() != target_columns.len() {
            return Err(DbError::InvalidQuery(format!(
                "INSERT has {} target columns but SELECT returns {} projected columns",
                target_columns.len(),
                source_projection.len()
            )));
        }

        let source_keys = source_projection
            .iter()
            .map(|field| {
                field
                    .alias
                    .clone()
                    .unwrap_or_else(|| infer_project_key_for_insert(&field.expr))
            })
            .collect::<Vec<_>>();

        let mut out = Vec::with_capacity(source_rows.len());
        for row in source_rows {
            let mut object = Object::new();
            for (idx, source_key) in source_keys.iter().enumerate() {
                let value = row.get(source_key).ok_or_else(|| {
                    DbError::InvalidQuery(format!(
                        "INSERT ... SELECT expected source column '{}' in SELECT row",
                        source_key
                    ))
                })?;
                object.insert(target_columns[idx].clone(), value.clone());
            }
            out.push(object);
        }
        Ok(out)
    }

    pub fn execute_physical_plan(
        &self,
        plan: &semantic_db_core::PhysicalPlan,
        default_collection: Option<&str>,
    ) -> std::result::Result<Vec<Object>, DbError> {
        let context = self.query_context();
        let source = KvPhysicalDataSource {
            db: self,
            catalog: self.catalog(),
            default_collection: default_collection.map(ToOwned::to_owned),
        };
        semantic_db_core::execute_physical_plan_with_source(plan, &source, &context)
            .map_err(|err| DbError::InvalidQuery(err.to_string()))
    }

    pub fn update_where(
        &mut self,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        self.update_where_returning(query)
            .map(|result| result.stats)
    }

    pub fn update_where_returning(
        &mut self,
        query: UpdateQuery,
    ) -> std::result::Result<semantic_db_core::UpdateResult, DbError> {
        let collection = query.collection_or_default().to_string();
        let catalog = self.catalog();
        let collection_schema = catalog
            .collection_by_name(&collection)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: collection.clone(),
            })?
            .clone();
        ensure_collection_mutable(&collection_schema)?;

        let query = canonicalize_update_query(&query, catalog.as_ref(), &collection_schema)?;
        let collection_name = collection_schema.name.clone();
        let touched = Batch::new().with_op(BatchOperation::Update {
            collection: collection_name.clone(),
            query: query.clone(),
        });
        let txn_result = run_with_transaction_retries(TransactionOptions::default(), |_| {
            let catalog_snapshot = self.catalog.snapshot();
            let read_revision = self.store.current_revision()?;
            let before = self.load_dataset_for_batch(
                catalog_snapshot.catalog.as_ref(),
                &touched,
                read_revision,
            )?;
            let mut after = before.clone();

            let mut result = semantic_db_core::UpdateResult {
                stats: MutationStats {
                    matched: 0,
                    affected: 0,
                },
                returning: Vec::new(),
            };
            if let Some(coll) = after.get_mut(&collection_name) {
                let mut entities = coll
                    .iter()
                    .map(|(id, object)| semantic_db_core::Entity {
                        id: id.clone(),
                        collection: collection_name.clone(),
                        object: object.clone(),
                    })
                    .collect::<Vec<_>>();
                result = semantic_db_core::apply_update_with_returning(&query, &mut entities)
                    .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
                coll.clear();
                for entity in entities {
                    coll.insert(entity.id, entity.object);
                }
            }

            if self.catalog.snapshot().version != catalog_snapshot.version {
                return Err(DbError::TransactionConflict(
                    "catalog changed during transaction".to_string(),
                ));
            }

            match self.persist_dataset_delta(
                catalog_snapshot.catalog.as_ref(),
                &before,
                &after,
                read_revision,
                &[],
            )? {
                KvCommitOutcome::Committed { .. } => Ok(result),
                KvCommitOutcome::Conflict {
                    expected_revision,
                    actual_revision,
                } => Err(DbError::TransactionConflict(format!(
                    "expected revision {:?}, found {:?}",
                    expected_revision, actual_revision
                ))),
            }
        })?;
        let mut value = txn_result.value;
        let catalog = self.catalog();
        value.returning =
            self.format_output_rows(catalog.as_ref(), value.returning, query.field_format);
        Ok(value)
    }

    pub fn delete_where(&mut self, query: DeleteQuery) -> std::result::Result<usize, DbError> {
        self.delete_where_returning(query)
            .map(|result| result.deleted)
    }

    pub fn delete_where_returning(
        &mut self,
        query: DeleteQuery,
    ) -> std::result::Result<semantic_db_core::DeleteResult, DbError> {
        let collection = query.collection_or_default().to_string();
        let catalog = self.catalog();
        let collection_schema = catalog
            .collection_by_name(&collection)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: collection.clone(),
            })?
            .clone();
        ensure_collection_mutable(&collection_schema)?;

        let query = canonicalize_delete_query(&query, catalog.as_ref(), &collection_schema)?;
        let collection_name = collection_schema.name.clone();
        let touched = Batch::new().with_op(BatchOperation::Delete {
            collection: collection_name.clone(),
            query: query.clone(),
        });
        let txn_result = run_with_transaction_retries(TransactionOptions::default(), |_| {
            let catalog_snapshot = self.catalog.snapshot();
            let read_revision = self.store.current_revision()?;
            let before = self.load_dataset_for_batch(
                catalog_snapshot.catalog.as_ref(),
                &touched,
                read_revision,
            )?;
            let mut after = before.clone();

            let mut result = semantic_db_core::DeleteResult {
                deleted: 0,
                returning: Vec::new(),
            };
            if let Some(coll) = after.get_mut(&collection_name) {
                let entities = coll
                    .iter()
                    .map(|(id, object)| semantic_db_core::Entity {
                        id: id.clone(),
                        collection: collection_name.clone(),
                        object: object.clone(),
                    })
                    .collect::<Vec<_>>();
                result = semantic_db_core::apply_delete_with_returning(&query, entities);

                let stripped = DeleteQuery {
                    collection: query.collection.clone(),
                    predicate: query.predicate.clone(),
                    limit: query.limit.clone(),
                    returning: Vec::new(),
                    field_format: query.field_format,
                };
                let (remaining, _) = semantic_db_core::apply_delete(
                    &stripped,
                    coll.iter()
                        .map(|(id, object)| semantic_db_core::Entity {
                            id: id.clone(),
                            collection: collection_name.clone(),
                            object: object.clone(),
                        })
                        .collect(),
                );
                coll.clear();
                for entity in remaining {
                    coll.insert(entity.id, entity.object);
                }
            }

            if self.catalog.snapshot().version != catalog_snapshot.version {
                return Err(DbError::TransactionConflict(
                    "catalog changed during transaction".to_string(),
                ));
            }

            match self.persist_dataset_delta(
                catalog_snapshot.catalog.as_ref(),
                &before,
                &after,
                read_revision,
                &[],
            )? {
                KvCommitOutcome::Committed { .. } => Ok(result),
                KvCommitOutcome::Conflict {
                    expected_revision,
                    actual_revision,
                } => Err(DbError::TransactionConflict(format!(
                    "expected revision {:?}, found {:?}",
                    expected_revision, actual_revision
                ))),
            }
        })?;
        let mut value = txn_result.value;
        let catalog = self.catalog();
        value.returning =
            self.format_output_rows(catalog.as_ref(), value.returning, query.field_format);
        Ok(value)
    }

    pub fn transact(&mut self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        self.transact_with_options(batch, TransactionOptions::default())
    }

    pub fn transact_with_options(
        &mut self,
        batch: Batch,
        options: TransactionOptions,
    ) -> std::result::Result<BatchOutcome, DbError> {
        let caps = self.store.tx_capabilities();
        if options.concurrency == TransactionConcurrency::Mvcc && !caps.mvcc {
            return Err(DbError::InvalidQuery(
                "mvcc transaction requested but backend does not support mvcc".to_string(),
            ));
        }
        if options.read_only && !batch.operations.is_empty() {
            return Err(DbError::InvalidQuery(
                "read-only transaction cannot contain mutation operations".to_string(),
            ));
        }

        let txn_result = run_with_transaction_retries(options, |_| {
            let catalog_snapshot = self.catalog.snapshot();
            let batch = self.canonicalize_batch(&batch, catalog_snapshot.catalog.as_ref())?;
            let read_revision = self.store.current_revision()?;
            let dataset = self.load_dataset_for_batch(
                catalog_snapshot.catalog.as_ref(),
                &batch,
                read_revision,
            )?;
            let out = execute_batch(&dataset, &batch)
                .map_err(|err| DbError::InvalidQuery(err.to_string()))?;

            if self.catalog.snapshot().version != catalog_snapshot.version {
                return Err(DbError::TransactionConflict(
                    "catalog changed during transaction".to_string(),
                ));
            }

            match self.persist_dataset_delta(
                catalog_snapshot.catalog.as_ref(),
                &dataset,
                &out.dataset,
                read_revision,
                &[],
            )? {
                KvCommitOutcome::Committed { .. } => Ok(out),
                KvCommitOutcome::Conflict {
                    expected_revision,
                    actual_revision,
                } => Err(DbError::TransactionConflict(format!(
                    "expected revision {:?}, found {:?}",
                    expected_revision, actual_revision
                ))),
            }
        })?;
        Ok(txn_result.value)
    }

    pub fn execute_batch(&mut self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        self.transact_with_options(batch, TransactionOptions::default())
    }

    pub fn transact_ddl_with_options(
        &mut self,
        ddl: DdlBatch,
        options: TransactionOptions,
    ) -> std::result::Result<DdlOutcome, DbError> {
        if options.read_only && !ddl.operations.is_empty() {
            return Err(DbError::InvalidQuery(
                "read-only transaction cannot contain ddl operations".to_string(),
            ));
        }

        let txn_result = run_with_transaction_retries(options, |_| {
            let catalog_snapshot = self.catalog.snapshot();
            let read_revision = self.store.current_revision()?;
            let (next_catalog, ddl_outcome) =
                apply_ddl_batch(catalog_snapshot.catalog.as_ref(), &ddl)
                    .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
            let mut extra_ops =
                self.ddl_cleanup_ops(catalog_snapshot.catalog.as_ref(), &next_catalog)?;
            extra_ops.extend(catalog_write_ops(&self.store, &next_catalog)?);
            self.rebuild_relationship_edges(&next_catalog, &BTreeMap::new(), &mut extra_ops)?;

            let dataset = BTreeMap::new();
            match self.persist_dataset_delta(
                catalog_snapshot.catalog.as_ref(),
                &dataset,
                &dataset,
                read_revision,
                &extra_ops,
            )? {
                KvCommitOutcome::Committed { .. } => {
                    self.catalog
                        .compare_and_swap(catalog_snapshot.version, next_catalog)
                        .map_err(|mismatch| {
                            DbError::TransactionConflict(format!(
                                "catalog version changed: expected {}, actual {}",
                                mismatch.expected, mismatch.actual
                            ))
                        })?;
                    Ok(ddl_outcome)
                }
                KvCommitOutcome::Conflict {
                    expected_revision,
                    actual_revision,
                } => Err(DbError::TransactionConflict(format!(
                    "expected revision {:?}, found {:?}",
                    expected_revision, actual_revision
                ))),
            }
        })?;

        Ok(txn_result.value)
    }

    pub fn transact_ddl(&mut self, ddl: DdlBatch) -> std::result::Result<DdlOutcome, DbError> {
        self.transact_ddl_with_options(ddl, TransactionOptions::default())
    }

    fn apply_package_update(
        &self,
        base_catalog: &Catalog,
        read_revision: Option<u64>,
        package: &Package,
    ) -> std::result::Result<
        (
            Catalog,
            BTreeMap<String, BTreeMap<String, Object>>,
            BTreeMap<String, BTreeMap<String, Object>>,
            Vec<AppliedMigration>,
        ),
        DbError,
    > {
        let mut next_catalog = base_catalog.clone();
        let mut before = BTreeMap::<String, BTreeMap<String, Object>>::new();
        let mut after = BTreeMap::<String, BTreeMap<String, Object>>::new();
        let mut executed_migrations = Vec::<AppliedMigration>::new();

        for migration in &package.migrations {
            if let Some(applied) =
                next_catalog.applied_migration(&package.name, &migration.module, &migration.name)
            {
                let applied_migration = applied.migration.clone();
                if applied_migration != *migration {
                    let message = format!(
                        "applied migration '{}::{}' for package '{}' differs from the stored definition",
                        migration.module, migration.name, package.name
                    );
                    match self.config.migration_mismatch_policy {
                        MigrationMismatchPolicy::Fail => {
                            return Err(DbError::InvalidQuery(message));
                        }
                        MigrationMismatchPolicy::Log => {
                            tracing::error!("{message}");
                        }
                    }
                }
                Self::reconcile_applied_migration_schema(&mut next_catalog, &applied_migration)?;
                continue;
            }

            let mut pending_ddl = Vec::new();
            for operation in &migration.operations {
                match operation {
                    MigrationOperation::Ddl(operation) => {
                        pending_ddl.push(operation);
                    }
                    MigrationOperation::Insert {
                        collection,
                        id,
                        object,
                    } => {
                        if !pending_ddl.is_empty() {
                            apply_migration_ddl_batch(
                                &mut next_catalog,
                                &migration.module,
                                pending_ddl.iter().copied(),
                            )
                            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
                            pending_ddl.clear();
                        }
                        let batch = Batch::new().with_op(BatchOperation::Upsert {
                            collection: collection.clone(),
                            id: id.clone(),
                            object: object.clone(),
                        });
                        self.apply_package_data_batch(
                            base_catalog,
                            &next_catalog,
                            read_revision,
                            batch,
                            &mut before,
                            &mut after,
                        )?;
                    }
                    MigrationOperation::Update { query } => {
                        if !pending_ddl.is_empty() {
                            apply_migration_ddl_batch(
                                &mut next_catalog,
                                &migration.module,
                                pending_ddl.iter().copied(),
                            )
                            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
                            pending_ddl.clear();
                        }
                        let query: UpdateQuery = query.clone().into();
                        let batch = Batch::new().with_op(BatchOperation::Update {
                            collection: query.collection_or_default().to_string(),
                            query,
                        });
                        self.apply_package_data_batch(
                            base_catalog,
                            &next_catalog,
                            read_revision,
                            batch,
                            &mut before,
                            &mut after,
                        )?;
                    }
                    MigrationOperation::Delete { query } => {
                        if !pending_ddl.is_empty() {
                            apply_migration_ddl_batch(
                                &mut next_catalog,
                                &migration.module,
                                pending_ddl.iter().copied(),
                            )
                            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
                            pending_ddl.clear();
                        }
                        let query: DeleteQuery = query.clone().into();
                        let batch = Batch::new().with_op(BatchOperation::Delete {
                            collection: query.collection_or_default().to_string(),
                            query,
                        });
                        self.apply_package_data_batch(
                            base_catalog,
                            &next_catalog,
                            read_revision,
                            batch,
                            &mut before,
                            &mut after,
                        )?;
                    }
                }
            }
            if !pending_ddl.is_empty() {
                apply_migration_ddl_batch(
                    &mut next_catalog,
                    &migration.module,
                    pending_ddl.iter().copied(),
                )
                .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
            }

            let applied = AppliedMigration {
                package: package.name.clone(),
                migration: migration.clone(),
            };
            next_catalog.record_applied_migration(applied.clone());
            executed_migrations.push(applied);
        }

        next_catalog.upsert_package(package.clone());

        Ok((next_catalog, before, after, executed_migrations))
    }

    fn reconcile_applied_migration_schema(
        catalog: &mut Catalog,
        migration: &Migration,
    ) -> std::result::Result<(), DbError> {
        apply_migration_ddl_batch(
            catalog,
            &migration.module,
            migration
                .operations
                .iter()
                .filter_map(|operation| match operation {
                    MigrationOperation::Ddl(operation) => Some(operation),
                    MigrationOperation::Insert { .. }
                    | MigrationOperation::Update { .. }
                    | MigrationOperation::Delete { .. } => None,
                }),
        )
        .map_err(|err| DbError::InvalidQuery(err.to_string()))
    }

    fn apply_package_data_batch(
        &self,
        base_catalog: &Catalog,
        current_catalog: &Catalog,
        read_revision: Option<u64>,
        batch: Batch,
        before: &mut BTreeMap<String, BTreeMap<String, Object>>,
        after: &mut BTreeMap<String, BTreeMap<String, Object>>,
    ) -> std::result::Result<(), DbError> {
        let batch = self.canonicalize_batch(&batch, current_catalog)?;
        let touched = touched_collections(&batch);
        for collection_name in &touched {
            if !before.contains_key(collection_name) {
                let rows =
                    self.load_collection_dataset(base_catalog, collection_name, read_revision)?;
                before.insert(collection_name.clone(), rows.clone());
                after.insert(collection_name.clone(), rows);
            }
        }

        let dataset = touched
            .iter()
            .map(|collection_name| {
                (
                    collection_name.clone(),
                    after.get(collection_name).cloned().unwrap_or_default(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let out = execute_batch(&dataset, &batch)
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
        for (collection_name, rows) in out.dataset {
            after.insert(collection_name, rows);
        }
        Ok(())
    }

    fn load_collection_dataset(
        &self,
        catalog: &Catalog,
        collection_name: &str,
        read_revision: Option<u64>,
    ) -> std::result::Result<BTreeMap<String, Object>, DbError> {
        let Some(collection) = catalog.collection_by_name(collection_name) else {
            return Ok(BTreeMap::new());
        };
        let rows = if self.store.tx_capabilities().snapshot_reads {
            if let Some(revision) = read_revision {
                self.store
                    .scan_collection_at_revision(collection.lid, revision)?
            } else {
                self.store.scan_collection(collection.lid)?
            }
        } else {
            self.store.scan_collection(collection.lid)?
        };
        Ok(rows
            .into_iter()
            .map(|row| (row.id, row.object))
            .collect::<BTreeMap<_, _>>())
    }

    fn canonicalize_batch(
        &self,
        batch: &Batch,
        catalog: &Catalog,
    ) -> std::result::Result<Batch, DbError> {
        let mut canonical_ops = Vec::with_capacity(batch.operations.len());
        for op in batch.operations.iter().cloned() {
            match op {
                BatchOperation::Upsert {
                    collection,
                    id,
                    object,
                } => {
                    let schema = catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    ensure_collection_mutable(schema)?;
                    canonical_ops.push(BatchOperation::Upsert {
                        collection,
                        id,
                        object,
                    });
                }
                BatchOperation::DeleteById { collection, id } => {
                    let schema = catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    ensure_collection_mutable(schema)?;
                    canonical_ops.push(BatchOperation::DeleteById { collection, id });
                }
                BatchOperation::DeleteByIds { collection, ids } => {
                    let schema = catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    ensure_collection_mutable(schema)?;
                    canonical_ops.push(BatchOperation::DeleteByIds { collection, ids });
                }
                BatchOperation::Update { collection, query } => {
                    let schema = catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    ensure_collection_mutable(schema)?;
                    let query = canonicalize_update_query(&query, catalog, schema)?;
                    canonical_ops.push(BatchOperation::Update { collection, query });
                }
                BatchOperation::Delete { collection, query } => {
                    let schema = catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    ensure_collection_mutable(schema)?;
                    let query = canonicalize_delete_query(&query, catalog, schema)?;
                    canonical_ops.push(BatchOperation::Delete { collection, query });
                }
            }
        }

        Ok(Batch {
            operations: canonical_ops,
        })
    }

    fn format_output_rows(
        &self,
        catalog: &Catalog,
        rows: Vec<Object>,
        format: FieldFormat,
    ) -> Vec<Object> {
        rows.into_iter()
            .map(|row| self.format_output_object(catalog, row, format))
            .collect()
    }

    fn format_output_object(
        &self,
        catalog: &Catalog,
        object: Object,
        format: FieldFormat,
    ) -> Object {
        if format == FieldFormat::Qualified {
            return object;
        }

        let mut out = Object::new();
        for (key, value) in object {
            let next_key = if let Some(attribute) = catalog.attribute_by_id(&key) {
                match format {
                    FieldFormat::Qualified => attribute.names.qualified_name.clone(),
                    FieldFormat::Underscore => attribute.names.underscore_name.clone(),
                    FieldFormat::Plain => attribute.names.plain_name.clone(),
                }
            } else {
                key
            };
            out.insert(next_key, value);
        }
        out
    }

    fn ddl_cleanup_ops(
        &self,
        before: &Catalog,
        after: &Catalog,
    ) -> std::result::Result<Vec<KvWriteOp>, DbError> {
        let mut ops = Vec::new();

        for (index_lid, _) in before.indexes() {
            if after.index_by_lid(index_lid).is_none() {
                for key in self.store.index_keys(index_lid)? {
                    ops.push(KvWriteOp::Delete { key });
                }
            }
        }

        for (collection_lid, _) in before.collections() {
            if after.collection_by_lid(collection_lid).is_none() {
                for key in self.store.collection_keys(collection_lid)? {
                    ops.push(KvWriteOp::Delete { key });
                }
            }
        }

        Ok(ops)
    }

    pub fn collection_rows(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<Vec<EntityRecord>, DbError> {
        let catalog = self.catalog();
        let collection_schema = catalog
            .collection_by_lid(collection)
            .ok_or(DbError::UnknownCollection(collection))?;

        self.store
            .scan_collection(collection)?
            .into_iter()
            .map(|item| {
                Ok(EntityRecord {
                    id: item.id,
                    collection: collection_schema.name.clone(),
                    object: item.object,
                })
            })
            .collect()
    }

    pub fn tx_capabilities(&self) -> KvTransactionCapabilities {
        self.store.tx_capabilities()
    }

    fn load_dataset_for_batch(
        &self,
        catalog: &Catalog,
        batch: &Batch,
        read_revision: Option<u64>,
    ) -> std::result::Result<BTreeMap<String, BTreeMap<String, Object>>, DbError> {
        let mut dataset = BTreeMap::new();
        for collection_name in touched_collections(batch) {
            let collection = catalog
                .collection_by_name(&collection_name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: collection_name.clone(),
                })?;

            let rows = if self.store.tx_capabilities().snapshot_reads {
                if let Some(revision) = read_revision {
                    self.store
                        .scan_collection_at_revision(collection.lid, revision)?
                } else {
                    self.store.scan_collection(collection.lid)?
                }
            } else {
                self.store.scan_collection(collection.lid)?
            };

            let mut objects = BTreeMap::new();
            for row in rows {
                objects.insert(row.id, row.object);
            }
            dataset.insert(collection_name, objects);
        }
        Ok(dataset)
    }

    fn persist_dataset_delta(
        &mut self,
        catalog: &Catalog,
        before: &BTreeMap<String, BTreeMap<String, Object>>,
        after: &BTreeMap<String, BTreeMap<String, Object>>,
        expected_revision: Option<u64>,
        additional_ops: &[KvWriteOp],
    ) -> std::result::Result<KvCommitOutcome, DbError> {
        let mut ops = Vec::<KvWriteOp>::new();
        let mut normalized_after = BTreeMap::<String, BTreeMap<String, Object>>::new();

        for (collection_name, new_rows) in after {
            let collection_schema = catalog
                .collection_by_name(collection_name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: collection_name.clone(),
                })?
                .clone();

            let mut normalized_rows = BTreeMap::<String, Object>::new();
            for (id, object) in new_rows {
                let mut object = object.clone();
                normalize_object_for_collection(catalog, &collection_schema, &mut object)?;
                self.validate_primary_id(&collection_schema, id, &object)?;
                normalized_rows.insert(id.clone(), object);
            }
            self.validate_unique_indexes(catalog, &collection_schema, &normalized_rows)?;
            normalized_after.insert(collection_name.clone(), normalized_rows.clone());
        }

        for (collection_name, normalized_rows) in &normalized_after {
            let collection_schema =
                catalog.collection_by_name(collection_name).ok_or_else(|| {
                    DbError::UnknownCollectionByName {
                        name: collection_name.clone(),
                    }
                })?;
            self.validate_ref_fields(
                catalog,
                collection_schema,
                normalized_rows,
                &normalized_after,
            )?;
        }

        for (collection_name, normalized_rows) in &normalized_after {
            let collection_schema = catalog
                .collection_by_name(collection_name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: collection_name.clone(),
                })?
                .clone();

            let old_rows = before.get(collection_name);
            if old_rows == Some(normalized_rows) {
                continue;
            }

            for key in self.store.collection_keys(collection_schema.lid)? {
                ops.push(KvWriteOp::Delete { key });
            }

            let indexes: Vec<_> = catalog
                .indexes_for_collection(collection_schema.lid)
                .cloned()
                .collect();
            for index in &indexes {
                for key in self.store.index_keys(index.lid)? {
                    ops.push(KvWriteOp::Delete { key });
                }
            }

            for index in &indexes {
                ops.push(KvWriteOp::Put {
                    key: crate::storage::index_format_key(index.lid),
                    value: crate::storage::index_format_value(),
                });
            }

            for (id, object) in normalized_rows {
                let entity = StoredEntity {
                    id: id.clone(),
                    collection: collection_schema.lid.0,
                    kind: infer_entity_kind(catalog, object),
                    object: object.clone(),
                };
                self.push_entity_ops(&mut ops, &entity)?;
                for index in &indexes {
                    self.push_index_ops(&mut ops, index, id, object)?;
                }
            }
        }

        self.rebuild_relationship_edges(catalog, &normalized_after, &mut ops)?;
        ops.extend_from_slice(additional_ops);

        if self.store.tx_capabilities().conflict_detection {
            self.store.write_batch_conditional(&ops, expected_revision)
        } else {
            self.store.write_batch(&ops)?;
            Ok(KvCommitOutcome::Committed {
                revision: self.store.current_revision()?,
            })
        }
    }

    fn push_entity_ops(
        &self,
        ops: &mut Vec<KvWriteOp>,
        entity: &StoredEntity,
    ) -> std::result::Result<(), DbError> {
        let key = crate::storage::entity_key(LocalCollectionId(entity.collection), &entity.id);
        let value = crate::storage::encode_entity(entity)?;
        ops.push(KvWriteOp::Put { key, value });
        Ok(())
    }

    fn push_index_ops(
        &self,
        ops: &mut Vec<KvWriteOp>,
        index: &semantic_db_core::catalog::IndexSchema,
        entity_id: &str,
        object: &Object,
    ) -> std::result::Result<(), DbError> {
        match index.schema.kind {
            IndexKind::Equality => {
                if let Some(value) = object.get(&index.canonical_field) {
                    let key = crate::storage::index_key(index.lid, None, value, entity_id)?;
                    ops.push(KvWriteOp::Put {
                        key,
                        value: Vec::new(),
                    });
                }
            }
            IndexKind::PathEquality => {
                for (path, value) in crate::storage::collect_index_entries(object) {
                    let key = crate::storage::index_key(index.lid, Some(&path), &value, entity_id)?;
                    ops.push(KvWriteOp::Put {
                        key,
                        value: Vec::new(),
                    });
                }
            }
            IndexKind::Range | IndexKind::FullText => {}
        }
        Ok(())
    }

    fn validate_unique_indexes(
        &self,
        catalog: &Catalog,
        collection: &CollectionSchema,
        rows: &BTreeMap<String, Object>,
    ) -> std::result::Result<(), DbError> {
        let indexes: Vec<_> = catalog
            .indexes_for_collection(collection.lid)
            .filter(|idx| idx.schema.unique && idx.schema.kind == IndexKind::Equality)
            .cloned()
            .collect();

        for index in indexes {
            let mut seen = BTreeMap::<Value, String>::new();
            for (id, object) in rows {
                let Some(value) = object.get(&index.canonical_field) else {
                    continue;
                };
                if let Some(existing) = seen.insert(value.clone(), id.clone()) {
                    return Err(DbError::InvalidQuery(format!(
                        "unique index violation on field '{}' ({existing} vs {id})",
                        index.canonical_field
                    )));
                }
            }
        }

        Ok(())
    }

    fn validate_ref_fields(
        &self,
        catalog: &Catalog,
        collection: &CollectionSchema,
        rows: &BTreeMap<String, Object>,
        all_after: &BTreeMap<String, BTreeMap<String, Object>>,
    ) -> std::result::Result<(), DbError> {
        let target_rows =
            all_after
                .get(&collection.name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: collection.name.clone(),
                })?;

        for object in rows.values() {
            let field_types = resolved_field_types_for_object(catalog, collection, object);
            for (field, ty) in field_types {
                if field == ATTR_RELATION_FROM || field == ATTR_RELATION_TO {
                    continue;
                }
                let Some(value) = object.get(&field) else {
                    continue;
                };
                validate_ref_value(catalog, collection, target_rows, &field, &ty, value)?;
            }
        }

        Ok(())
    }

    fn validate_primary_id(
        &self,
        collection: &CollectionSchema,
        id: &str,
        object: &Object,
    ) -> std::result::Result<(), DbError> {
        let canonical_id = collection.canonical_field_name("id").to_string();
        let Some(value) = object.get(&canonical_id) else {
            return Err(DbError::InvalidQuery(format!(
                "collection '{}' requires primary key field '{}'",
                collection.name, canonical_id
            )));
        };
        let Some(object_id) = value.as_str() else {
            return Err(DbError::InvalidQuery(format!(
                "collection '{}' primary key field '{}' must be a string",
                collection.name, canonical_id
            )));
        };
        if object_id != id {
            return Err(DbError::InvalidQuery(format!(
                "primary key mismatch in collection '{}': object id '{}' does not match row id '{}'",
                collection.name, object_id, id
            )));
        }
        Ok(())
    }

    fn extract_insert_id(
        &self,
        collection: &CollectionSchema,
        object: &Object,
    ) -> std::result::Result<String, DbError> {
        let canonical_id = collection.canonical_field_name("id");
        let mut id_value = None;
        for (field, value) in object {
            if collection.canonical_field_name(field) == canonical_id {
                if id_value.is_some() {
                    return Err(DbError::InvalidQuery(format!(
                        "insert row for collection '{}' provides multiple primary key aliases",
                        collection.name
                    )));
                }
                id_value = Some(value);
            }
        }

        let Some(value) = id_value else {
            return Err(DbError::InvalidQuery(format!(
                "collection '{}' requires primary key field '{}'",
                collection.name, canonical_id
            )));
        };
        let Some(id) = value.as_str() else {
            return Err(DbError::InvalidQuery(format!(
                "collection '{}' primary key field '{}' must be a string",
                collection.name, canonical_id
            )));
        };
        Ok(id.to_string())
    }

    fn rebuild_relationship_edges(
        &self,
        catalog: &Catalog,
        after: &BTreeMap<String, BTreeMap<String, Object>>,
        ops: &mut Vec<KvWriteOp>,
    ) -> std::result::Result<(), DbError> {
        let Some(rel_collection) = catalog.collection_by_name(RELATION_EDGES_COLLECTION) else {
            return Ok(());
        };
        for key in self.store.collection_keys(rel_collection.lid)? {
            ops.push(KvWriteOp::Delete { key });
        }
        let indexes: Vec<_> = catalog
            .indexes_for_collection(rel_collection.lid)
            .cloned()
            .collect();
        for index in &indexes {
            for key in self.store.index_keys(index.lid)? {
                ops.push(KvWriteOp::Delete { key });
            }
        }
        for index in &indexes {
            ops.push(KvWriteOp::Put {
                key: crate::storage::index_format_key(index.lid),
                value: crate::storage::index_format_value(),
            });
        }

        let rows = self.compute_relationship_edges(catalog, after)?;
        for (id, object) in rows {
            let entity = StoredEntity {
                id: id.clone(),
                collection: rel_collection.lid.0,
                kind: StoredEntityKind::Untyped,
                object,
            };
            self.push_entity_ops(ops, &entity)?;
            for index in &indexes {
                self.push_index_ops(ops, index, &id, &entity.object)?;
            }
        }
        Ok(())
    }

    fn compute_relationship_edges(
        &self,
        catalog: &Catalog,
        after: &BTreeMap<String, BTreeMap<String, Object>>,
    ) -> std::result::Result<Vec<(String, Object)>, DbError> {
        let mut out = Vec::new();
        for (_, rel_schema) in catalog.relationships() {
            let relationship = &rel_schema.relationship;
            if relationship.source_collection == RELATION_EDGES_COLLECTION {
                continue;
            }
            let source_collection = catalog
                .collection_by_name(&relationship.source_collection)
                .ok_or_else(|| {
                    DbError::InvalidQuery(format!(
                        "relationship '{}' references unknown source collection '{}'",
                        relationship.id, relationship.source_collection
                    ))
                })?;
            let source_rows = if let Some(rows) = after.get(&relationship.source_collection) {
                rows.iter()
                    .map(|(id, object)| (id.clone(), object.clone()))
                    .collect::<Vec<_>>()
            } else {
                self.store
                    .scan_collection(source_collection.lid)?
                    .into_iter()
                    .map(|row| (row.id, row.object))
                    .collect::<Vec<_>>()
            };
            let mut direct = Vec::<(String, String)>::new();
            match &relationship.mode {
                RelationMode::Embedded { attribute } => {
                    let canonical_field = catalog
                        .attribute_by_id(attribute)
                        .map(|attr| attr.attribute.id.clone())
                        .unwrap_or_else(|| {
                            source_collection
                                .canonical_field_name(attribute)
                                .to_string()
                        });
                    for (source_id, object) in &source_rows {
                        let Some(target_id) = object
                            .get(&canonical_field)
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                        else {
                            continue;
                        };
                        if target_id.is_empty() {
                            continue;
                        }
                        direct.push((source_id.clone(), target_id));
                    }
                }
                RelationMode::External => {
                    for (_, object) in &source_rows {
                        let Some(source_id) = Self::external_relation_field_value(
                            catalog,
                            source_collection,
                            object,
                            "from",
                            ATTR_RELATION_FROM,
                        ) else {
                            continue;
                        };
                        let Some(target_id) = object
                            .get(&Self::external_relation_field_name(
                                catalog,
                                source_collection,
                                object,
                                "to",
                                ATTR_RELATION_TO,
                            ))
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                        else {
                            continue;
                        };
                        if source_id.is_empty() || target_id.is_empty() {
                            continue;
                        }
                        direct.push((source_id, target_id));
                    }
                }
            }
            let mut shortest = BTreeMap::<(String, String), usize>::new();
            if relationship.indexing_mode == RelationIndexingMode::Enabled {
                let mut adjacency = BTreeMap::<String, Vec<String>>::new();
                for (source, target) in &direct {
                    adjacency
                        .entry(source.clone())
                        .or_default()
                        .push(target.clone());
                    let key = (source.clone(), target.clone());
                    shortest
                        .entry(key)
                        .and_modify(|depth| *depth = (*depth).min(1))
                        .or_insert(1);
                }
                for source in adjacency.keys() {
                    let mut queue = std::collections::VecDeque::new();
                    let mut seen = BTreeMap::<String, usize>::new();
                    queue.push_back((source.clone(), 0usize));
                    seen.insert(source.clone(), 0);
                    while let Some((node, depth)) = queue.pop_front() {
                        let Some(targets) = adjacency.get(&node) else {
                            continue;
                        };
                        for next in targets {
                            let next_depth = depth.saturating_add(1);
                            let entry = seen.get(next).copied();
                            if entry.is_none_or(|existing| next_depth < existing) {
                                seen.insert(next.clone(), next_depth);
                                queue.push_back((next.clone(), next_depth));
                                let key = (source.clone(), next.clone());
                                shortest
                                    .entry(key)
                                    .and_modify(|existing| *existing = (*existing).min(next_depth))
                                    .or_insert(next_depth);
                            }
                        }
                    }
                }
            } else {
                for (source, target) in &direct {
                    let key = (source.clone(), target.clone());
                    shortest
                        .entry(key)
                        .and_modify(|depth| *depth = (*depth).min(1))
                        .or_insert(1);
                }
            }
            for ((source, target), depth) in shortest {
                let id = format!("{}|{}|{}", relationship.id, source, target);
                let mut object = Object::new();
                object.insert("id", Value::String(id.clone()));
                object.insert(
                    REL_EDGE_RELATION_FIELD.to_string(),
                    Value::String(relationship.id.clone()),
                );
                object.insert(
                    REL_EDGE_SOURCE_FIELD.to_string(),
                    Value::String(source.clone()),
                );
                object.insert(
                    REL_EDGE_TARGET_FIELD.to_string(),
                    Value::String(target.clone()),
                );
                object.insert(REL_EDGE_DEPTH_FIELD.to_string(), Value::U64(depth as u64));
                object.insert(
                    REL_EDGE_SOURCE_KEY_FIELD.to_string(),
                    Value::String(format!("{}|{}", relationship.id, source)),
                );
                object.insert(
                    REL_EDGE_TARGET_KEY_FIELD.to_string(),
                    Value::String(format!("{}|{}", relationship.id, target)),
                );
                out.push((id, object));
            }
        }
        Ok(out)
    }

    fn external_relation_field_name(
        catalog: &Catalog,
        source_collection: &CollectionSchema,
        object: &Object,
        alias: &str,
        fallback_attr: &str,
    ) -> String {
        if let Some(object_type) = object.get(OBJECT_TYPE_FIELD).and_then(Value::as_str) {
            let class_ids = catalog.class_ids(object_type);
            if class_ids.len() == 1
                && let Some(field) = Self::class_field_for_alias(catalog, class_ids[0], alias)
            {
                return field;
            }
        }

        source_collection
            .canonical_field_name(fallback_attr)
            .to_string()
    }

    fn external_relation_field_value(
        catalog: &Catalog,
        source_collection: &CollectionSchema,
        object: &Object,
        alias: &str,
        fallback_attr: &str,
    ) -> Option<String> {
        let field = Self::external_relation_field_name(
            catalog,
            source_collection,
            object,
            alias,
            fallback_attr,
        );
        object
            .get(&field)
            .and_then(Value::as_str)
            .map(ToString::to_string)
    }

    fn class_field_for_alias(
        catalog: &Catalog,
        class_lid: LocalClassId,
        alias: &str,
    ) -> Option<String> {
        fn visit(
            catalog: &Catalog,
            class_lid: LocalClassId,
            alias: &str,
            visited: &mut BTreeSet<LocalClassId>,
        ) -> Option<String> {
            if !visited.insert(class_lid) {
                return None;
            }

            let class = catalog.class_by_lid(class_lid)?;
            let mut out = None;

            if let Some(inherits) = &class.class.inherits
                && let Some(base_lid) = catalog.class_id(&inherits.id)
            {
                out = visit(catalog, base_lid, alias, visited);
            }

            for ext in &class.class.extends {
                if let Some(ext_lid) = catalog.class_id(&ext.id)
                    && let Some(field) = visit(catalog, ext_lid, alias, visited)
                {
                    out = Some(field);
                }
            }

            for (field_alias, class_attr) in &class.class.attributes {
                let Some(attr) = catalog.attribute_by_id(&class_attr.attribute.id) else {
                    continue;
                };
                if field_alias == alias
                    || attr.attribute.id == alias
                    || attr.names.plain_name == alias
                    || attr.names.underscore_name == alias
                {
                    out = Some(attr.attribute.id.clone());
                }
            }

            out
        }

        visit(catalog, class_lid, alias, &mut BTreeSet::new())
    }

    fn stats_for_collection(
        &self,
        collection: &CollectionSchema,
    ) -> std::result::Result<CollectionStatsSnapshot, DbError> {
        let row_count = self.store.scan_collection(collection.lid)?.len() as f64;
        let mut indexed_fields = BTreeSet::new();
        let mut unique_fields = BTreeSet::new();
        let mut indexed_field_ids = BTreeSet::new();
        let mut unique_field_ids = BTreeSet::new();
        let mut indexed_attr_ids = BTreeSet::new();
        let mut unique_attr_ids = BTreeSet::new();

        let catalog = self.catalog();
        for index in catalog.indexes_for_collection(collection.lid) {
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

        Ok(CollectionStatsSnapshot {
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
        })
    }

    fn access_path_from_physical(
        &self,
        collection: &CollectionSchema,
        physical: &semantic_db_core::PhysicalPlan,
    ) -> AccessPath {
        fn find_lookup(
            plan: &semantic_db_core::PhysicalPlan,
        ) -> Option<(&semantic_db_core::FieldRef, &Value)> {
            match plan {
                semantic_db_core::PhysicalPlan::Source(
                    semantic_db_core::PhysicalSource::IndexLookup { field, value, .. },
                ) => Some((field, value)),
                semantic_db_core::PhysicalPlan::Filter { input, .. }
                | semantic_db_core::PhysicalPlan::Sort { input, .. }
                | semantic_db_core::PhysicalPlan::Project { input, .. }
                | semantic_db_core::PhysicalPlan::Aggregate { input, .. }
                | semantic_db_core::PhysicalPlan::Limit { input, .. }
                | semantic_db_core::PhysicalPlan::Distinct { input, .. }
                | semantic_db_core::PhysicalPlan::Materialize { input, .. }
                | semantic_db_core::PhysicalPlan::Exchange { input, .. }
                | semantic_db_core::PhysicalPlan::RepartitionHash { input, .. } => {
                    find_lookup(input)
                }
                semantic_db_core::PhysicalPlan::Union { .. }
                | semantic_db_core::PhysicalPlan::Values { .. }
                | semantic_db_core::PhysicalPlan::Join(..)
                | semantic_db_core::PhysicalPlan::ApplyExists { .. }
                | semantic_db_core::PhysicalPlan::ApplyInSubquery { .. }
                | semantic_db_core::PhysicalPlan::Source(_) => None,
            }
        }

        let Some((field_ref, value)) = find_lookup(physical) else {
            return AccessPath::FullScan;
        };

        let field_path = match field_ref {
            semantic_db_core::FieldRef::CanonicalName(name) => Some(FieldPath::from_fields([name])),
            semantic_db_core::FieldRef::FieldId(field_id) => collection
                .field_name_by_id(*field_id)
                .map(|name| FieldPath::from_fields([name])),
            semantic_db_core::FieldRef::AttrId(attr_id) => {
                let mut out = None;
                for (field_id, _) in collection.fields() {
                    if collection.attr_for_field_id(field_id) == Some(*attr_id) {
                        out = collection
                            .field_name_by_id(field_id)
                            .map(|name| FieldPath::from_fields([name]));
                        break;
                    }
                }
                out
            }
            semantic_db_core::FieldRef::Path(path) => Some(path.clone()),
        };
        let Some(field_path) = field_path else {
            return AccessPath::FullScan;
        };

        let catalog = self.catalog();
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
}

fn infer_project_key_for_insert(expr: &semantic_db_core::Expr) -> String {
    if let semantic_db_core::Expr::Operand(semantic_db_core::Operand::Field(path)) = expr {
        for segment in path.segments().iter().rev() {
            if let PathSegment::Field(name) = segment {
                return name.clone();
            }
        }
    }
    "value".to_string()
}

fn infer_entity_kind(catalog: &Catalog, object: &Object) -> StoredEntityKind {
    let Some(object_type) = object.get(OBJECT_TYPE_FIELD).and_then(Value::as_str) else {
        return StoredEntityKind::Untyped;
    };

    if catalog.class_id(object_type).is_some() {
        StoredEntityKind::Class
    } else if catalog.record_type_id(object_type).is_some() {
        StoredEntityKind::Record
    } else {
        StoredEntityKind::Untyped
    }
}

fn equality_expr(path: FieldPath, value: Value) -> semantic_db_core::Expr {
    semantic_db_core::Expr::Binary {
        op: semantic_data::query::BinaryOp::Eq,
        left: Box::new(semantic_db_core::Expr::Operand(
            semantic_db_core::Operand::Field(path),
        )),
        right: Box::new(semantic_db_core::Expr::Operand(
            semantic_db_core::Operand::Literal(value),
        )),
    }
}

struct KvPhysicalDataSource<'a, E: KvEngine> {
    db: &'a KvDb<E>,
    catalog: std::sync::Arc<Catalog>,
    default_collection: Option<String>,
}

struct KvCollectionScan {
    collection_id: LocalCollectionId,
    rows: Box<dyn Iterator<Item = semantic_db_core::CoreResult<StoredEntity>> + Send>,
    field_names: BTreeMap<LocalFieldId, String>,
    attr_names: BTreeMap<LocalAttrId, String>,
    local_ref_lookup: Arc<BTreeMap<String, Object>>,
}

impl<E: KvEngine> KvPhysicalDataSource<'_, E> {
    fn try_relation_lookup_ids(
        &self,
        source_collection: &CollectionSchema,
        predicate: &semantic_db_core::Expr,
    ) -> semantic_db_core::CoreResult<Option<Vec<String>>> {
        let semantic_db_core::Expr::RelationExists {
            relation,
            source,
            target,
            transitive,
            max_depth,
        } = predicate
        else {
            return Ok(None);
        };
        let relation_id = semantic_db_core::evaluate_expr(&Object::new(), relation)
            .and_then(|value| value.as_str().map(ToString::to_string));
        let max_depth = max_depth
            .as_ref()
            .map(|expr| {
                semantic_db_core::evaluate_expr(&Object::new(), expr)
                    .and_then(|value| value_to_usize(&value))
            })
            .flatten();
        let Some(relation_id) = relation_id else {
            return Ok(None);
        };
        let Some(rel_collection) = self.catalog.collection_by_name(RELATION_EDGES_COLLECTION)
        else {
            return Ok(Some(Vec::new()));
        };

        let source_is_row_id = is_row_id_expr(source, source_collection);
        let target_is_row_id = is_row_id_expr(target, source_collection);
        if source_is_row_id && !target_is_row_id {
            let Some(target_id) = semantic_db_core::evaluate_expr(&Object::new(), target)
                .and_then(|value| value.as_str().map(ToString::to_string))
            else {
                return Ok(None);
            };
            let ids = self.lookup_edge_endpoint_ids(
                rel_collection.lid,
                REL_EDGE_TARGET_KEY_FIELD,
                format!("{relation_id}|{target_id}"),
                REL_EDGE_SOURCE_FIELD,
                *transitive,
                max_depth,
            )?;
            return Ok(Some(ids));
        }
        if target_is_row_id && !source_is_row_id {
            let Some(source_id) = semantic_db_core::evaluate_expr(&Object::new(), source)
                .and_then(|value| value.as_str().map(ToString::to_string))
            else {
                return Ok(None);
            };
            let ids = self.lookup_edge_endpoint_ids(
                rel_collection.lid,
                REL_EDGE_SOURCE_KEY_FIELD,
                format!("{relation_id}|{source_id}"),
                REL_EDGE_TARGET_FIELD,
                *transitive,
                max_depth,
            )?;
            return Ok(Some(ids));
        }

        Ok(None)
    }

    fn lookup_edge_endpoint_ids(
        &self,
        rel_collection: LocalCollectionId,
        key_field: &str,
        key_value: String,
        endpoint_field: &str,
        transitive: bool,
        max_depth: Option<usize>,
    ) -> semantic_db_core::CoreResult<Vec<String>> {
        let ids = if let Some(index) = self.catalog.find_equality_index(rel_collection, key_field) {
            self.db
                .store
                .scan_index_value(index.lid, None, &Value::String(key_value))
                .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
        } else {
            self.db
                .store
                .scan_collection(rel_collection)
                .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
                .into_iter()
                .map(|entity| entity.id)
                .collect()
        };
        let mut out = BTreeSet::new();
        for id in ids {
            let Some(edge) = self
                .db
                .store
                .get_entity(rel_collection, &id)
                .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
            else {
                continue;
            };
            let depth = edge
                .object
                .get(REL_EDGE_DEPTH_FIELD)
                .and_then(value_to_usize)
                .unwrap_or(usize::MAX);
            if !transitive && depth != 1 {
                continue;
            }
            if let Some(max_depth) = max_depth
                && depth > max_depth
            {
                continue;
            }
            if let Some(endpoint_id) = edge.object.get(endpoint_field).and_then(Value::as_str) {
                out.insert(endpoint_id.to_string());
            }
        }
        Ok(out.into_iter().collect())
    }

    fn source_name<'a>(&'a self, source: &'a semantic_db_core::SourceRef) -> Option<&'a str> {
        source
            .source_name
            .as_deref()
            .or(self.default_collection.as_deref())
    }

    fn resolve_collection<'a>(
        &'a self,
        source: &'a semantic_db_core::SourceRef,
    ) -> std::result::Result<&'a CollectionSchema, DbError> {
        if let Some(collection_id) = source.collection_id {
            return self
                .catalog
                .collection_by_lid(collection_id)
                .ok_or(DbError::UnknownCollection(collection_id));
        }
        let name = self.source_name(source).ok_or_else(|| {
            DbError::InvalidQuery("physical source did not specify a collection".to_string())
        })?;
        self.catalog
            .collection_by_name(name)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: name.to_string(),
            })
    }

    fn scan_all_collections(&self) -> semantic_db_core::CoreResult<Vec<KvCollectionScan>> {
        let mut scans = Vec::new();
        for (_, collection) in self.catalog.collections() {
            scans.push(self.collection_scan(collection)?);
        }
        Ok(scans)
    }

    fn scan_collections(
        &self,
        source: &semantic_db_core::SourceRef,
    ) -> semantic_db_core::CoreResult<Vec<KvCollectionScan>> {
        if self.is_all_alias_source(source) {
            return self.scan_all_collections();
        }
        let collection = self
            .resolve_collection(source)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
        Ok(vec![self.collection_scan(collection)?])
    }

    fn collection_scan(
        &self,
        collection: &CollectionSchema,
    ) -> semantic_db_core::CoreResult<KvCollectionScan> {
        let lookup_rows = self
            .db
            .store
            .scan_collection_stream(collection.lid)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
        let local_ref_lookup =
            build_local_ref_lookup(self.catalog.as_ref(), collection, lookup_rows)?;
        let (field_names, attr_names) = collection_field_maps(collection);
        let rows = self
            .db
            .store
            .scan_collection_stream(collection.lid)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
        Ok(KvCollectionScan {
            collection_id: collection.lid,
            rows: Box::new(
                rows.map(|row| {
                    row.map_err(|err| semantic_db_core::CoreError::new(err.to_string()))
                }),
            ),
            field_names,
            attr_names,
            local_ref_lookup,
        })
    }

    fn scans_to_stream(
        scans: Vec<KvCollectionScan>,
        predicate: Option<semantic_db_core::Expr>,
    ) -> semantic_db_core::SendableRecordBatchStream {
        let batch_size = semantic_db_core::DEFAULT_EXECUTION_BATCH_SIZE;
        stream::unfold(
            (scans.into_iter(), None::<KvCollectionScan>, predicate),
            move |(mut scans, mut current, predicate)| async move {
                let mut batch = Vec::with_capacity(batch_size);
                while batch.len() < batch_size {
                    if current.is_none() {
                        current = scans.next();
                    }
                    let Some(scan) = &mut current else {
                        break;
                    };
                    let Some(row) = scan.rows.next() else {
                        current = None;
                        continue;
                    };
                    let row = match row {
                        Ok(row) => row,
                        Err(err) => return Some((Err(err), (scans, current, predicate))),
                    };
                    let view = KvObjectView {
                        object: row.object,
                        collection_id: scan.collection_id,
                        field_names: scan.field_names.clone(),
                        attr_names: scan.attr_names.clone(),
                        local_ref_lookup: Some(scan.local_ref_lookup.clone()),
                    };
                    if predicate.as_ref().is_none_or(|predicate| {
                        semantic_db_core::evaluate_filter_expr(&view, predicate)
                    }) {
                        batch.push(Box::new(view) as semantic_db_core::DynObject);
                    }
                }
                if batch.is_empty() {
                    None
                } else {
                    Some((Ok(batch), (scans, current, predicate)))
                }
            },
        )
        .boxed()
    }

    fn collection_scan_filtered_with_relationships(
        &self,
        collection: &CollectionSchema,
        rows: Vec<StoredEntity>,
        predicate: &semantic_db_core::Expr,
    ) -> semantic_db_core::CoreResult<KvCollectionScan> {
        let local_ref_lookup = build_local_ref_lookup(
            self.catalog.as_ref(),
            collection,
            rows.iter().cloned().map(Ok),
        )?;
        let (field_names, attr_names) = collection_field_maps(collection);
        let mut filtered = Vec::new();
        for row in rows {
            let view = KvObjectView {
                object: row.object.clone(),
                collection_id: collection.lid,
                field_names: field_names.clone(),
                attr_names: attr_names.clone(),
                local_ref_lookup: Some(local_ref_lookup.clone()),
            };
            if self.evaluate_predicate_with_relationships(&view, predicate)? {
                filtered.push(row);
            }
        }
        Ok(KvCollectionScan {
            collection_id: collection.lid,
            rows: Box::new(filtered.into_iter().map(Ok)),
            field_names,
            attr_names,
            local_ref_lookup,
        })
    }

    fn scan_filtered_collections(
        &self,
        source: &semantic_db_core::SourceRef,
        predicate: &semantic_db_core::Expr,
    ) -> semantic_db_core::CoreResult<(Vec<KvCollectionScan>, bool)> {
        if let Ok(collection) = self.resolve_collection(source)
            && let Some(ids) = self.try_relation_lookup_ids(collection, predicate)?
        {
            return Ok((vec![self.materialize_ids(collection, ids)?], true));
        }
        if !expr_contains_relationship(predicate) {
            return Ok((self.scan_collections(source)?, false));
        }
        if self.is_all_alias_source(source) {
            let mut scans = Vec::new();
            for (_, collection) in self.catalog.collections() {
                let rows = self
                    .db
                    .store
                    .scan_collection(collection.lid)
                    .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
                scans.push(
                    self.collection_scan_filtered_with_relationships(collection, rows, predicate)?,
                );
            }
            return Ok((scans, true));
        }
        let collection = self
            .resolve_collection(source)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
        let rows = self
            .db
            .store
            .scan_collection(collection.lid)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
        Ok((
            vec![self.collection_scan_filtered_with_relationships(collection, rows, predicate)?],
            true,
        ))
    }

    fn materialize_ids(
        &self,
        collection: &CollectionSchema,
        ids: Vec<String>,
    ) -> semantic_db_core::CoreResult<KvCollectionScan> {
        let lookup_rows = self
            .db
            .store
            .scan_collection_stream(collection.lid)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
        let local_ref_lookup =
            build_local_ref_lookup(self.catalog.as_ref(), collection, lookup_rows)?;
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(entity) = self
                .db
                .store
                .get_entity(collection.lid, &id)
                .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
            {
                rows.push(entity);
            }
        }
        let (field_names, attr_names) = collection_field_maps(collection);
        Ok(KvCollectionScan {
            collection_id: collection.lid,
            rows: Box::new(rows.into_iter().map(Ok)),
            field_names,
            attr_names,
            local_ref_lookup,
        })
    }

    fn is_all_alias_source(&self, source: &semantic_db_core::SourceRef) -> bool {
        self.source_name(source)
            .is_some_and(is_all_collection_alias)
    }

    fn evaluate_predicate_with_relationships(
        &self,
        row: &dyn semantic_db_core::ObjectAccess,
        predicate: &semantic_db_core::Expr,
    ) -> semantic_db_core::CoreResult<bool> {
        match predicate {
            semantic_db_core::Expr::RelationExists {
                relation,
                source,
                target,
                transitive,
                max_depth,
            } => {
                let relation_id = semantic_db_core::evaluate_expr(row, relation)
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .ok_or_else(|| {
                        semantic_db_core::CoreError::new(
                            "relationship expression requires string relation id",
                        )
                    })?;
                let source_id = semantic_db_core::evaluate_expr(row, source)
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .ok_or_else(|| {
                        semantic_db_core::CoreError::new(
                            "relationship expression requires string source id",
                        )
                    })?;
                let target_id = semantic_db_core::evaluate_expr(row, target)
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .ok_or_else(|| {
                        semantic_db_core::CoreError::new(
                            "relationship expression requires string target id",
                        )
                    })?;
                let max_depth = max_depth
                    .as_ref()
                    .map(|expr| {
                        semantic_db_core::evaluate_expr(row, expr)
                            .and_then(|value| value_to_usize(&value))
                            .ok_or_else(|| {
                                semantic_db_core::CoreError::new(
                                    "relationship expression max_depth must be a positive integer",
                                )
                            })
                    })
                    .transpose()?;
                self.relationship_exists(
                    &relation_id,
                    &source_id,
                    &target_id,
                    *transitive,
                    max_depth,
                )
            }
            semantic_db_core::Expr::Binary {
                op: semantic_data::query::BinaryOp::And,
                left,
                right,
            } => Ok(self.evaluate_predicate_with_relationships(row, left)?
                && self.evaluate_predicate_with_relationships(row, right)?),
            semantic_db_core::Expr::Binary {
                op: semantic_data::query::BinaryOp::Or,
                left,
                right,
            } => Ok(self.evaluate_predicate_with_relationships(row, left)?
                || self.evaluate_predicate_with_relationships(row, right)?),
            semantic_db_core::Expr::Unary {
                op: semantic_data::query::UnaryOp::Not,
                expr,
            } => Ok(!self.evaluate_predicate_with_relationships(row, expr)?),
            _ => Ok(semantic_db_core::evaluate_filter_expr(row, predicate)),
        }
    }

    fn relationship_exists(
        &self,
        relation_id: &str,
        source_id: &str,
        target_id: &str,
        transitive: bool,
        max_depth: Option<usize>,
    ) -> semantic_db_core::CoreResult<bool> {
        let Some(rel_collection) = self.catalog.collection_by_name(RELATION_EDGES_COLLECTION)
        else {
            return Ok(false);
        };
        let source_key = Value::String(format!("{relation_id}|{source_id}"));
        let candidate_ids = if let Some(index) = self
            .catalog
            .find_equality_index(rel_collection.lid, REL_EDGE_SOURCE_KEY_FIELD)
        {
            self.db
                .store
                .scan_index_value(index.lid, None, &source_key)
                .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
        } else {
            self.db
                .store
                .scan_collection(rel_collection.lid)
                .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
                .into_iter()
                .map(|entity| entity.id)
                .collect()
        };
        for candidate_id in candidate_ids {
            let Some(edge) = self
                .db
                .store
                .get_entity(rel_collection.lid, &candidate_id)
                .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
            else {
                continue;
            };
            let Some(edge_relation) = edge
                .object
                .get(REL_EDGE_RELATION_FIELD)
                .and_then(Value::as_str)
            else {
                continue;
            };
            if edge_relation != relation_id {
                continue;
            }
            let Some(edge_source) = edge
                .object
                .get(REL_EDGE_SOURCE_FIELD)
                .and_then(Value::as_str)
            else {
                continue;
            };
            if edge_source != source_id {
                continue;
            }
            let Some(edge_target) = edge
                .object
                .get(REL_EDGE_TARGET_FIELD)
                .and_then(Value::as_str)
            else {
                continue;
            };
            if edge_target != target_id {
                continue;
            }
            let depth = edge
                .object
                .get(REL_EDGE_DEPTH_FIELD)
                .and_then(value_to_usize)
                .unwrap_or(usize::MAX);
            if !transitive && depth != 1 {
                continue;
            }
            if let Some(max_depth) = max_depth
                && depth > max_depth
            {
                continue;
            }
            return Ok(true);
        }
        Ok(false)
    }
}
#[derive(Clone)]
struct KvObjectView {
    object: Object,
    collection_id: LocalCollectionId,
    field_names: BTreeMap<LocalFieldId, String>,
    attr_names: BTreeMap<LocalAttrId, String>,
    local_ref_lookup: Option<Arc<BTreeMap<String, Object>>>,
}

impl semantic_db_core::ObjectAccess for KvObjectView {
    fn value_at_path_ref<'a>(&'a self, path: &FieldPath) -> Option<ValueRef<'a>> {
        if let Some(value) = semantic_db_core::ObjectAccess::value_at_path_ref(&self.object, path) {
            return Some(value);
        }
        if let [PathSegment::Field(field)] = path.segments()
            && let Some(value) = value_from_object_with_alias_fallback(&self.object, field)
        {
            return Some(ValueRef::Owned(value));
        }
        if path.segments().len() < 2 {
            return None;
        }
        resolve_path_with_local_refs(&self.object, path, self.local_ref_lookup.as_deref())
            .map(ValueRef::Owned)
    }

    fn value_at_attr_ref<'a>(&'a self, attr: LocalAttrId) -> Option<ValueRef<'a>> {
        let name = self.attr_names.get(&attr)?;
        let path = FieldPath::from_fields([name.as_str()]);
        semantic_db_core::ObjectAccess::value_at_path_ref(&self.object, &path)
    }

    fn value_at_field_ref<'a>(&'a self, field: LocalFieldId) -> Option<ValueRef<'a>> {
        let name = self.field_names.get(&field)?;
        let path = FieldPath::from_fields([name.as_str()]);
        semantic_db_core::ObjectAccess::value_at_path_ref(&self.object, &path)
    }

    fn collection_id(&self) -> Option<LocalCollectionId> {
        Some(self.collection_id)
    }

    fn to_object(&self) -> Object {
        self.object.clone()
    }
}

fn build_local_ref_lookup(
    catalog: &Catalog,
    collection: &CollectionSchema,
    rows: impl IntoIterator<Item = std::result::Result<StoredEntity, DbError>>,
) -> semantic_db_core::CoreResult<Arc<BTreeMap<String, Object>>> {
    let mut lookup = BTreeMap::new();
    let mut id_keys = vec![
        collection.canonical_field_name("id").to_string(),
        "id".to_string(),
    ];
    for attr_id in catalog.attribute_ids("id") {
        if let Some(attr) = catalog.attribute_by_lid(attr_id) {
            let candidate = attr.attribute.id.clone();
            if !id_keys.contains(&candidate) {
                id_keys.push(candidate);
            }
        }
    }
    for row in rows {
        let row = row.map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
        let id = id_keys
            .iter()
            .find_map(|key| row.object.get(key).and_then(Value::as_str));
        if let Some(id) = id {
            lookup.insert(id.to_string(), row.object.clone());
        }
    }
    Ok(Arc::new(lookup))
}

fn resolve_path_with_local_refs(
    object: &Object,
    path: &FieldPath,
    lookup: Option<&BTreeMap<String, Object>>,
) -> Option<Value> {
    let mut current = match path.segments().first()? {
        PathSegment::Field(field) => value_from_object_with_alias_fallback(object, field)?,
        PathSegment::Index(_) => return None,
    };
    for segment in path.segments().iter().skip(1) {
        current = match (&current, segment) {
            (Value::Object(map), PathSegment::Field(field)) => {
                value_from_object_with_alias_fallback(map, field)?
            }
            (Value::List(items), PathSegment::Index(index)) => items.get(*index)?.clone(),
            // Fallback: treat string ids as same-collection refs.
            (Value::String(id), PathSegment::Field(field)) => {
                let target = lookup?.get(id)?;
                value_from_object_with_alias_fallback(target, field)?
            }
            _ => return None,
        };
    }
    Some(current)
}

fn value_from_object_with_alias_fallback(object: &Object, field: &str) -> Option<Value> {
    if let Some(value) = object.get(field) {
        return Some(value.clone());
    }
    let wanted_plain = field.rsplit(':').next().unwrap_or(field);
    let mut matching = object.iter().filter(|(key, _)| {
        key.rsplit(':')
            .next()
            .is_some_and(|plain| plain == wanted_plain)
    });
    let (_, value) = matching.next()?;
    if matching.next().is_some() {
        return None;
    }
    Some(value.clone())
}

fn collection_field_maps(
    collection: &CollectionSchema,
) -> (
    BTreeMap<LocalFieldId, String>,
    BTreeMap<LocalAttrId, String>,
) {
    let mut field_names = BTreeMap::new();
    let mut attr_names = BTreeMap::new();
    for (field_id, name) in collection.fields() {
        field_names.insert(field_id, name.to_string());
        if let Some(attr_id) = collection.attr_for_field_id(field_id) {
            attr_names.insert(attr_id, name.to_string());
        }
    }
    (field_names, attr_names)
}

fn expr_contains_relationship(expr: &semantic_db_core::Expr) -> bool {
    match expr {
        semantic_db_core::Expr::RelationExists { .. } => true,
        semantic_db_core::Expr::Binary { left, right, .. } => {
            expr_contains_relationship(left) || expr_contains_relationship(right)
        }
        semantic_db_core::Expr::Unary { expr, .. } => expr_contains_relationship(expr),
        semantic_db_core::Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_contains_relationship(cond)
                || expr_contains_relationship(then_expr)
                || expr_contains_relationship(else_expr)
        }
        semantic_db_core::Expr::Coalesce(exprs) => exprs.iter().any(expr_contains_relationship),
        semantic_db_core::Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            semantic_db_core::FunctionArg::Expr(expr) => expr_contains_relationship(expr),
            semantic_db_core::FunctionArg::Wildcard => false,
        }),
        semantic_db_core::Expr::Aggregate { arg, .. } => match arg.as_ref() {
            semantic_db_core::FunctionArg::Expr(expr) => expr_contains_relationship(expr),
            semantic_db_core::FunctionArg::Wildcard => false,
        },
        semantic_db_core::Expr::InList { expr, list, .. } => {
            expr_contains_relationship(expr) || list.iter().any(expr_contains_relationship)
        }
        semantic_db_core::Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_relationship(expr)
                || expr_contains_relationship(low)
                || expr_contains_relationship(high)
        }
        semantic_db_core::Expr::PatternMatch { expr, pattern, .. }
        | semantic_db_core::Expr::RegexMatch { expr, pattern, .. } => {
            expr_contains_relationship(expr) || expr_contains_relationship(pattern)
        }
        semantic_db_core::Expr::IsNull { expr, .. } => expr_contains_relationship(expr),
        semantic_db_core::Expr::Operand(_)
        | semantic_db_core::Expr::Subquery(_)
        | semantic_db_core::Expr::Exists { .. } => false,
    }
}

fn mark_collection_internal(
    catalog: &mut Catalog,
    name: &str,
) -> std::result::Result<bool, DbError> {
    let Some(collection) = catalog.collection_by_name(name) else {
        return Ok(false);
    };
    if collection.internal {
        return Ok(false);
    }
    catalog
        .set_collection_internal(name, true)
        .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
    Ok(true)
}

fn ensure_collection_mutable(collection: &CollectionSchema) -> std::result::Result<(), DbError> {
    if collection.internal {
        return Err(DbError::InvalidQuery(format!(
            "collection '{}' is internal and cannot be modified directly",
            collection.name
        )));
    }
    Ok(())
}

fn validate_ref_value(
    catalog: &Catalog,
    collection: &CollectionSchema,
    target_rows: &BTreeMap<String, Object>,
    field: &str,
    ty: &Type,
    value: &Value,
) -> std::result::Result<(), DbError> {
    match &ty.kind {
        TypeKind::Ref(_) => validate_one_ref(catalog, collection, target_rows, field, ty, value),
        TypeKind::Optional(optional) => {
            if value.is_nullish() {
                Ok(())
            } else {
                validate_ref_value(
                    catalog,
                    collection,
                    target_rows,
                    field,
                    &optional.inner,
                    value,
                )
            }
        }
        TypeKind::Union(union) => {
            if value.is_nullish() && union.variants.iter().any(type_allows_nullish) {
                return Ok(());
            }
            for variant in &union.variants {
                if contains_ref_type(variant) {
                    return validate_ref_value(
                        catalog,
                        collection,
                        target_rows,
                        field,
                        variant,
                        value,
                    );
                }
            }
            Ok(())
        }
        TypeKind::List(list) if contains_ref_type(&list.items) => {
            let Value::List(items) = value else {
                return Err(DbError::InvalidQuery(format!(
                    "ref field '{}' in collection '{}' must be a list of string ids",
                    field, collection.name
                )));
            };
            for item in items {
                validate_ref_value(catalog, collection, target_rows, field, &list.items, item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_one_ref(
    catalog: &Catalog,
    collection: &CollectionSchema,
    target_rows: &BTreeMap<String, Object>,
    field: &str,
    ty: &Type,
    value: &Value,
) -> std::result::Result<(), DbError> {
    if value.is_nullish() {
        return Ok(());
    }

    let Value::String(target_id) = value else {
        return Err(DbError::InvalidQuery(format!(
            "ref field '{}' in collection '{}' must be a string id",
            field, collection.name
        )));
    };
    if target_id.is_empty() {
        return Err(DbError::InvalidQuery(format!(
            "ref field '{}' in collection '{}' must be a non-empty string id",
            field, collection.name
        )));
    }
    let Some(target) = target_rows.get(target_id) else {
        return Err(DbError::InvalidQuery(format!(
            "ref field '{}' in collection '{}' points to missing target id '{}'",
            field, collection.name, target_id
        )));
    };

    let allowed_class_ids = ref_target_class_ids(catalog, ty);
    if allowed_class_ids.is_empty() {
        return Ok(());
    }

    let target_type = target
        .get(OBJECT_TYPE_FIELD)
        .and_then(Value::as_str)
        .unwrap_or("");
    let resolved_target_type = catalog
        .class_ids(target_type)
        .first()
        .and_then(|lid| catalog.class_by_lid(*lid))
        .map(|class| class.class.id.as_str())
        .unwrap_or(target_type);
    if allowed_class_ids
        .iter()
        .any(|class_id| class_id == resolved_target_type)
    {
        return Ok(());
    }

    Err(DbError::InvalidQuery(format!(
        "ref field '{}' in collection '{}' points to target id '{}' with type '{}', expected one of: {}",
        field,
        collection.name,
        target_id,
        target_type,
        allowed_class_ids.join(", ")
    )))
}

fn contains_ref_type(ty: &Type) -> bool {
    match &ty.kind {
        TypeKind::Ref(_) => true,
        TypeKind::Optional(optional) => contains_ref_type(&optional.inner),
        TypeKind::Union(union) => union.variants.iter().any(contains_ref_type),
        TypeKind::List(list) => contains_ref_type(&list.items),
        _ => false,
    }
}

fn type_allows_nullish(ty: &Type) -> bool {
    match &ty.kind {
        TypeKind::Optional(_) | TypeKind::Null(_) => true,
        TypeKind::Union(union) => union.variants.iter().any(type_allows_nullish),
        _ => false,
    }
}

impl<E: KvEngine> semantic_db_core::AsyncPhysicalDataSource for KvPhysicalDataSource<'_, E> {
    fn scan_stream(
        &self,
        source: semantic_db_core::SourceRef,
    ) -> semantic_db_core::SendableRecordBatchStream {
        match self.scan_collections(&source) {
            Ok(scans) => Self::scans_to_stream(scans, None),
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }

    fn scan_filtered_stream(
        &self,
        source: semantic_db_core::SourceRef,
        predicate: semantic_db_core::Expr,
    ) -> semantic_db_core::SendableRecordBatchStream {
        match self.scan_filtered_collections(&source, &predicate) {
            Ok((scans, true)) => Self::scans_to_stream(scans, None),
            Ok((scans, false)) => Self::scans_to_stream(scans, Some(predicate)),
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }

    fn index_lookup_stream(
        &self,
        source: semantic_db_core::SourceRef,
        field: semantic_db_core::FieldRef,
        value: Value,
    ) -> semantic_db_core::SendableRecordBatchStream {
        match self.index_lookup_collections(&source, &field, &value) {
            Ok((scans, predicate)) => Self::scans_to_stream(scans, predicate),
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }
}

impl<E: KvEngine> KvPhysicalDataSource<'_, E> {
    fn index_lookup_collections(
        &self,
        source: &semantic_db_core::SourceRef,
        field: &semantic_db_core::FieldRef,
        value: &Value,
    ) -> semantic_db_core::CoreResult<(Vec<KvCollectionScan>, Option<semantic_db_core::Expr>)> {
        let collection = self
            .resolve_collection(source)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;

        let field_path = match field {
            semantic_db_core::FieldRef::CanonicalName(name) => Some(FieldPath::from_fields([name])),
            semantic_db_core::FieldRef::FieldId(field_id) => collection
                .field_name_by_id(*field_id)
                .map(|name| FieldPath::from_fields([name])),
            semantic_db_core::FieldRef::AttrId(attr_id) => {
                let mut found = None;
                for (field_id, _) in collection.fields() {
                    if collection.attr_for_field_id(field_id) == Some(*attr_id) {
                        found = collection
                            .field_name_by_id(field_id)
                            .map(|name| FieldPath::from_fields([name]));
                        break;
                    }
                }
                found
            }
            semantic_db_core::FieldRef::Path(path) => Some(path.clone()),
        };
        let Some(field_path) = field_path else {
            return Ok((self.scan_collections(source)?, None));
        };

        let top_level = field_path
            .segments()
            .first()
            .and_then(|segment| match segment {
                PathSegment::Field(field) => Some(field.as_str()),
                _ => None,
            });
        let ids = if let Some(top_level) = top_level {
            if field_path.segments().len() == 1 {
                if let Some(index) = self.catalog.find_equality_index(collection.lid, top_level) {
                    self.db
                        .store
                        .scan_index_value(index.lid, None, value)
                        .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
                } else if let Some(index) = self.catalog.find_path_equality_index(collection.lid) {
                    self.db
                        .store
                        .scan_index_value(index.lid, Some(&field_path), value)
                        .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
                } else {
                    let predicate = equality_expr(field_path, value.clone());
                    return Ok((self.scan_collections(source)?, Some(predicate)));
                }
            } else if let Some(index) = self.catalog.find_path_equality_index(collection.lid) {
                let has_index_segment = field_path
                    .segments()
                    .iter()
                    .any(|segment| matches!(segment, PathSegment::Index(_)));
                if has_index_segment {
                    self.db
                        .store
                        .scan_index_value(index.lid, Some(&field_path), value)
                        .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
                } else {
                    let predicate = equality_expr(field_path, value.clone());
                    return Ok((self.scan_collections(source)?, Some(predicate)));
                }
            } else {
                let predicate = equality_expr(field_path, value.clone());
                return Ok((self.scan_collections(source)?, Some(predicate)));
            }
        } else {
            return Ok((self.scan_collections(source)?, None));
        };

        Ok((vec![self.materialize_ids(collection, ids)?], None))
    }
}

fn value_to_usize(value: &Value) -> Option<usize> {
    match value {
        Value::U8(v) => Some((*v).into()),
        Value::U16(v) => Some((*v).into()),
        Value::U32(v) => usize::try_from(*v).ok(),
        Value::U64(v) => usize::try_from(*v).ok(),
        Value::U128(v) => usize::try_from(*v).ok(),
        Value::I8(v) => usize::try_from(*v).ok(),
        Value::I16(v) => usize::try_from(*v).ok(),
        Value::I32(v) => usize::try_from(*v).ok(),
        Value::I64(v) => usize::try_from(*v).ok(),
        Value::I128(v) => usize::try_from(*v).ok(),
        _ => None,
    }
}

fn is_row_id_expr(expr: &semantic_db_core::Expr, source_collection: &CollectionSchema) -> bool {
    let semantic_db_core::Expr::Operand(semantic_db_core::Operand::Field(path)) = expr else {
        return false;
    };
    let Some(PathSegment::Field(first)) = path.segments().first() else {
        return false;
    };
    path.segments().len() == 1 && source_collection.canonical_field_name(first) == "id"
}

struct CollectionStatsSnapshot {
    source: String,
    collection_id: LocalCollectionId,
    row_count: f64,
    indexed_fields: BTreeSet<String>,
    unique_fields: BTreeSet<String>,
    indexed_field_ids: BTreeSet<LocalFieldId>,
    unique_field_ids: BTreeSet<LocalFieldId>,
    indexed_attr_ids: BTreeSet<LocalAttrId>,
    unique_attr_ids: BTreeSet<LocalAttrId>,
    has_path_equality_index: bool,
}

impl semantic_db_core::StatsProvider for CollectionStatsSnapshot {
    fn relation_stats(
        &self,
        source: &semantic_db_core::SourceRef,
    ) -> Option<semantic_db_core::RelationStats> {
        if source.source_name.as_deref() == Some(self.source.as_str())
            || source.collection_id == Some(self.collection_id)
        {
            Some(semantic_db_core::RelationStats {
                row_count: self.row_count,
            })
        } else {
            None
        }
    }

    fn field_stats(
        &self,
        source: &semantic_db_core::SourceRef,
        field: &semantic_db_core::FieldRef,
    ) -> Option<semantic_db_core::FieldStats> {
        if source.source_name.as_deref() != Some(self.source.as_str())
            && source.collection_id != Some(self.collection_id)
        {
            return None;
        }

        let unique = match field {
            semantic_db_core::FieldRef::CanonicalName(name) => self.unique_fields.contains(name),
            semantic_db_core::FieldRef::FieldId(field_id) => {
                self.unique_field_ids.contains(field_id)
            }
            semantic_db_core::FieldRef::AttrId(attr_id) => self.unique_attr_ids.contains(attr_id),
            semantic_db_core::FieldRef::Path(path) => path
                .segments()
                .first()
                .and_then(|segment| match segment {
                    semantic_data::value::PathSegment::Field(name) => Some(name),
                    _ => None,
                })
                .is_some_and(|name| self.unique_fields.contains(name)),
        };
        if unique {
            return Some(semantic_db_core::FieldStats {
                distinct_count: Some(self.row_count.max(1.0)),
                null_fraction: Some(0.0),
            });
        }

        let indexed = match field {
            semantic_db_core::FieldRef::CanonicalName(name) => {
                self.indexed_fields.contains(name) || self.has_path_equality_index
            }
            semantic_db_core::FieldRef::FieldId(field_id) => {
                self.indexed_field_ids.contains(field_id) || self.has_path_equality_index
            }
            semantic_db_core::FieldRef::AttrId(attr_id) => {
                self.indexed_attr_ids.contains(attr_id) || self.has_path_equality_index
            }
            semantic_db_core::FieldRef::Path(path) => {
                path.segments()
                    .first()
                    .and_then(|segment| match segment {
                        semantic_data::value::PathSegment::Field(name) => Some(name),
                        _ => None,
                    })
                    .is_some_and(|name| self.indexed_fields.contains(name))
                    || self.has_path_equality_index
            }
        };
        if indexed {
            return Some(semantic_db_core::FieldStats {
                distinct_count: Some((self.row_count / 8.0).max(1.0)),
                null_fraction: Some(0.0),
            });
        }

        None
    }

    fn has_equality_index(
        &self,
        source: &semantic_db_core::SourceRef,
        field: &semantic_db_core::FieldRef,
    ) -> Option<bool> {
        if source.source_name.as_deref() != Some(self.source.as_str())
            && source.collection_id != Some(self.collection_id)
        {
            return None;
        }
        let indexed = match field {
            semantic_db_core::FieldRef::CanonicalName(name) => {
                self.indexed_fields.contains(name) || self.has_path_equality_index
            }
            semantic_db_core::FieldRef::FieldId(field_id) => {
                self.indexed_field_ids.contains(field_id) || self.has_path_equality_index
            }
            semantic_db_core::FieldRef::AttrId(attr_id) => {
                self.indexed_attr_ids.contains(attr_id) || self.has_path_equality_index
            }
            semantic_db_core::FieldRef::Path(path) => {
                path.segments()
                    .first()
                    .and_then(|segment| match segment {
                        semantic_data::value::PathSegment::Field(name) => Some(name),
                        _ => None,
                    })
                    .is_some_and(|name| self.indexed_fields.contains(name))
                    || self.has_path_equality_index
            }
        };
        Some(indexed)
    }
}

fn format_field_path(path: &FieldPath) -> String {
    if path.segments().is_empty() {
        return "<empty>".to_string();
    }
    let mut out = String::new();
    for segment in path.segments() {
        match segment {
            PathSegment::Field(field) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(field);
            }
            PathSegment::Index(index) => {
                out.push('[');
                out.push_str(&index.to_string());
                out.push(']');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semantic_data::{
        schema::{
            Migration, MigrationDdlOperation, MigrationOperation, Module, Package,
            attribute::attribute_ref::AttributeRef,
            attribute::attribute_type::AttributeType,
            class::class_attribute::ClassAttribute,
            class::class_ref::ClassRef,
            class::class_type::ClassType,
            collections::optional_type::OptionalType,
            core::{meta::Meta, type_kind::TypeKind, type_node::Type, type_ref::TypeRef},
            primitives::{
                bool_type::BoolType, number_type::NumberType, string_type::StringType,
                uint_width::UIntWidth,
            },
            record::field::Field,
            record::record_type::RecordType,
        },
        value::{FieldPath, Object, PathSegment, Value},
    };

    use semantic_data::query::{BinaryOp, FieldFormat, SortDirection};
    use semantic_db_core::catalog::{CollectionKind, IntegrityMode};
    use semantic_db_core::{
        ALL_COLLECTION_ALIAS, Batch, BatchOperation, CORE_CATALOG_SCHEMA_COLLECTION,
        DEFAULT_COLLECTION, DdlBatch, DdlCollectionKind, DdlOperation, Expr, Operand, OrderBy,
        Query, QueryField, QueryResult, SelectQuery, TransactionConcurrency, TransactionOptions,
        UpdateQuery, canonicalize_select_query,
    };
    use semantic_db_core::{DbConfig, MigrationMismatchPolicy};

    use super::{KvDb, QueryPlan, RELATION_EDGES_COLLECTION};

    #[test]
    fn initialization_creates_default_entities_collection() {
        let db = KvDb::in_memory();
        let catalog = db.catalog();
        let entities = catalog
            .collection_by_name(DEFAULT_COLLECTION)
            .expect("default entities collection should exist");
        assert_eq!(
            entities.integrity_mode,
            IntegrityMode::StrictRegisteredSchema
        );
    }

    #[test]
    fn initialization_marks_kv_internal_collections() {
        let db = KvDb::in_memory();
        let catalog = db.catalog();

        assert!(
            catalog
                .collection_by_name(CORE_CATALOG_SCHEMA_COLLECTION)
                .expect("schema collection")
                .internal
        );
        assert!(
            catalog
                .collection_by_name(RELATION_EDGES_COLLECTION)
                .expect("relationship edges collection")
                .internal
        );
        assert!(
            !catalog
                .collection_by_name(DEFAULT_COLLECTION)
                .expect("default collection")
                .internal
        );
    }

    #[test]
    fn rejects_direct_internal_collection_mutation() {
        let mut db = KvDb::in_memory();
        let err = db
            .insert(CORE_CATALOG_SCHEMA_COLLECTION, "entry", Object::new())
            .expect_err("internal insert should be rejected");

        assert!(
            err.to_string()
                .contains("is internal and cannot be modified directly"),
            "{err}"
        );
    }

    #[test]
    fn internal_schema_collection_remains_queryable() {
        let db = KvDb::in_memory();
        let rows = db
            .select(SelectQuery::new().with_collection(CORE_CATALOG_SCHEMA_COLLECTION))
            .expect("schema collection should remain queryable");

        assert!(!rows.is_empty());
    }

    #[test]
    fn initialization_enables_auto_indexing() {
        let db = KvDb::in_memory();
        assert!(db.auto_index_enabled());
    }

    #[test]
    fn applied_migration_mismatch_fails_by_default() {
        let mut db = KvDb::in_memory();
        let package = simple_schema_package("Original migration.");
        db.upsert_package(package.clone()).unwrap();

        let mut changed = package;
        changed.migrations[0].description = Some("Changed migration.".to_string());

        let err = db.upsert_package(changed).unwrap_err();
        assert!(
            err.to_string()
                .contains("differs from the stored definition"),
            "{err}"
        );
    }

    #[test]
    fn applied_migration_mismatch_can_be_logged() {
        let mut db = KvDb::in_memory_with_config(DbConfig {
            migration_mismatch_policy: MigrationMismatchPolicy::Log,
        });
        let package = simple_schema_package("Original migration.");
        db.upsert_package(package.clone()).unwrap();

        let mut changed = package;
        changed.migrations[0].description = Some("Changed migration.".to_string());
        let outcome = db.upsert_package(changed).unwrap();

        assert!(outcome.executed_migrations.is_empty());
        assert!(db.catalog().class_id("shared.test.note").is_some());
    }

    #[test]
    fn query_executes_ddl_variant_and_returns_unit_result() {
        let mut db = KvDb::in_memory();

        let result = db
            .query(Query::Ddl(semantic_db_core::DdlQuery {
                batch: DdlBatch::new().with_op(DdlOperation::UpsertAttribute {
                    attribute: AttributeType {
                        id: "shared:test:age".to_string(),
                        name: "age".to_string(),
                        ty: Type {
                            kind: TypeKind::Number(NumberType::UInt(UIntWidth::U32)),
                            constraints: vec![],
                            annotations: vec![],
                        },
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                }),
            }))
            .unwrap();
        assert!(matches!(result, QueryResult::Ddl(())));
        assert!(db.catalog().attribute_by_id("shared:test:age").is_some());
    }

    #[test]
    fn nested_ref_path_lookup_matches_child_row() {
        let mut db = KvDb::in_memory();
        db.create_collection("ref_paths", CollectionKind::Polymorphic)
            .unwrap();

        let mut grand = Object::new();
        grand.insert("id", Value::String("ref-grand".to_string()));
        grand.insert("kind", Value::String("top".to_string()));
        grand.insert("title", Value::String("root".to_string()));
        db.insert("ref_paths", "ref-grand", grand).unwrap();

        let mut parent = Object::new();
        parent.insert("id", Value::String("ref-parent".to_string()));
        parent.insert("kind", Value::String("blah".to_string()));
        parent.insert("title", Value::String("abc".to_string()));
        parent.insert("parent", Value::String("ref-grand".to_string()));
        db.insert("ref_paths", "ref-parent", parent).unwrap();

        let mut child = Object::new();
        child.insert("id", Value::String("ref-child".to_string()));
        child.insert("kind", Value::String("leaf".to_string()));
        child.insert("parent", Value::String("ref-parent".to_string()));
        db.insert("ref_paths", "ref-child", child).unwrap();

        let query = SelectQuery::new()
            .with_collection("ref_paths")
            .with_predicate(eq_predicate(
                FieldPath::from_fields(["parent", "title"]),
                Value::String("abc".to_string()),
            ))
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "id",
                ])))),
                alias: Some("id".to_string()),
            }]);
        let catalog = db.catalog();
        let collection = catalog.collection_by_name("ref_paths").unwrap();
        let stored_rows = db.store.scan_collection(collection.lid).unwrap();
        let lookup = super::build_local_ref_lookup(
            catalog.as_ref(),
            collection,
            stored_rows.into_iter().map(Ok),
        )
        .unwrap();
        let canonical = canonicalize_select_query(&query, catalog.as_ref(), collection).unwrap();
        let explain = db.explain_query(Query::Select(query.clone())).unwrap();
        let rows = db.select(query).unwrap();

        let ids = rows
            .iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let stored_parent = db.get("ref_paths", "ref-parent").unwrap().unwrap();
        let stored_child = db.get("ref_paths", "ref-child").unwrap().unwrap();
        let resolved = super::resolve_path_with_local_refs(
            &stored_child.object,
            &FieldPath::from_fields(["local:core:parent", "title"]),
            Some(lookup.as_ref()),
        );
        assert_eq!(
            ids,
            vec!["ref-child".to_string()],
            "canonical={:?}, explain={:?}, resolved={:?}, lookup_keys={:?}, parent={:?}, child={:?}",
            canonical,
            explain,
            resolved,
            lookup.keys().cloned().collect::<Vec<_>>(),
            stored_parent.object,
            stored_child.object
        );
    }

    #[test]
    fn ref_field_rejects_missing_target() {
        let mut db = KvDb::in_memory();
        register_ref_schema(&mut db, ref_ty("person"));

        let err = db
            .insert(
                DEFAULT_COLLECTION,
                "article-1",
                entity(
                    "article-1",
                    "article",
                    [("author", Value::String("person-99".to_string()))],
                ),
            )
            .expect_err("missing ref target should be rejected");

        assert!(matches!(err, semantic_db_core::DbError::InvalidQuery(_)));
        assert!(
            err.to_string().contains("missing target id 'person-99'"),
            "{err}"
        );
    }

    #[test]
    fn ref_field_accepts_existing_target() {
        let mut db = KvDb::in_memory();
        register_ref_schema(&mut db, ref_ty("person"));

        db.insert(
            DEFAULT_COLLECTION,
            "person-1",
            entity("person-1", "person", []),
        )
        .unwrap();
        db.insert(
            DEFAULT_COLLECTION,
            "article-1",
            entity(
                "article-1",
                "article",
                [("author", Value::String("person-1".to_string()))],
            ),
        )
        .unwrap();
    }

    #[test]
    fn ref_field_accepts_same_batch_target() {
        let mut db = KvDb::in_memory();
        register_ref_schema(&mut db, ref_ty("person"));

        db.execute_batch(
            Batch::new()
                .with_op(BatchOperation::Upsert {
                    collection: DEFAULT_COLLECTION.to_string(),
                    id: "person-1".to_string(),
                    object: entity("person-1", "person", []),
                })
                .with_op(BatchOperation::Upsert {
                    collection: DEFAULT_COLLECTION.to_string(),
                    id: "article-1".to_string(),
                    object: entity(
                        "article-1",
                        "article",
                        [("author", Value::String("person-1".to_string()))],
                    ),
                }),
        )
        .unwrap();
    }

    #[test]
    fn ref_field_rejects_wrong_target_class() {
        let mut db = KvDb::in_memory();
        register_ref_schema(&mut db, ref_ty("person"));

        db.insert(
            DEFAULT_COLLECTION,
            "org-1",
            entity("org-1", "organization", []),
        )
        .unwrap();
        let err = db
            .insert(
                DEFAULT_COLLECTION,
                "article-1",
                entity(
                    "article-1",
                    "article",
                    [("author", Value::String("org-1".to_string()))],
                ),
            )
            .expect_err("wrong ref target class should be rejected");

        assert!(matches!(err, semantic_db_core::DbError::InvalidQuery(_)));
        assert!(err.to_string().contains("local:person"), "{err}");
    }

    #[test]
    fn ref_field_accepts_subclass_target() {
        let mut db = KvDb::in_memory();
        register_ref_schema(&mut db, ref_ty("person"));

        db.insert(DEFAULT_COLLECTION, "emp-1", entity("emp-1", "employee", []))
            .unwrap();
        db.insert(
            DEFAULT_COLLECTION,
            "article-1",
            entity(
                "article-1",
                "article",
                [("author", Value::String("emp-1".to_string()))],
            ),
        )
        .unwrap();
    }

    #[test]
    fn optional_ref_allows_null() {
        let mut db = KvDb::in_memory();
        register_ref_schema(
            &mut db,
            ty(TypeKind::Optional(OptionalType {
                inner: Box::new(ref_ty("person")),
            })),
        );

        db.insert(
            DEFAULT_COLLECTION,
            "article-1",
            entity("article-1", "article", [("author", Value::Null)]),
        )
        .unwrap();
    }

    #[test]
    fn ref_requires_string_value() {
        let mut db = KvDb::in_memory();
        register_ref_schema(&mut db, ref_ty("person"));

        let err = db
            .insert(
                DEFAULT_COLLECTION,
                "article-1",
                entity("article-1", "article", [("author", Value::I64(42))]),
            )
            .expect_err("non-string ref should be rejected");

        assert!(matches!(err, semantic_db_core::DbError::InvalidQuery(_)));
        assert!(err.to_string().contains("must be a string id"), "{err}");
    }

    #[test]
    fn untyped_query_works() {
        let mut db = KvDb::in_memory();
        db.create_collection("events", CollectionKind::Polymorphic)
            .unwrap();

        let mut a = Object::new();
        a.insert("id", Value::String("a".to_string()));
        a.insert("kind", Value::String("music".to_string()));
        a.insert("score", Value::I64(10));
        db.insert("events", "a", a).unwrap();

        let mut b = Object::new();
        b.insert("id", Value::String("b".to_string()));
        b.insert("kind", Value::String("video".to_string()));
        b.insert("score", Value::I64(2));
        db.insert("events", "b", b).unwrap();

        let query = SelectQuery::new().with_predicate(eq_predicate(
            FieldPath::from_fields(["kind"]),
            Value::String("music".to_string()),
        ));

        let rows = db.select(query.with_collection("events")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("score"), Some(&Value::I64(10)));
    }

    #[test]
    fn select_from_all_collection_alias_combines_collections() {
        let mut db = KvDb::in_memory();
        db.create_collection("events", CollectionKind::Polymorphic)
            .unwrap();
        db.create_collection("articles", CollectionKind::Polymorphic)
            .unwrap();

        let mut event = Object::new();
        event.insert("id", Value::String("e1".to_string()));
        event.insert("kind", Value::String("music".to_string()));
        db.insert("events", "e1", event).unwrap();

        let mut article = Object::new();
        article.insert("id", Value::String("a1".to_string()));
        article.insert("kind", Value::String("music".to_string()));
        db.insert("articles", "a1", article).unwrap();

        let query = SelectQuery::new()
            .with_collection(ALL_COLLECTION_ALIAS)
            .with_predicate(eq_predicate(
                FieldPath::from_fields(["kind"]),
                Value::String("music".to_string()),
            ));
        let rows = db.select(query).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn typed_class_rows_normalize_aliases_and_store_class_kind() {
        let mut db = KvDb::in_memory();

        let title_attr = AttributeType {
            id: "core.title".to_string(),
            name: "title".to_string(),
            ty: ty(TypeKind::String(StringType {
                format: None,
                normalization: None,
            })),
            constraints: vec![],
            meta: Meta::default(),
        };

        db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertAttribute {
            attribute: title_attr,
        }))
        .unwrap();

        let mut attrs = BTreeMap::new();
        attrs.insert(
            "title".to_string(),
            ClassAttribute {
                attribute: AttributeRef {
                    id: "core.title".to_string(),
                },
                required: true,
                ui_order: None,
                computed: None,
                constraints: vec![],
                meta: Meta::default(),
            },
        );

        let class = ClassType {
            id: "core.article".to_string(),
            name: "Article".to_string(),
            inherits: None,
            extends: vec![],
            attributes: attrs,
            constraints: vec![],
            meta: Meta::default(),
        };

        db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertClass { class }))
            .unwrap();
        db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertCollection {
            name: "articles".to_string(),
            kind: DdlCollectionKind::Polymorphic,
            integrity_mode: IntegrityMode::Permissive,
        }))
        .unwrap();

        let mut object = Object::new();
        object.insert("id", Value::String("art-1".to_string()));
        object.insert("type", Value::String("core.article".to_string()));
        object.insert("title", Value::String("Hello".to_string()));
        db.insert("articles", "art-1", object).unwrap();

        let row = db.get("articles", "art-1").unwrap().unwrap();
        assert_eq!(
            row.object.get("core:title"),
            Some(&Value::String("Hello".to_string()))
        );
        assert!(row.object.get("title").is_none());
        assert_eq!(
            row.object.get("type"),
            Some(&Value::String("core:article".to_string()))
        );

        let collection = db.catalog().collection_by_name("articles").unwrap().clone();
        let stored = db
            .store
            .get_entity(collection.lid, "art-1")
            .unwrap()
            .unwrap();
        assert_eq!(stored.kind, crate::storage::StoredEntityKind::Class);
    }

    #[test]
    fn typed_closed_record_rows_reject_unknown_fields() {
        let mut db = KvDb::in_memory();

        let mut fields = BTreeMap::new();
        fields.insert(
            "name".to_string(),
            Field {
                ty: ty(TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                })),
                required: true,
                readonly: false,
                writeonly: false,
                default: None,
                meta: Meta::default(),
            },
        );
        fields.insert(
            "active".to_string(),
            Field {
                ty: ty(TypeKind::Bool(BoolType)),
                required: true,
                readonly: false,
                writeonly: false,
                default: None,
                meta: Meta::default(),
            },
        );

        db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertRecordType {
            id: "person.record".to_string(),
            name: "PersonRecord".to_string(),
            record: RecordType {
                fields,
                open: false,
                additional: None,
                required_order: None,
            },
        }))
        .unwrap();
        db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertCollection {
            name: "people".to_string(),
            kind: DdlCollectionKind::Polymorphic,
            integrity_mode: IntegrityMode::Permissive,
        }))
        .unwrap();

        let mut obj = Object::new();
        obj.insert("id", Value::String("p1".to_string()));
        obj.insert("type", Value::String("person.record".to_string()));
        obj.insert("name", Value::String("Ada".to_string()));
        obj.insert("active", Value::Bool(true));
        obj.insert("extra", Value::Bool(true));

        let err = db.insert("people", "p1", obj).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn indexed_equality_query_and_unique_enforcement() {
        let mut db = KvDb::in_memory();
        let people = db
            .create_collection("people", CollectionKind::Polymorphic)
            .unwrap();
        db.create_index("people_email_uq", people, "email", true)
            .unwrap();

        let mut p1 = Object::new();
        p1.insert("id", Value::String("p1".to_string()));
        p1.insert("email", Value::String("a@example.com".to_string()));
        p1.insert("name", Value::String("A".to_string()));
        db.insert("people", "p1", p1).unwrap();

        let mut p2 = Object::new();
        p2.insert("id", Value::String("p2".to_string()));
        p2.insert("email", Value::String("b@example.com".to_string()));
        p2.insert("name", Value::String("B".to_string()));
        db.insert("people", "p2", p2).unwrap();

        let q = SelectQuery::new().with_predicate(eq_predicate(
            FieldPath::from_fields(["email"]),
            Value::String("a@example.com".to_string()),
        ));
        let rows = db.select(q.with_collection("people")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("name"), Some(&Value::String("A".to_string())));

        let mut p3 = Object::new();
        p3.insert("id", Value::String("p3".to_string()));
        p3.insert("email", Value::String("a@example.com".to_string()));
        p3.insert("name", Value::String("C".to_string()));
        let err = db.insert("people", "p3", p3).unwrap_err();
        assert!(err.to_string().contains("unique index violation"));
    }

    #[test]
    fn index_entries_update_on_upsert_and_delete() {
        let mut db = KvDb::in_memory();
        let events = db
            .create_collection("events", CollectionKind::Polymorphic)
            .unwrap();
        db.create_index("events_kind_idx", events, "kind", false)
            .unwrap();

        let mut e = Object::new();
        e.insert("id", Value::String("e1".to_string()));
        e.insert("kind", Value::String("music".to_string()));
        db.insert("events", "e1", e).unwrap();

        let q_music = SelectQuery::new().with_predicate(eq_predicate(
            FieldPath::from_fields(["kind"]),
            Value::String("music".to_string()),
        ));
        assert_eq!(
            db.select(q_music.with_collection("events")).unwrap().len(),
            1
        );

        let mut updated = Object::new();
        updated.insert("id", Value::String("e1".to_string()));
        updated.insert("kind", Value::String("video".to_string()));
        db.insert("events", "e1", updated).unwrap();

        let q_music = SelectQuery::new().with_predicate(eq_predicate(
            FieldPath::from_fields(["kind"]),
            Value::String("music".to_string()),
        ));
        assert_eq!(
            db.select(q_music.with_collection("events")).unwrap().len(),
            0
        );

        let q_video = SelectQuery::new().with_predicate(eq_predicate(
            FieldPath::from_fields(["kind"]),
            Value::String("video".to_string()),
        ));
        assert_eq!(
            db.select(q_video.with_collection("events")).unwrap().len(),
            1
        );

        db.delete("events", "e1").unwrap();
        let q_video = SelectQuery::new().with_predicate(eq_predicate(
            FieldPath::from_fields(["kind"]),
            Value::String("video".to_string()),
        ));
        assert_eq!(
            db.select(q_video.with_collection("events")).unwrap().len(),
            0
        );
    }

    #[test]
    fn query_order_by_sorts_rows() {
        let mut db = KvDb::in_memory();
        db.create_collection("events", CollectionKind::Polymorphic)
            .unwrap();

        let mut a = Object::new();
        a.insert("id", Value::String("a".to_string()));
        a.insert("score", Value::I64(7));
        db.insert("events", "a", a).unwrap();

        let mut b = Object::new();
        b.insert("id", Value::String("b".to_string()));
        b.insert("score", Value::I64(2));
        db.insert("events", "b", b).unwrap();

        let mut c = Object::new();
        c.insert("id", Value::String("c".to_string()));
        c.insert("score", Value::I64(9));
        db.insert("events", "c", c).unwrap();

        let asc = SelectQuery::new().with_order_by(vec![OrderBy {
            expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["score"]))),
            direction: SortDirection::Asc,
        }]);
        let asc_rows = db.select(asc.with_collection("events")).unwrap();
        assert_eq!(asc_rows[0].get("score"), Some(&Value::I64(2)));
        assert_eq!(asc_rows[2].get("score"), Some(&Value::I64(9)));

        let desc = SelectQuery::new().with_order_by(vec![OrderBy {
            expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["score"]))),
            direction: SortDirection::Desc,
        }]);
        let desc_rows = db.select(desc.with_collection("events")).unwrap();
        assert_eq!(desc_rows[0].get("score"), Some(&Value::I64(9)));
        assert_eq!(desc_rows[2].get("score"), Some(&Value::I64(2)));
    }

    #[test]
    fn planner_reports_index_lookup() {
        let mut db = KvDb::in_memory();
        let events = db
            .create_collection("events", CollectionKind::Polymorphic)
            .unwrap();
        db.create_index("events_kind_idx", events, "kind", false)
            .unwrap();

        let q = SelectQuery::new().with_predicate(eq_predicate(
            FieldPath::from_fields(["kind"]),
            Value::String("music".to_string()),
        ));

        let plan = db
            .plan_query(Query::Select(q.with_collection("events")))
            .unwrap();
        match plan {
            QueryPlan::IndexLookup { index_name, .. } => {
                assert_eq!(index_name, "events_kind_idx");
            }
            _ => panic!("expected index lookup plan"),
        }
    }

    #[test]
    fn planner_uses_auto_index_for_simple_equality_without_explicit_index() {
        let mut db = KvDb::in_memory();
        db.create_collection("events", CollectionKind::Polymorphic)
            .unwrap();
        db.set_auto_index_enabled(true).unwrap();

        let q = SelectQuery::new().with_predicate(eq_predicate(
            FieldPath::from_fields(["kind"]),
            Value::String("music".to_string()),
        ));

        let plan = db
            .plan_query(Query::Select(q.with_collection("events")))
            .unwrap();
        match plan {
            QueryPlan::IndexLookup { index_name, .. } => {
                assert_eq!(index_name, semantic_db_core::catalog::AUTO_PATH_INDEX_NAME);
            }
            _ => panic!("expected index lookup plan"),
        }
    }

    #[test]
    fn auto_index_simple_equality_query_returns_matching_entities() {
        let mut db = KvDb::in_memory();
        db.create_collection("events", CollectionKind::Polymorphic)
            .unwrap();
        db.set_auto_index_enabled(true).unwrap();

        let mut e1 = Object::new();
        e1.insert("id", Value::String("e1".to_string()));
        e1.insert("kind", Value::String("music".to_string()));
        db.insert("events", "e1", e1).unwrap();

        let mut e2 = Object::new();
        e2.insert("id", Value::String("e2".to_string()));
        e2.insert("kind", Value::String("music".to_string()));
        db.insert("events", "e2", e2).unwrap();

        let mut e3 = Object::new();
        e3.insert("id", Value::String("e3".to_string()));
        e3.insert("kind", Value::String("video".to_string()));
        db.insert("events", "e3", e3).unwrap();

        let q = SelectQuery::new().with_predicate(eq_predicate(
            FieldPath::from_fields(["kind"]),
            Value::String("music".to_string()),
        ));
        let rows = db.select(q.with_collection("events")).unwrap();
        assert_eq!(rows.len(), 2);
        let mut ids = rows
            .iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, vec!["e1".to_string(), "e2".to_string()]);
    }

    #[test]
    fn auto_index_nested_paths_work_with_planner_and_mutations() {
        let mut db = KvDb::in_memory();
        db.create_collection("events", CollectionKind::Polymorphic)
            .unwrap();
        db.set_auto_index_enabled(true).unwrap();

        let mut nested_obj = Object::new();
        nested_obj.insert("nestkey", Value::String("nestvalue".to_string()));
        let mut row = Object::new();
        row.insert("id", Value::String("e1".to_string()));
        row.insert("mylist", Value::List(vec![Value::Object(nested_obj)]));
        db.insert("events", "e1", row).unwrap();

        let nested_path = FieldPath::from(vec![
            PathSegment::Field("mylist".to_string()),
            PathSegment::Index(0),
            PathSegment::Field("nestkey".to_string()),
        ]);
        let q_nested = SelectQuery::new().with_predicate(eq_predicate(
            nested_path.clone(),
            Value::String("nestvalue".to_string()),
        ));
        assert_eq!(
            db.select(q_nested.clone().with_collection("events"))
                .unwrap()
                .len(),
            1
        );
        let plan = db
            .plan_query(Query::Select(q_nested.clone().with_collection("events")))
            .unwrap();
        match plan {
            QueryPlan::IndexLookup { index_name, .. } => {
                assert_eq!(index_name, semantic_db_core::catalog::AUTO_PATH_INDEX_NAME);
            }
            _ => panic!("expected index lookup plan"),
        }

        let mut updated_nested_obj = Object::new();
        updated_nested_obj.insert("nestkey", Value::String("newvalue".to_string()));
        let mut updated_row = Object::new();
        updated_row.insert("id", Value::String("e1".to_string()));
        updated_row.insert(
            "mylist",
            Value::List(vec![Value::Object(updated_nested_obj)]),
        );
        db.insert("events", "e1", updated_row).unwrap();

        assert_eq!(
            db.select(q_nested.with_collection("events")).unwrap().len(),
            0
        );
        let q_new = SelectQuery::new().with_predicate(eq_predicate(
            nested_path,
            Value::String("newvalue".to_string()),
        ));
        assert_eq!(
            db.select(q_new.clone().with_collection("events"))
                .unwrap()
                .len(),
            1
        );

        db.delete("events", "e1").unwrap();
        assert_eq!(db.select(q_new.with_collection("events")).unwrap().len(), 0);
    }

    #[test]
    fn update_query_supports_returning_projection() {
        let mut db = KvDb::in_memory();
        db.create_collection("items", CollectionKind::Polymorphic)
            .unwrap();

        let mut row = Object::new();
        row.insert("id", Value::String("i1".to_string()));
        row.insert("score", Value::I64(2));
        db.insert("items", "i1", row).unwrap();

        let update = UpdateQuery::new()
            .with_predicate(eq_predicate(
                FieldPath::from_fields(["id"]),
                Value::String("i1".to_string()),
            ))
            .set(
                FieldPath::from_fields(["score"]),
                Expr::Operand(Operand::Literal(Value::I64(9))),
            )
            .with_returning(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "score",
                ])))),
                alias: Some("new_score".to_string()),
            }]);

        let out = db
            .query(Query::Update(update.with_collection("items")))
            .unwrap();
        let QueryResult::Update(out) = out else {
            panic!("expected update result");
        };
        assert_eq!(out.stats.matched, 1);
        assert_eq!(out.stats.affected, 1);
        assert_eq!(out.returning.len(), 1);
        assert_eq!(out.returning[0].get("new_score"), Some(&Value::I64(9)),);
    }

    #[test]
    fn delete_query_supports_returning_projection() {
        let mut db = KvDb::in_memory();
        db.create_collection("items", CollectionKind::Polymorphic)
            .unwrap();

        let mut row = Object::new();
        row.insert("id", Value::String("i1".to_string()));
        row.insert("kind", Value::String("music".to_string()));
        db.insert("items", "i1", row).unwrap();

        let delete = semantic_db_core::DeleteQuery::new()
            .with_predicate(eq_predicate(
                FieldPath::from_fields(["kind"]),
                Value::String("music".to_string()),
            ))
            .with_returning(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "id",
                ])))),
                alias: None,
            }]);

        let out = db
            .query(Query::Delete(delete.with_collection("items")))
            .unwrap();
        let QueryResult::Delete(out) = out else {
            panic!("expected delete result");
        };
        assert_eq!(out.deleted, 1);
        assert_eq!(out.returning.len(), 1);
        assert_eq!(
            out.returning[0].get("id"),
            Some(&Value::String("i1".to_string())),
        );
    }

    #[test]
    fn mvcc_transaction_requires_mvcc_backend() {
        let mut db = KvDb::in_memory();
        db.create_collection("events", CollectionKind::Polymorphic)
            .unwrap();

        let err = db
            .transact_with_options(
                super::Batch::new(),
                TransactionOptions {
                    concurrency: TransactionConcurrency::Mvcc,
                    ..TransactionOptions::default()
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("mvcc transaction requested"));
    }

    #[test]
    fn select_output_field_format_rewrites_registered_attribute_keys() {
        let mut db = KvDb::in_memory();
        db.create_collection("items", CollectionKind::Schema)
            .unwrap();
        db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: "semantic:title".to_string(),
                name: "title".to_string(),
                ty: ty(TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                })),
                constraints: vec![],
                meta: Meta::default(),
            },
        }))
        .unwrap();

        let mut row = Object::new();
        row.insert("id", Value::String("i1".to_string()));
        row.insert("semantic:title", Value::String("hello".to_string()));
        db.insert("items", "i1", row).unwrap();

        let plain = db
            .select(
                SelectQuery::new()
                    .with_collection("items")
                    .with_field_format(FieldFormat::Plain),
            )
            .unwrap();
        assert_eq!(
            plain[0].get("title"),
            Some(&Value::String("hello".to_string()))
        );

        let underscore = db
            .select(
                SelectQuery::new()
                    .with_collection("items")
                    .with_field_format(FieldFormat::Underscore),
            )
            .unwrap();
        assert_eq!(
            underscore[0].get("semantic_title"),
            Some(&Value::String("hello".to_string()))
        );
    }

    fn simple_schema_package(description: &str) -> Package {
        let attr = AttributeType {
            id: "shared.test.title".to_string(),
            name: "title".to_string(),
            ty: ty(TypeKind::String(StringType {
                format: None,
                normalization: None,
            })),
            constraints: vec![],
            meta: Meta::default(),
        };
        let mut attrs = BTreeMap::new();
        attrs.insert(
            "title".to_string(),
            ClassAttribute {
                attribute: AttributeRef {
                    id: attr.id.clone(),
                },
                required: false,
                ui_order: None,
                computed: None,
                constraints: vec![],
                meta: Meta::default(),
            },
        );
        let class = ClassType {
            id: "shared.test.note".to_string(),
            name: "Note".to_string(),
            inherits: None,
            extends: vec![],
            attributes: attrs,
            constraints: vec![],
            meta: Meta::default(),
        };

        Package {
            name: "shared.test".to_string(),
            root: Module {
                name: "test".to_string(),
                constants: BTreeMap::new(),
                types: BTreeMap::new(),
                attributes: BTreeMap::from([(attr.id.clone(), attr.clone())]),
                classes: BTreeMap::from([(class.id.clone(), class.clone())]),
                interfaces: BTreeMap::new(),
                contracts: BTreeMap::new(),
                meta: Meta::default(),
            },
            modules: BTreeMap::new(),
            migrations: vec![Migration {
                module: "test".to_string(),
                name: "001_init".to_string(),
                description: Some(description.to_string()),
                operations: vec![
                    MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                        attribute: attr,
                    }),
                    MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }),
                ],
                meta: Meta::default(),
            }],
            version: None,
            meta: Meta::default(),
        }
    }

    fn register_ref_schema(db: &mut KvDb<crate::storage::MemoryKvEngine>, author_ty: Type) {
        db.transact_ddl(
            DdlBatch::new()
                .with_op(DdlOperation::UpsertAttribute {
                    attribute: AttributeType {
                        id: "author".to_string(),
                        name: "author".to_string(),
                        ty: author_ty,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                })
                .with_op(DdlOperation::UpsertClass {
                    class: test_class("person", "Person", None, BTreeMap::new()),
                })
                .with_op(DdlOperation::UpsertClass {
                    class: test_class("organization", "Organization", None, BTreeMap::new()),
                })
                .with_op(DdlOperation::UpsertClass {
                    class: test_class("employee", "Employee", Some("person"), BTreeMap::new()),
                })
                .with_op(DdlOperation::UpsertClass {
                    class: test_class(
                        "article",
                        "Article",
                        None,
                        BTreeMap::from([(
                            "author".to_string(),
                            ClassAttribute {
                                attribute: AttributeRef {
                                    id: "author".to_string(),
                                },
                                required: true,
                                ui_order: None,
                                computed: None,
                                constraints: vec![],
                                meta: Meta::default(),
                            },
                        )]),
                    ),
                }),
        )
        .unwrap();
    }

    fn test_class(
        id: &str,
        name: &str,
        inherits: Option<&str>,
        attributes: BTreeMap<String, ClassAttribute>,
    ) -> ClassType {
        ClassType {
            id: id.to_string(),
            name: name.to_string(),
            inherits: inherits.map(|id| ClassRef { id: id.to_string() }),
            extends: vec![],
            attributes,
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn entity<const N: usize>(id: &str, class_id: &str, fields: [(&str, Value); N]) -> Object {
        let mut object = Object::new();
        object.insert("id", Value::String(id.to_string()));
        object.insert("type", Value::String(class_id.to_string()));
        for (field, value) in fields {
            object.insert(field, value);
        }
        object
    }

    fn ref_ty(class_id: &str) -> Type {
        ty(TypeKind::Ref(TypeRef::new(class_id)))
    }

    fn ty(kind: TypeKind) -> Type {
        Type {
            kind,
            constraints: vec![],
            annotations: vec![],
        }
    }

    fn eq_predicate(path: FieldPath, value: Value) -> Expr {
        Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(path))),
            right: Box::new(Expr::Operand(Operand::Literal(value))),
        }
    }
}
