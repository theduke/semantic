//! Point/index execution for ID batches. Every lookup is revision-bound and
//! adjusted for the transaction overlay before final-state validation.
use super::*;
use crate::batch_return::{ChangeSet, EntityKey, RowChange};

const REFERENCES: &str = "__semantic.reverse_references";
const TARGET_INDEX: &str = "__reverse_reference_target_idx";
const MARKER: &str = "backfill:v1";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionCounts {
    pub point_reads: usize,
    pub index_reads: usize,
    pub fallback_scans: usize,
    pub visited_rows: usize,
}

pub(crate) use crate::ResolvedReference;

pub(crate) struct TxView<'a, S: EntityStorage> {
    pub catalog: &'a Catalog,
    storage: &'a S,
    revision: Option<u64>,
    snapshot: BTreeMap<EntityKey, Option<Object>>,
    overlay: BTreeMap<EntityKey, Option<Object>>,
    pub counts: ExecutionCounts,
}

impl<'a, S: EntityStorage> TxView<'a, S> {
    pub(crate) fn new(catalog: &'a Catalog, storage: &'a S, revision: Option<u64>) -> Self {
        Self {
            catalog,
            storage,
            revision,
            snapshot: BTreeMap::new(),
            overlay: BTreeMap::new(),
            counts: ExecutionCounts::default(),
        }
    }

    pub(crate) fn get(&mut self, key: &EntityKey) -> Result<Option<Object>, DbError> {
        if let Some(row) = self.overlay.get(key).or_else(|| self.snapshot.get(key)) {
            return Ok(row.clone());
        }
        let collection = self.catalog.collection_by_name(&key.0).ok_or_else(|| {
            DbError::UnknownCollectionByName {
                name: key.0.clone(),
            }
        })?;
        let row = self
            .storage
            .get_entity_at_revision(collection.lid, &key.1, self.revision)?
            .map(|row| row.object);
        self.counts.point_reads += 1;
        self.counts.visited_rows += usize::from(row.is_some());
        self.snapshot.insert(key.clone(), row.clone());
        Ok(row)
    }

    pub(crate) fn put(&mut self, key: EntityKey, object: Object) -> Result<(), DbError> {
        self.get(&key)?;
        self.overlay.insert(key, Some(object));
        Ok(())
    }

    pub(crate) fn delete(&mut self, key: EntityKey) -> Result<bool, DbError> {
        let exists = self.get(&key)?.is_some();
        self.overlay.insert(key, None);
        Ok(exists)
    }

    pub(crate) fn unique(
        &mut self,
        index: &crate::catalog::IndexSchema,
        value: &Value,
    ) -> Result<Vec<EntityKey>, DbError> {
        let collection = self
            .catalog
            .collection_by_lid(index.collection)
            .ok_or_else(|| DbError::Storage("index collection missing".into()))?;
        self.counts.index_reads += 1;
        let mut ids: BTreeSet<String> = self
            .storage
            .scan_index_value_at_revision(index.lid, None, value, self.revision)?
            .into_iter()
            .collect();
        for ((name, id), object) in &self.overlay {
            if name != &collection.name {
                continue;
            }
            ids.remove(id);
            if object
                .as_ref()
                .and_then(|object| object.get(&index.canonical_field))
                == Some(value)
            {
                ids.insert(id.clone());
            }
        }
        Ok(ids
            .into_iter()
            .map(|id| (collection.name.clone(), id))
            .collect())
    }

    pub(crate) fn incoming(
        &mut self,
        target: &EntityKey,
    ) -> Result<Vec<(EntityKey, FieldPath)>, DbError> {
        let collection = self
            .catalog
            .collection_by_name(REFERENCES)
            .ok_or_else(|| DbError::Storage("reverse references are not initialized".into()))?;
        let index = self
            .catalog
            .find_equality_index(collection.lid, "target")
            .ok_or_else(|| DbError::Storage("reverse reference index missing".into()))?;
        self.counts.index_reads += 1;
        let ids = self.storage.scan_index_value_at_revision(
            index.lid,
            None,
            &Value::String(entity_key(target)),
            self.revision,
        )?;
        let mut refs = BTreeSet::new();
        for id in ids {
            let row = self
                .storage
                .get_entity_at_revision(collection.lid, &id, self.revision)?
                .ok_or_else(|| {
                    DbError::Storage("reverse reference index points to missing entry".into())
                })?;
            self.counts.point_reads += 1;
            self.counts.visited_rows += 1;
            let owner = (
                required_string(&row.object, "collection")?,
                required_string(&row.object, "owner")?,
            );
            if !self.overlay.contains_key(&owner) {
                refs.insert((owner, read_path(&row.object)?));
            }
        }
        for (owner, row) in &self.overlay {
            if let Some(row) = row {
                for reference in resolved_references(self.catalog, owner, row) {
                    if &reference.target == target {
                        refs.insert((owner.clone(), reference.path));
                    }
                }
            }
        }
        Ok(refs.into_iter().collect())
    }

    pub(crate) fn changes(&self) -> ChangeSet {
        self.overlay
            .iter()
            .filter_map(|(key, after)| {
                let before = self.snapshot.get(key).cloned().flatten();
                (before != *after).then(|| {
                    (
                        key.clone(),
                        RowChange {
                            before,
                            after: after.clone(),
                        },
                    )
                })
            })
            .collect()
    }
}

fn entity_key(key: &EntityKey) -> String {
    format!("{}:{}{}:{}", key.0.len(), key.0, key.1.len(), key.1)
}

fn required_string(object: &Object, field: &str) -> Result<String, DbError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| DbError::Storage(format!("invalid reverse reference {field}")))
}

fn read_path(object: &Object) -> Result<FieldPath, DbError> {
    let Some(Value::List(values)) = object.get("path") else {
        return Err(DbError::Storage("invalid reverse reference path".into()));
    };
    values
        .iter()
        .map(|value| match value {
            Value::String(field) => Ok(PathSegment::Field(field.clone())),
            Value::U64(index) => usize::try_from(*index)
                .map(PathSegment::Index)
                .map_err(|_| DbError::Storage("invalid reverse reference index".into())),
            _ => Err(DbError::Storage(
                "invalid reverse reference path segment".into(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(FieldPath)
}

fn reference_rows(references: Vec<ResolvedReference>) -> BTreeMap<String, Object> {
    references
        .into_iter()
        .map(|reference| {
            let path = Value::List(
                reference
                    .path
                    .0
                    .iter()
                    .map(|segment| match segment {
                        PathSegment::Field(field) => Value::String(field.clone()),
                        PathSegment::Index(index) => Value::U64(*index as u64),
                    })
                    .collect(),
            );
            let id = format!(
                "{}{:?}{}",
                entity_key(&reference.owner),
                reference.path,
                entity_key(&reference.target)
            );
            let object = Object::from_iter([
                ("id".into(), Value::String(id.clone())),
                ("collection".into(), Value::String(reference.owner.0)),
                ("owner".into(), Value::String(reference.owner.1)),
                ("path".into(), path),
                (
                    "target".into(),
                    Value::String(entity_key(&reference.target)),
                ),
            ]);
            (id, object)
        })
        .collect()
}

pub(crate) fn resolved_references(
    catalog: &Catalog,
    owner: &EntityKey,
    row: &Object,
) -> Vec<ResolvedReference> {
    crate::validation::stored_references(catalog, owner, row)
}

pub(crate) fn validate_row<S: EntityStorage>(
    view: &mut TxView<'_, S>,
    key: &EntityKey,
    object: &Object,
) -> Result<(), DbError> {
    let enabled = if view
        .catalog
        .collection_by_name(super::validation::STATE)
        .is_some()
    {
        view.get(&(
            super::validation::STATE.into(),
            super::validation::ACTIVE.into(),
        ))?
        .is_some()
    } else {
        false
    };
    if enabled {
        crate::validate_stored_object(view.catalog, key, object, |target| view.get(target))?;
        return Ok(());
    }
    let collection = view.catalog.collection_by_name(&key.0).ok_or_else(|| {
        DbError::UnknownCollectionByName {
            name: key.0.clone(),
        }
    })?;
    let mut targets = BTreeMap::new();
    for reference in resolved_references(view.catalog, key, object) {
        if let Some(target) = view.get(&reference.target)? {
            targets.insert(reference.target.1, target);
        }
    }
    for (field, ty) in resolved_field_types_for_object(view.catalog, collection, object) {
        if field == ATTR_RELATION_FROM || field == ATTR_RELATION_TO {
            continue;
        }
        if let Some(value) = object.get(&field) {
            validate_ref_value(view.catalog, collection, &targets, &field, &ty, value)?;
        }
    }
    Ok(())
}

impl<S: EntityStorage> EmbeddedDb<S> {
    pub(super) fn initialize_reverse_references(&mut self) -> Result<(), DbError> {
        if self.catalog().collection_by_name(REFERENCES).is_none() {
            self.create_collection(REFERENCES, CollectionKind::Polymorphic)?;
        }
        self.mark_collection_internal(REFERENCES)?;
        let catalog = self.catalog();
        let collection = catalog.collection_by_name(REFERENCES).unwrap();
        if catalog
            .find_equality_index(collection.lid, "target")
            .is_none()
        {
            self.create_index(TARGET_INDEX, collection.lid, "target", false)?;
        }
        let catalog = self.catalog();
        let collection = catalog.collection_by_name(REFERENCES).unwrap();
        if self.storage.get_entity(collection.lid, MARKER)?.is_none() {
            let revision = self.storage.current_revision()?;
            let mut ops = Vec::new();
            self.rebuild_reverse_references(&catalog, &BTreeMap::new(), &mut ops)?;
            match self.storage.apply_batch_conditional(&ops, revision)? {
                StorageCommitOutcome::Committed { .. } => {}
                StorageCommitOutcome::Conflict { .. } => {
                    return Err(DbError::TransactionConflict(
                        "reverse reference backfill conflicted".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn rebuild_reverse_references(
        &self,
        catalog: &Catalog,
        after: &crate::Dataset,
        ops: &mut Vec<StorageWriteOp>,
    ) -> Result<(), DbError> {
        let Some(collection) = catalog.collection_by_name(REFERENCES) else {
            return Ok(());
        };
        ops.push(StorageWriteOp::ClearCollection(collection.lid));
        for index in catalog.indexes_for_collection(collection.lid) {
            ops.push(StorageWriteOp::ResetIndex(index.lid));
        }
        let mut refs = Vec::new();
        for (_, schema) in catalog
            .collections()
            .filter(|(_, schema)| schema.name != REFERENCES)
        {
            let stored;
            let rows = if let Some(rows) = after.get(&schema.name) {
                rows
            } else {
                stored = self
                    .storage
                    .scan_collection(schema.lid)?
                    .into_iter()
                    .map(|row| (row.id, row.object))
                    .collect();
                &stored
            };
            for (id, object) in rows {
                refs.extend(resolved_references(
                    catalog,
                    &(schema.name.clone(), id.clone()),
                    object,
                ));
            }
        }
        self.push_row_delta(
            catalog,
            collection.lid,
            &BTreeMap::new(),
            &reference_rows(refs),
            ops,
        )?;
        ops.push(StorageWriteOp::PutEntity(StoredEntity {
            collection: collection.lid.0,
            kind: StoredEntityKind::Untyped,
            id: MARKER.into(),
            object: Object::new(),
        }));
        Ok(())
    }

    pub(super) fn update_reverse_references(
        &self,
        catalog: &Catalog,
        changes: &ChangeSet,
        ops: &mut Vec<StorageWriteOp>,
    ) -> Result<(), DbError> {
        let Some(collection) = catalog.collection_by_name(REFERENCES) else {
            return Ok(());
        };
        for (key, change) in changes {
            let before = reference_rows(
                change
                    .before
                    .as_ref()
                    .map(|row| resolved_references(catalog, key, row))
                    .unwrap_or_default(),
            );
            let after = reference_rows(
                change
                    .after
                    .as_ref()
                    .map(|row| resolved_references(catalog, key, row))
                    .unwrap_or_default(),
            );
            self.push_row_delta(catalog, collection.lid, &before, &after, ops)?;
        }
        Ok(())
    }

    pub(super) fn try_compact_batch(
        &mut self,
        catalog: &Catalog,
        batch: &Batch,
        revision: Option<u64>,
        returning: &crate::BatchReturn,
        catalog_version: u64,
    ) -> Result<Option<crate::BatchReply>, DbError> {
        self.execution_counts = ExecutionCounts::default();
        let fallback = if !self.storage.tx_capabilities().conflict_detection || revision.is_none() {
            Some("backend_without_revision_conflicts")
        } else if batch.operations.iter().any(|op| {
            matches!(
                op,
                BatchOperation::Update { .. } | BatchOperation::Delete { .. }
            )
        }) {
            Some("predicate_mutation")
        } else if catalog
            .collection_by_name(REFERENCES)
            .is_none_or(|collection| {
                catalog
                    .find_equality_index(collection.lid, "target")
                    .is_none()
            })
        {
            Some("reverse_references_unavailable")
        } else {
            None
        };
        if let Some(reason) = fallback {
            self.record_compact_fallback(reason);
            return Ok(None);
        }
        let references = catalog.collection_by_name(REFERENCES).unwrap();
        let index = catalog
            .find_equality_index(references.lid, "target")
            .unwrap();
        if self
            .storage
            .get_entity_at_revision(references.lid, MARKER, revision)?
            .is_none()
            || self.storage.index_needs_rebuild(index.lid)?
        {
            self.record_compact_fallback("reverse_reference_backfill_incomplete");
            return Ok(None);
        }
        let mut view = TxView::new(catalog, &self.storage, revision);
        let mut stats = crate::BatchStats {
            upserted: 0,
            updated: 0,
            deleted: 0,
        };
        let context = DefaultExpressionContext::now();
        for operation in &batch.operations {
            match operation {
                BatchOperation::Upsert {
                    collection,
                    id,
                    object,
                }
                | BatchOperation::Create {
                    collection,
                    id,
                    object,
                } => {
                    let key = (collection.clone(), id.clone());
                    if matches!(operation, BatchOperation::Create { .. })
                        && view.get(&key)?.is_some()
                    {
                        return Err(DbError::EntityExists {
                            collection: collection.clone(),
                            id: id.clone(),
                        });
                    }
                    let schema = catalog.collection_by_name(collection).ok_or_else(|| {
                        DbError::UnknownCollectionByName {
                            name: collection.clone(),
                        }
                    })?;
                    let mut object = object.clone();
                    (if self.validation_enabled()? {
                        crate::validation::normalize_for_recursive_validation(
                            catalog,
                            schema,
                            &mut object,
                            Some(&context),
                        )
                    } else {
                        prepare_object_for_write(catalog, schema, &mut object, &context)
                    })
                    .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
                    view.put(key, object)?;
                    stats.upserted += 1;
                }
                BatchOperation::DeleteById { collection, id } => {
                    stats.deleted += usize::from(view.delete((collection.clone(), id.clone()))?);
                }
                BatchOperation::DeleteByIds { collection, ids } => {
                    for id in ids {
                        stats.deleted +=
                            usize::from(view.delete((collection.clone(), id.clone()))?);
                    }
                }
                _ => unreachable!("predicate mutations use fallback"),
            }
        }
        let changes = view.changes();
        for (key, change) in &changes {
            let collection = catalog.collection_by_name(&key.0).unwrap();
            for (_, relation) in catalog.relationships() {
                let relation = &relation.relationship;
                if relation.source_collection == key.0
                    && relation.indexing_mode == RelationIndexingMode::Enabled
                {
                    let contribution = |row: &Object| {
                        Self::contribution(catalog, relation, collection, &key.1, row)
                    };
                    if change.before.as_ref().and_then(contribution)
                        != change.after.as_ref().and_then(contribution)
                    {
                        self.execution_counts = view.counts;
                        self.record_compact_fallback("transitive_relationship_change");
                        return Ok(None);
                    }
                }
            }
        }
        let mut validate = BTreeSet::new();
        for (key, change) in &changes {
            if let Some(object) = &change.after {
                let collection = catalog.collection_by_name(&key.0).unwrap();
                self.validate_primary_id(collection, &key.1, object)?;
                for index in catalog
                    .indexes_for_collection(collection.lid)
                    .filter(|index| index.schema.unique && index.schema.kind == IndexKind::Equality)
                {
                    if let Some(value) = object.get(&index.canonical_field) {
                        let ids = view.unique(index, value)?;
                        if ids.len() > 1 {
                            return Err(DbError::InvalidQuery(format!(
                                "unique index violation on field '{}' ({} vs {})",
                                index.canonical_field, ids[0].1, ids[1].1
                            )));
                        }
                    }
                }
                validate.insert(key.clone());
            }
            if change.before.is_some()
                && (change.after.is_none()
                    || change
                        .before
                        .as_ref()
                        .and_then(|row| row.get(OBJECT_TYPE_FIELD))
                        != change
                            .after
                            .as_ref()
                            .and_then(|row| row.get(OBJECT_TYPE_FIELD)))
            {
                for (owner, _) in view.incoming(key)? {
                    validate.insert(owner);
                }
            }
        }
        for key in validate {
            if let Some(object) = view.get(&key)? {
                validate_row(&mut view, &key, &object)?;
            }
        }
        let (before, after) = change_datasets(&changes);
        // Keep addressed no-op rows available for dynamic projection names, but
        // derive output exclusively from the normalized committing changes.
        let mut projection_before = crate::Dataset::new();
        let mut projection_after = crate::Dataset::new();
        for (key, row) in &view.overlay {
            let old = projection_before.entry(key.0.clone()).or_default();
            let new = projection_after.entry(key.0.clone()).or_default();
            if let Some(object) = view.snapshot.get(key).and_then(Option::as_ref) {
                old.insert(key.1.clone(), object.clone());
            }
            if let Some(object) = row {
                new.insert(key.1.clone(), object.clone());
            }
        }
        let reply = match crate::batch_return::compact_reply_from_changes(
            catalog,
            &projection_before,
            &projection_after,
            &changes,
            &stats,
            returning,
        ) {
            Ok(reply) => reply.expect("compact returning mode"),
            Err(DbError::BatchReturn {
                reason: crate::BatchReturnErrorReason::UnknownField,
                ..
            }) => {
                // Permissive collections can discover a dynamic field on an
                // untouched row. Preserve dataset projection resolution when
                // the addressed rows and catalog cannot establish its name.
                self.execution_counts = view.counts;
                self.record_compact_fallback("projection_field_requires_schema_scan");
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let mut ops = Vec::new();
        for (name, rows) in &after {
            self.push_row_delta(
                catalog,
                catalog.collection_by_name(name).unwrap().lid,
                &before[name],
                rows,
                &mut ops,
            )?;
        }
        self.update_reverse_references(catalog, &changes, &mut ops)?;
        self.update_relationship_edges(catalog, &before, &after, &mut ops)?;
        self.execution_counts = view.counts;
        self.storage.ensure_revision(revision)?;
        if self.catalog.snapshot().version != catalog_version {
            return Err(DbError::TransactionConflict(
                "catalog changed during transaction".into(),
            ));
        }
        match self.storage.apply_batch_conditional(&ops, revision)? {
            StorageCommitOutcome::Committed { .. } => {
                tracing::debug!(
                    point_reads = self.execution_counts.point_reads,
                    index_reads = self.execution_counts.index_reads,
                    visited_rows = self.execution_counts.visited_rows,
                    writes = ops.len(),
                    "compact batch committed"
                );
                Ok(Some(reply))
            }
            StorageCommitOutcome::Conflict {
                expected_revision,
                actual_revision,
            } => Err(DbError::TransactionConflict(format!(
                "expected revision {expected_revision:?}, found {actual_revision:?}"
            ))),
        }
    }

    fn record_compact_fallback(&mut self, reason: &'static str) {
        self.execution_counts.fallback_scans += 1;
        tracing::debug!(
            reason,
            fallback_scans = self.execution_counts.fallback_scans,
            "compact batch scan fallback"
        );
    }
}

fn change_datasets(changes: &ChangeSet) -> (crate::Dataset, crate::Dataset) {
    let mut before = crate::Dataset::new();
    let mut after = crate::Dataset::new();
    for ((collection, id), change) in changes {
        let old = before.entry(collection.clone()).or_default();
        let new = after.entry(collection.clone()).or_default();
        if let Some(object) = &change.before {
            old.insert(id.clone(), object.clone());
        }
        if let Some(object) = &change.after {
            new.insert(id.clone(), object.clone());
        }
    }
    (before, after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BatchReply, BatchReturn, embedded::MemoryEntityStorage};

    fn row(id: &str, name: &str) -> Object {
        Object::from_iter([
            ("id".into(), Value::String(id.into())),
            ("name".into(), Value::String(name.into())),
        ])
    }

    fn upsert(id: &str, name: &str) -> BatchOperation {
        BatchOperation::Upsert {
            collection: "items".into(),
            id: id.into(),
            object: row(id, name),
        }
    }

    fn delete(id: &str) -> BatchOperation {
        BatchOperation::DeleteById {
            collection: "items".into(),
            id: id.into(),
        }
    }

    fn db() -> EmbeddedDb<MemoryEntityStorage> {
        let mut db = EmbeddedDb::in_memory();
        let collection = db
            .create_collection("items", CollectionKind::Polymorphic)
            .unwrap();
        db.create_index("unique_name", collection, "name", true)
            .unwrap();
        db
    }

    #[test]
    fn compact_append_reads_are_independent_of_unrelated_rows() {
        let mut counts = Vec::new();
        for size in [10, 1_000] {
            let mut db = db();
            db.execute_batch(Batch {
                operations: (0..size)
                    .map(|i| upsert(&format!("row-{i}"), &format!("row-{i}")))
                    .collect(),
            })
            .unwrap();
            db.execute_batch_returning(
                Batch::new().with_op(upsert("new", "new")),
                BatchReturn::Changes,
            )
            .unwrap();
            assert_eq!(db.execution_counts.fallback_scans, 0);
            assert_eq!(db.execution_counts.point_reads, 1);
            assert_eq!(db.execution_counts.index_reads, 2);
            assert_eq!(db.execution_counts.visited_rows, 0);
            counts.push(db.execution_counts.clone());
        }
        assert_eq!(counts[0], counts[1]);
    }

    #[test]
    fn compact_matches_dataset_for_repeated_ids_swaps_and_errors() {
        let mut compact = db();
        let mut dataset = db();
        let batches = [
            Batch::new()
                .with_op(upsert("one", "first"))
                .with_op(upsert("two", "second")),
            Batch::new()
                .with_op(upsert("one", "second"))
                .with_op(upsert("two", "first")),
            Batch::new()
                .with_op(delete("one"))
                .with_op(BatchOperation::Create {
                    collection: "items".into(),
                    id: "one".into(),
                    object: row("one", "third"),
                })
                .with_op(delete("absent")),
            Batch::new().with_op(upsert("one", "first")),
            Batch::new().with_op(BatchOperation::Create {
                collection: "items".into(),
                id: "one".into(),
                object: row("one", "oops"),
            }),
            Batch::new()
                .with_op(upsert("transient", "transient"))
                .with_op(delete("transient"))
                .with_op(upsert("one", "third")),
        ];
        for batch in batches {
            let before = dataset
                .collection_rows(dataset.catalog().collection_by_name("items").unwrap().lid)
                .unwrap();
            let slow = dataset.execute_batch(batch.clone());
            let fast = compact.execute_batch_returning(batch, BatchReturn::Changes);
            match (slow, fast) {
                (Ok(outcome), Ok(reply)) => {
                    let before = BTreeMap::from([(
                        "items".into(),
                        before.into_iter().map(|row| (row.id, row.object)).collect(),
                    )]);
                    let expected = crate::batch_return::compact_reply(
                        &dataset.catalog(),
                        &before,
                        &outcome,
                        &BatchReturn::Changes,
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!(reply, expected);
                }
                (Err(slow), Err(fast)) => assert_eq!(slow.to_string(), fast.to_string()),
                (slow, fast) => panic!("executor mismatch: {slow:?} vs {fast:?}"),
            }
            assert_eq!(
                compact
                    .collection_rows(compact.catalog().collection_by_name("items").unwrap().lid)
                    .unwrap(),
                dataset
                    .collection_rows(dataset.catalog().collection_by_name("items").unwrap().lid)
                    .unwrap()
            );
        }
    }

    fn typed(id: &str, ty: &str, author: Option<&str>) -> BatchOperation {
        let mut object = Object::new();
        object.insert("id", id.to_string());
        object.insert("type", format!("local:{ty}"));
        if let Some(author) = author {
            object.insert("author", author.to_string());
        }
        BatchOperation::Upsert {
            collection: DEFAULT_COLLECTION.into(),
            id: id.into(),
            object,
        }
    }

    fn delete_typed(id: &str) -> BatchOperation {
        BatchOperation::DeleteById {
            collection: DEFAULT_COLLECTION.into(),
            id: id.into(),
        }
    }

    fn ref_db() -> EmbeddedDb<MemoryEntityStorage> {
        let mut db = EmbeddedDb::in_memory();
        super::super::tests::register_ref_schema(&mut db, super::super::tests::ref_ty("person"));
        db
    }

    #[test]
    fn compact_refs_validate_final_targets_and_incoming_dependents_after_reopen() {
        let mut db = ref_db();
        let loaded = load_catalog(&db.storage, &fresh_catalog_with_core_schema().unwrap())
            .unwrap()
            .unwrap();
        assert!(
            loaded.classes().next().is_some(),
            "catalog reload must preserve class projections"
        );
        db.execute_batch_returning(
            Batch::new()
                .with_op(typed("article", "article", Some("person")))
                .with_op(typed("person", "person", None)),
            BatchReturn::Stats,
        )
        .unwrap();
        let classes: BTreeSet<_> = db
            .catalog()
            .classes()
            .map(|(_, class)| class.class.id.clone())
            .collect();
        let (_, storage) = db.into_parts();
        let mut db = EmbeddedDb::open(storage).unwrap();
        assert_eq!(
            classes,
            db.catalog()
                .classes()
                .map(|(_, class)| class.class.id.clone())
                .collect()
        );
        let revision = db.storage.current_revision().unwrap();
        for operation in [
            delete_typed("person"),
            typed("person", "organization", None),
        ] {
            let error = db
                .execute_batch_returning(Batch::new().with_op(operation), BatchReturn::Changes)
                .unwrap_err();
            assert!(error.to_string().contains("ref field"), "{error}");
            assert_eq!(db.storage.current_revision().unwrap(), revision);
        }
        // Both directions see the final overlay: retyping is valid once the
        // dependent disappears, even when it is removed later in the batch.
        db.execute_batch_returning(
            Batch::new()
                .with_op(typed("person", "organization", None))
                .with_op(delete_typed("article")),
            BatchReturn::Stats,
        )
        .unwrap();
        db.execute_batch_returning(
            Batch::new().with_op(delete_typed("person")),
            BatchReturn::Stats,
        )
        .unwrap();
        let catalog = db.catalog();
        let mut view = TxView::new(
            &catalog,
            &db.storage,
            db.storage.current_revision().unwrap(),
        );
        assert!(
            view.incoming(&(DEFAULT_COLLECTION.into(), "person".into()))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn compact_refs_match_dataset_errors_and_only_read_targets() {
        for batch in [
            Batch::new().with_op(typed("article", "article", Some("missing"))),
            Batch::new()
                .with_op(typed("article", "article", Some("target")))
                .with_op(typed("target", "organization", None)),
        ] {
            let mut fast = ref_db();
            let mut slow = ref_db();
            assert_eq!(
                fast.execute_batch_returning(batch.clone(), BatchReturn::Stats)
                    .unwrap_err()
                    .to_string(),
                slow.execute_batch(batch).unwrap_err().to_string()
            );
        }
        let mut db = ref_db();
        db.execute_batch(Batch::new().with_op(typed("person", "person", None)))
            .unwrap();
        db.execute_batch_returning(
            Batch::new().with_op(typed("article", "article", Some("person"))),
            BatchReturn::Stats,
        )
        .unwrap();
        assert_eq!(db.execution_counts.point_reads, 2);
        assert_eq!(db.execution_counts.fallback_scans, 0);
    }

    #[test]
    fn compact_reverse_reference_backfill_and_predicate_fallback() {
        let mut db = ref_db();
        db.execute_batch(
            Batch::new()
                .with_op(typed("person", "person", None))
                .with_op(typed("article", "article", Some("person"))),
        )
        .unwrap();
        let references = db.catalog().collection_by_name(REFERENCES).unwrap().lid;
        db.storage
            .apply_batch(&[StorageWriteOp::ClearCollection(references)])
            .unwrap();
        let (_, storage) = db.into_parts();
        let mut db = EmbeddedDb::open(storage).unwrap();
        assert!(
            db.execute_batch_returning(
                Batch::new().with_op(delete_typed("person")),
                BatchReturn::Stats
            )
            .is_err()
        );
        let reply = db
            .execute_batch_returning(
                Batch::new().with_op(BatchOperation::Delete {
                    collection: DEFAULT_COLLECTION.into(),
                    query: DeleteQuery::new(),
                }),
                BatchReturn::Stats,
            )
            .unwrap();
        assert!(matches!(reply, BatchReply::Stats { stats } if stats.deleted == 2));
        assert_eq!(db.execution_counts.fallback_scans, 1);
        assert_eq!(db.execution_counts.visited_rows, 2);
    }

    #[test]
    fn compact_revision_fence_rejects_stale_reads() {
        let mut db = db();
        let revision = db.storage.current_revision().unwrap();
        db.execute_batch(Batch::new().with_op(upsert("one", "one")))
            .unwrap();
        let catalog = db.catalog();
        let mut view = TxView::new(&catalog, &db.storage, revision);
        assert!(matches!(
            view.get(&("items".into(), "one".into())),
            Err(DbError::TransactionConflict(_))
        ));
        let index = catalog
            .indexes_for_collection(catalog.collection_by_name("items").unwrap().lid)
            .find(|index| index.schema.unique)
            .unwrap();
        assert!(matches!(
            view.unique(index, &Value::String("one".into())),
            Err(DbError::TransactionConflict(_))
        ));
    }
}
