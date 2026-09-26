use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::{
    ALL_COLLECTION_ALIAS, AccessPath, AppliedMigration, Batch, BatchOperation, BatchOutcome,
    CORE_CATALOG_SCHEMA_COLLECTION, DEFAULT_COLLECTION, DeleteQuery, EntityRecord, InsertQuery,
    InsertSource, MutationStats, PackageRegistrationOutcome, Query, QueryExplain, QueryPlan,
    QueryResult, SelectQuery, UpdateQuery, apply_core_schema_migrations, apply_migration_ddl_batch,
    canonicalize_delete_query, canonicalize_insert_query, canonicalize_query,
    canonicalize_select_query, canonicalize_update_query, evaluate_mutation_limit,
    execute_batch_with_prepare, is_all_collection_alias, normalize_object_for_collection,
    normalize_package_definition, prepare_object_for_write, ref_target_class_ids,
    resolved_field_types_for_object, touched_collections, validate_package_migrations_with_catalog,
};
use crate::{CoreError, DbConfig, DbError, DefaultExpressionContext, MigrationMismatchPolicy};
use futures::{StreamExt, stream};
use semantic_data::attr::{ATTR_RELATION_FROM, ATTR_RELATION_RELATION, ATTR_RELATION_TO};
use semantic_data::query::FieldFormat;
use semantic_data::schema::{IndexKind, core::type_kind::TypeKind, core::type_node::Type};
use semantic_data::schema::{
    Migration, MigrationOperation, Package, RelationIndexingMode, RelationMode, RelationType,
};
use semantic_data::value::{FieldPath, Object, PathSegment, Value, ValueRef};

use crate::catalog::{
    Catalog, CollectionKind, CollectionSchema, IntegrityMode, LocalAttrId, LocalCollectionId,
    LocalFieldId, LocalIndexId, OBJECT_TYPE_FIELD, SharedCatalog,
};
use crate::embedded::{
    schema_store::{catalog_write_ops, load_catalog},
    storage::{
        EntityReadSnapshot, EntityStorage, StorageCommitOutcome, StorageTransactionCapabilities,
        StorageWriteOp, StoredEntity, StoredEntityKind,
    },
};
use crate::{
    DdlBatch, DdlCollectionKind, DdlOperation, DdlOutcome, IsolationLevel, QueryContext,
    StorageErrorKind, TransactionConcurrency, TransactionOptions, apply_ddl_batch,
    fresh_catalog_with_core_schema, run_with_transaction_retries,
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

#[cfg(test)]
mod change_feed_tests;
mod commit;
pub(crate) mod compact;
mod incremental;
mod index_scan;
mod interactive;
#[cfg(test)]
mod interactive_tests;
#[cfg(test)]
mod isolation_tests;
mod local_refs;
mod maintenance;
mod mutation;
mod overlay;
mod reader;
mod text_search;
mod validation;

pub use interactive::EmbeddedTransaction;
pub(crate) use interactive::{CommittedTransaction, PreparedTransaction};
pub use maintenance::write_backup;
pub use reader::DbReader;
use reader::QueryReader;

use crate::embedded::storage::{RevisionReader, snapshot_isolation_unsupported};
use commit::CommitIntent;
use local_refs::{
    LocalRefResolver, RowLocalRefs, resolve_path_with_local_refs,
    value_from_object_with_alias_fallback,
};

#[derive(Debug)]
pub struct EmbeddedDb<S: EntityStorage> {
    catalog: SharedCatalog,
    storage: S,
    config: DbConfig,
    execution_counts: compact::ExecutionCounts,
    change_feed: crate::ChangeFeed,
}

#[cfg(test)]
impl EmbeddedDb<crate::embedded::storage::MemoryEntityStorage> {
    fn in_memory() -> Self {
        Self::new(crate::embedded::storage::MemoryEntityStorage::new())
    }

    fn in_memory_with_config(config: DbConfig) -> Self {
        Self::new_with_config(crate::embedded::storage::MemoryEntityStorage::new(), config)
    }
}

impl<S: EntityStorage> EmbeddedDb<S> {
    fn query_context(&self) -> QueryContext {
        QueryContext::from_shared(&self.catalog)
    }

    pub fn new(engine: S) -> Self {
        Self::open(engine).expect("database initialization failed")
    }

    pub fn new_with_config(engine: S, config: DbConfig) -> Self {
        Self::open_with_config(engine, config).expect("database initialization failed")
    }

    pub fn open(engine: S) -> std::result::Result<Self, DbError> {
        Self::open_with_config(engine, DbConfig::default())
    }

    pub fn open_with_config(engine: S, config: DbConfig) -> std::result::Result<Self, DbError> {
        let mut storage = engine;
        storage.prepare_open()?;
        let bootstrap_catalog = fresh_catalog_with_core_schema()
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
        let loaded_catalog = if let Some(catalog) = load_catalog(&storage, &bootstrap_catalog)? {
            catalog
        } else {
            let ops = catalog_write_ops(&storage, &bootstrap_catalog)?;
            storage.apply_batch(&ops)?;
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
            let ops = catalog_write_ops(&storage, &catalog)?;
            storage.apply_batch(&ops)?;
        }
        let mut db = Self {
            catalog: SharedCatalog::new(catalog),
            storage,
            config,
            execution_counts: compact::ExecutionCounts::default(),
            change_feed: crate::ChangeFeed::default(),
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
        let mut index_ops = Vec::new();
        // Range and full-text indexes registered before they were maintained
        // have no entries; rebuild them once, when the migration introducing
        // their maintenance is applied.
        let applied = |name: &str| {
            executed_core_migrations
                .iter()
                .any(|applied| applied.migration.name == name)
        };
        let rebuild_kinds = [
            (crate::ddl::INDEX_DEFINITIONS_MIGRATION, IndexKind::Range),
            (crate::ddl::FULL_TEXT_INDEXES_MIGRATION, IndexKind::FullText),
        ]
        .into_iter()
        .filter_map(|(migration, kind)| applied(migration).then_some(kind))
        .collect::<Vec<_>>();
        db.backfill_missing_index_storage(&rebuild_kinds, &mut index_ops)?;
        if !index_ops.is_empty() {
            db.storage.apply_batch(&index_ops)?;
        }
        db.initialize_relationship_contributors()?;
        db.initialize_reverse_references()?;
        Ok(db)
    }

    fn mark_collection_internal(&mut self, name: &str) -> std::result::Result<(), DbError> {
        let snapshot = self.catalog.snapshot();
        let mut catalog = snapshot.catalog.as_ref().clone();
        if !mark_collection_internal(&mut catalog, name)? {
            return Ok(());
        }
        // Dropping the path index of a raw system collection clears its
        // entries.
        let mut ops = self.ddl_cleanup_ops(&snapshot.catalog, &catalog)?;
        ops.extend(catalog_write_ops(&self.storage, &catalog)?);
        let revision = self.storage.current_revision()?;
        let intent = CommitIntent::with_catalog(
            crate::ChangeSource::Ddl,
            snapshot.version,
            Arc::new(catalog),
        );
        match self.commit_write(&ops, revision, intent, Default::default())? {
            StorageCommitOutcome::Committed { .. } => Ok(()),
            StorageCommitOutcome::Conflict { .. } => Err(DbError::TransactionConflict(
                "database changed while marking a collection internal".into(),
            )),
        }
    }

    fn backfill_missing_index_storage(
        &self,
        rebuild_kinds: &[IndexKind],
        ops: &mut Vec<StorageWriteOp>,
    ) -> std::result::Result<(), DbError> {
        let catalog = self.catalog();
        let mut indexes = Vec::new();
        for (index_id, index) in catalog.indexes() {
            if rebuild_kinds.contains(&index.schema.kind)
                || self.storage.index_needs_rebuild(index_id)?
            {
                indexes.push(index.clone());
            }
        }
        self.backfill_indexes(catalog.as_ref(), &indexes, None, ops)
    }

    fn backfill_new_indexes(
        &self,
        before: &Catalog,
        after: &Catalog,
        read_revision: Option<u64>,
        ops: &mut Vec<StorageWriteOp>,
    ) -> std::result::Result<(), DbError> {
        let indexes = after
            .indexes()
            .filter_map(|(index_id, index)| {
                let changed = before.index_by_lid(index_id).is_none_or(|before_index| {
                    before_index.collection != index.collection
                        || before_index.canonical_field != index.canonical_field
                        || before_index.schema.kind != index.schema.kind
                        || before_index.schema.unique != index.schema.unique
                        || before_index.schema.extra_key_paths != index.schema.extra_key_paths
                        || before_index.schema.predicate != index.schema.predicate
                        || before_index.schema.analyzer != index.schema.analyzer
                });
                changed.then(|| index.clone())
            })
            .collect::<Vec<_>>();
        self.backfill_indexes(after, &indexes, read_revision, ops)
    }

    fn backfill_indexes(
        &self,
        catalog: &Catalog,
        indexes: &[crate::catalog::IndexSchema],
        read_revision: Option<u64>,
        ops: &mut Vec<StorageWriteOp>,
    ) -> std::result::Result<(), DbError> {
        if indexes.is_empty() {
            return Ok(());
        }

        let mut collection_ids = BTreeSet::new();
        for index in indexes {
            collection_ids.insert(index.collection);
        }

        for collection_id in collection_ids {
            let Some(collection) = catalog.collection_by_lid(collection_id) else {
                continue;
            };
            let collection_indexes = indexes
                .iter()
                .filter(|index| index.collection == collection_id)
                .cloned()
                .collect::<Vec<_>>();
            if collection_indexes.is_empty() {
                continue;
            }

            for index in &collection_indexes {
                ops.push(StorageWriteOp::ResetIndex(index.lid));
            }

            let rows = if self.storage.tx_capabilities().snapshot_reads {
                if let Some(revision) = read_revision {
                    self.storage
                        .scan_collection_at_revision(collection.lid, revision)?
                } else {
                    self.storage.scan_collection(collection.lid)?
                }
            } else {
                self.storage.scan_collection(collection.lid)?
            };
            for row in rows {
                for index in &collection_indexes {
                    self.push_index_ops(ops, index, &row.id, &row.object)?;
                }
            }
        }
        Ok(())
    }

    pub fn catalog(&self) -> std::sync::Arc<Catalog> {
        self.catalog.catalog_arc()
    }

    pub fn shared_catalog(&self) -> &SharedCatalog {
        &self.catalog
    }

    /// The configuration the database was opened with.
    pub fn config(&self) -> DbConfig {
        self.config
    }

    /// A reader over the current committed state.
    ///
    /// The reader owns its storage snapshot when the storage supports owned
    /// snapshots (see [`Self::owned_reader`]); otherwise it borrows the
    /// storage and keeps `self` borrowed while it is used.
    pub fn reader(&self) -> std::result::Result<DbReader<'_>, DbError> {
        Ok(match self.owned_reader()? {
            Some(reader) => reader,
            None => DbReader::new(
                self.catalog.snapshot(),
                QueryReader::Borrowed(self.storage.snapshot()?),
            ),
        })
    }

    /// A `'static` reader over the current committed state, or `None` when
    /// the storage has no owned snapshots ([`EntityStorage::owned_snapshot`]).
    ///
    /// The reader does not borrow the database: it can be moved to another
    /// thread and keeps reading the state it was created at while writers
    /// commit newer states.
    pub fn owned_reader(&self) -> std::result::Result<Option<DbReader<'static>>, DbError> {
        Ok(self
            .storage
            .owned_snapshot()?
            .map(|snapshot| DbReader::new(self.catalog.snapshot(), QueryReader::Shared(snapshot))))
    }

    /// Replace the in-memory catalog with an authoritative durable snapshot.
    /// Storage adapters use this when their catalog snapshot is committed
    /// atomically outside the entity storage implementation.
    pub fn replace_catalog_snapshot(&mut self, catalog: Catalog) {
        self.catalog.replace(catalog);
    }

    pub fn create_collection(
        &mut self,
        name: impl Into<String>,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        let name = name.into();
        let ddl_kind = match kind {
            CollectionKind::Untyped => crate::DdlCollectionKind::Untyped,
            CollectionKind::Schema => crate::DdlCollectionKind::Schema,
            CollectionKind::Polymorphic => crate::DdlCollectionKind::Polymorphic,
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
            kind: IndexKind::Equality,
            extra_fields: Vec::new(),
            predicate: None,
            analyzer: Default::default(),
        });
        self.transact_ddl(ddl)?;
        Ok(())
    }

    /// Create or replace an equality or range index, including composite
    /// and partial indexes, and backfill it.
    pub fn create_index_definition(
        &mut self,
        definition: crate::catalog::IndexDefinition,
    ) -> std::result::Result<(), DbError> {
        let collection_name = self
            .catalog()
            .collection_by_lid(definition.collection)
            .ok_or(DbError::UnknownCollection(definition.collection))?
            .name
            .clone();
        let mut fields = definition.fields.into_iter();
        let field = fields.next().ok_or_else(|| {
            DbError::InvalidQuery(format!("index '{}' has no key column", definition.name))
        })?;
        let ddl = DdlBatch::new().with_op(DdlOperation::UpsertIndex {
            name: definition.name,
            collection: collection_name,
            field,
            unique: definition.unique,
            kind: definition.kind,
            extra_fields: fields.collect(),
            predicate: definition.predicate,
            analyzer: definition.analyzer,
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
        let package = normalize_package_definition(&package)
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;

        let txn_result = run_with_transaction_retries(TransactionOptions::default(), |_| {
            let catalog_snapshot = self.catalog.snapshot();
            validate_package_migrations_with_catalog(&package, catalog_snapshot.catalog.as_ref())
                .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
            let read_revision = self.storage.current_revision()?;
            let (next_catalog, before, after, executed_migrations) = self.apply_package_update(
                catalog_snapshot.catalog.as_ref(),
                read_revision,
                &package,
            )?;
            let next_catalog = Arc::new(next_catalog);
            // Reconcile applied migrations before checking for a no-op: older
            // catalogs may need repairs, including behavioral typedef constraints.
            if executed_migrations.is_empty()
                && before.is_empty()
                && after.is_empty()
                && next_catalog.to_storage_snapshot()
                    == catalog_snapshot.catalog.to_storage_snapshot()
            {
                self.storage.ensure_revision(read_revision)?;
                return Ok(PackageRegistrationOutcome {
                    executed_migrations,
                });
            }
            let mut extra_ops =
                self.ddl_cleanup_ops(catalog_snapshot.catalog.as_ref(), &next_catalog)?;
            self.backfill_new_indexes(
                catalog_snapshot.catalog.as_ref(),
                &next_catalog,
                read_revision,
                &mut extra_ops,
            )?;
            extra_ops.extend(catalog_write_ops(&self.storage, &next_catalog)?);
            self.rebuild_reverse_references(&next_catalog, &after, &mut extra_ops)?;
            if self.validation_enabled()? {
                self.validate_catalog_rows(&next_catalog, &after)?;
            }

            match self.persist_dataset_delta(
                &next_catalog,
                &before,
                &after,
                read_revision,
                &extra_ops,
                CommitIntent::with_catalog(
                    crate::ChangeSource::Migration,
                    catalog_snapshot.version,
                    Arc::clone(&next_catalog),
                ),
            )? {
                StorageCommitOutcome::Committed { .. } => Ok(PackageRegistrationOutcome {
                    executed_migrations,
                }),
                StorageCommitOutcome::Conflict {
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

    /// The underlying entity storage.
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// The storage revision of the latest commit. Every change event carries
    /// the revision its commit created, so a client that read state at this
    /// revision applies only events with a higher one.
    pub fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        self.storage.current_revision()
    }

    /// The feed publishing one event per committed write (see
    /// [`crate::ChangeFeed`]).
    pub fn change_feed(&self) -> &crate::ChangeFeed {
        &self.change_feed
    }

    /// Subscribe to changes committed from now on.
    pub fn subscribe_changes(
        &self,
        options: crate::ChangeSubscriptionOptions,
    ) -> crate::ChangeSubscription {
        self.change_feed.subscribe(options)
    }

    /// Replace the change feed by one retaining `capacity` undelivered
    /// events per subscriber. Existing subscriptions end.
    pub fn with_change_feed_capacity(mut self, capacity: usize) -> Self {
        self.change_feed = crate::ChangeFeed::new(capacity);
        self
    }

    pub fn into_parts(self) -> (SharedCatalog, S) {
        (self.catalog, self.storage)
    }

    pub fn insert(
        &mut self,
        collection: &str,
        id: impl Into<String>,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        self.execute_batch_returning(
            Batch::new().with_op(BatchOperation::Upsert {
                collection: collection.to_string(),
                id: id.into(),
                object,
            }),
            crate::BatchReturn::Stats,
        )?;
        Ok(())
    }

    pub fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        self.reader()?.get(collection, id)
    }

    pub fn delete(&mut self, collection: &str, id: &str) -> std::result::Result<(), DbError> {
        self.execute_batch_returning(
            Batch::new().with_op(BatchOperation::DeleteById {
                collection: collection.to_string(),
                id: id.to_string(),
            }),
            crate::BatchReturn::Stats,
        )?;
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
        self.reader()?.select(query)
    }

    pub fn plan_query(&self, query: Query) -> std::result::Result<QueryPlan, DbError> {
        self.reader()?.plan_query(query)
    }

    pub fn explain_query(&self, query: Query) -> std::result::Result<QueryExplain, DbError> {
        self.reader()?.explain_query(query)
    }

    pub fn insert_query(
        &mut self,
        query: InsertQuery,
    ) -> std::result::Result<crate::InsertResult, DbError> {
        let catalog = self.catalog();
        let plan = InsertPlan::new(&catalog, query, |select| self.select(select))?;
        let crate::BatchReply::Dataset(outcome) =
            self.execute_batch_returning(plan.batch.clone(), plan.reply_mode())?
        else {
            unreachable!("dataset returning requested")
        };
        plan.result(&self.catalog(), &outcome.dataset)
    }

    pub fn execute_physical_plan(
        &self,
        plan: &crate::PhysicalPlan,
        default_collection: Option<&str>,
    ) -> std::result::Result<Vec<Object>, DbError> {
        self.reader()?
            .execute_physical_plan(plan, default_collection)
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
    ) -> std::result::Result<crate::UpdateResult, DbError> {
        evaluate_mutation_limit(query.limit.as_ref())
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
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
        let txn_result = run_with_transaction_retries(TransactionOptions::default(), |_| {
            let mut repaired = false;
            loop {
                let catalog_snapshot = self.catalog.snapshot();
                let read_revision = self.storage.current_revision()?;
                let scope = TxScope::new(
                    catalog_snapshot.catalog.as_ref(),
                    read_revision,
                    IsolationLevel::ReadCommitted,
                );
                let context = DefaultExpressionContext::now();
                let recursive_validation = self.validation_enabled()?;
                let query_context = self.query_context();
                if let Some(result) = self.run_compact(
                    scope,
                    catalog_snapshot.version,
                    crate::WriteSettings::default(),
                    false,
                    |view, _| {
                        mutation::tx_update(
                            view,
                            &query_context,
                            &collection_name,
                            &query,
                            &context,
                            recursive_validation,
                        )
                    },
                    |_, _, _, result| Ok(result),
                )? {
                    return Ok::<_, DbError>(result);
                }
                compact::repaired_once(&mut repaired)?;
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
    ) -> std::result::Result<crate::DeleteResult, DbError> {
        evaluate_mutation_limit(query.limit.as_ref())
            .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
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
        let txn_result = run_with_transaction_retries(TransactionOptions::default(), |_| {
            let mut repaired = false;
            loop {
                let catalog_snapshot = self.catalog.snapshot();
                let read_revision = self.storage.current_revision()?;
                let scope = TxScope::new(
                    catalog_snapshot.catalog.as_ref(),
                    read_revision,
                    IsolationLevel::ReadCommitted,
                );
                let query_context = self.query_context();
                if let Some(result) = self.run_compact(
                    scope,
                    catalog_snapshot.version,
                    crate::WriteSettings::default(),
                    false,
                    |view, _| mutation::tx_delete(view, &query_context, &collection_name, &query),
                    |_, _, _, result| Ok(result),
                )? {
                    return Ok::<_, DbError>(result);
                }
                compact::repaired_once(&mut repaired)?;
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
        let crate::BatchReply::Dataset(outcome) = self.transact_returning(
            batch,
            options,
            crate::BatchReturn::Dataset,
            crate::WriteSettings::default(),
            false,
        )?
        else {
            unreachable!("dataset returning requested")
        };
        Ok(outcome)
    }

    pub fn execute_batch_returning(
        &mut self,
        batch: Batch,
        returning: crate::BatchReturn,
    ) -> Result<crate::BatchReply, DbError> {
        self.execute_batch_returning_with_settings(
            batch,
            returning,
            crate::WriteSettings::default(),
        )
    }

    pub fn execute_batch_returning_with_settings(
        &mut self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        self.transact_returning(
            batch,
            TransactionOptions::default(),
            returning,
            settings,
            false,
        )
    }

    pub fn execute_batch_returning_bounded_with_settings(
        &mut self,
        batch: Batch,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
    ) -> Result<crate::BatchReply, DbError> {
        self.transact_returning(
            batch,
            TransactionOptions::default(),
            returning,
            settings,
            true,
        )
    }

    /// Reject an isolation level the storage cannot provide, before any
    /// transaction attempt.
    fn ensure_isolation_supported(&self, isolation: IsolationLevel) -> Result<(), DbError> {
        if !isolation.requires_snapshot() {
            return Ok(());
        }
        if !self.storage.snapshot()?.is_consistent() {
            return Err(snapshot_isolation_unsupported(isolation));
        }
        if isolation == IsolationLevel::Serializable
            && !self.storage.tx_capabilities().conflict_detection
        {
            return Err(DbError::storage(
                StorageErrorKind::Unsupported,
                "Serializable isolation requires commit-time conflict detection, which this \
                 storage does not provide",
            ));
        }
        Ok(())
    }

    fn transact_returning(
        &mut self,
        batch: Batch,
        options: TransactionOptions,
        returning: crate::BatchReturn,
        settings: crate::WriteSettings,
        require_bounded: bool,
    ) -> Result<crate::BatchReply, DbError> {
        validate_batch_mutation_limits(&batch)?;
        let caps = self.storage.tx_capabilities();
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
        self.ensure_isolation_supported(options.isolation)?;

        let txn_result = run_with_transaction_retries(options, |_| {
            let mut repaired = false;
            loop {
                let catalog_snapshot = self.catalog.snapshot();
                let batch = canonicalize_batch(batch.clone(), catalog_snapshot.catalog.as_ref())?;
                let read_revision = self.storage.current_revision()?;
                let scope = TxScope::new(
                    catalog_snapshot.catalog.as_ref(),
                    read_revision,
                    options.isolation,
                );
                if let Some(reply) = self.compact_batch(
                    scope,
                    &batch,
                    &returning,
                    catalog_snapshot.version,
                    settings,
                    require_bounded,
                )? {
                    return Ok::<_, DbError>(reply);
                }
                compact::repaired_once(&mut repaired)?;
            }
        })?;
        let metrics =
            crate::WriteMetrics::from(&self.execution_counts).with_transaction(txn_result.metrics);
        Ok(txn_result.value.with_metrics(metrics))
    }

    pub fn execute_batch(&mut self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        self.transact_with_options(batch, TransactionOptions::default())
    }

    pub fn execute_batch_with_settings(
        &mut self,
        batch: Batch,
        settings: crate::WriteSettings,
    ) -> Result<BatchOutcome, DbError> {
        let crate::BatchReply::Dataset(outcome) = self.transact_returning(
            batch,
            TransactionOptions::default(),
            crate::BatchReturn::Dataset,
            settings,
            false,
        )?
        else {
            unreachable!("dataset returning requested")
        };
        Ok(outcome)
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
            let read_revision = self.storage.current_revision()?;
            let (next_catalog, ddl_outcome) =
                apply_ddl_batch(catalog_snapshot.catalog.as_ref(), &ddl)
                    .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
            let next_catalog = Arc::new(next_catalog);
            let mut extra_ops =
                self.ddl_cleanup_ops(catalog_snapshot.catalog.as_ref(), &next_catalog)?;
            self.backfill_new_indexes(
                catalog_snapshot.catalog.as_ref(),
                &next_catalog,
                read_revision,
                &mut extra_ops,
            )?;
            extra_ops.extend(catalog_write_ops(&self.storage, &next_catalog)?);
            self.rebuild_relationship_edges(&next_catalog, &BTreeMap::new(), &mut extra_ops)?;
            if self.validation_enabled()? {
                self.validate_catalog_rows(&next_catalog, &BTreeMap::new())?;
            }

            let dataset = BTreeMap::new();
            match self.persist_dataset_delta(
                catalog_snapshot.catalog.as_ref(),
                &dataset,
                &dataset,
                read_revision,
                &extra_ops,
                CommitIntent::with_catalog(
                    crate::ChangeSource::Ddl,
                    catalog_snapshot.version,
                    Arc::clone(&next_catalog),
                ),
            )? {
                StorageCommitOutcome::Committed { .. } => Ok(ddl_outcome),
                StorageCommitOutcome::Conflict {
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
        let batch = canonicalize_batch(batch, current_catalog)?;
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
        let out = Self::execute_batch_with_write_defaults(
            current_catalog,
            &dataset,
            &batch,
            self.validation_enabled()?,
        )?;
        for (collection_name, rows) in out.dataset {
            after.insert(collection_name, rows);
        }
        Ok(())
    }

    fn execute_batch_with_write_defaults(
        catalog: &Catalog,
        dataset: &BTreeMap<String, BTreeMap<String, Object>>,
        batch: &Batch,
        recursive_validation: bool,
    ) -> std::result::Result<BatchOutcome, DbError> {
        let default_context = DefaultExpressionContext::now();
        execute_batch_with_prepare(dataset, batch, |collection, _, object| {
            let collection_schema = catalog
                .collection_by_name(collection)
                .ok_or_else(|| CoreError::new(format!("collection '{collection}' not found")))?;
            prepare_row_for_write(
                catalog,
                collection_schema,
                object,
                &default_context,
                recursive_validation,
            )
            .map_err(|err| CoreError::new(err.to_string()))
        })
        .map_err(|err| match err.entity_exists {
            Some((collection, id)) => DbError::EntityExists { collection, id },
            None => DbError::InvalidQuery(err.message),
        })
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
        let rows = if self.storage.tx_capabilities().snapshot_reads {
            if let Some(revision) = read_revision {
                self.storage
                    .scan_collection_at_revision(collection.lid, revision)?
            } else {
                self.storage.scan_collection(collection.lid)?
            }
        } else {
            self.storage.scan_collection(collection.lid)?
        };
        Ok(rows
            .into_iter()
            .map(|row| (row.id, row.object))
            .collect::<BTreeMap<_, _>>())
    }

    fn format_output_rows(
        &self,
        catalog: &Catalog,
        rows: Vec<Object>,
        format: FieldFormat,
    ) -> Vec<Object> {
        crate::format_output_rows(catalog, rows, format)
    }

    fn ddl_cleanup_ops(
        &self,
        before: &Catalog,
        after: &Catalog,
    ) -> std::result::Result<Vec<StorageWriteOp>, DbError> {
        let mut ops = Vec::new();

        for (index_lid, _) in before.indexes() {
            if after.index_by_lid(index_lid).is_none() {
                ops.push(StorageWriteOp::ClearIndex(index_lid));
            }
        }

        for (collection_lid, _) in before.collections() {
            if after.collection_by_lid(collection_lid).is_none() {
                ops.push(StorageWriteOp::ClearCollection(collection_lid));
            }
        }

        Ok(ops)
    }

    pub fn collection_rows(
        &self,
        collection: LocalCollectionId,
    ) -> std::result::Result<Vec<EntityRecord>, DbError> {
        self.reader()?.collection_rows(collection)
    }

    pub fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.storage.tx_capabilities()
    }

    /// Persist a package migration or DDL change: every row of `after` is
    /// re-normalized and re-validated under `catalog`, and cascade deletes
    /// are expanded.
    ///
    /// This is the only write path that materializes whole collections:
    /// migrations may rewrite every row of a collection under a new
    /// catalog. Data writes use the point write path ([`compact`]).
    fn persist_dataset_delta(
        &mut self,
        catalog: &Catalog,
        before: &BTreeMap<String, BTreeMap<String, Object>>,
        after: &BTreeMap<String, BTreeMap<String, Object>>,
        expected_revision: Option<u64>,
        prelude_ops: &[StorageWriteOp],
        intent: CommitIntent,
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        let settings = crate::WriteSettings::default();
        let validation_enabled = self.validation_enabled()?;
        if validation_enabled {
            crate::validation::validate_enforcement_support(catalog)?;
        }
        // Apply DDL cleanup, index backfills, and catalog persistence first. A
        // package migration can both create an index and rewrite its collection;
        // the post-migration dataset below must be the final source of index rows.
        let mut ops = prelude_ops.to_vec();
        let mut normalized_after = BTreeMap::<String, BTreeMap<String, Object>>::new();

        for (collection_name, new_rows) in after {
            let collection_schema =
                catalog.collection_by_name(collection_name).ok_or_else(|| {
                    DbError::UnknownCollectionByName {
                        name: collection_name.clone(),
                    }
                })?;
            let mut normalized_rows = BTreeMap::<String, Object>::new();
            for (id, object) in new_rows {
                let mut object = object.clone();
                if validation_enabled {
                    crate::validation::normalize_for_recursive_validation(
                        catalog,
                        collection_schema,
                        &mut object,
                        None,
                    )?;
                } else {
                    normalize_object_for_collection(catalog, collection_schema, &mut object)?;
                }
                validate_primary_id(collection_schema, id, &object)?;
                normalized_rows.insert(id.clone(), object);
            }
            self.validate_unique_indexes(catalog, collection_schema, &normalized_rows)?;
            normalized_after.insert(collection_name.clone(), normalized_rows);
        }
        expand_dataset_cascade_deletes(catalog, before, &mut normalized_after);

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
                settings,
            )?;
        }

        for (collection_name, normalized_rows) in &normalized_after {
            let collection_schema =
                catalog.collection_by_name(collection_name).ok_or_else(|| {
                    DbError::UnknownCollectionByName {
                        name: collection_name.clone(),
                    }
                })?;
            let old_rows = before.get(collection_name);
            if old_rows == Some(normalized_rows) {
                continue;
            }
            incremental::push_row_delta(
                catalog,
                collection_schema.lid,
                old_rows.unwrap_or(&BTreeMap::new()),
                normalized_rows,
                &mut ops,
            );
        }

        let changes = crate::batch_return::changes(before, &normalized_after);
        if !before.is_empty() || !normalized_after.is_empty() {
            self.update_relationship_edges(catalog, before, &normalized_after, &mut ops)?;
            compact::update_reverse_references(catalog, &changes, &mut ops);
        }

        self.commit_write(&ops, expected_revision, intent, changes)
    }

    fn push_entity_ops(
        &self,
        ops: &mut Vec<StorageWriteOp>,
        entity: &StoredEntity,
    ) -> std::result::Result<(), DbError> {
        ops.push(StorageWriteOp::PutEntity(entity.clone()));
        Ok(())
    }

    fn push_index_ops(
        &self,
        ops: &mut Vec<StorageWriteOp>,
        index: &crate::catalog::IndexSchema,
        entity_id: &str,
        object: &Object,
    ) -> std::result::Result<(), DbError> {
        ops.push(StorageWriteOp::IndexEntity {
            index: index.clone(),
            entity_id: entity_id.to_string(),
            object: object.clone(),
        });
        Ok(())
    }

    fn validate_unique_indexes(
        &self,
        catalog: &Catalog,
        collection: &CollectionSchema,
        rows: &BTreeMap<String, Object>,
    ) -> std::result::Result<(), DbError> {
        for index in unique_indexes(catalog, collection) {
            check_unique_index_rows(collection, index, rows)?;
        }
        Ok(())
    }

    /// Validate the references (and, with recursive validation, the stored
    /// values) of `rows` of `collection` against the final dataset.
    fn validate_ref_fields<'r>(
        &self,
        catalog: &Catalog,
        collection: &CollectionSchema,
        rows: impl IntoIterator<Item = (&'r String, &'r Object)>,
        all_after: &BTreeMap<String, BTreeMap<String, Object>>,
        settings: crate::WriteSettings,
    ) -> std::result::Result<(), DbError> {
        let target_rows =
            all_after
                .get(&collection.name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: collection.name.clone(),
                })?;

        if self.validation_enabled()? {
            for (id, object) in rows {
                crate::validate_stored_object_with_settings(
                    catalog,
                    &(collection.name.clone(), id.clone()),
                    object,
                    |key| {
                        Ok(all_after
                            .get(&key.0)
                            .and_then(|rows| rows.get(&key.1))
                            .cloned())
                    },
                    settings,
                )?;
            }
            return Ok(());
        }
        for (_, object) in rows {
            let field_types = resolved_field_types_for_object(catalog, collection, object);
            for (field, ty) in field_types {
                let Some(value) = object.get(&field) else {
                    continue;
                };
                validate_ref_value(
                    catalog,
                    collection,
                    target_rows,
                    &field,
                    &ty,
                    value,
                    settings.validate_foreign_keys,
                )?;
            }
        }

        Ok(())
    }

    fn rebuild_relationship_edges(
        &self,
        catalog: &Catalog,
        after: &BTreeMap<String, BTreeMap<String, Object>>,
        ops: &mut Vec<StorageWriteOp>,
    ) -> std::result::Result<(), DbError> {
        let Some(rel_collection) = catalog.collection_by_name(RELATION_EDGES_COLLECTION) else {
            return Ok(());
        };
        ops.push(StorageWriteOp::ClearCollection(rel_collection.lid));
        let indexes: Vec<_> = catalog
            .indexes_for_collection(rel_collection.lid)
            .cloned()
            .collect();
        for index in &indexes {
            ops.push(StorageWriteOp::ResetIndex(index.lid));
        }

        let rows = self.compute_relationship_edges(catalog, after, None)?;
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
        self.rebuild_relationship_contributors(catalog, after, ops)?;
        self.rebuild_reverse_references(catalog, after, ops)?;
        Ok(())
    }

    fn compute_relationship_edges(
        &self,
        catalog: &Catalog,
        after: &BTreeMap<String, BTreeMap<String, Object>>,
        affected: Option<&BTreeSet<String>>,
    ) -> std::result::Result<Vec<(String, Object)>, DbError> {
        let mut out = Vec::new();
        for (_, rel_schema) in catalog.relationships() {
            let relationship = &rel_schema.relationship;
            if affected.is_some_and(|ids| !ids.contains(&relationship.id)) {
                continue;
            }
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
                self.storage
                    .scan_collection(source_collection.lid)?
                    .into_iter()
                    .map(|row| (row.id, row.object))
                    .collect::<Vec<_>>()
            };
            let direct: Vec<_> = source_rows
                .iter()
                .filter_map(|(id, object)| {
                    incremental::contribution(catalog, relationship, source_collection, id, object)
                })
                .collect();
            out.extend(Self::edges_from_direct(relationship, &direct));
        }
        Ok(out)
    }

    /// The edges of `relationship` given its direct `(source, target)`
    /// pairs: one edge per pair, plus, with indexing enabled, one edge per
    /// transitively reachable pair at its shortest depth.
    pub(super) fn edges_from_direct(
        relationship: &RelationType,
        direct: &[(String, String)],
    ) -> Vec<(String, Object)> {
        let mut shortest = BTreeMap::<(String, String), usize>::new();
        if relationship.indexing_mode == RelationIndexingMode::Enabled {
            let mut adjacency = BTreeMap::<String, Vec<String>>::new();
            for (source, target) in direct {
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
            for (source, target) in direct {
                let key = (source.clone(), target.clone());
                shortest
                    .entry(key)
                    .and_modify(|depth| *depth = (*depth).min(1))
                    .or_insert(1);
            }
        }
        shortest
            .into_iter()
            .map(|((source, target), depth)| {
                incremental::relationship_edge(&relationship.id, &source, &target, depth)
            })
            .collect()
    }
}

/// An `INSERT` lowered to a batch of upserts.
pub(super) struct InsertPlan {
    pub batch: Batch,
    collection: String,
    inserted_ids: Vec<String>,
    returning: Vec<crate::QueryField>,
    field_format: FieldFormat,
}

impl InsertPlan {
    /// Lower `query` under `catalog`; `select` runs the source query of
    /// `INSERT ... SELECT`.
    pub(super) fn new(
        catalog: &Catalog,
        query: InsertQuery,
        select: impl FnOnce(SelectQuery) -> std::result::Result<Vec<Object>, DbError>,
    ) -> std::result::Result<Self, DbError> {
        let collection_name = query.collection_or_default().to_string();
        let collection = catalog
            .collection_by_name(&collection_name)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: collection_name.clone(),
            })?;
        let query = canonicalize_insert_query(&query, catalog, collection)?;
        let target_columns = query
            .columns
            .iter()
            .map(|column| collection.canonical_field_name(column).to_string())
            .collect::<Vec<_>>();
        let field_format = query.field_format;
        let InsertQuery {
            source, returning, ..
        } = query;
        let rows = materialize_insert_rows(collection, &target_columns, source, select)?;
        if rows.is_empty() {
            return Err(DbError::InvalidQuery(
                "INSERT requires at least one row".to_string(),
            ));
        }
        let mut batch = Batch::new();
        let mut inserted_ids = Vec::with_capacity(rows.len());
        for object in rows {
            let id = extract_insert_id(collection, &object)?;
            inserted_ids.push(id.clone());
            batch = batch.with_op(BatchOperation::Upsert {
                collection: collection.name.clone(),
                id,
                object,
            });
        }
        Ok(Self {
            batch,
            collection: collection.name.clone(),
            inserted_ids,
            returning,
            field_format,
        })
    }

    /// The reply the batch needs: the written rows only when the insert
    /// returns a projection of them.
    pub(super) fn reply_mode(&self) -> crate::BatchReturn {
        if self.returning.is_empty() {
            crate::BatchReturn::Stats
        } else {
            crate::BatchReturn::Dataset
        }
    }

    /// The result of the insert, given the rows the batch wrote.
    pub(super) fn result(
        &self,
        catalog: &Catalog,
        written: &crate::Dataset,
    ) -> std::result::Result<crate::InsertResult, DbError> {
        let returning_rows = if self.returning.is_empty() {
            Vec::new()
        } else {
            let stored_rows = written.get(&self.collection).ok_or_else(|| {
                DbError::Storage(
                    format!(
                        "inserted collection '{}' missing from batch outcome",
                        self.collection
                    )
                    .into(),
                )
            })?;
            self.inserted_ids
                .iter()
                .map(|id| {
                    stored_rows
                        .get(id)
                        .map(|object| crate::project_object(object, &self.returning))
                        .ok_or_else(|| DbError::EntityNotFound {
                            collection: self.collection.clone(),
                            id: id.clone(),
                        })
                })
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(crate::InsertResult {
            inserted: self.inserted_ids.len(),
            returning: crate::format_output_rows(catalog, returning_rows, self.field_format),
        })
    }
}

fn materialize_insert_rows(
    collection: &CollectionSchema,
    target_columns: &[String],
    source: InsertSource,
    select: impl FnOnce(SelectQuery) -> std::result::Result<Vec<Object>, DbError>,
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
            materialize_insert_value_rows(collection, target_columns, rows)
        }
        InsertSource::Select(query) => {
            materialize_insert_select_rows(target_columns, query, select)
        }
    }
}

fn materialize_insert_value_rows(
    collection: &CollectionSchema,
    target_columns: &[String],
    rows: Vec<Vec<crate::Expr>>,
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
            let value = crate::evaluate_expr(&Object::new(), expr).ok_or_else(|| {
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
    target_columns: &[String],
    query: SelectQuery,
    select: impl FnOnce(SelectQuery) -> std::result::Result<Vec<Object>, DbError>,
) -> std::result::Result<Vec<Object>, DbError> {
    let source_projection = query.projection.clone();
    let source_rows = select(query)?;

    if target_columns.is_empty() {
        return Ok(source_rows);
    }
    if source_projection.is_empty() {
        return Err(DbError::InvalidQuery(
            "INSERT ... SELECT with target columns requires explicit SELECT projection".to_string(),
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

fn extract_insert_id(
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

/// Check that `object` carries `id` as its primary key.
pub(super) fn validate_primary_id(
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

/// Row count of `collection`: the maintained count when the storage keeps
/// one, otherwise counted from storage keys.
fn collection_row_count(
    reader: &dyn EntityReadSnapshot,
    collection: LocalCollectionId,
) -> std::result::Result<u64, DbError> {
    match reader.collection_row_count(collection)? {
        Some(count) => Ok(count),
        None => reader.count_collection_entities(collection),
    }
}

/// Normalize a row written by a mutation, applying write defaults.
fn prepare_row_for_write(
    catalog: &Catalog,
    collection: &CollectionSchema,
    object: &mut Object,
    context: &DefaultExpressionContext,
    recursive_validation: bool,
) -> std::result::Result<(), crate::ObjectNormalizationError> {
    if recursive_validation {
        crate::validation::normalize_for_recursive_validation(
            catalog,
            collection,
            object,
            Some(context),
        )
    } else {
        prepare_object_for_write(catalog, collection, object, context)
    }
}

/// The index scan a physical plan reads its rows through, if any.
fn find_index_range(plan: &crate::PhysicalPlan) -> Option<&crate::PhysicalIndexScan> {
    match plan {
        crate::PhysicalPlan::Source(crate::PhysicalSource::IndexRange(scan)) => Some(scan),
        crate::PhysicalPlan::Filter { input, .. }
        | crate::PhysicalPlan::Sort { input, .. }
        | crate::PhysicalPlan::TopN { input, .. }
        | crate::PhysicalPlan::Project { input, .. }
        | crate::PhysicalPlan::Aggregate { input, .. }
        | crate::PhysicalPlan::Limit { input, .. }
        | crate::PhysicalPlan::Distinct { input, .. }
        | crate::PhysicalPlan::Materialize { input, .. }
        | crate::PhysicalPlan::Exchange { input, .. }
        | crate::PhysicalPlan::RepartitionHash { input, .. } => find_index_range(input),
        _ => None,
    }
}

/// The index lookup a physical plan reads its rows through, if any.
fn find_index_lookup(plan: &crate::PhysicalPlan) -> Option<(&crate::FieldRef, &Value)> {
    match plan {
        crate::PhysicalPlan::Source(crate::PhysicalSource::IndexLookup {
            field, value, ..
        }) => Some((field, value)),
        crate::PhysicalPlan::Filter { input, .. }
        | crate::PhysicalPlan::Sort { input, .. }
        | crate::PhysicalPlan::TopN { input, .. }
        | crate::PhysicalPlan::Project { input, .. }
        | crate::PhysicalPlan::Aggregate { input, .. }
        | crate::PhysicalPlan::Limit { input, .. }
        | crate::PhysicalPlan::Distinct { input, .. }
        | crate::PhysicalPlan::Materialize { input, .. }
        | crate::PhysicalPlan::Exchange { input, .. }
        | crate::PhysicalPlan::RepartitionHash { input, .. } => find_index_lookup(input),
        crate::PhysicalPlan::Union { .. }
        | crate::PhysicalPlan::Values { .. }
        | crate::PhysicalPlan::Join(..)
        | crate::PhysicalPlan::ApplyExists { .. }
        | crate::PhysicalPlan::ApplyInSubquery { .. }
        | crate::PhysicalPlan::Source(_) => None,
    }
}

/// Field path of an index lookup field in `collection`.
fn lookup_field_path(collection: &CollectionSchema, field: &crate::FieldRef) -> Option<FieldPath> {
    match field {
        crate::FieldRef::CanonicalName(name) => Some(FieldPath::from_fields([name])),
        crate::FieldRef::FieldId(field_id) => collection
            .field_name_by_id(*field_id)
            .map(|name| FieldPath::from_fields([name])),
        crate::FieldRef::AttrId(attr_id) => collection.fields().find_map(|(field_id, _)| {
            (collection.attr_for_field_id(field_id) == Some(*attr_id))
                .then(|| collection.field_name_by_id(field_id))
                .flatten()
                .map(|name| FieldPath::from_fields([name]))
        }),
        crate::FieldRef::Path(path) => Some(path.clone()),
    }
}

/// Equality index answering a lookup of `field_path` in `collection`, with
/// the path to probe for path-equality indexes.
///
/// A single top-level field uses its equality index, falling back to the
/// path-equality index; nested paths use the path-equality index only when
/// they address list elements.
fn equality_lookup_index<'c>(
    catalog: &'c Catalog,
    collection: LocalCollectionId,
    field_path: &FieldPath,
) -> Option<(&'c crate::catalog::IndexSchema, Option<FieldPath>)> {
    let segments = field_path.segments();
    if segments.len() == 1 {
        let PathSegment::Field(field) = &segments[0] else {
            return None;
        };
        catalog
            .find_lookup_index(collection, field)
            .map(|index| (index, None))
            .or_else(|| {
                catalog
                    .find_path_equality_index(collection)
                    .map(|index| (index, Some(field_path.clone())))
            })
    } else if segments
        .iter()
        .any(|segment| matches!(segment, PathSegment::Index(_)))
    {
        catalog
            .find_path_equality_index(collection)
            .map(|index| (index, Some(field_path.clone())))
    } else {
        None
    }
}

fn infer_project_key_for_insert(expr: &crate::Expr) -> String {
    if let crate::Expr::Operand(crate::Operand::Field(path)) = expr {
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

fn equality_expr(path: FieldPath, value: Value) -> crate::Expr {
    crate::Expr::Binary {
        op: semantic_data::query::BinaryOp::Eq,
        left: Box::new(crate::Expr::Operand(crate::Operand::Field(path))),
        right: Box::new(crate::Expr::Operand(crate::Operand::Literal(value))),
    }
}

/// Physical query source reading every row through one storage snapshot.
struct EmbeddedPhysicalDataSource<'a> {
    reader: QueryReader<'a>,
    catalog: std::sync::Arc<Catalog>,
    default_collection: Option<String>,
    /// Present only when the plan evaluates paths that can follow a string
    /// id to a same-collection row.
    local_refs: Option<Arc<LocalRefResolver>>,
}

/// Row views of one collection.
struct EmbeddedCollectionScan {
    collection_id: LocalCollectionId,
    rows: Box<dyn Iterator<Item = crate::CoreResult<KvObjectView>> + Send>,
}

impl EmbeddedPhysicalDataSource<'_> {
    /// Wrap stored rows of `collection` into row views.
    ///
    /// Rows stay lazy unless the query resolves local references without an
    /// owned snapshot: those references must be prefetched while the
    /// borrowed snapshot is reachable, which reads the rows eagerly.
    fn row_views(
        &self,
        collection: &CollectionSchema,
        rows: impl Iterator<Item = crate::CoreResult<StoredEntity>> + Send + 'static,
    ) -> crate::CoreResult<EmbeddedCollectionScan> {
        let collection_id = collection.lid;
        let fields = Arc::new(CollectionFieldMaps::new(collection));
        let scope = self
            .local_refs
            .as_ref()
            .and_then(|resolver| resolver.scope(&self.catalog, collection));
        let rows: Box<dyn Iterator<Item = crate::CoreResult<KvObjectView>> + Send> = match scope {
            None => Box::new(rows.map(move |row| {
                row.map(|row| KvObjectView {
                    object: row.object,
                    collection_id,
                    fields: fields.clone(),
                    local_refs: None,
                })
            })),
            Some(scope) if scope.is_lazy() => {
                let local_refs = RowLocalRefs::Lazy(scope);
                Box::new(rows.map(move |row| {
                    row.map(|row| KvObjectView {
                        object: row.object,
                        collection_id,
                        fields: fields.clone(),
                        local_refs: Some(local_refs.clone()),
                    })
                }))
            }
            Some(scope) => {
                let views = rows
                    .map(|row| {
                        let row = row?;
                        let local_refs = scope.row_refs(&*self.reader, &row.object)?;
                        Ok(KvObjectView {
                            object: row.object,
                            collection_id,
                            fields: fields.clone(),
                            local_refs,
                        })
                    })
                    .collect::<crate::CoreResult<Vec<_>>>()?;
                Box::new(views.into_iter().map(Ok))
            }
        };
        Ok(EmbeddedCollectionScan {
            collection_id,
            rows,
        })
    }

    fn try_relation_lookup_ids(
        &self,
        source_collection: &CollectionSchema,
        predicate: &crate::Expr,
    ) -> crate::CoreResult<Option<(Vec<String>, Option<crate::Expr>)>> {
        let Some((relation, residual)) = split_first_relation_conjunct(predicate) else {
            return Ok(None);
        };
        Ok(self
            .try_exact_relation_lookup_ids(source_collection, &relation)?
            .map(|ids| (ids, residual)))
    }

    fn try_exact_relation_lookup_ids(
        &self,
        source_collection: &CollectionSchema,
        predicate: &crate::Expr,
    ) -> crate::CoreResult<Option<Vec<String>>> {
        let crate::Expr::RelationExists {
            relation,
            source,
            target,
            transitive,
            max_depth,
        } = predicate
        else {
            return Ok(None);
        };
        let relation_id = crate::evaluate_expr(&Object::new(), relation)
            .and_then(|value| value.as_str().map(ToString::to_string));
        let max_depth = match max_depth.as_ref() {
            None => None,
            Some(expr) => {
                let Some(value) = crate::evaluate_expr(&Object::new(), expr) else {
                    return Ok(None);
                };
                let Some(depth) = value_to_usize(&value) else {
                    return Ok(None);
                };
                Some(depth)
            }
        };
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
            let Some(target_id) = crate::evaluate_expr(&Object::new(), target)
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
            let Some(source_id) = crate::evaluate_expr(&Object::new(), source)
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
    ) -> crate::CoreResult<Vec<String>> {
        let ids = if let Some(index) = self.catalog.find_equality_index(rel_collection, key_field) {
            self.reader
                .scan_index_value(index.lid, None, &Value::String(key_value))
                .map_err(|err| crate::CoreError::new(err.to_string()))?
        } else {
            self.reader
                .scan_collection(rel_collection)
                .map_err(|err| crate::CoreError::new(err.to_string()))?
                .into_iter()
                .map(|entity| entity.id)
                .collect()
        };
        let mut out = BTreeSet::new();
        for id in ids {
            let Some(edge) = self
                .reader
                .get_entity(rel_collection, &id)
                .map_err(|err| crate::CoreError::new(err.to_string()))?
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

    fn source_name<'a>(&'a self, source: &'a crate::SourceRef) -> Option<&'a str> {
        source
            .source_name
            .as_deref()
            .or(self.default_collection.as_deref())
    }

    fn resolve_collection<'a>(
        &'a self,
        source: &'a crate::SourceRef,
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

    fn scan_all_collections(&self) -> crate::CoreResult<Vec<EmbeddedCollectionScan>> {
        let mut scans = Vec::new();
        for (_, collection) in self.catalog.collections() {
            scans.push(self.collection_scan(collection)?);
        }
        Ok(scans)
    }

    fn scan_collections(
        &self,
        source: &crate::SourceRef,
    ) -> crate::CoreResult<Vec<EmbeddedCollectionScan>> {
        if self.is_all_alias_source(source) {
            return self.scan_all_collections();
        }
        let collection = self
            .resolve_collection(source)
            .map_err(|err| crate::CoreError::new(err.to_string()))?;
        Ok(vec![self.collection_scan(collection)?])
    }

    fn collection_scan(
        &self,
        collection: &CollectionSchema,
    ) -> crate::CoreResult<EmbeddedCollectionScan> {
        let rows = self
            .reader
            .scan_collection_stream(collection.lid)
            .map_err(|err| crate::CoreError::new(err.to_string()))?;
        self.row_views(
            collection,
            rows.map(|row| row.map_err(|err| crate::CoreError::new(err.to_string()))),
        )
    }

    fn scans_to_stream(
        scans: Vec<EmbeddedCollectionScan>,
        predicate: Option<crate::Expr>,
    ) -> crate::SendableRecordBatchStream {
        Self::limited_scans_to_stream(scans, predicate, None)
    }

    /// Stream the rows of `scans` that match `predicate` in batches. With a
    /// `limit_hint` (the rows the consumer reads at most), batches hold at
    /// most that many rows, so a limit that is satisfied by the first batch
    /// stops the scan after reading only the rows it needed.
    fn limited_scans_to_stream(
        scans: Vec<EmbeddedCollectionScan>,
        predicate: Option<crate::Expr>,
        limit_hint: Option<usize>,
    ) -> crate::SendableRecordBatchStream {
        let batch_size = limit_hint.map_or(crate::DEFAULT_EXECUTION_BATCH_SIZE, |limit| {
            limit.clamp(1, crate::DEFAULT_EXECUTION_BATCH_SIZE)
        });
        stream::unfold(
            (scans.into_iter(), None::<EmbeddedCollectionScan>, predicate),
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
                    let view = match row {
                        Ok(view) => view,
                        Err(err) => return Some((Err(err), (scans, current, predicate))),
                    };
                    if predicate
                        .as_ref()
                        .is_none_or(|predicate| crate::evaluate_filter_expr(&view, predicate))
                    {
                        batch.push(Box::new(view) as crate::DynObject);
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

    /// Filter `scan` by a predicate that may contain relationship checks.
    fn filter_scan_with_relationships(
        &self,
        scan: EmbeddedCollectionScan,
        predicate: &crate::Expr,
    ) -> crate::CoreResult<EmbeddedCollectionScan> {
        let mut filtered = Vec::new();
        for view in scan.rows {
            let view = view?;
            if self.evaluate_predicate_with_relationships(&view, predicate)? {
                filtered.push(view);
            }
        }
        Ok(EmbeddedCollectionScan {
            collection_id: scan.collection_id,
            rows: Box::new(filtered.into_iter().map(Ok)),
        })
    }

    fn scan_filtered_collections(
        &self,
        source: &crate::SourceRef,
        predicate: &crate::Expr,
    ) -> crate::CoreResult<(Vec<EmbeddedCollectionScan>, bool)> {
        if let Ok(collection) = self.resolve_collection(source)
            && let Some((ids, residual)) = self.try_relation_lookup_ids(collection, predicate)?
        {
            let scan = self.materialize_ids(collection, ids)?;
            if let Some(residual) = residual {
                return Ok((
                    vec![self.filter_scan_with_relationships(scan, &residual)?],
                    true,
                ));
            }
            return Ok((vec![scan], true));
        }
        if !expr_contains_relationship(predicate) {
            return Ok((self.scan_collections(source)?, false));
        }
        let scans = self
            .scan_collections(source)?
            .into_iter()
            .map(|scan| self.filter_scan_with_relationships(scan, predicate))
            .collect::<crate::CoreResult<Vec<_>>>()?;
        Ok((scans, true))
    }

    /// The rows with `ids`, read lazily when the reader owns its snapshot
    /// (so a limited consumer reads only the rows it takes).
    fn materialize_ids(
        &self,
        collection: &CollectionSchema,
        ids: Vec<String>,
    ) -> crate::CoreResult<EmbeddedCollectionScan> {
        if let Some(snapshot) = self.reader.shared() {
            let lid = collection.lid;
            let rows = ids.into_iter().filter_map(move |id| {
                snapshot
                    .get_entity(lid, &id)
                    .map_err(|err| crate::CoreError::new(err.to_string()))
                    .transpose()
            });
            return self.row_views(collection, rows);
        }
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(entity) = self
                .reader
                .get_entity(collection.lid, &id)
                .map_err(|err| crate::CoreError::new(err.to_string()))?
            {
                rows.push(entity);
            }
        }
        self.row_views(collection, rows.into_iter().map(Ok))
    }

    fn is_all_alias_source(&self, source: &crate::SourceRef) -> bool {
        self.source_name(source)
            .is_some_and(is_all_collection_alias)
    }

    fn evaluate_predicate_with_relationships(
        &self,
        row: &dyn crate::ObjectAccess,
        predicate: &crate::Expr,
    ) -> crate::CoreResult<bool> {
        match predicate {
            crate::Expr::RelationExists {
                relation,
                source,
                target,
                transitive,
                max_depth,
            } => {
                let relation_id = crate::evaluate_expr(row, relation)
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .ok_or_else(|| {
                        crate::CoreError::new("relationship expression requires string relation id")
                    })?;
                let source_id = crate::evaluate_expr(row, source)
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .ok_or_else(|| {
                        crate::CoreError::new("relationship expression requires string source id")
                    })?;
                let target_id = crate::evaluate_expr(row, target)
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .ok_or_else(|| {
                        crate::CoreError::new("relationship expression requires string target id")
                    })?;
                let max_depth = max_depth
                    .as_ref()
                    .map(|expr| {
                        crate::evaluate_expr(row, expr)
                            .and_then(|value| value_to_usize(&value))
                            .ok_or_else(|| {
                                crate::CoreError::new(
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
            crate::Expr::Binary {
                op: semantic_data::query::BinaryOp::And,
                left,
                right,
            } => Ok(self.evaluate_predicate_with_relationships(row, left)?
                && self.evaluate_predicate_with_relationships(row, right)?),
            crate::Expr::Binary {
                op: semantic_data::query::BinaryOp::Or,
                left,
                right,
            } => Ok(self.evaluate_predicate_with_relationships(row, left)?
                || self.evaluate_predicate_with_relationships(row, right)?),
            crate::Expr::Unary {
                op: semantic_data::query::UnaryOp::Not,
                expr,
            } => Ok(!self.evaluate_predicate_with_relationships(row, expr)?),
            _ => Ok(crate::evaluate_filter_expr(row, predicate)),
        }
    }

    fn relationship_exists(
        &self,
        relation_id: &str,
        source_id: &str,
        target_id: &str,
        transitive: bool,
        max_depth: Option<usize>,
    ) -> crate::CoreResult<bool> {
        let Some(rel_collection) = self.catalog.collection_by_name(RELATION_EDGES_COLLECTION)
        else {
            return Ok(false);
        };
        let source_key = Value::String(format!("{relation_id}|{source_id}"));
        let candidate_ids = if let Some(index) = self
            .catalog
            .find_equality_index(rel_collection.lid, REL_EDGE_SOURCE_KEY_FIELD)
        {
            self.reader
                .scan_index_value(index.lid, None, &source_key)
                .map_err(|err| crate::CoreError::new(err.to_string()))?
        } else {
            self.reader
                .scan_collection(rel_collection.lid)
                .map_err(|err| crate::CoreError::new(err.to_string()))?
                .into_iter()
                .map(|entity| entity.id)
                .collect()
        };
        for candidate_id in candidate_ids {
            let Some(edge) = self
                .reader
                .get_entity(rel_collection.lid, &candidate_id)
                .map_err(|err| crate::CoreError::new(err.to_string()))?
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
/// Field and attribute id to name maps of one collection, shared by every row
/// view of a scan.
struct CollectionFieldMaps {
    field_names: BTreeMap<LocalFieldId, String>,
    attr_names: BTreeMap<LocalAttrId, String>,
}

impl CollectionFieldMaps {
    fn new(collection: &CollectionSchema) -> Self {
        let mut field_names = BTreeMap::new();
        let mut attr_names = BTreeMap::new();
        for (field_id, name) in collection.fields() {
            field_names.insert(field_id, name.to_string());
            if let Some(attr_id) = collection.attr_for_field_id(field_id) {
                attr_names.insert(attr_id, name.to_string());
            }
        }
        Self {
            field_names,
            attr_names,
        }
    }
}

#[derive(Clone)]
struct KvObjectView {
    object: Object,
    collection_id: LocalCollectionId,
    fields: Arc<CollectionFieldMaps>,
    /// Resolution of same-collection references through string ids, when
    /// the plan's paths may follow them for this collection.
    local_refs: Option<RowLocalRefs>,
}

impl crate::ObjectAccess for KvObjectView {
    fn value_at_path_ref<'a>(&'a self, path: &FieldPath) -> Option<ValueRef<'a>> {
        if let Some(value) = crate::ObjectAccess::value_at_path_ref(&self.object, path) {
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
        match &self.local_refs {
            Some(local_refs) => local_refs.resolve(&self.object, path),
            None => resolve_path_with_local_refs(&self.object, path, &mut |_| None),
        }
        .map(ValueRef::Owned)
    }

    fn value_at_attr_ref<'a>(&'a self, attr: LocalAttrId) -> Option<ValueRef<'a>> {
        let name = self.fields.attr_names.get(&attr)?;
        let path = FieldPath::from_fields([name.as_str()]);
        crate::ObjectAccess::value_at_path_ref(&self.object, &path)
    }

    fn value_at_field_ref<'a>(&'a self, field: LocalFieldId) -> Option<ValueRef<'a>> {
        let name = self.fields.field_names.get(&field)?;
        let path = FieldPath::from_fields([name.as_str()]);
        crate::ObjectAccess::value_at_path_ref(&self.object, &path)
    }

    fn collection_id(&self) -> Option<LocalCollectionId> {
        Some(self.collection_id)
    }

    fn to_object(&self) -> Object {
        self.object.clone()
    }
}

fn expr_contains_relationship(expr: &crate::Expr) -> bool {
    match expr {
        crate::Expr::RelationExists { .. } => true,
        crate::Expr::Binary { left, right, .. } => {
            expr_contains_relationship(left) || expr_contains_relationship(right)
        }
        crate::Expr::Unary { expr, .. } => expr_contains_relationship(expr),
        crate::Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_contains_relationship(cond)
                || expr_contains_relationship(then_expr)
                || expr_contains_relationship(else_expr)
        }
        crate::Expr::Coalesce(exprs) => exprs.iter().any(expr_contains_relationship),
        crate::Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            crate::FunctionArg::Expr(expr) => expr_contains_relationship(expr),
            crate::FunctionArg::Wildcard => false,
        }),
        crate::Expr::Aggregate { arg, .. } => match arg.as_ref() {
            crate::FunctionArg::Expr(expr) => expr_contains_relationship(expr),
            crate::FunctionArg::Wildcard => false,
        },
        crate::Expr::InList { expr, list, .. } => {
            expr_contains_relationship(expr) || list.iter().any(expr_contains_relationship)
        }
        crate::Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_relationship(expr)
                || expr_contains_relationship(low)
                || expr_contains_relationship(high)
        }
        crate::Expr::PatternMatch { expr, pattern, .. }
        | crate::Expr::RegexMatch { expr, pattern, .. } => {
            expr_contains_relationship(expr) || expr_contains_relationship(pattern)
        }
        crate::Expr::TextMatch { exprs, query, .. } => {
            exprs.iter().any(expr_contains_relationship) || expr_contains_relationship(query)
        }
        crate::Expr::IsNull { expr, .. } => expr_contains_relationship(expr),
        crate::Expr::Operand(_) | crate::Expr::Subquery(_) | crate::Expr::Exists { .. } => false,
    }
}

fn split_first_relation_conjunct(expr: &crate::Expr) -> Option<(crate::Expr, Option<crate::Expr>)> {
    if matches!(expr, crate::Expr::RelationExists { .. }) {
        return Some((expr.clone(), None));
    }
    let crate::Expr::Binary {
        op: semantic_data::query::BinaryOp::And,
        left,
        right,
    } = expr
    else {
        return None;
    };
    if let Some((relation, left_residual)) = split_first_relation_conjunct(left) {
        return Some((
            relation,
            combine_optional_predicates(left_residual, Some((**right).clone())),
        ));
    }
    split_first_relation_conjunct(right).map(|(relation, right_residual)| {
        (
            relation,
            combine_optional_predicates(Some((**left).clone()), right_residual),
        )
    })
}

fn combine_optional_predicates(
    left: Option<crate::Expr>,
    right: Option<crate::Expr>,
) -> Option<crate::Expr> {
    match (left, right) {
        (None, None) => None,
        (Some(expr), None) | (None, Some(expr)) => Some(expr),
        (Some(left), Some(right)) => Some(crate::Expr::Binary {
            op: semantic_data::query::BinaryOp::And,
            left: Box::new(left),
            right: Box::new(right),
        }),
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
        return Ok(catalog.drop_raw_system_path_index(name));
    }
    catalog
        .set_collection_internal(name, true)
        .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
    Ok(true)
}

/// Check that every operation of `batch` targets a known, mutable collection
/// and canonicalize its predicate mutations.
pub(super) fn canonicalize_batch(
    batch: Batch,
    catalog: &Catalog,
) -> std::result::Result<Batch, DbError> {
    let mut canonical_ops = Vec::with_capacity(batch.operations.len());
    for op in batch.operations {
        match op {
            BatchOperation::Create {
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
                canonical_ops.push(BatchOperation::Create {
                    collection,
                    id,
                    object,
                });
            }
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

fn ensure_collection_mutable(collection: &CollectionSchema) -> std::result::Result<(), DbError> {
    if collection.internal {
        return Err(DbError::InvalidQuery(format!(
            "collection '{}' is internal and cannot be modified directly",
            collection.name
        )));
    }
    Ok(())
}

fn validate_batch_mutation_limits(batch: &Batch) -> std::result::Result<(), DbError> {
    for operation in &batch.operations {
        let limit = match operation {
            BatchOperation::Update { query, .. } => query.limit.as_ref(),
            BatchOperation::Delete { query, .. } => query.limit.as_ref(),
            _ => continue,
        };
        evaluate_mutation_limit(limit).map_err(|err| DbError::InvalidQuery(err.to_string()))?;
    }
    Ok(())
}

fn expand_dataset_cascade_deletes(
    catalog: &Catalog,
    before: &BTreeMap<String, BTreeMap<String, Object>>,
    after: &mut BTreeMap<String, BTreeMap<String, Object>>,
) -> usize {
    let mut deleted = BTreeSet::new();
    for (collection, rows) in before {
        let surviving = after.get(collection);
        for id in rows.keys() {
            if surviving.is_none_or(|rows| !rows.contains_key(id)) {
                deleted.insert((collection.clone(), id.clone()));
            }
        }
    }

    if deleted.is_empty() {
        return 0;
    }
    let mut count = 0;
    loop {
        let mut cascaded = BTreeSet::new();
        for (collection, rows) in after.iter() {
            for (id, row) in rows {
                let owner = (collection.clone(), id.clone());
                if crate::validation::stored_references(catalog, &owner, row)
                    .into_iter()
                    .any(|reference| {
                        reference.on_delete == semantic_data::schema::OnDelete::Cascade
                            && deleted.contains(&reference.target)
                    })
                {
                    cascaded.insert(owner);
                }
            }
        }
        if cascaded.is_empty() {
            break;
        }
        count += cascaded.len();
        for (collection, id) in &cascaded {
            if let Some(rows) = after.get_mut(collection) {
                rows.remove(id);
            }
        }
        deleted.extend(cascaded);
    }
    count
}

/// What one write-transaction attempt reads against.
#[derive(Clone, Copy)]
pub(super) struct TxScope<'c> {
    /// Catalog snapshot of the attempt.
    pub catalog: &'c Catalog,
    /// Storage revision the attempt reads at; its commit is conditional on
    /// the storage still being at this revision.
    pub revision: Option<u64>,
    pub isolation: IsolationLevel,
}

impl<'c> TxScope<'c> {
    fn new(catalog: &'c Catalog, revision: Option<u64>, isolation: IsolationLevel) -> Self {
        Self {
            catalog,
            revision,
            isolation,
        }
    }

    /// Reader for the attempt's reads, honouring its isolation level.
    fn reader<'a, S: EntityStorage>(
        &self,
        storage: &'a S,
    ) -> Result<RevisionReader<'a, S>, DbError> {
        RevisionReader::for_isolation(storage, self.revision, self.isolation)
    }
}

fn unique_indexes<'c>(
    catalog: &'c Catalog,
    collection: &CollectionSchema,
) -> impl Iterator<Item = &'c crate::catalog::IndexSchema> {
    catalog
        .indexes_for_collection(collection.lid)
        .filter(|index| index.schema.unique && index.schema.kind.is_value_index())
}

/// Check `index` over every row of the collection, reporting the first
/// duplicate in id order.
fn check_unique_index_rows(
    collection: &CollectionSchema,
    index: &crate::catalog::IndexSchema,
    rows: &BTreeMap<String, Object>,
) -> std::result::Result<(), DbError> {
    let mut seen = BTreeMap::<Value, &String>::new();
    for (id, object) in rows {
        let Some(value) = index.key_value(object) else {
            continue;
        };
        if let Some(existing) = seen.get(&value) {
            return Err(compact::unique_violation(
                collection, index, &value, existing, id,
            ));
        }
        seen.insert(value, id);
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
    validate_foreign_keys: bool,
) -> std::result::Result<(), DbError> {
    match &ty.kind {
        TypeKind::Named(reference) => {
            let definition = catalog.type_def_by_name(&reference.name).ok_or_else(|| {
                DbError::InvalidQuery(format!("named type '{}' is not registered", reference.name))
            })?;
            validate_ref_value(
                catalog,
                collection,
                target_rows,
                field,
                &definition.type_def.ty,
                value,
                validate_foreign_keys,
            )
        }
        TypeKind::Ref(_) => validate_one_ref(
            catalog,
            collection,
            target_rows,
            field,
            ty,
            value,
            validate_foreign_keys,
        ),
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
                    validate_foreign_keys,
                )
            }
        }
        TypeKind::Union(union) => {
            if value.is_nullish() && union.variants.iter().any(type_allows_nullish) {
                return Ok(());
            }
            for variant in &union.variants {
                if contains_ref_type(catalog, variant) {
                    return validate_ref_value(
                        catalog,
                        collection,
                        target_rows,
                        field,
                        variant,
                        value,
                        validate_foreign_keys,
                    );
                }
            }
            Ok(())
        }
        TypeKind::List(list) if contains_ref_type(catalog, &list.items) => {
            let Value::List(items) = value else {
                return Err(DbError::InvalidQuery(format!(
                    "ref field '{}' in collection '{}' must be a list of string ids",
                    field, collection.name
                )));
            };
            for item in items {
                validate_ref_value(
                    catalog,
                    collection,
                    target_rows,
                    field,
                    &list.items,
                    item,
                    validate_foreign_keys,
                )?;
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
    validate_foreign_keys: bool,
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
    if !validate_foreign_keys {
        return Ok(());
    }
    let Some(target) = target_rows.get(target_id) else {
        return Err(DbError::ReferenceTargetNotFound {
            collection: collection.name.clone(),
            field: field.into(),
            id: target_id.clone(),
        });
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

fn contains_ref_type(catalog: &Catalog, ty: &Type) -> bool {
    fn visit(catalog: &Catalog, ty: &Type, seen: &mut BTreeSet<String>) -> bool {
        match &ty.kind {
            TypeKind::Named(reference) => {
                seen.insert(reference.name.clone())
                    && catalog
                        .type_def_by_name(&reference.name)
                        .is_some_and(|definition| visit(catalog, &definition.type_def.ty, seen))
            }
            TypeKind::Ref(_) => true,
            TypeKind::Optional(optional) => visit(catalog, &optional.inner, seen),
            TypeKind::Union(union) => union
                .variants
                .iter()
                .any(|variant| visit(catalog, variant, seen)),
            TypeKind::List(list) => visit(catalog, &list.items, seen),
            _ => false,
        }
    }

    visit(catalog, ty, &mut BTreeSet::new())
}

fn type_allows_nullish(ty: &Type) -> bool {
    match &ty.kind {
        TypeKind::Optional(_) | TypeKind::Null(_) => true,
        TypeKind::Union(union) => union.variants.iter().any(type_allows_nullish),
        _ => false,
    }
}

impl crate::AsyncPhysicalDataSource for EmbeddedPhysicalDataSource<'_> {
    fn index_range_stream(
        &self,
        scan: crate::PhysicalIndexScan,
    ) -> crate::SendableRecordBatchStream {
        match self.index_range_scan(&scan) {
            Ok(Some(rows)) => {
                Self::limited_scans_to_stream(vec![rows], scan.residual_predicate, scan.limit_hint)
            }
            Ok(None) => crate::index_range_fallback_stream(self, scan),
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }

    fn text_search_stream(
        &self,
        search: crate::PhysicalTextSearch,
    ) -> crate::SendableRecordBatchStream {
        match self.text_search_scan(&search) {
            Ok(Some(rows)) => Self::limited_scans_to_stream(
                vec![rows],
                search.residual_predicate,
                search.limit_hint,
            ),
            Ok(None) => match search.predicate {
                Some(predicate) => self.scan_filtered_stream(search.source, predicate),
                None => self.scan_stream(search.source),
            },
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }

    fn scan_stream(&self, source: crate::SourceRef) -> crate::SendableRecordBatchStream {
        match self.scan_collections(&source) {
            Ok(scans) => Self::scans_to_stream(scans, None),
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }

    fn scan_filtered_stream(
        &self,
        source: crate::SourceRef,
        predicate: crate::Expr,
    ) -> crate::SendableRecordBatchStream {
        match self.scan_filtered_collections(&source, &predicate) {
            Ok((scans, true)) => Self::scans_to_stream(scans, None),
            Ok((scans, false)) => Self::scans_to_stream(scans, Some(predicate)),
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }

    fn index_lookup_stream(
        &self,
        source: crate::SourceRef,
        field: crate::FieldRef,
        value: Value,
    ) -> crate::SendableRecordBatchStream {
        match self.index_lookup_collections(&source, &field, &value) {
            Ok((scans, predicate)) => Self::scans_to_stream(scans, predicate),
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }

    fn index_lookup_filtered_stream(
        &self,
        source: crate::SourceRef,
        field: crate::FieldRef,
        value: Value,
        residual_predicate: Option<crate::Expr>,
    ) -> crate::SendableRecordBatchStream {
        self.index_lookup_limited_stream(source, field, value, residual_predicate, None)
    }

    fn index_lookup_limited_stream(
        &self,
        source: crate::SourceRef,
        field: crate::FieldRef,
        value: Value,
        residual_predicate: Option<crate::Expr>,
        limit_hint: Option<usize>,
    ) -> crate::SendableRecordBatchStream {
        let result = (|| {
            let (scans, lookup_predicate) =
                self.index_lookup_collections(&source, &field, &value)?;
            let predicate = combine_optional_predicates(lookup_predicate, residual_predicate);
            let Some(predicate) = predicate else {
                return Ok(Self::limited_scans_to_stream(scans, None, limit_hint));
            };
            if !expr_contains_relationship(&predicate) {
                return Ok(Self::limited_scans_to_stream(
                    scans,
                    Some(predicate),
                    limit_hint,
                ));
            }

            let filtered_scans = scans
                .into_iter()
                .map(|scan| self.filter_scan_with_relationships(scan, &predicate))
                .collect::<crate::CoreResult<Vec<_>>>()?;
            Ok(Self::limited_scans_to_stream(
                filtered_scans,
                None,
                limit_hint,
            ))
        })();

        match result {
            Ok(stream) => stream,
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }

    fn index_lookup_many_stream(
        &self,
        source: crate::SourceRef,
        field: crate::FieldRef,
        values: Vec<Value>,
        residual_predicate: Option<crate::Expr>,
    ) -> crate::SendableRecordBatchStream {
        let result = (|| {
            let collection = self
                .resolve_collection(&source)
                .map_err(|err| crate::CoreError::new(err.to_string()))?;
            let Some(field_path) = lookup_field_path(collection, &field) else {
                return Ok(None);
            };
            let Some((index, index_path)) =
                equality_lookup_index(&self.catalog, collection.lid, &field_path)
            else {
                return Ok(None);
            };
            let index_id = index.lid;

            let mut ids = BTreeSet::new();
            for value in &values {
                ids.extend(
                    self.reader
                        .scan_index_value(index_id, index_path.as_ref(), value)
                        .map_err(|err| crate::CoreError::new(err.to_string()))?,
                );
            }
            let scan = self.materialize_ids(collection, ids.into_iter().collect())?;
            let scans = if let Some(predicate) = residual_predicate.clone() {
                if expr_contains_relationship(&predicate) {
                    vec![self.filter_scan_with_relationships(scan, &predicate)?]
                } else {
                    return Ok(Some(Self::scans_to_stream(vec![scan], Some(predicate))));
                }
            } else {
                vec![scan]
            };
            Ok(Some(Self::scans_to_stream(scans, None)))
        })();

        match result {
            Ok(Some(stream)) => stream,
            Ok(None) => stream::select_all(values.into_iter().map(|value| {
                self.index_lookup_filtered_stream(
                    source.clone(),
                    field.clone(),
                    value,
                    residual_predicate.clone(),
                )
            }))
            .boxed(),
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }
}

impl EmbeddedPhysicalDataSource<'_> {
    fn index_lookup_collections(
        &self,
        source: &crate::SourceRef,
        field: &crate::FieldRef,
        value: &Value,
    ) -> crate::CoreResult<(Vec<EmbeddedCollectionScan>, Option<crate::Expr>)> {
        let collection = self
            .resolve_collection(source)
            .map_err(|err| crate::CoreError::new(err.to_string()))?;

        let Some(field_path) = lookup_field_path(collection, field) else {
            return Ok((self.scan_collections(source)?, None));
        };
        if !matches!(field_path.segments().first(), Some(PathSegment::Field(_))) {
            return Ok((self.scan_collections(source)?, None));
        }
        let Some((index, index_path)) =
            equality_lookup_index(&self.catalog, collection.lid, &field_path)
        else {
            let predicate = equality_expr(field_path, value.clone());
            return Ok((self.scan_collections(source)?, Some(predicate)));
        };
        let ids = self
            .reader
            .scan_index_value(index.lid, index_path.as_ref(), value)
            .map_err(|err| crate::CoreError::new(err.to_string()))?;
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

fn is_row_id_expr(expr: &crate::Expr, source_collection: &CollectionSchema) -> bool {
    let crate::Expr::Operand(crate::Operand::Field(path)) = expr else {
        return false;
    };
    let Some(PathSegment::Field(first)) = path.segments().first() else {
        return false;
    };
    path.segments().len() == 1 && source_collection.canonical_field_name(first) == "id"
}

struct QueryStatsSnapshot {
    collections: Vec<CollectionStatsEntry>,
}

struct CollectionStatsEntry {
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
    /// Maintained entry counts of simple equality and range indexes.
    index_entries: Vec<IndexEntryStats>,
    /// Maintained entry counts of all equality and range indexes.
    index_entry_counts: BTreeMap<crate::catalog::LocalIndexId, f64>,
}

/// Entry count of an equality index: the number of rows storing its field.
struct IndexEntryStats {
    canonical_field: String,
    field_id: Option<LocalFieldId>,
    attr_id: Option<LocalAttrId>,
    entries: f64,
}

impl CollectionStatsEntry {
    /// Rows storing `field`, from its equality index entry count when known.
    fn rows_with_field(&self, field: &crate::FieldRef) -> f64 {
        let entries = self.index_entries.iter().find(|index| match field {
            crate::FieldRef::CanonicalName(name) => &index.canonical_field == name,
            crate::FieldRef::FieldId(field_id) => index.field_id == Some(*field_id),
            crate::FieldRef::AttrId(attr_id) => index.attr_id == Some(*attr_id),
            crate::FieldRef::Path(path) => matches!(
                path.segments(),
                [PathSegment::Field(name)] if &index.canonical_field == name
            ),
        });
        entries.map_or(self.row_count, |index| index.entries.min(self.row_count))
    }

    fn null_fraction(&self, rows_with_field: f64) -> f64 {
        if self.row_count > 0.0 {
            1.0 - rows_with_field / self.row_count
        } else {
            0.0
        }
    }
}

impl QueryStatsSnapshot {
    fn collection(&self, source: &crate::SourceRef) -> Option<&CollectionStatsEntry> {
        self.collections.iter().find(|collection| {
            source.source_name.as_deref() == Some(collection.source.as_str())
                || source.collection_id == Some(collection.collection_id)
        })
    }
}

impl crate::StatsProvider for QueryStatsSnapshot {
    fn index_entry_count(
        &self,
        source: &crate::SourceRef,
        index: crate::catalog::LocalIndexId,
    ) -> Option<f64> {
        self.collection(source)?
            .index_entry_counts
            .get(&index)
            .copied()
    }

    fn relation_stats(&self, source: &crate::SourceRef) -> Option<crate::RelationStats> {
        let collection = self.collection(source)?;
        Some(crate::RelationStats {
            row_count: collection.row_count,
        })
    }

    fn field_stats(
        &self,
        source: &crate::SourceRef,
        field: &crate::FieldRef,
    ) -> Option<crate::FieldStats> {
        let collection = self.collection(source)?;

        let unique = match field {
            crate::FieldRef::CanonicalName(name) => collection.unique_fields.contains(name),
            crate::FieldRef::FieldId(field_id) => collection.unique_field_ids.contains(field_id),
            crate::FieldRef::AttrId(attr_id) => collection.unique_attr_ids.contains(attr_id),
            crate::FieldRef::Path(path) => path
                .segments()
                .first()
                .and_then(|segment| match segment {
                    semantic_data::value::PathSegment::Field(name) => Some(name),
                    _ => None,
                })
                .is_some_and(|name| collection.unique_fields.contains(name)),
        };
        let rows_with_field = collection.rows_with_field(field);
        if unique {
            return Some(crate::FieldStats {
                distinct_count: Some(rows_with_field.max(1.0)),
                null_fraction: Some(collection.null_fraction(rows_with_field)),
            });
        }

        let indexed = match field {
            crate::FieldRef::CanonicalName(name) => {
                collection.indexed_fields.contains(name) || collection.has_path_equality_index
            }
            crate::FieldRef::FieldId(field_id) => {
                collection.indexed_field_ids.contains(field_id)
                    || collection.has_path_equality_index
            }
            crate::FieldRef::AttrId(attr_id) => {
                collection.indexed_attr_ids.contains(attr_id) || collection.has_path_equality_index
            }
            crate::FieldRef::Path(path) => {
                path.segments()
                    .first()
                    .and_then(|segment| match segment {
                        semantic_data::value::PathSegment::Field(name) => Some(name),
                        _ => None,
                    })
                    .is_some_and(|name| collection.indexed_fields.contains(name))
                    || collection.has_path_equality_index
            }
        };
        if indexed {
            // Without distinct-value statistics, assume eight rows per value.
            return Some(crate::FieldStats {
                distinct_count: Some((rows_with_field / 8.0).max(1.0)),
                null_fraction: Some(collection.null_fraction(rows_with_field)),
            });
        }

        None
    }

    fn has_equality_index(
        &self,
        source: &crate::SourceRef,
        field: &crate::FieldRef,
    ) -> Option<bool> {
        let collection = self.collection(source)?;
        let indexed = match field {
            crate::FieldRef::CanonicalName(name) => {
                collection.indexed_fields.contains(name) || collection.has_path_equality_index
            }
            crate::FieldRef::FieldId(field_id) => {
                collection.indexed_field_ids.contains(field_id)
                    || collection.has_path_equality_index
            }
            crate::FieldRef::AttrId(attr_id) => {
                collection.indexed_attr_ids.contains(attr_id) || collection.has_path_equality_index
            }
            crate::FieldRef::Path(path) => {
                path.segments()
                    .first()
                    .and_then(|segment| match segment {
                        semantic_data::value::PathSegment::Field(name) => Some(name),
                        _ => None,
                    })
                    .is_some_and(|name| collection.indexed_fields.contains(name))
                    || collection.has_path_equality_index
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
    use std::sync::Arc;

    use semantic_data::{
        expr::{CallExpr, Callee, Expr as SchemaExpr},
        schema::{
            Constraint, Migration, MigrationDdlOperation, MigrationOperation, Module, Package,
            attribute::attribute_ref::AttributeRef,
            attribute::attribute_type::AttributeType,
            class::class_attribute::ClassAttribute,
            class::class_ref::ClassRef,
            class::class_type::ClassType,
            collections::optional_type::OptionalType,
            core::{meta::Meta, type_kind::TypeKind, type_node::Type},
            primitives::{
                bool_type::BoolType, number_type::NumberType, string_type::StringType,
                temporal_type::TemporalType, uint_width::UIntWidth,
            },
            record::field::Field,
            record::record_type::RecordType,
        },
        value::{DateTime, FieldPath, Object, PathSegment, Value},
    };

    use crate::catalog::{CollectionKind, IntegrityMode};
    use crate::{
        ALL_COLLECTION_ALIAS, Batch, BatchOperation, CORE_CATALOG_SCHEMA_COLLECTION,
        DEFAULT_COLLECTION, DdlBatch, DdlCollectionKind, DdlOperation, Expr, JoinCondition,
        JoinQuery, JoinSource, Operand, OrderBy, Query, QueryField, QueryResult, SelectQuery,
        TransactionConcurrency, TransactionOptions, UpdateQuery, canonicalize_select_query,
    };
    use crate::{DbConfig, DbError, MigrationMismatchPolicy};
    use semantic_data::query::{BinaryOp, FieldFormat, JoinType, SortDirection};

    use super::{
        EmbeddedDb, EmbeddedPhysicalDataSource, LocalRefResolver, QueryPlan, QueryReader,
        RELATION_EDGES_COLLECTION,
    };
    use crate::embedded::storage::{CountingEntityStorage, EntityStorage};

    fn physical_source<S: EntityStorage>(db: &EmbeddedDb<S>) -> EmbeddedPhysicalDataSource<'_> {
        EmbeddedPhysicalDataSource {
            reader: QueryReader::Borrowed(db.storage.snapshot().unwrap()),
            catalog: db.catalog(),
            default_collection: None,
            local_refs: None,
        }
    }

    /// A data source resolving local references along `paths`, lazily
    /// through an owned snapshot or by prefetching through a borrowed one.
    fn physical_source_with_local_refs<'a, S: EntityStorage>(
        db: &'a EmbeddedDb<S>,
        paths: &[FieldPath],
        lazy: bool,
    ) -> EmbeddedPhysicalDataSource<'a> {
        let reader = if lazy {
            QueryReader::Shared(db.storage.owned_snapshot().unwrap().unwrap())
        } else {
            QueryReader::Borrowed(db.storage.snapshot().unwrap())
        };
        let local_refs = LocalRefResolver::new(paths.iter().cloned(), reader.shared());
        EmbeddedPhysicalDataSource {
            reader,
            catalog: db.catalog(),
            default_collection: None,
            local_refs,
        }
    }

    fn string_object(fields: &[(&str, &str)]) -> Object {
        let mut object = Object::new();
        for (field, value) in fields {
            object.insert(*field, Value::String((*value).to_string()));
        }
        object
    }

    fn field_projection(path: &[&str], alias: &str) -> QueryField {
        QueryField {
            expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(
                path.iter().copied(),
            )))),
            alias: Some(alias.to_string()),
            wildcard: None,
        }
    }

    fn relation_with_max_depth(max_depth: Option<Expr>) -> Expr {
        Expr::RelationExists {
            relation: Box::new(Expr::Operand(Operand::Literal(Value::String(
                "test.parent".to_string(),
            )))),
            source: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                "id",
            ])))),
            target: Box::new(Expr::Operand(Operand::Literal(Value::String(
                "root".to_string(),
            )))),
            transitive: true,
            max_depth: max_depth.map(Box::new),
        }
    }

    #[test]
    fn indexed_materialization_resolves_local_refs_by_memoized_point_reads() {
        let (storage, counts) = CountingEntityStorage::new();
        let mut db = EmbeddedDb::new(storage);
        db.create_collection("cache_items", CollectionKind::Polymorphic)
            .unwrap();
        db.insert(
            "cache_items",
            "a",
            string_object(&[("id", "a"), ("title", "first")]),
        )
        .unwrap();
        for id in ["b", "c"] {
            db.insert(
                "cache_items",
                id,
                string_object(&[("id", id), ("parent", "a")]),
            )
            .unwrap();
        }
        let catalog = db.catalog();
        let collection = catalog.collection_by_name("cache_items").unwrap();
        let title = FieldPath::from_fields(["parent", "title"]);
        for lazy in [true, false] {
            let source = physical_source_with_local_refs(&db, std::slice::from_ref(&title), lazy);
            counts.reset();

            let first = source
                .materialize_ids(collection, vec!["b".to_string()])
                .unwrap();
            let second = source
                .materialize_ids(collection, vec!["c".to_string()])
                .unwrap();

            for scan in [first, second] {
                let views = scan.rows.collect::<Result<Vec<_>, _>>().unwrap();
                assert_eq!(views.len(), 1);
                for _ in 0..2 {
                    assert_eq!(
                        crate::ObjectAccess::value_at_path_ref(&views[0], &title)
                            .map(|value| value.into_owned()),
                        Some(Value::String("first".to_string()))
                    );
                }
            }
            assert_eq!(counts.collection_scans(), 0, "lazy={lazy}");
            // Two materialized rows plus one memoized read of the shared parent.
            assert_eq!(counts.entity_gets(), 3, "lazy={lazy}");
        }
    }

    #[test]
    fn limited_scan_with_nested_object_path_streams_without_point_reads() {
        let (storage, counts) = CountingEntityStorage::new();
        let mut db = EmbeddedDb::new(storage);
        db.create_collection("people", CollectionKind::Polymorphic)
            .unwrap();
        // Scans produce batches of `DEFAULT_EXECUTION_BATCH_SIZE` rows, so the
        // collection is larger than one batch.
        let total = 3 * crate::DEFAULT_EXECUTION_BATCH_SIZE;
        let mut batch = Batch::new();
        for index in 0..total {
            let id = format!("person-{index:05}");
            let mut object = string_object(&[("id", &id)]);
            object.insert(
                "address",
                Value::Object(string_object(&[("city", &format!("city-{index}"))])),
            );
            batch = batch.with_op(BatchOperation::Upsert {
                collection: "people".to_string(),
                id,
                object,
            });
        }
        db.transact(batch).unwrap();

        counts.reset();
        let rows = db
            .select(
                SelectQuery::new()
                    .with_collection("people")
                    .with_projection(vec![field_projection(&["address", "city"], "city")])
                    .with_limit(10usize),
            )
            .unwrap();
        assert_eq!(rows.len(), 10);
        assert!(rows.iter().all(|row| row.get("city").is_some()));
        assert_eq!(counts.collection_scans(), 1);
        assert_eq!(counts.entity_gets(), 0);
        assert!(
            counts.rows_yielded() < total,
            "the scan must stop early, yielded {}",
            counts.rows_yielded()
        );
    }

    #[test]
    fn limit_reads_one_batch_and_top_n_retains_only_the_bound() {
        let (storage, counts) = CountingEntityStorage::new();
        let mut db = EmbeddedDb::new(storage);
        db.create_collection("people", CollectionKind::Polymorphic)
            .unwrap();
        let total = 5_000;
        let mut batch = Batch::new();
        for index in 0..total {
            let id = format!("person-{index:05}");
            let mut object = string_object(&[("id", &id)]);
            // A permutation of 0..total, so the scan order is not the rank order.
            object.insert("rank", Value::I64((index * 7_919 % total) as i64));
            batch = batch.with_op(BatchOperation::Upsert {
                collection: "people".to_string(),
                id,
                object,
            });
        }
        db.transact(batch).unwrap();
        let rank = || vec![field_projection(&["rank"], "rank")];

        counts.reset();
        let rows = db
            .select(
                SelectQuery::new()
                    .with_collection("people")
                    .with_projection(rank())
                    .with_limit(10usize),
            )
            .unwrap();
        assert_eq!(rows.len(), 10);
        assert_eq!(counts.collection_scans(), 1);
        assert!(
            counts.rows_yielded() <= crate::DEFAULT_EXECUTION_BATCH_SIZE,
            "LIMIT without ORDER BY must read at most one batch, read {}",
            counts.rows_yielded()
        );

        let sorted = SelectQuery::new()
            .with_collection("people")
            .with_projection(rank())
            .with_order_by(vec![OrderBy {
                expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["rank"]))),
                direction: SortDirection::Asc,
            }])
            .with_offset(5usize)
            .with_limit(10usize);
        let explain = db.explain_query(Query::Select(sorted.clone())).unwrap();
        let crate::PhysicalPlan::Project { input, .. } = &explain.physical else {
            panic!("expected a projected TopN, got {:?}", explain.physical);
        };
        assert!(
            matches!(input.as_ref(), crate::PhysicalPlan::TopN { .. }),
            "{:?}",
            explain.physical
        );

        counts.reset();
        crate::take_top_n_peak_rows();
        let rows = db.select(sorted).unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.get("rank").cloned().unwrap())
                .collect::<Vec<_>>(),
            (5..15).map(Value::I64).collect::<Vec<_>>()
        );
        assert_eq!(counts.rows_yielded(), total);
        let peak = crate::take_top_n_peak_rows();
        assert_eq!(peak, 15, "TopN must retain exactly offset + limit rows");
    }

    #[test]
    fn ref_paths_resolve_lazily_with_one_point_read_per_distinct_target() {
        let (storage, counts) = CountingEntityStorage::new();
        let mut db = EmbeddedDb::new(storage);
        register_ref_schema(&mut db, ref_ty("person"));
        let mut batch = Batch::new();
        for person in 0..3 {
            let id = format!("person-{person}");
            batch = batch.with_op(BatchOperation::Upsert {
                collection: DEFAULT_COLLECTION.to_string(),
                object: entity(
                    &id,
                    "local:person",
                    [("name", Value::String(format!("Person {person}")))],
                ),
                id,
            });
        }
        db.transact(batch).unwrap();
        let mut batch = Batch::new();
        for article in 0..12 {
            let id = format!("article-{article:02}");
            batch = batch.with_op(BatchOperation::Upsert {
                collection: DEFAULT_COLLECTION.to_string(),
                object: entity(
                    &id,
                    "local:article",
                    [("author", Value::String(format!("person-{}", article % 3)))],
                ),
                id,
            });
        }
        db.transact(batch).unwrap();

        // Selects lift typed ref paths into index joins; this exercises the
        // data source's own resolution of a `Ref`-declared first segment.
        let catalog = db.catalog();
        let collection = catalog.collection_by_name(DEFAULT_COLLECTION).unwrap();
        let author = collection.canonical_field_name("author").to_string();
        let path = FieldPath::from_fields([author.as_str(), "name"]);
        let source = physical_source_with_local_refs(&db, std::slice::from_ref(&path), true);
        counts.reset();
        let views = source
            .collection_scan(collection)
            .unwrap()
            .rows
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(counts.entity_gets(), 0, "references resolve on demand");
        let names = views
            .iter()
            .filter_map(|view| {
                let id = view.object.get("id")?.as_str()?.to_string();
                let name = crate::ObjectAccess::value_at_path_ref(view, &path)?.into_owned();
                Some((id, name))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(names.len(), 12);
        assert_eq!(names["article-04"], Value::String("Person 1".to_string()));
        assert_eq!(counts.collection_scans(), 1);
        assert_eq!(counts.entity_gets(), 3);
    }

    #[test]
    fn typed_ref_paths_join_targets_stored_with_alias_or_subclass_types() {
        let mut db = EmbeddedDb::in_memory();
        register_ref_schema(&mut db, ref_ty("person"));
        let upsert = |id: &str, object: Object| BatchOperation::Upsert {
            collection: DEFAULT_COLLECTION.to_string(),
            id: id.to_string(),
            object,
        };
        let name = |value: &str| ("name", Value::String(value.to_string()));
        let author = |id: &str| ("author", Value::String(id.to_string()));
        db.transact(
            Batch::new()
                // Short class names are stored as written.
                .with_op(upsert(
                    "p-alias",
                    entity("p-alias", "person", [name("Alias")]),
                ))
                .with_op(upsert(
                    "p-canonical",
                    entity("p-canonical", "local:person", [name("Canonical")]),
                ))
                .with_op(upsert(
                    "p-employee",
                    entity("p-employee", "employee", [name("Employee")]),
                ))
                .with_op(upsert(
                    "a-alias",
                    entity("a-alias", "article", [author("p-alias")]),
                ))
                .with_op(upsert(
                    "a-canonical",
                    entity("a-canonical", "article", [author("p-canonical")]),
                ))
                .with_op(upsert(
                    "a-employee",
                    entity("a-employee", "article", [author("p-employee")]),
                )),
        )
        .unwrap();
        let stored = db.get(DEFAULT_COLLECTION, "p-alias").unwrap().unwrap();
        assert_eq!(
            stored.object.get("type"),
            Some(&Value::String("person".into()))
        );

        let rows = db
            .select(
                SelectQuery::new()
                    .with_collection(DEFAULT_COLLECTION)
                    .with_predicate(eq_predicate(
                        FieldPath::from_fields(["type"]),
                        Value::String("article".into()),
                    ))
                    .with_projection(vec![
                        field_projection(&["id"], "id"),
                        field_projection(&["author", "name"], "author_name"),
                    ]),
            )
            .unwrap();
        let names = rows
            .iter()
            .map(|row| {
                (
                    row.get("id").and_then(Value::as_str).unwrap().to_string(),
                    row.get("author_name").cloned(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let expected = |name: &str| Some(Value::String(name.to_string()));
        assert_eq!(
            names,
            BTreeMap::from([
                ("a-alias".to_string(), expected("Alias")),
                ("a-canonical".to_string(), expected("Canonical")),
                ("a-employee".to_string(), expected("Employee")),
            ])
        );
    }

    #[test]
    fn indexed_select_reads_matches_only_and_full_scan_reads_collection_once() {
        let (storage, counts) = CountingEntityStorage::new();
        let mut db = EmbeddedDb::new(storage);
        let lid = db
            .create_collection("bulk_items", CollectionKind::Polymorphic)
            .unwrap();
        db.create_index("bulk_items_by_kind", lid, "kind", false)
            .unwrap();
        let mut batch = Batch::new();
        for index in 0..1000 {
            let id = format!("item-{index:04}");
            let kind = if index % 250 == 0 { "rare" } else { "common" };
            let linked = format!("item-{:04}", (index + 1) % 1000);
            batch = batch.with_op(BatchOperation::Upsert {
                collection: "bulk_items".to_string(),
                object: string_object(&[("id", &id), ("kind", kind), ("linked", &linked)]),
                id,
            });
        }
        db.transact(batch).unwrap();
        let rare = || {
            SelectQuery::new()
                .with_collection("bulk_items")
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["kind"]),
                    Value::String("rare".to_string()),
                ))
        };

        counts.reset();
        let rows = db
            .select(rare().with_projection(vec![field_projection(&["id"], "id")]))
            .unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(counts.collection_scans(), 0);
        assert_eq!(counts.entity_gets(), 4);
        // Planner statistics read maintained row counts.
        assert_eq!(counts.collection_counts(), 0);

        // Following a local reference adds one point read per match.
        counts.reset();
        let rows = db
            .select(
                rare().with_projection(vec![field_projection(&["linked", "kind"], "linked_kind")]),
            )
            .unwrap();
        assert_eq!(rows.len(), 4);
        assert!(
            rows.iter()
                .all(|row| row.get("linked_kind") == Some(&Value::String("common".to_string())))
        );
        assert_eq!(counts.collection_scans(), 0);
        assert_eq!(counts.entity_gets(), 8);

        counts.reset();
        let rows = db
            .select(SelectQuery::new().with_collection("bulk_items"))
            .unwrap();
        assert_eq!(rows.len(), 1000);
        assert_eq!(counts.collection_scans(), 1);
        assert_eq!(counts.entity_gets(), 0);

        counts.reset();
        let rows = db
            .select(
                SelectQuery::new()
                    .with_collection("bulk_items")
                    .with_predicate(eq_predicate(
                        FieldPath::from_fields(["linked", "kind"]),
                        Value::String("rare".to_string()),
                    )),
            )
            .unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(counts.collection_scans(), 1);
    }

    fn count_star_query(alias: Option<&str>) -> SelectQuery {
        SelectQuery::new()
            .with_collection("counted_items")
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Aggregate {
                    op: semantic_data::query::AggregateOp::Count,
                    distinct: false,
                    arg: Box::new(crate::FunctionArg::Wildcard),
                }),
                alias: alias.map(ToString::to_string),
                wildcard: None,
            }])
    }

    #[test]
    fn count_star_is_answered_without_scanning() {
        let (storage, counts) = CountingEntityStorage::new();
        let mut db = EmbeddedDb::new(storage);
        db.create_collection("counted_items", CollectionKind::Polymorphic)
            .unwrap();
        let mut batch = Batch::new();
        for index in 0..300 {
            let id = format!("item-{index:03}");
            let kind = if index % 3 == 0 { "third" } else { "other" };
            batch = batch.with_op(BatchOperation::Upsert {
                collection: "counted_items".to_string(),
                object: string_object(&[("id", &id), ("kind", kind)]),
                id,
            });
        }
        db.transact(batch).unwrap();

        counts.reset();
        let rows = db.select(count_star_query(Some("n"))).unwrap();
        assert_eq!(
            rows,
            vec![Object::from_iter([("n".to_string(), Value::I64(300))])]
        );
        assert_eq!(counts.collection_scans(), 0);
        assert_eq!(
            counts.collection_counts(),
            0,
            "maintained counts need no key counting"
        );

        // The unaliased output column matches the regular aggregate.
        let explain = db
            .explain_query(Query::Select(count_star_query(None)))
            .unwrap();
        let expected = db
            .execute_physical_plan(&explain.physical, Some("counted_items"))
            .unwrap();
        counts.reset();
        assert_eq!(db.select(count_star_query(None)).unwrap(), expected);
        assert_eq!(counts.collection_scans(), 0);

        // Without maintained counts the storage keys are counted instead.
        counts
            .hide_row_counts
            .store(true, std::sync::atomic::Ordering::Relaxed);
        counts.reset();
        let rows = db.select(count_star_query(Some("n"))).unwrap();
        assert_eq!(rows[0].get("n"), Some(&Value::I64(300)));
        assert_eq!(counts.collection_scans(), 0);
        assert!(counts.collection_counts() > 0);

        // Filtered counts still read rows.
        counts.reset();
        let rows = db
            .select(count_star_query(Some("n")).with_predicate(eq_predicate(
                FieldPath::from_fields(["kind"]),
                Value::String("third".to_string()),
            )))
            .unwrap();
        assert_eq!(rows[0].get("n"), Some(&Value::I64(100)));
        assert!(counts.collection_scans() + counts.entity_gets() > 0);
    }

    #[test]
    fn relation_lookup_only_accepts_constant_nonnegative_max_depth() {
        let mut db = EmbeddedDb::in_memory();
        db.create_collection("relation_nodes", CollectionKind::Polymorphic)
            .unwrap();
        let catalog = db.catalog();
        let collection = catalog.collection_by_name("relation_nodes").unwrap();
        let source = physical_source(&db);

        for invalid in [
            Expr::Operand(Operand::Field(FieldPath::from_fields(["depth"]))),
            Expr::Operand(Operand::Literal(Value::Null)),
            Expr::Operand(Operand::Literal(Value::String("1".to_string()))),
            Expr::Operand(Operand::Literal(Value::I64(-1))),
        ] {
            assert_eq!(
                source
                    .try_exact_relation_lookup_ids(
                        collection,
                        &relation_with_max_depth(Some(invalid)),
                    )
                    .unwrap(),
                None
            );
        }

        assert!(
            source
                .try_exact_relation_lookup_ids(
                    collection,
                    &relation_with_max_depth(Some(Expr::Operand(Operand::Literal(Value::U64(1,))))),
                )
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn initialization_creates_default_entities_collection() {
        let db = EmbeddedDb::in_memory();
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
    fn external_relation_indexes_respect_discriminators_in_shared_collections() {
        use semantic_data::attr::{
            ATTR_RELATION_FROM, ATTR_RELATION_RELATION, ATTR_RELATION_TO, RELATION_CLASS_ID,
        };
        use semantic_data::schema::{RelationIndexingMode, RelationMode, RelationType};

        let mut db = EmbeddedDb::in_memory();
        for id in ["first", "second"] {
            db.upsert_relationship(RelationType {
                id: id.into(),
                name: id.into(),
                source_collection: DEFAULT_COLLECTION.into(),
                mode: RelationMode::External,
                indexing_mode: RelationIndexingMode::Enabled,
                meta: Meta::default(),
            })
            .unwrap();
        }
        for relation_id in ["canonical", "aliases", "legacy"] {
            for suffix in ["source", "target"] {
                let id = format!("{relation_id}-{suffix}");
                let mut endpoint = Object::new();
                endpoint.insert("id", Value::String(id.clone()));
                db.insert(DEFAULT_COLLECTION, &id, endpoint).unwrap();
            }
        }
        // Untyped canonical attributes, typed aliases, and legacy rows all share
        // the collection. Distinct endpoints also detect accidental path mixing.
        for (id, relation, typed) in [
            ("canonical", Some("first"), false),
            ("aliases", Some("second"), true),
            ("legacy", None, false),
        ] {
            let mut object = Object::new();
            object.insert("id", Value::String(id.into()));
            if typed {
                object.insert("type", Value::String(RELATION_CLASS_ID.into()));
            }
            object.insert(
                if typed { "from" } else { ATTR_RELATION_FROM },
                Value::String(format!("{id}-source")),
            );
            object.insert(
                if typed { "to" } else { ATTR_RELATION_TO },
                Value::String(format!("{id}-target")),
            );
            if let Some(relation) = relation {
                object.insert(
                    if typed {
                        "relation"
                    } else {
                        ATTR_RELATION_RELATION
                    },
                    Value::String(relation.into()),
                );
            }
            db.insert(DEFAULT_COLLECTION, id, object).unwrap();
        }
        let edges = db
            .select(SelectQuery::new().with_collection(RELATION_EDGES_COLLECTION))
            .unwrap();
        let actual: std::collections::BTreeSet<_> = edges
            .iter()
            .map(|edge| {
                (
                    edge.get("relation").unwrap().as_str().unwrap().to_string(),
                    edge.get("source").unwrap().as_str().unwrap().to_string(),
                    edge.get("target").unwrap().as_str().unwrap().to_string(),
                )
            })
            .collect();
        let expected = [
            ("first", "canonical"),
            ("second", "aliases"),
            ("first", "legacy"),
            ("second", "legacy"),
        ]
        .into_iter()
        .map(|(relation, id)| {
            (
                relation.to_string(),
                format!("{id}-source"),
                format!("{id}-target"),
            )
        })
        .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn directory_shaped_self_join_uses_indexed_probe_and_returns_order_rows() {
        let mut db = EmbeddedDb::in_memory();
        db.create_collection("tree_entities", CollectionKind::Polymorphic)
            .unwrap();
        for (id, kind, from, to, order) in [
            ("dir-a", "directory", None, None, None),
            ("dir-b", "directory", None, None, None),
            ("file-a", "file", None, None, None),
            ("node-a", "node", Some("root"), Some("dir-a"), Some(2_u64)),
            ("node-b", "node", Some("root"), Some("dir-b"), Some(1_u64)),
            ("node-c", "node", Some("root"), Some("file-a"), Some(3_u64)),
            ("node-d", "node", Some("root"), Some("dir-a"), Some(4_u64)),
            (
                "other-parent",
                "node",
                Some("elsewhere"),
                Some("dir-b"),
                Some(0_u64),
            ),
            (
                "missing-target",
                "node",
                Some("root"),
                Some("does-not-exist"),
                Some(5_u64),
            ),
        ] {
            let mut object = Object::new();
            object.insert("id", Value::String(id.to_string()));
            object.insert("kind", Value::String(kind.to_string()));
            if let Some(from) = from {
                object.insert("parent_id", Value::String(from.to_string()));
            }
            if let Some(to) = to {
                object.insert("child_id", Value::String(to.to_string()));
            }
            if let Some(order) = order {
                object.insert("order", Value::U64(order));
            }
            db.insert("tree_entities", id, object).unwrap();
        }

        let field = |path| Expr::Operand(Operand::Field(FieldPath::from_fields(path)));
        let query = SelectQuery::new()
            .with_collection("tree_entities")
            .with_source_alias("n")
            .with_joins(vec![JoinQuery {
                source: JoinSource {
                    collection: Some("tree_entities".to_string()),
                    class: None,
                },
                alias: Some("child".to_string()),
                join_type: JoinType::Inner,
                condition: JoinCondition::OnExpr(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(field(["n", "child_id"])),
                    right: Box::new(field(["child", "id"])),
                }),
                predicate: None,
            }])
            .with_predicate(Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(field(["n", "parent_id"])),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "root".to_string(),
                    )))),
                }),
                right: Box::new(Expr::InList {
                    expr: Box::new(field(["child", "kind"])),
                    list: vec![Expr::Operand(Operand::Literal(Value::String(
                        "directory".to_string(),
                    )))],
                    negated: false,
                }),
            });

        let explain = db.explain_query(Query::Select(query.clone())).unwrap();
        let crate::PhysicalPlan::Join(join) = explain.physical else {
            panic!("expected physical join")
        };
        assert_eq!(
            join.algorithm,
            crate::PhysicalJoinAlgorithm::IndexNestedLoop
        );
        assert!(join.index_probe.is_some());

        let query = query
            .with_projection(vec![
                QueryField {
                    expr: Box::new(field(["n", "id"])),
                    alias: Some("node_id".to_string()),
                    wildcard: None,
                },
                QueryField {
                    expr: Box::new(field(["child", "id"])),
                    alias: Some("child_id".to_string()),
                    wildcard: None,
                },
                QueryField {
                    expr: Box::new(field(["n", "order"])),
                    alias: Some("order".to_string()),
                    wildcard: None,
                },
            ])
            .with_order_by(vec![OrderBy {
                expr: field(["n", "order"]),
                direction: SortDirection::Asc,
            }])
            .with_offset(1)
            .with_limit(2);
        let rows = db.select(query).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].get("node_id"),
            Some(&Value::String("node-a".into()))
        );
        assert_eq!(
            rows[0].get("child_id"),
            Some(&Value::String("dir-a".into()))
        );
        assert_eq!(rows[0].get("order"), Some(&Value::U64(2)));
        assert_eq!(
            rows[1].get("node_id"),
            Some(&Value::String("node-d".into()))
        );
        assert_eq!(
            rows[1].get("child_id"),
            Some(&Value::String("dir-a".into()))
        );
        assert_eq!(rows[1].get("order"), Some(&Value::U64(4)));
    }

    #[test]
    fn cross_collection_join_uses_right_collection_index_stats() {
        let mut db = EmbeddedDb::in_memory();
        db.create_collection("orders", CollectionKind::Polymorphic)
            .unwrap();
        db.create_collection("customers", CollectionKind::Polymorphic)
            .unwrap();

        let mut order = Object::new();
        order.insert("id", Value::String("order-1".to_string()));
        order.insert("customer_id", Value::String("customer-1".to_string()));
        db.insert("orders", "order-1", order).unwrap();
        for index in 0..8 {
            let id = format!("customer-{index}");
            let mut customer = Object::new();
            customer.insert("id", Value::String(id.clone()));
            db.insert("customers", id, customer).unwrap();
        }

        let field = |path| Expr::Operand(Operand::Field(FieldPath::from_fields(path)));
        let query = SelectQuery::new()
            .with_collection("orders")
            .with_source_alias("o")
            .with_joins(vec![JoinQuery {
                source: JoinSource {
                    collection: Some("customers".to_string()),
                    class: None,
                },
                alias: Some("c".to_string()),
                join_type: JoinType::Inner,
                condition: JoinCondition::OnExpr(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(field(["o", "customer_id"])),
                    right: Box::new(field(["c", "id"])),
                }),
                predicate: None,
            }]);

        let explain = db.explain_query(Query::Select(query)).unwrap();
        let crate::PhysicalPlan::Join(join) = explain.physical else {
            panic!("expected physical join")
        };
        assert_eq!(
            join.algorithm,
            crate::PhysicalJoinAlgorithm::IndexNestedLoop
        );
        assert_eq!(
            join.index_probe
                .as_ref()
                .and_then(|probe| probe.source.source_name.as_deref()),
            Some("customers")
        );
    }

    #[test]
    fn initialization_marks_kv_internal_collections() {
        let db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();
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
        let db = EmbeddedDb::in_memory();
        let rows = db
            .select(SelectQuery::new().with_collection(CORE_CATALOG_SCHEMA_COLLECTION))
            .expect("schema collection should remain queryable");

        assert!(!rows.is_empty());
    }

    #[test]
    fn initialization_enables_auto_indexing() {
        let db = EmbeddedDb::in_memory();
        assert!(db.auto_index_enabled());
    }

    #[test]
    fn package_metadata_changes_are_persisted_without_new_migrations() {
        let mut db = EmbeddedDb::in_memory();
        let mut package = simple_schema_package("Original migration.");
        db.upsert_package(package.clone()).unwrap();
        package.meta.description = Some("Updated package metadata.".into());
        assert!(
            db.upsert_package(package.clone())
                .unwrap()
                .executed_migrations
                .is_empty()
        );

        let (_, storage) = db.into_parts();
        let reopened = EmbeddedDb::open(storage).unwrap();
        assert_eq!(
            reopened
                .catalog()
                .package_by_name(&package.name)
                .unwrap()
                .meta
                .description,
            package.meta.description,
        );
    }

    #[test]
    fn unchanged_package_still_reconciles_missing_schema() {
        let mut db = EmbeddedDb::in_memory();
        let package = simple_schema_package("Original migration.");
        db.upsert_package(package.clone()).unwrap();
        db.transact_ddl(DdlBatch::new().with_op(DdlOperation::DeleteClass {
            id: "shared.test.note".into(),
        }))
        .unwrap();
        assert!(db.catalog().class_id("shared.test.note").is_none());

        let outcome = db.upsert_package(package).unwrap();
        assert!(outcome.executed_migrations.is_empty());
        assert!(db.catalog().class_id("shared.test.note").is_some());
    }

    #[test]
    fn applied_migration_mismatch_fails_by_default() {
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory_with_config(DbConfig {
            migration_mismatch_policy: MigrationMismatchPolicy::Log,
            ..DbConfig::default()
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
        let mut db = EmbeddedDb::in_memory();

        let result = db
            .query(Query::Ddl(crate::DdlQuery {
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
        let mut db = EmbeddedDb::in_memory();
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
                wildcard: None,
            }]);
        let catalog = db.catalog();
        let collection = catalog.collection_by_name("ref_paths").unwrap();
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
            &mut |id| {
                db.get("ref_paths", id)
                    .unwrap()
                    .map(|entity| Arc::new(entity.object))
            },
        );
        assert_eq!(
            ids,
            vec!["ref-child".to_string()],
            "canonical={:?}, explain={:?}, resolved={:?}, parent={:?}, child={:?}",
            canonical,
            explain,
            resolved,
            stored_parent.object,
            stored_child.object
        );
        assert_eq!(resolved, Some(Value::String("abc".to_string())));
    }

    #[test]
    fn ref_field_rejects_missing_target() {
        let mut db = EmbeddedDb::in_memory();
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

        assert!(matches!(
            err,
            crate::DbError::ReferenceTargetNotFound { .. }
        ));
        assert!(
            err.to_string().contains("missing target id 'person-99'"),
            "{err}"
        );
    }

    #[test]
    fn ref_field_accepts_existing_target() {
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();
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

        assert!(matches!(err, crate::DbError::InvalidQuery(_)));
        assert!(err.to_string().contains("local:person"), "{err}");
    }

    #[test]
    fn ref_field_accepts_subclass_target() {
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();
        register_ref_schema(&mut db, ref_ty("person"));

        let err = db
            .insert(
                DEFAULT_COLLECTION,
                "article-1",
                entity("article-1", "article", [("author", Value::I64(42))]),
            )
            .expect_err("non-string ref should be rejected");

        assert!(matches!(err, crate::DbError::InvalidQuery(_)));
        assert!(err.to_string().contains("must be a string id"), "{err}");
    }

    #[test]
    fn untyped_query_works() {
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();

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
            strict_schema: false,
            creatable_in_ui: None,
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
            .storage
            .get_entity(collection.lid, "art-1")
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.kind,
            crate::embedded::storage::StoredEntityKind::Class
        );
    }

    #[test]
    fn default_expressions_apply_to_unset_fields_on_insert_and_update() {
        let mut db = EmbeddedDb::in_memory();
        db.transact_ddl(
            DdlBatch::new()
                .with_op(DdlOperation::UpsertAttribute {
                    attribute: AttributeType {
                        id: "test:timestamp".to_string(),
                        name: "timestamp".to_string(),
                        ty: ty(TypeKind::Optional(OptionalType {
                            inner: Box::new(ty(TypeKind::Temporal(TemporalType::DateTime))),
                        })),
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                })
                .with_op(DdlOperation::UpsertAttribute {
                    attribute: AttributeType {
                        id: "test:title".to_string(),
                        name: "title".to_string(),
                        ty: ty(TypeKind::String(StringType {
                            format: None,
                            normalization: None,
                        })),
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                })
                .with_op(DdlOperation::UpsertClass {
                    class: default_expression_test_class(false),
                }),
        )
        .unwrap();

        db.insert(
            DEFAULT_COLLECTION,
            "legacy",
            entity(
                "legacy",
                "test:default_expression",
                [("title", Value::String("before".to_string()))],
            ),
        )
        .unwrap();
        assert!(
            !db.get(DEFAULT_COLLECTION, "legacy")
                .unwrap()
                .unwrap()
                .object
                .contains_key("test:timestamp")
        );

        db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertClass {
            class: default_expression_test_class(true),
        }))
        .unwrap();

        let update_title = |db: &mut EmbeddedDb<_>, id: &str, title: &str| {
            db.update_where(
                UpdateQuery::new()
                    .with_collection(DEFAULT_COLLECTION)
                    .with_predicate(eq_predicate(
                        FieldPath::from_fields(["id"]),
                        Value::String(id.to_string()),
                    ))
                    .set(
                        FieldPath::from_fields(["test:title"]),
                        Expr::Operand(Operand::Literal(Value::String(title.to_string()))),
                    ),
            )
            .unwrap()
        };

        let stats = update_title(&mut db, "legacy", "before");
        assert_eq!(stats.affected, 1);
        assert!(matches!(
            db.get(DEFAULT_COLLECTION, "legacy")
                .unwrap()
                .unwrap()
                .object
                .get("test:timestamp"),
            Some(Value::DateTime(_))
        ));

        db.insert(
            DEFAULT_COLLECTION,
            "missing",
            entity(
                "missing",
                "test:default_expression",
                [("title", Value::String("missing".to_string()))],
            ),
        )
        .unwrap();
        assert!(matches!(
            db.get(DEFAULT_COLLECTION, "missing")
                .unwrap()
                .unwrap()
                .object
                .get("test:timestamp"),
            Some(Value::DateTime(_))
        ));

        let fixed = DateTime::now_utc();
        db.insert(
            DEFAULT_COLLECTION,
            "set",
            entity(
                "set",
                "test:default_expression",
                [
                    ("title", Value::String("set".to_string())),
                    ("timestamp", Value::DateTime(fixed)),
                ],
            ),
        )
        .unwrap();
        let _ = update_title(&mut db, "set", "changed");
        assert_eq!(
            db.get(DEFAULT_COLLECTION, "set")
                .unwrap()
                .unwrap()
                .object
                .get("test:timestamp"),
            Some(&Value::DateTime(fixed))
        );

        db.update_where(
            UpdateQuery::new()
                .with_collection(DEFAULT_COLLECTION)
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["id"]),
                    Value::String("set".to_string()),
                ))
                .set(
                    FieldPath::from_fields(["timestamp"]),
                    Expr::Operand(Operand::Literal(Value::Void)),
                ),
        )
        .unwrap();
        let regenerated = db
            .get(DEFAULT_COLLECTION, "set")
            .unwrap()
            .unwrap()
            .object
            .get("test:timestamp")
            .cloned();
        assert!(matches!(regenerated, Some(Value::DateTime(_))));
        assert_ne!(regenerated, Some(Value::DateTime(fixed)));

        db.insert(
            DEFAULT_COLLECTION,
            "null",
            entity(
                "null",
                "test:default_expression",
                [
                    ("title", Value::String("null".to_string())),
                    ("timestamp", Value::Null),
                ],
            ),
        )
        .unwrap();
        let _ = update_title(&mut db, "null", "still null");
        assert_eq!(
            db.get(DEFAULT_COLLECTION, "null")
                .unwrap()
                .unwrap()
                .object
                .get("test:timestamp"),
            Some(&Value::Null)
        );

        db.insert(
            DEFAULT_COLLECTION,
            "void",
            entity(
                "void",
                "test:default_expression",
                [
                    ("title", Value::String("void".to_string())),
                    ("timestamp", Value::Void),
                ],
            ),
        )
        .unwrap();
        assert!(matches!(
            db.get(DEFAULT_COLLECTION, "void")
                .unwrap()
                .unwrap()
                .object
                .get("test:timestamp"),
            Some(Value::DateTime(_))
        ));
    }

    #[test]
    fn typed_closed_record_rows_reject_unknown_fields() {
        let mut db = EmbeddedDb::in_memory();

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
        let mut db = EmbeddedDb::in_memory();
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
        assert_eq!(
            rows[0].get("semantic:name"),
            Some(&Value::String("A".to_string()))
        );

        let mut p3 = Object::new();
        p3.insert("id", Value::String("p3".to_string()));
        p3.insert("email", Value::String("a@example.com".to_string()));
        p3.insert("name", Value::String("C".to_string()));
        let err = db.insert("people", "p3", p3).unwrap_err();
        assert!(err.to_string().contains("unique index violation"));
        assert!(matches!(
            err,
            DbError::UniqueViolation { ref existing_id, ref id, .. }
                if existing_id == "p1" && id == "p3"
        ));
    }

    #[test]
    fn dataset_unique_check_probes_the_index_for_changed_rows() {
        let mut db = EmbeddedDb::in_memory();
        let people = db
            .create_collection("people", CollectionKind::Polymorphic)
            .unwrap();
        db.create_index("people_email_uq", people, "email", true)
            .unwrap();
        let person = |id: &str, email: &str| BatchOperation::Upsert {
            collection: "people".to_string(),
            id: id.to_string(),
            object: string_object(&[("id", id), ("email", email)]),
        };
        let violation = |err: DbError| match err {
            DbError::UniqueViolation {
                existing_id, id, ..
            } => (existing_id, id),
            other => panic!("unique violation expected: {other:?}"),
        };
        db.transact(
            Batch::new()
                .with_op(person("p1", "a@example.com"))
                .with_op(person("p2", "b@example.com")),
        )
        .unwrap();
        // Stored holders release a value they no longer hold.
        db.transact(
            Batch::new()
                .with_op(person("p1", "b@example.com"))
                .with_op(person("p2", "a@example.com")),
        )
        .unwrap();
        let err = db
            .transact(
                Batch::new()
                    .with_op(person("p4", "c@example.com"))
                    .with_op(person("p3", "c@example.com")),
            )
            .unwrap_err();
        assert_eq!(violation(err), ("p3".to_string(), "p4".to_string()));
        let err = db
            .transact(Batch::new().with_op(person("p0", "b@example.com")))
            .unwrap_err();
        assert_eq!(violation(err), ("p0".to_string(), "p1".to_string()));
        db.transact(
            Batch::new()
                .with_op(BatchOperation::DeleteById {
                    collection: "people".to_string(),
                    id: "p1".to_string(),
                })
                .with_op(person("p0", "b@example.com")),
        )
        .unwrap();
    }

    #[test]
    fn dataset_writes_revalidate_changed_rows_and_their_dependents() {
        let mut db = EmbeddedDb::in_memory();
        register_ref_schema(&mut db, ref_ty("person"));
        let upsert = |id: &str, object: Object| BatchOperation::Upsert {
            collection: DEFAULT_COLLECTION.to_string(),
            id: id.to_string(),
            object,
        };
        let delete = |id: &str| BatchOperation::DeleteById {
            collection: DEFAULT_COLLECTION.to_string(),
            id: id.to_string(),
        };
        let article = |id: &str, author: &str| {
            upsert(
                id,
                entity(id, "article", [("author", Value::String(author.into()))]),
            )
        };
        db.transact(
            Batch::new()
                .with_op(upsert("person-1", entity("person-1", "person", [])))
                .with_op(article("article-1", "person-1")),
        )
        .unwrap();

        // Unchanged rows referencing a deleted or retyped row are re-validated.
        let err = db
            .transact(Batch::new().with_op(delete("person-1")))
            .unwrap_err();
        assert!(
            matches!(err, DbError::ReferenceTargetNotFound { ref id, .. } if id == "person-1"),
            "{err}"
        );
        let err = db
            .transact(
                Batch::new().with_op(upsert("person-1", entity("person-1", "organization", []))),
            )
            .unwrap_err();
        assert!(err.to_string().contains("expected one of:"), "{err}");

        // Rows written without foreign-key checks stay as they are until
        // they change: other writes validate only the rows they touch.
        db.execute_batch_with_settings(
            Batch::new().with_op(article("article-2", "missing")),
            crate::WriteSettings {
                validate_foreign_keys: false,
            },
        )
        .unwrap();
        db.transact(Batch::new().with_op(upsert("person-2", entity("person-2", "person", []))))
            .unwrap();
        let err = db
            .transact(Batch::new().with_op(article("article-2", "still-missing")))
            .unwrap_err();
        assert!(
            matches!(err, DbError::ReferenceTargetNotFound { .. }),
            "{err}"
        );

        db.transact(
            Batch::new()
                .with_op(delete("article-1"))
                .with_op(delete("person-1")),
        )
        .unwrap();
    }

    #[test]
    fn index_entries_update_on_upsert_and_delete() {
        let mut db = EmbeddedDb::in_memory();
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
    fn open_rebuilds_outdated_index_storage() {
        let mut db = EmbeddedDb::in_memory();
        let events = db
            .create_collection("events", CollectionKind::Polymorphic)
            .unwrap();
        db.create_index("events_kind_idx", events, "kind", false)
            .unwrap();

        let mut event = Object::new();
        event.insert("id", Value::String("event1".to_string()));
        event.insert("kind", Value::String("music".to_string()));
        db.insert("events", "event1", event).unwrap();

        let kind_index = db
            .catalog()
            .find_equality_index(events, "kind")
            .unwrap()
            .lid;
        db.storage.corrupt_index(kind_index);

        let query = SelectQuery::new()
            .with_collection("events")
            .with_predicate(eq_predicate(
                FieldPath::from_fields(["kind"]),
                Value::String("music".to_string()),
            ));
        assert_eq!(db.select(query.clone()).unwrap().len(), 0);

        let (_catalog, engine) = db.into_parts();
        let reopened = EmbeddedDb::open(engine).unwrap();

        let rows = reopened.select(query).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("id"),
            Some(&Value::String("event1".to_string()))
        );
    }

    #[test]
    fn creating_index_backfills_existing_rows() {
        let mut db = EmbeddedDb::in_memory();
        let events = db
            .create_collection("events", CollectionKind::Polymorphic)
            .unwrap();

        let mut event = Object::new();
        event.insert("id", Value::String("event1".to_string()));
        event.insert("kind", Value::String("music".to_string()));
        db.insert("events", "event1", event).unwrap();

        db.create_index("events_kind_idx", events, "kind", false)
            .unwrap();

        let query = SelectQuery::new()
            .with_collection("events")
            .with_predicate(eq_predicate(
                FieldPath::from_fields(["kind"]),
                Value::String("music".to_string()),
            ));
        let rows = db.select(query).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("id"),
            Some(&Value::String("event1".to_string()))
        );
    }

    #[test]
    fn query_order_by_sorts_rows() {
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();
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
                assert_eq!(index_name, crate::catalog::AUTO_PATH_INDEX_NAME);
            }
            _ => panic!("expected index lookup plan"),
        }
    }

    #[test]
    fn auto_index_simple_equality_query_returns_matching_entities() {
        let mut db = EmbeddedDb::in_memory();
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
    fn builtin_type_field_can_be_projected_and_filtered() {
        let mut db = EmbeddedDb::in_memory();
        db.create_collection("entities", CollectionKind::Polymorphic)
            .unwrap();

        let mut directory = Object::new();
        directory.insert("id", Value::String("dir1".to_string()));
        directory.insert("type", Value::String("semantic:base:directory".to_string()));
        directory.insert("title", Value::String("Dir1".to_string()));
        db.insert("entities", "dir1", directory).unwrap();

        let projected = db
            .select(
                SelectQuery::new()
                    .with_collection("entities")
                    .with_projection(vec![crate::QueryField {
                        expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "type",
                        ])))),
                        alias: None,
                        wildcard: None,
                    }]),
            )
            .unwrap();
        assert_eq!(
            projected[0].get("type"),
            Some(&Value::String("semantic:base:directory".to_string()))
        );

        let filtered = db
            .select(
                SelectQuery::new()
                    .with_collection("entities")
                    .with_predicate(eq_predicate(
                        FieldPath::from_fields(["type"]),
                        Value::String("semantic:base:directory".to_string()),
                    )),
            )
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(
            filtered[0].get("id"),
            Some(&Value::String("dir1".to_string()))
        );

        let sql_project = crate::sql::parse_sql_query(
            "SELECT type FROM entities LIMIT 10",
            crate::SqlDialectKind::Generic,
        )
        .unwrap()
        .query;
        let Query::Select(sql_project) = sql_project else {
            panic!("expected SQL select query");
        };
        let sql_projected = db.select(sql_project).unwrap();
        assert_eq!(
            sql_projected[0].get("type"),
            Some(&Value::String("semantic:base:directory".to_string()))
        );

        let sql_filter = crate::sql::parse_sql_query(
            "SELECT * FROM entities WHERE type = 'semantic:base:directory' LIMIT 10",
            crate::SqlDialectKind::Generic,
        )
        .unwrap()
        .query;
        let Query::Select(sql_filter) = sql_filter else {
            panic!("expected SQL select query");
        };
        let sql_filtered = db.select(sql_filter).unwrap();
        assert_eq!(sql_filtered.len(), 1);
        assert_eq!(
            sql_filtered[0].get("id"),
            Some(&Value::String("dir1".to_string()))
        );
    }

    #[test]
    fn auto_index_nested_paths_work_with_planner_and_mutations() {
        let mut db = EmbeddedDb::in_memory();
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
                assert_eq!(index_name, crate::catalog::AUTO_PATH_INDEX_NAME);
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
        let mut db = EmbeddedDb::in_memory();
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
                wildcard: None,
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
        let mut db = EmbeddedDb::in_memory();
        db.create_collection("items", CollectionKind::Polymorphic)
            .unwrap();

        let mut row = Object::new();
        row.insert("id", Value::String("i1".to_string()));
        row.insert("kind", Value::String("music".to_string()));
        db.insert("items", "i1", row).unwrap();

        let delete = crate::DeleteQuery::new()
            .with_predicate(eq_predicate(
                FieldPath::from_fields(["kind"]),
                Value::String("music".to_string()),
            ))
            .with_returning(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "id",
                ])))),
                alias: None,
                wildcard: None,
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
    fn delete_returning_rebuilds_from_single_limited_traversal() {
        let mut db = EmbeddedDb::in_memory();
        db.create_collection("items", CollectionKind::Polymorphic)
            .unwrap();
        for id in ["i1", "i2", "i3"] {
            let mut row = Object::new();
            row.insert("id", Value::String(id.to_string()));
            db.insert("items", id, row).unwrap();
        }

        let query = crate::DeleteQuery::new()
            .with_collection("items")
            .with_limit(2)
            .with_returning(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "id",
                ])))),
                alias: None,
                wildcard: None,
            }]);
        let result = db.delete_where_returning(query).unwrap();

        assert_eq!(result.deleted, 2);
        assert_eq!(
            result
                .returning
                .iter()
                .filter_map(|row| row.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["i1", "i2"],
        );
        assert!(db.get("items", "i1").unwrap().is_none());
        assert!(db.get("items", "i2").unwrap().is_none());
        assert!(db.get("items", "i3").unwrap().is_some());
    }

    #[test]
    fn programmatic_mutations_reject_invalid_limits_without_writes() {
        let mut db = EmbeddedDb::in_memory();
        db.create_collection("items", CollectionKind::Polymorphic)
            .unwrap();

        let mut row = Object::new();
        row.insert("id", Value::String("i1".to_string()));
        row.insert("value", Value::I64(1));
        db.insert("items", "i1", row).unwrap();

        let invalid_limit = Expr::Operand(Operand::Literal(Value::I64(-1)));
        let update = UpdateQuery::new()
            .with_collection("items")
            .set(
                FieldPath::from_fields(["value"]),
                Expr::Operand(Operand::Literal(Value::I64(2))),
            )
            .with_limit(invalid_limit.clone());
        let error = db.update_where(update).unwrap_err();
        assert!(matches!(error, DbError::InvalidQuery(_)));
        assert_eq!(
            db.get("items", "i1")
                .unwrap()
                .and_then(|row| row.object.get("value").cloned()),
            Some(Value::I64(1))
        );

        let delete = crate::DeleteQuery::new()
            .with_collection("items")
            .with_limit(invalid_limit);
        let error = db.delete_where(delete).unwrap_err();
        assert!(matches!(error, DbError::InvalidQuery(_)));
        assert!(db.get("items", "i1").unwrap().is_some());
    }

    #[test]
    fn mvcc_transaction_requires_mvcc_backend() {
        let mut db = EmbeddedDb::in_memory();
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
        let mut db = EmbeddedDb::in_memory();
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

        let qualified = db
            .select(SelectQuery::new().with_collection("items"))
            .unwrap();
        assert_eq!(
            qualified[0].get("semantic:title"),
            Some(&Value::String("hello".to_string()))
        );
        assert!(!qualified[0].contains_key("title"));

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
            strict_schema: false,
            creatable_in_ui: None,
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

    pub(super) fn register_ref_schema<S: EntityStorage>(db: &mut EmbeddedDb<S>, author_ty: Type) {
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
            strict_schema: false,
            creatable_in_ui: None,
            attributes,
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn default_expression_test_class(with_default: bool) -> ClassType {
        let constraints = if with_default {
            vec![Constraint::DefaultExpr {
                expr: SchemaExpr::Call(Box::new(CallExpr {
                    callee: Callee::Name(vec!["time".to_string(), "now".to_string()]),
                    args: Vec::new(),
                    over: None,
                })),
            }]
        } else {
            Vec::new()
        };
        test_class(
            "test:default_expression",
            "DefaultExpression",
            None,
            BTreeMap::from([
                (
                    "timestamp".to_string(),
                    ClassAttribute {
                        attribute: AttributeRef {
                            id: "test:timestamp".to_string(),
                        },
                        required: false,
                        ui_order: None,
                        computed: None,
                        constraints,
                        meta: Meta::default(),
                    },
                ),
                (
                    "title".to_string(),
                    ClassAttribute {
                        attribute: AttributeRef {
                            id: "test:title".to_string(),
                        },
                        required: false,
                        ui_order: None,
                        computed: None,
                        constraints: Vec::new(),
                        meta: Meta::default(),
                    },
                ),
            ]),
        )
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

    pub(super) fn ref_ty(class_id: &str) -> Type {
        ty(TypeKind::Ref(semantic_data::schema::EntityRef::new(
            class_id,
        )))
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
