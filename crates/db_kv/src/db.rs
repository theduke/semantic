use std::collections::{BTreeMap, BTreeSet};

use semantic_data::value::{FieldPath, Object, Value, ValueRef};

use crate::{
    error::DbError,
    query::{
        Batch, BatchOperation, DeleteQuery, MutationStats, SelectQuery, UpdateQuery,
        canonicalize_delete_query, canonicalize_select_query, canonicalize_update_query,
        execute_batch, normalize_object_for_collection,
    },
    schema_store::{catalog_snapshot_key, catalog_write_op, decode_catalog},
    storage::{
        EntityStore, KvCommitOutcome, KvEngine, KvTransactionCapabilities, KvWriteOp,
        MemoryKvEngine, StoredEntity, StoredEntityKind,
    },
};
use semantic_db_core::catalog::{
    Catalog, CollectionKind, CollectionSchema, LocalAttrId, LocalCollectionId, LocalFieldId,
    SharedCatalog,
};
use semantic_db_core::{
    DdlBatch, DdlOperation, DdlOutcome, QueryContext, TransactionConcurrency, TransactionOptions,
    apply_ddl_batch, run_with_transaction_retries,
};

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
    pub logical: semantic_db_core::LogicalPlan,
    pub physical: semantic_db_core::PhysicalPlan,
    pub access_path: AccessPath,
}

#[derive(Debug)]
pub struct Database<E: KvEngine> {
    catalog: SharedCatalog,
    store: EntityStore<E>,
}

impl Database<MemoryKvEngine> {
    pub fn in_memory() -> Self {
        Self::new(MemoryKvEngine::new())
    }

    pub fn in_memory_mvcc() -> Self {
        Self::new(MemoryKvEngine::with_mvcc(true))
    }
}

impl<E: KvEngine> Database<E> {
    fn query_context(&self) -> QueryContext {
        QueryContext::from_shared(&self.catalog)
    }

    pub fn new(engine: E) -> Self {
        Self::open(engine).expect("database initialization failed")
    }

    pub fn open(engine: E) -> std::result::Result<Self, DbError> {
        let mut store = EntityStore::new(engine);
        let catalog = if let Some(bytes) = store.get_raw(&catalog_snapshot_key())? {
            decode_catalog(&bytes)?
        } else {
            let catalog = Catalog::new();
            let op = catalog_write_op(&catalog)?;
            store.write_batch(&[op])?;
            catalog
        };

        Ok(Self {
            catalog: SharedCatalog::new(catalog),
            store,
        })
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
        let ddl = DdlBatch::new().with_op(DdlOperation::UpsertCollection {
            name: name.clone(),
            kind: match kind {
                CollectionKind::Untyped => semantic_db_core::DdlCollectionKind::Untyped,
                CollectionKind::Record { record_type } => {
                    let snapshot = self.catalog();
                    let ty = snapshot
                        .record_type_by_lid(record_type)
                        .ok_or(DbError::UnknownRecordType(record_type))?;
                    semantic_db_core::DdlCollectionKind::Record {
                        record_type: ty.id.clone(),
                    }
                }
                CollectionKind::Class { class } => {
                    let snapshot = self.catalog();
                    let class = snapshot
                        .class_by_lid(class)
                        .ok_or(DbError::UnknownClass(class))?;
                    semantic_db_core::DdlCollectionKind::Class {
                        class: class.class.id.clone(),
                    }
                }
            },
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

    pub fn query(
        &self,
        collection: &str,
        query: SelectQuery,
    ) -> std::result::Result<Vec<Object>, DbError> {
        let catalog = self.catalog();
        let collection = catalog.collection_by_name(collection).ok_or_else(|| {
            DbError::UnknownCollectionByName {
                name: collection.to_string(),
            }
        })?;

        let query = canonicalize_select_query(&query, collection)?;

        let optimizer = semantic_db_core::Optimizer::core();
        let stats = self.stats_for_collection(collection)?;
        let context = self.query_context();
        let pair = optimizer.optimize_query(
            &query,
            Some(collection.name.clone()),
            Some(&stats),
            &context,
        );
        self.execute_physical_plan(&pair.physical, Some(collection.name.as_str()))
    }

    pub fn plan_query(
        &self,
        collection: &str,
        query: SelectQuery,
    ) -> std::result::Result<QueryPlan, DbError> {
        let explain = self.explain_query(collection, query)?;
        match explain.access_path {
            AccessPath::FullScan => Ok(QueryPlan::FullScan {
                collection: collection.to_string(),
            }),
            AccessPath::IndexLookup {
                index_name,
                field,
                value,
            } => Ok(QueryPlan::IndexLookup {
                collection: collection.to_string(),
                index_name,
                field,
                value,
            }),
        }
    }

    pub fn explain_query(
        &self,
        collection: &str,
        query: SelectQuery,
    ) -> std::result::Result<QueryExplain, DbError> {
        let catalog = self.catalog();
        let collection = catalog.collection_by_name(collection).ok_or_else(|| {
            DbError::UnknownCollectionByName {
                name: collection.to_string(),
            }
        })?;

        let query = canonicalize_select_query(&query, collection)?;

        let optimizer = semantic_db_core::Optimizer::core();
        let stats = self.stats_for_collection(collection)?;
        let context = self.query_context();
        let pair = optimizer.optimize_query(
            &query,
            Some(collection.name.clone()),
            Some(&stats),
            &context,
        );
        let access_path = self.access_path_from_physical(collection, &pair.physical);

        Ok(QueryExplain {
            logical: pair.logical,
            physical: pair.physical,
            access_path,
        })
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
        collection: &str,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        let catalog = self.catalog();
        let collection_schema = catalog
            .collection_by_name(collection)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: collection.to_string(),
            })?
            .clone();

        let query = canonicalize_update_query(&query, &collection_schema)?;

        let mut batch = Batch::new();
        batch.operations.push(BatchOperation::Update {
            collection: collection.to_string(),
            query,
        });
        let out = self.execute_batch(batch)?;
        Ok(MutationStats {
            matched: out.stats.updated,
            affected: out.stats.updated,
        })
    }

    pub fn delete_where(
        &mut self,
        collection: &str,
        query: DeleteQuery,
    ) -> std::result::Result<usize, DbError> {
        let catalog = self.catalog();
        let collection_schema = catalog
            .collection_by_name(collection)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: collection.to_string(),
            })?
            .clone();

        let query = canonicalize_delete_query(&query, &collection_schema)?;

        let mut batch = Batch::new();
        batch.operations.push(BatchOperation::Delete {
            collection: collection.to_string(),
            query,
        });
        let out = self.execute_batch(batch)?;
        Ok(out.stats.deleted)
    }

    pub fn transact(
        &mut self,
        batch: Batch,
    ) -> std::result::Result<crate::query::BatchOutcome, DbError> {
        self.transact_with_options(batch, TransactionOptions::default())
    }

    pub fn transact_with_options(
        &mut self,
        batch: Batch,
        options: TransactionOptions,
    ) -> std::result::Result<crate::query::BatchOutcome, DbError> {
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

    pub fn execute_batch(
        &mut self,
        batch: Batch,
    ) -> std::result::Result<crate::query::BatchOutcome, DbError> {
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
            extra_ops.push(catalog_write_op(&next_catalog)?);

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
                    catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    canonical_ops.push(BatchOperation::Upsert {
                        collection,
                        id,
                        object,
                    });
                }
                BatchOperation::DeleteById { collection, id } => {
                    catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    canonical_ops.push(BatchOperation::DeleteById { collection, id });
                }
                BatchOperation::DeleteByIds { collection, ids } => {
                    catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    canonical_ops.push(BatchOperation::DeleteByIds { collection, ids });
                }
                BatchOperation::Update { collection, query } => {
                    let schema = catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    let query = canonicalize_update_query(&query, schema)?;
                    canonical_ops.push(BatchOperation::Update { collection, query });
                }
                BatchOperation::Delete { collection, query } => {
                    let schema = catalog.collection_by_name(&collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    let query = canonicalize_delete_query(&query, schema)?;
                    canonical_ops.push(BatchOperation::Delete { collection, query });
                }
            }
        }

        Ok(Batch {
            operations: canonical_ops,
        })
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
        for collection_name in crate::query::touched_collections(batch) {
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
                normalize_object_for_collection(&collection_schema, &mut object)?;
                normalized_rows.insert(id.clone(), object);
            }
            self.validate_unique_indexes(catalog, &collection_schema, &normalized_rows)?;

            let old_rows = before.get(collection_name);
            if old_rows == Some(&normalized_rows) {
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

            let kind = match collection_schema.kind {
                CollectionKind::Untyped => StoredEntityKind::Untyped,
                CollectionKind::Record { .. } => StoredEntityKind::Record,
                CollectionKind::Class { .. } => StoredEntityKind::Class,
            };

            for (id, object) in &normalized_rows {
                let entity = StoredEntity {
                    id: id.clone(),
                    collection: collection_schema.lid.0,
                    kind: kind.clone(),
                    object: object.clone(),
                };
                self.push_entity_ops(&mut ops, &entity)?;

                for index in &indexes {
                    if let Some(value) = object.get(&index.canonical_field) {
                        let key = crate::storage::index_key(index.lid, value, id)?;
                        ops.push(KvWriteOp::Put {
                            key,
                            value: Vec::new(),
                        });
                    }
                }
            }
        }

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

    fn validate_unique_indexes(
        &self,
        catalog: &Catalog,
        collection: &CollectionSchema,
        rows: &BTreeMap<String, Object>,
    ) -> std::result::Result<(), DbError> {
        let indexes: Vec<_> = catalog
            .indexes_for_collection(collection.lid)
            .filter(|idx| idx.schema.unique)
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

        let field = match field_ref {
            semantic_db_core::FieldRef::CanonicalName(name) => name.clone(),
            semantic_db_core::FieldRef::FieldId(field_id) => collection
                .field_name_by_id(*field_id)
                .unwrap_or_default()
                .to_string(),
            semantic_db_core::FieldRef::AttrId(attr_id) => {
                let mut out = None;
                for (field_id, _) in collection.fields() {
                    if collection.attr_for_field_id(field_id) == Some(*attr_id) {
                        out = collection.field_name_by_id(field_id).map(ToOwned::to_owned);
                        break;
                    }
                }
                out.unwrap_or_default()
            }
            semantic_db_core::FieldRef::Path(path) => match path.segments().first() {
                Some(semantic_data::value::PathSegment::Field(field)) => field.clone(),
                _ => String::new(),
            },
        };
        if field.is_empty() {
            return AccessPath::FullScan;
        }

        let catalog = self.catalog();
        let Some(index) = catalog.find_equality_index(collection.lid, &field) else {
            return AccessPath::FullScan;
        };

        AccessPath::IndexLookup {
            index_name: index.schema.name.clone(),
            field: field.to_string(),
            value: value.clone(),
        }
    }
}

struct KvPhysicalDataSource<'a, E: KvEngine> {
    db: &'a Database<E>,
    catalog: std::sync::Arc<Catalog>,
    default_collection: Option<String>,
}

impl<E: KvEngine> KvPhysicalDataSource<'_, E> {
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
        let name = source
            .source_name
            .as_deref()
            .or(self.default_collection.as_deref())
            .ok_or_else(|| {
                DbError::InvalidQuery("physical source did not specify a collection".to_string())
            })?;
        self.catalog
            .collection_by_name(name)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: name.to_string(),
            })
    }
}

#[derive(Clone)]
struct KvObjectView {
    object: Object,
    collection_id: LocalCollectionId,
    field_names: BTreeMap<LocalFieldId, String>,
    attr_names: BTreeMap<LocalAttrId, String>,
}

impl semantic_db_core::ObjectAccess for KvObjectView {
    fn value_at_path_ref<'a>(&'a self, path: &FieldPath) -> Option<ValueRef<'a>> {
        semantic_db_core::ObjectAccess::value_at_path_ref(&self.object, path)
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

impl<E: KvEngine> semantic_db_core::PhysicalDataSource for KvPhysicalDataSource<'_, E> {
    fn scan(
        &self,
        source: &semantic_db_core::SourceRef,
    ) -> semantic_db_core::CoreResult<Vec<semantic_db_core::DynObject>> {
        let collection = self
            .resolve_collection(source)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
        let rows = self
            .db
            .store
            .scan_collection(collection.lid)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;
        let mut field_names = BTreeMap::new();
        let mut attr_names = BTreeMap::new();
        for (field_id, name) in collection.fields() {
            field_names.insert(field_id, name.to_string());
            if let Some(attr_id) = collection.attr_for_field_id(field_id) {
                attr_names.insert(attr_id, name.to_string());
            }
        }
        Ok(rows
            .into_iter()
            .map(|row| {
                Box::new(KvObjectView {
                    object: row.object,
                    collection_id: collection.lid,
                    field_names: field_names.clone(),
                    attr_names: attr_names.clone(),
                }) as semantic_db_core::DynObject
            })
            .collect())
    }

    fn index_lookup(
        &self,
        source: &semantic_db_core::SourceRef,
        field: &semantic_db_core::FieldRef,
        value: &Value,
    ) -> semantic_db_core::CoreResult<Vec<semantic_db_core::DynObject>> {
        let collection = self
            .resolve_collection(source)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;

        let canonical_field = match field {
            semantic_db_core::FieldRef::CanonicalName(name) => Some(name.as_str()),
            semantic_db_core::FieldRef::FieldId(field_id) => collection.field_name_by_id(*field_id),
            semantic_db_core::FieldRef::AttrId(attr_id) => {
                let mut found = None;
                for (field_id, _) in collection.fields() {
                    if collection.attr_for_field_id(field_id) == Some(*attr_id) {
                        found = collection.field_name_by_id(field_id);
                        break;
                    }
                }
                found
            }
            semantic_db_core::FieldRef::Path(path) => match path.segments().first() {
                Some(semantic_data::value::PathSegment::Field(field)) => Some(field.as_str()),
                _ => None,
            },
        };

        let Some(canonical_field) = canonical_field else {
            if let semantic_db_core::FieldRef::Path(path) = field {
                return semantic_db_core::PhysicalDataSource::scan_filtered(
                    self,
                    source,
                    &semantic_db_core::Predicate::Compare {
                        op: semantic_db_core::CompareOp::Eq,
                        left: semantic_db_core::Operand::Field(path.clone()),
                        right: semantic_db_core::Operand::Literal(value.clone()),
                    },
                );
            }
            return semantic_db_core::PhysicalDataSource::scan(self, source);
        };

        let Some(index) = self
            .catalog
            .find_equality_index(collection.lid, canonical_field)
        else {
            return semantic_db_core::PhysicalDataSource::scan_filtered(
                self,
                source,
                &semantic_db_core::Predicate::Compare {
                    op: semantic_db_core::CompareOp::Eq,
                    left: semantic_db_core::Operand::Field(
                        semantic_data::value::FieldPath::from_fields([canonical_field]),
                    ),
                    right: semantic_db_core::Operand::Literal(value.clone()),
                },
            );
        };

        let ids = self
            .db
            .store
            .scan_index_value(index.lid, value)
            .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?;

        let mut out = Vec::with_capacity(ids.len());
        let mut field_names = BTreeMap::new();
        let mut attr_names = BTreeMap::new();
        for (field_id, name) in collection.fields() {
            field_names.insert(field_id, name.to_string());
            if let Some(attr_id) = collection.attr_for_field_id(field_id) {
                attr_names.insert(attr_id, name.to_string());
            }
        }
        for id in ids {
            if let Some(entity) = self
                .db
                .store
                .get_entity(collection.lid, &id)
                .map_err(|err| semantic_db_core::CoreError::new(err.to_string()))?
            {
                out.push(Box::new(KvObjectView {
                    object: entity.object,
                    collection_id: collection.lid,
                    field_names: field_names.clone(),
                    attr_names: attr_names.clone(),
                }) as semantic_db_core::DynObject);
            }
        }
        Ok(out)
    }
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
            semantic_db_core::FieldRef::CanonicalName(name) => self.indexed_fields.contains(name),
            semantic_db_core::FieldRef::FieldId(field_id) => {
                self.indexed_field_ids.contains(field_id)
            }
            semantic_db_core::FieldRef::AttrId(attr_id) => self.indexed_attr_ids.contains(attr_id),
            semantic_db_core::FieldRef::Path(path) => path
                .segments()
                .first()
                .and_then(|segment| match segment {
                    semantic_data::value::PathSegment::Field(name) => Some(name),
                    _ => None,
                })
                .is_some_and(|name| self.indexed_fields.contains(name)),
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
            semantic_db_core::FieldRef::CanonicalName(name) => self.indexed_fields.contains(name),
            semantic_db_core::FieldRef::FieldId(field_id) => {
                self.indexed_field_ids.contains(field_id)
            }
            semantic_db_core::FieldRef::AttrId(attr_id) => self.indexed_attr_ids.contains(attr_id),
            semantic_db_core::FieldRef::Path(path) => path
                .segments()
                .first()
                .and_then(|segment| match segment {
                    semantic_data::value::PathSegment::Field(name) => Some(name),
                    _ => None,
                })
                .is_some_and(|name| self.indexed_fields.contains(name)),
        };
        Some(indexed)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semantic_data::{
        schema::{
            attribute::attribute_ref::AttributeRef,
            attribute::attribute_type::AttributeType,
            class::class_attribute::ClassAttribute,
            class::class_type::ClassType,
            core::{meta::Meta, type_kind::TypeKind, type_node::Type},
            primitives::{bool_type::BoolType, string_type::StringType},
            record::field::Field,
            record::record_type::RecordType,
        },
        value::{FieldPath, Object, Value},
    };

    use crate::{
        CollectionKind,
        query::{
            CompareOp, DdlBatch, DdlCollectionKind, DdlOperation, Operand, OrderBy, Predicate,
            QueryField, SelectQuery, SortDirection,
        },
        storage::FileKvEngine,
    };
    use semantic_db_core::{TransactionConcurrency, TransactionOptions};

    use super::{Database, QueryPlan};

    #[test]
    fn untyped_query_works() {
        let mut db = Database::in_memory();
        db.create_collection("events", CollectionKind::Untyped)
            .unwrap();

        let mut a = Object::new();
        a.insert("kind", Value::String("music".to_string()));
        a.insert("score", Value::I64(10));
        db.insert("events", "a", a).unwrap();

        let mut b = Object::new();
        b.insert("kind", Value::String("video".to_string()));
        b.insert("score", Value::I64(2));
        db.insert("events", "b", b).unwrap();

        let query = SelectQuery::new().with_predicate(Predicate::Compare {
            op: CompareOp::Eq,
            left: Operand::Field(FieldPath::from_fields(["kind"])),
            right: Operand::Literal(Value::String("music".to_string())),
        });

        let rows = db.query("events", query).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("score"), Some(&Value::I64(10)));
    }

    #[test]
    fn class_collection_normalizes_aliases_and_queries_by_alias() {
        let mut db = Database::in_memory();

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
            kind: DdlCollectionKind::Class {
                class: "core.article".to_string(),
            },
        }))
        .unwrap();

        let mut object = Object::new();
        object.insert("title", Value::String("Hello".to_string()));
        db.insert("articles", "art-1", object).unwrap();

        let row = db.get("articles", "art-1").unwrap().unwrap();
        assert_eq!(
            row.object.get("core.title"),
            Some(&Value::String("Hello".to_string()))
        );
        assert!(row.object.get("title").is_none());

        let query = SelectQuery::new()
            .with_predicate(Predicate::Compare {
                op: CompareOp::Eq,
                left: Operand::Field(FieldPath::from_fields(["title"])),
                right: Operand::Literal(Value::String("Hello".to_string())),
            })
            .with_projection(vec![QueryField {
                path: FieldPath::from_fields(["title"]),
                alias: Some("t".to_string()),
            }]);

        let rows = db.query("articles", query).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("t"), Some(&Value::String("Hello".to_string())));
    }

    #[test]
    fn closed_record_collection_rejects_unknown_fields() {
        let mut db = Database::in_memory();

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
            kind: DdlCollectionKind::Record {
                record_type: "person.record".to_string(),
            },
        }))
        .unwrap();

        let mut obj = Object::new();
        obj.insert("name", Value::String("Ada".to_string()));
        obj.insert("active", Value::Bool(true));
        obj.insert("extra", Value::Bool(true));

        let err = db.insert("people", "p1", obj).unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[test]
    fn indexed_equality_query_and_unique_enforcement() {
        let mut db = Database::in_memory();
        let people = db
            .create_collection("people", CollectionKind::Untyped)
            .unwrap();
        db.create_index("people_email_uq", people, "email", true)
            .unwrap();

        let mut p1 = Object::new();
        p1.insert("email", Value::String("a@example.com".to_string()));
        p1.insert("name", Value::String("A".to_string()));
        db.insert("people", "p1", p1).unwrap();

        let mut p2 = Object::new();
        p2.insert("email", Value::String("b@example.com".to_string()));
        p2.insert("name", Value::String("B".to_string()));
        db.insert("people", "p2", p2).unwrap();

        let q = SelectQuery::new().with_predicate(Predicate::Compare {
            op: CompareOp::Eq,
            left: Operand::Field(FieldPath::from_fields(["email"])),
            right: Operand::Literal(Value::String("a@example.com".to_string())),
        });
        let rows = db.query("people", q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("name"), Some(&Value::String("A".to_string())));

        let mut p3 = Object::new();
        p3.insert("email", Value::String("a@example.com".to_string()));
        p3.insert("name", Value::String("C".to_string()));
        let err = db.insert("people", "p3", p3).unwrap_err();
        assert!(err.to_string().contains("unique index violation"));
    }

    #[test]
    fn index_entries_update_on_upsert_and_delete() {
        let mut db = Database::in_memory();
        let events = db
            .create_collection("events", CollectionKind::Untyped)
            .unwrap();
        db.create_index("events_kind_idx", events, "kind", false)
            .unwrap();

        let mut e = Object::new();
        e.insert("kind", Value::String("music".to_string()));
        db.insert("events", "e1", e).unwrap();

        let q_music = SelectQuery::new().with_predicate(Predicate::Compare {
            op: CompareOp::Eq,
            left: Operand::Field(FieldPath::from_fields(["kind"])),
            right: Operand::Literal(Value::String("music".to_string())),
        });
        assert_eq!(db.query("events", q_music).unwrap().len(), 1);

        let mut updated = Object::new();
        updated.insert("kind", Value::String("video".to_string()));
        db.insert("events", "e1", updated).unwrap();

        let q_music = SelectQuery::new().with_predicate(Predicate::Compare {
            op: CompareOp::Eq,
            left: Operand::Field(FieldPath::from_fields(["kind"])),
            right: Operand::Literal(Value::String("music".to_string())),
        });
        assert_eq!(db.query("events", q_music).unwrap().len(), 0);

        let q_video = SelectQuery::new().with_predicate(Predicate::Compare {
            op: CompareOp::Eq,
            left: Operand::Field(FieldPath::from_fields(["kind"])),
            right: Operand::Literal(Value::String("video".to_string())),
        });
        assert_eq!(db.query("events", q_video).unwrap().len(), 1);

        db.delete("events", "e1").unwrap();
        let q_video = SelectQuery::new().with_predicate(Predicate::Compare {
            op: CompareOp::Eq,
            left: Operand::Field(FieldPath::from_fields(["kind"])),
            right: Operand::Literal(Value::String("video".to_string())),
        });
        assert_eq!(db.query("events", q_video).unwrap().len(), 0);
    }

    #[test]
    fn file_kv_engine_persists_data() {
        let path = std::env::temp_dir().join(format!(
            "semantic-db-core-test-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        {
            let engine = FileKvEngine::open(&path).unwrap();
            let mut db = Database::new(engine);
            db.create_collection("items", CollectionKind::Untyped)
                .unwrap();

            let mut obj = Object::new();
            obj.insert("name", Value::String("persisted".to_string()));
            db.insert("items", "i1", obj).unwrap();
        }

        {
            let engine = FileKvEngine::open(&path).unwrap();
            let db = Database::new(engine);
            assert!(db.catalog().collection_by_name("items").is_some());

            let item = db.get("items", "i1").unwrap().unwrap();
            assert_eq!(
                item.object.get("name"),
                Some(&Value::String("persisted".to_string()))
            );
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn query_order_by_sorts_rows() {
        let mut db = Database::in_memory();
        db.create_collection("events", CollectionKind::Untyped)
            .unwrap();

        let mut a = Object::new();
        a.insert("score", Value::I64(7));
        db.insert("events", "a", a).unwrap();

        let mut b = Object::new();
        b.insert("score", Value::I64(2));
        db.insert("events", "b", b).unwrap();

        let mut c = Object::new();
        c.insert("score", Value::I64(9));
        db.insert("events", "c", c).unwrap();

        let asc = SelectQuery::new().with_order_by(vec![OrderBy {
            path: FieldPath::from_fields(["score"]),
            direction: SortDirection::Asc,
        }]);
        let asc_rows = db.query("events", asc).unwrap();
        assert_eq!(asc_rows[0].get("score"), Some(&Value::I64(2)));
        assert_eq!(asc_rows[2].get("score"), Some(&Value::I64(9)));

        let desc = SelectQuery::new().with_order_by(vec![OrderBy {
            path: FieldPath::from_fields(["score"]),
            direction: SortDirection::Desc,
        }]);
        let desc_rows = db.query("events", desc).unwrap();
        assert_eq!(desc_rows[0].get("score"), Some(&Value::I64(9)));
        assert_eq!(desc_rows[2].get("score"), Some(&Value::I64(2)));
    }

    #[test]
    fn planner_reports_index_lookup() {
        let mut db = Database::in_memory();
        let events = db
            .create_collection("events", CollectionKind::Untyped)
            .unwrap();
        db.create_index("events_kind_idx", events, "kind", false)
            .unwrap();

        let q = SelectQuery::new().with_predicate(Predicate::Compare {
            op: CompareOp::Eq,
            left: Operand::Field(FieldPath::from_fields(["kind"])),
            right: Operand::Literal(Value::String("music".to_string())),
        });

        let plan = db.plan_query("events", q).unwrap();
        match plan {
            QueryPlan::IndexLookup { index_name, .. } => {
                assert_eq!(index_name, "events_kind_idx");
            }
            _ => panic!("expected index lookup plan"),
        }
    }

    #[test]
    fn mvcc_transaction_requires_mvcc_backend() {
        let mut db = Database::in_memory();
        db.create_collection("events", CollectionKind::Untyped)
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

    fn ty(kind: TypeKind) -> Type {
        Type {
            kind,
            constraints: vec![],
            annotations: vec![],
            meta: Meta::default(),
        }
    }
}
