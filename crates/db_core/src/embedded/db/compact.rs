//! Point/index execution for ID batches. Every lookup is revision-bound and
//! adjusted for the transaction overlay before final-state validation.
use super::*;
use crate::StorageErrorKind;
use crate::batch_return::{ChangeSet, EntityKey, RowChange};
use crate::embedded::storage::RevisionReader;

const REFERENCES: &str = "__semantic.reverse_references";
const TARGET_INDEX: &str = "__reverse_reference_target_idx";
const MARKER: &str = "backfill:v1";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionCounts {
    pub point_reads: usize,
    pub index_reads: usize,
    /// Collection scans made by predicate mutations without an index.
    pub collection_scans: usize,
    pub fallback_scans: usize,
    pub visited_rows: usize,
}

pub(crate) use crate::ResolvedReference;

/// Result of building a point-path reply.
pub(super) enum CompactReply<R> {
    Ready(R),
    /// The dataset path must execute the transaction, for the given reason.
    Fallback(&'static str),
}

/// Revision-bound reads of one transaction.
///
/// Implemented by [`RevisionReader`] (reads of a single-call write borrowing
/// the storage) and by the owned snapshot reader of interactive
/// transactions.
pub(crate) trait TxRead {
    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError>;

    fn scan_index_value(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<Vec<String>, DbError>;

    /// Ids of the entries of `index` within `range`, in key order.
    fn scan_index_range_ids(
        &self,
        index: LocalIndexId,
        range: &crate::IndexScanRange,
    ) -> Result<Vec<String>, DbError>;

    /// Stream every entity of `collection` into `visit`.
    fn scan_collection(
        &self,
        collection: LocalCollectionId,
        visit: &mut dyn FnMut(StoredEntity) -> Result<(), DbError>,
    ) -> Result<(), DbError>;

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError>;

    /// A snapshot to read planner statistics from. Statistics only steer
    /// access path selection, so the handle need not be consistent.
    fn planning_snapshot(&self) -> Result<QueryReader<'_>, DbError>;
}

impl<S: EntityStorage> TxRead for RevisionReader<'_, S> {
    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        RevisionReader::get_entity(self, collection, id)
    }

    fn scan_index_value(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<Vec<String>, DbError> {
        RevisionReader::scan_index_value(self, index, path, value)
    }

    fn scan_index_range_ids(
        &self,
        index: LocalIndexId,
        range: &crate::IndexScanRange,
    ) -> Result<Vec<String>, DbError> {
        RevisionReader::scan_index_range_ids(self, index, range)
    }

    fn scan_collection(
        &self,
        collection: LocalCollectionId,
        visit: &mut dyn FnMut(StoredEntity) -> Result<(), DbError>,
    ) -> Result<(), DbError> {
        RevisionReader::scan_collection(self, collection, visit)
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        RevisionReader::index_needs_rebuild(self, index)
    }

    fn planning_snapshot(&self) -> Result<QueryReader<'_>, DbError> {
        Ok(match self.snapshot() {
            Some(snapshot) => QueryReader::Ref(snapshot),
            None => QueryReader::Borrowed(self.storage().snapshot()?),
        })
    }
}

/// Transaction-local state: the rows a transaction read and wrote.
///
/// Owned and `'static`, so interactive transactions can keep it between
/// statements; [`TxView`] pairs it with a reader for each statement.
#[derive(Debug, Default)]
pub(crate) struct TxState {
    /// Rows read from the snapshot (`None`: absent), so repeated reads and
    /// the final change set agree.
    snapshot: BTreeMap<EntityKey, Option<Object>>,
    /// Rows written by the transaction (`None`: deleted).
    overlay: BTreeMap<EntityKey, Option<Object>>,
    /// While savepoints exist: the previous overlay entry of every write
    /// since the oldest savepoint, oldest first.
    undo: Option<Vec<(EntityKey, Option<Option<Object>>)>>,
    /// Existing rows deleted since the last [`Self::take_deleted`].
    deleted: Vec<EntityKey>,
    pub counts: ExecutionCounts,
}

impl TxState {
    /// Rows written by the transaction (`None`: deleted).
    pub(crate) fn overlay(&self) -> &BTreeMap<EntityKey, Option<Object>> {
        &self.overlay
    }

    fn write(&mut self, key: EntityKey, row: Option<Object>) {
        let previous = self.overlay.insert(key.clone(), row);
        if let Some(undo) = &mut self.undo {
            undo.push((key, previous));
        }
    }

    /// Position of the undo log, recording undo entries from now on.
    pub(crate) fn undo_position(&mut self) -> usize {
        self.undo.get_or_insert_with(Vec::new).len()
    }

    /// Undo every write after undo log `position`.
    pub(crate) fn rollback_to(&mut self, position: usize) {
        let Some(undo) = &mut self.undo else {
            return;
        };
        while undo.len() > position {
            let (key, previous) = undo.pop().expect("undo entry");
            match previous {
                Some(row) => self.overlay.insert(key, row),
                None => self.overlay.remove(&key),
            };
        }
        self.deleted.clear();
    }

    /// Stop recording undo entries (when no savepoint needs them).
    pub(crate) fn stop_undo(&mut self) {
        self.undo = None;
    }

    /// Rows deleted since the last call.
    pub(crate) fn take_deleted(&mut self) -> Vec<EntityKey> {
        std::mem::take(&mut self.deleted)
    }
}

/// Reads of one statement or commit of a transaction: its state, read
/// through `reader`.
pub(crate) struct TxView<'a> {
    pub catalog: &'a Catalog,
    reader: &'a dyn TxRead,
    state: &'a mut TxState,
}

impl<'a> TxView<'a> {
    /// A view reading `state`'s rows through `reader`.
    pub(crate) fn new(
        catalog: &'a Catalog,
        reader: &'a dyn TxRead,
        state: &'a mut TxState,
    ) -> Self {
        Self {
            catalog,
            reader,
            state,
        }
    }

    /// Revision-bound reads without the transaction overlay.
    pub(crate) fn reader(&self) -> &'a dyn TxRead {
        self.reader
    }

    pub(crate) fn state(&self) -> &TxState {
        self.state
    }

    pub(crate) fn get(&mut self, key: &EntityKey) -> Result<Option<Object>, DbError> {
        if let Some(row) = self
            .state
            .overlay
            .get(key)
            .or_else(|| self.state.snapshot.get(key))
        {
            return Ok(row.clone());
        }
        let collection = self.catalog.collection_by_name(&key.0).ok_or_else(|| {
            DbError::UnknownCollectionByName {
                name: key.0.clone(),
            }
        })?;
        let row = self
            .reader
            .get_entity(collection.lid, &key.1)?
            .map(|row| row.object);
        self.state.counts.point_reads += 1;
        self.state.counts.visited_rows += usize::from(row.is_some());
        self.state.snapshot.insert(key.clone(), row.clone());
        Ok(row)
    }

    pub(crate) fn put(&mut self, key: EntityKey, object: Object) -> Result<(), DbError> {
        self.get(&key)?;
        self.state.write(key, Some(object));
        Ok(())
    }

    pub(crate) fn delete(&mut self, key: EntityKey) -> Result<bool, DbError> {
        let exists = self.get(&key)?.is_some();
        if exists {
            self.state.deleted.push(key.clone());
        }
        self.state.write(key, None);
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
            .ok_or_else(|| {
                DbError::storage(StorageErrorKind::InvalidState, "index collection missing")
            })?;
        self.state.counts.index_reads += 1;
        let mut ids: BTreeSet<String> = self
            .reader
            .scan_index_value(index.lid, None, value)?
            .into_iter()
            .collect();
        for ((name, id), object) in &self.state.overlay {
            if name != &collection.name {
                continue;
            }
            ids.remove(id);
            if object
                .as_ref()
                .and_then(|object| index.key_value(object))
                .as_ref()
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
        let collection = self.catalog.collection_by_name(REFERENCES).ok_or_else(|| {
            DbError::storage(
                StorageErrorKind::InvalidState,
                "reverse references are not initialized",
            )
        })?;
        let index = self
            .catalog
            .find_equality_index(collection.lid, "target")
            .ok_or_else(|| {
                DbError::storage(
                    StorageErrorKind::InvalidState,
                    "reverse reference index missing",
                )
            })?;
        self.state.counts.index_reads += 1;
        let ids =
            self.reader
                .scan_index_value(index.lid, None, &Value::String(entity_key(target)))?;
        let mut refs = BTreeSet::new();
        for id in ids {
            let row = self
                .reader
                .get_entity(collection.lid, &id)?
                .ok_or_else(|| {
                    corrupt_reference("reverse reference index points to missing entry")
                })?;
            self.state.counts.point_reads += 1;
            self.state.counts.visited_rows += 1;
            let owner = (
                required_string(&row.object, "collection")?,
                required_string(&row.object, "owner")?,
            );
            if !self.state.overlay.contains_key(&owner) {
                refs.insert((owner, read_path(&row.object)?));
            }
        }
        for (owner, row) in &self.state.overlay {
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

    /// Ids of the snapshot's `index` entries equal to `value`.
    pub(crate) fn index_ids(
        &mut self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<Vec<String>, DbError> {
        self.state.counts.index_reads += 1;
        self.reader.scan_index_value(index, path, value)
    }

    /// Ids of the snapshot's `index` entries within `ranges`.
    pub(crate) fn index_range_ids(
        &mut self,
        index: LocalIndexId,
        ranges: &[crate::IndexScanRange],
    ) -> Result<Vec<String>, DbError> {
        let mut ids = Vec::new();
        for range in ranges {
            self.state.counts.index_reads += 1;
            ids.extend(self.reader.scan_index_range_ids(index, range)?);
        }
        Ok(ids)
    }

    /// Current rows of `collection` with the given ids, plus every row of the
    /// collection the transaction has written, ordered by id.
    pub(crate) fn rows_by_ids(
        &mut self,
        collection: &str,
        ids: impl IntoIterator<Item = String>,
    ) -> Result<BTreeMap<String, Object>, DbError> {
        let mut ids = ids.into_iter().collect::<BTreeSet<_>>();
        ids.extend(self.overlay_ids(collection));
        let mut rows = BTreeMap::new();
        for id in ids {
            let key = (collection.to_string(), id);
            if let Some(row) = self.get(&key)? {
                rows.insert(key.1, row);
            }
        }
        Ok(rows)
    }

    /// Current rows of `collection` matching `predicate`, ordered by id.
    ///
    /// Stored rows come from one streamed collection scan of the snapshot;
    /// rows that do not match are dropped as they are read. Rows the
    /// transaction has written are taken from the overlay instead.
    pub(crate) fn scan_matching(
        &mut self,
        collection: &CollectionSchema,
        predicate: Option<&crate::Expr>,
    ) -> Result<BTreeMap<String, Object>, DbError> {
        let written = self.overlay_ids(&collection.name);
        let mut rows = BTreeMap::new();
        let mut visited = 0;
        self.state.counts.collection_scans += 1;
        self.reader.scan_collection(collection.lid, &mut |entity| {
            visited += 1;
            if !written.contains(&entity.id) && matches_predicate(&entity.object, predicate) {
                rows.insert(entity.id, entity.object);
            }
            Ok(())
        })?;
        self.state.counts.visited_rows += visited;
        for (id, row) in &rows {
            self.state
                .snapshot
                .entry((collection.name.clone(), id.clone()))
                .or_insert_with(|| Some(row.clone()));
        }
        for id in written {
            let key = (collection.name.clone(), id);
            if let Some(row) = self.get(&key)?
                && matches_predicate(&row, predicate)
            {
                rows.insert(key.1, row);
            }
        }
        Ok(rows)
    }

    fn overlay_ids(&self, collection: &str) -> BTreeSet<String> {
        self.state
            .overlay
            .keys()
            .filter(|(name, _)| name == collection)
            .map(|(_, id)| id.clone())
            .collect()
    }

    pub(crate) fn changes(&self) -> ChangeSet {
        self.state
            .overlay
            .iter()
            .filter_map(|(key, after)| {
                let before = self.state.snapshot.get(key).cloned().flatten();
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

fn matches_predicate(row: &Object, predicate: Option<&crate::Expr>) -> bool {
    predicate.is_none_or(|predicate| crate::evaluate_filter_expr(row, predicate))
}

/// Unique violation between the two smallest ids holding `value` in `index`.
pub(super) fn unique_violation(
    collection: &CollectionSchema,
    index: &crate::catalog::IndexSchema,
    value: &Value,
    existing_id: &str,
    id: &str,
) -> DbError {
    DbError::UniqueViolation {
        collection: collection.name.clone(),
        index: index.schema.name.clone(),
        field: index.columns().collect::<Vec<_>>().join(", "),
        value: Box::new(value.clone()),
        existing_id: existing_id.to_string(),
        id: id.to_string(),
    }
}

fn required_string(object: &Object, field: &str) -> Result<String, DbError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| corrupt_reference(format!("invalid reverse reference {field}")))
}

fn read_path(object: &Object) -> Result<FieldPath, DbError> {
    let Some(Value::List(values)) = object.get("path") else {
        return Err(corrupt_reference("invalid reverse reference path"));
    };
    values
        .iter()
        .map(|value| match value {
            Value::String(field) => Ok(PathSegment::Field(field.clone())),
            Value::U64(index) => usize::try_from(*index)
                .map(PathSegment::Index)
                .map_err(|_| corrupt_reference("invalid reverse reference index")),
            _ => Err(corrupt_reference("invalid reverse reference path segment")),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(FieldPath)
}

fn corrupt_reference(message: impl Into<String>) -> DbError {
    DbError::storage(StorageErrorKind::Corruption, message)
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

pub(crate) fn validate_row(
    view: &mut TxView<'_>,
    key: &EntityKey,
    object: &Object,
    settings: crate::WriteSettings,
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
        crate::validate_stored_object_with_settings(
            view.catalog,
            key,
            object,
            |target| view.get(target),
            settings,
        )?;
        return Ok(());
    }
    let collection = view.catalog.collection_by_name(&key.0).ok_or_else(|| {
        DbError::UnknownCollectionByName {
            name: key.0.clone(),
        }
    })?;
    let mut targets = BTreeMap::new();
    if settings.validate_foreign_keys {
        for reference in resolved_references(view.catalog, key, object) {
            if let Some(target) = view.get(&reference.target)? {
                targets.insert(reference.target.1, target);
            }
        }
    }
    for (field, ty) in resolved_field_types_for_object(view.catalog, collection, object) {
        if let Some(value) = object.get(&field) {
            validate_ref_value(
                view.catalog,
                collection,
                &targets,
                &field,
                &ty,
                value,
                settings.validate_foreign_keys,
            )?;
        }
    }
    Ok(())
}

fn expand_cascade_deletes(
    view: &mut TxView<'_>,
    stats: &mut crate::BatchStats,
) -> Result<(), DbError> {
    let deleted = view
        .changes()
        .into_iter()
        .filter_map(|(key, change)| {
            (change.before.is_some() && change.after.is_none()).then_some(key)
        })
        .collect::<Vec<_>>();
    cascade_deletes_from(view, deleted, stats)
}

/// Delete the rows whose cascading references point at the `deleted` rows,
/// transitively.
pub(crate) fn cascade_deletes_from(
    view: &mut TxView<'_>,
    deleted: impl IntoIterator<Item = EntityKey>,
    stats: &mut crate::BatchStats,
) -> Result<(), DbError> {
    let mut pending = deleted
        .into_iter()
        .collect::<std::collections::VecDeque<_>>();
    let mut visited = BTreeSet::new();

    while let Some(target) = pending.pop_front() {
        if !visited.insert(target.clone()) {
            continue;
        }
        for (owner, path) in view.incoming(&target)? {
            let Some(row) = view.get(&owner)? else {
                continue;
            };
            let cascades = resolved_references(view.catalog, &owner, &row)
                .into_iter()
                .any(|reference| {
                    reference.target == target
                        && reference.path == path
                        && reference.on_delete == semantic_data::schema::OnDelete::Cascade
                });
            if cascades && view.delete(owner.clone())? {
                stats.deleted += 1;
                pending.push_back(owner);
            }
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
        scope: TxScope<'_>,
        batch: &Batch,
        returning: &crate::BatchReturn,
        catalog_version: u64,
        settings: crate::WriteSettings,
        require_bounded: bool,
    ) -> Result<Option<crate::BatchReply>, DbError> {
        let catalog = scope.catalog;
        self.run_compact(
            scope,
            catalog_version,
            settings,
            require_bounded,
            |db, view, stats| {
                let context = DefaultExpressionContext::now();
                let recursive_validation = db.validation_enabled()?;
                let query_context = db.query_context();
                for operation in &batch.operations {
                    apply_tx_operation(
                        view,
                        &query_context,
                        operation,
                        stats,
                        &context,
                        recursive_validation,
                    )?;
                }
                Ok(())
            },
            |view, changes, stats, ()| {
                // Keep addressed no-op rows available for dynamic projection
                // names, but derive output exclusively from the normalized
                // committing changes.
                let mut projection_before = crate::Dataset::new();
                let mut projection_after = crate::Dataset::new();
                let state = view.state();
                for (key, row) in &state.overlay {
                    let old = projection_before.entry(key.0.clone()).or_default();
                    let new = projection_after.entry(key.0.clone()).or_default();
                    if let Some(object) = state.snapshot.get(key).and_then(Option::as_ref) {
                        old.insert(key.1.clone(), object.clone());
                    }
                    if let Some(object) = row {
                        new.insert(key.1.clone(), object.clone());
                    }
                }
                match crate::batch_return::compact_reply_from_changes(
                    catalog,
                    &projection_before,
                    &projection_after,
                    changes,
                    stats,
                    returning,
                ) {
                    Ok(reply) => Ok(CompactReply::Ready(reply.expect("compact returning mode"))),
                    // Permissive collections can discover a dynamic field on
                    // an untouched row. Preserve dataset projection resolution
                    // when the addressed rows and catalog cannot establish its
                    // name.
                    Err(DbError::BatchReturn {
                        reason: crate::BatchReturnErrorReason::UnknownField,
                        ..
                    }) => Ok(CompactReply::Fallback(
                        "projection_field_requires_schema_scan",
                    )),
                    Err(error) => Err(error),
                }
            },
        )
    }

    /// One point-path transaction attempt.
    ///
    /// `apply` stages the mutations in the transaction overlay; cascades,
    /// final-state validation, index and relationship maintenance and the
    /// revision-conditional commit are shared by every caller. `reply` builds
    /// the result from the validated changes. Returns `None` (after recording
    /// the reason) when the dataset path must execute the transaction instead.
    pub(super) fn run_compact<T, R>(
        &mut self,
        scope: TxScope<'_>,
        catalog_version: u64,
        settings: crate::WriteSettings,
        require_bounded: bool,
        apply: impl FnOnce(&Self, &mut TxView<'_>, &mut crate::BatchStats) -> Result<T, DbError>,
        reply: impl FnOnce(
            &TxView<'_>,
            &ChangeSet,
            &crate::BatchStats,
            T,
        ) -> Result<CompactReply<R>, DbError>,
    ) -> Result<Option<R>, DbError> {
        let TxScope {
            catalog, revision, ..
        } = scope;
        self.execution_counts = ExecutionCounts::default();
        if let Some(reason) = self.compact_unsupported(catalog, revision) {
            self.record_compact_fallback(reason);
            return Ok(None);
        }
        let reader = scope.reader(&self.storage)?;
        if reverse_references_incomplete(catalog, &reader)? {
            drop(reader);
            self.record_compact_fallback("reverse_reference_backfill_incomplete");
            return Ok(None);
        }
        let mut state = TxState::default();
        let mut stats = crate::BatchStats {
            upserted: 0,
            updated: 0,
            deleted: 0,
        };
        let prepared = (|| {
            let mut view = TxView::new(catalog, &reader, &mut state);
            let value = apply(self, &mut view, &mut stats)?;
            self.prepare_compact_commit(
                &mut view,
                &mut stats,
                settings,
                require_bounded,
                |view, changes, stats| reply(view, changes, stats, value),
            )
        })();
        drop(reader);
        self.execution_counts = state.counts;
        let prepared = match prepared? {
            CompactReply::Ready(prepared) => prepared,
            CompactReply::Fallback(reason) => {
                self.record_compact_fallback(reason);
                return Ok(None);
            }
        };
        self.commit_compact_ops(
            revision,
            catalog_version,
            &prepared.ops,
            crate::ChangeSource::Batch,
            prepared.changes,
        )?;
        Ok(Some(prepared.reply))
    }

    /// Why the point path cannot run at `revision` under `catalog`, or
    /// `None` when it can: it needs commit-time conflict detection and
    /// reverse references (whose backfill [`reverse_references_incomplete`]
    /// checks in the snapshot).
    pub(super) fn compact_unsupported(
        &self,
        catalog: &Catalog,
        revision: Option<u64>,
    ) -> Option<&'static str> {
        if !self.storage.tx_capabilities().conflict_detection || revision.is_none() {
            Some("backend_without_revision_conflicts")
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
        }
    }

    /// Validate the staged changes of `view` and derive their storage
    /// writes.
    ///
    /// Expands cascade deletes, then checks primary ids, unique indexes and
    /// references against the final state (including the incoming
    /// references of deleted or retyped rows), and derives entity, index,
    /// reverse reference and relationship edge writes. `reply` builds the
    /// result from the validated changes.
    pub(super) fn prepare_compact_commit<R>(
        &self,
        view: &mut TxView<'_>,
        stats: &mut crate::BatchStats,
        settings: crate::WriteSettings,
        require_bounded: bool,
        reply: impl FnOnce(
            &TxView<'_>,
            &ChangeSet,
            &crate::BatchStats,
        ) -> Result<CompactReply<R>, DbError>,
    ) -> Result<CompactReply<PreparedCommit<R>>, DbError> {
        let catalog = view.catalog;
        expand_cascade_deletes(view, stats)?;
        let changes = view.changes();
        if require_bounded {
            for (key, change) in &changes {
                let collection = catalog.collection_by_name(&key.0).unwrap();
                for (_, schema) in catalog.relationships() {
                    let relationship = &schema.relationship;
                    if relationship.source_collection != key.0
                        || relationship.indexing_mode != RelationIndexingMode::Enabled
                    {
                        continue;
                    }
                    let contribution = |row: &Object| {
                        Self::contribution(catalog, relationship, collection, &key.1, row)
                    };
                    if change.before.as_ref().and_then(contribution)
                        != change.after.as_ref().and_then(contribution)
                    {
                        return Err(DbError::InvalidQuery(format!(
                            "bounded batch execution cannot update indexed relationship '{}'",
                            relationship.id
                        )));
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
                    .filter(|index| index.schema.unique && index.schema.kind.is_value_index())
                {
                    if let Some(value) = index.key_value(object) {
                        let ids = view.unique(index, &value)?;
                        if ids.len() > 1 {
                            return Err(unique_violation(
                                collection, index, &value, &ids[0].1, &ids[1].1,
                            ));
                        }
                    }
                }
                validate.insert(key.clone());
            }
            if settings.validate_foreign_keys
                && change.before.is_some()
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
                validate_row(view, &key, &object, settings)?;
            }
        }
        let reply = match reply(view, &changes, stats)? {
            CompactReply::Ready(reply) => reply,
            CompactReply::Fallback(reason) => return Ok(CompactReply::Fallback(reason)),
        };
        let (before, after) = change_datasets(&changes);
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
        Ok(CompactReply::Ready(PreparedCommit {
            ops,
            changes,
            reply,
        }))
    }

    /// Commit `ops` if the storage is still at `revision` and the catalog at
    /// `catalog_version`; otherwise fail with a transaction conflict.
    /// Publishes `changes` and returns the committed revision.
    pub(super) fn commit_compact_ops(
        &mut self,
        revision: Option<u64>,
        catalog_version: u64,
        ops: &[StorageWriteOp],
        source: crate::ChangeSource,
        changes: ChangeSet,
    ) -> Result<Option<u64>, DbError> {
        self.storage.ensure_revision(revision)?;
        if self.catalog.snapshot().version != catalog_version {
            return Err(DbError::TransactionConflict(
                "catalog changed during transaction".into(),
            ));
        }
        match self.commit_write(ops, revision, CommitIntent::data(source), changes)? {
            StorageCommitOutcome::Committed { revision } => {
                tracing::debug!(
                    point_reads = self.execution_counts.point_reads,
                    index_reads = self.execution_counts.index_reads,
                    collection_scans = self.execution_counts.collection_scans,
                    visited_rows = self.execution_counts.visited_rows,
                    writes = ops.len(),
                    "compact batch committed"
                );
                Ok(revision)
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

/// Validated staged changes of a transaction, ready to commit.
pub(super) struct PreparedCommit<R> {
    pub ops: Vec<StorageWriteOp>,
    /// Net row changes, published once committed.
    pub changes: ChangeSet,
    pub reply: R,
}

/// Apply one batch operation to the transaction overlay.
pub(crate) fn apply_tx_operation(
    view: &mut TxView<'_>,
    query_context: &QueryContext,
    operation: &BatchOperation,
    stats: &mut crate::BatchStats,
    context: &DefaultExpressionContext,
    recursive_validation: bool,
) -> Result<(), DbError> {
    let catalog = view.catalog;
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
            if matches!(operation, BatchOperation::Create { .. }) && view.get(&key)?.is_some() {
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
            prepare_row_for_write(catalog, schema, &mut object, context, recursive_validation)
                .map_err(|err| DbError::InvalidQuery(err.to_string()))?;
            view.put(key, object)?;
            stats.upserted += 1;
        }
        BatchOperation::DeleteById { collection, id } => {
            stats.deleted += usize::from(view.delete((collection.clone(), id.clone()))?);
        }
        BatchOperation::DeleteByIds { collection, ids } => {
            for id in ids {
                stats.deleted += usize::from(view.delete((collection.clone(), id.clone()))?);
            }
        }
        BatchOperation::Update { collection, query } => {
            let result = super::mutation::tx_update(
                view,
                query_context,
                collection,
                query,
                context,
                recursive_validation,
            )?;
            stats.updated += result.stats.affected;
        }
        BatchOperation::Delete { collection, query } => {
            stats.deleted +=
                super::mutation::tx_delete(view, query_context, collection, query)?.deleted;
        }
    }
    Ok(())
}

/// Whether the reverse reference backfill is missing from `reader`'s
/// snapshot. Requires reverse references in `catalog` (see
/// [`EmbeddedDb::compact_unsupported`]).
pub(super) fn reverse_references_incomplete(
    catalog: &Catalog,
    reader: &dyn TxRead,
) -> Result<bool, DbError> {
    let references = catalog.collection_by_name(REFERENCES).unwrap();
    let index = catalog
        .find_equality_index(references.lid, "target")
        .unwrap();
    Ok(reader.get_entity(references.lid, MARKER)?.is_none()
        || reader.index_needs_rebuild(index.lid)?)
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
        let reader =
            RevisionReader::new(&db.storage, db.storage.current_revision().unwrap()).unwrap();
        assert!(reader.is_snapshot(), "current revisions use one snapshot");
        let mut state = TxState::default();
        let mut view = TxView::new(&catalog, &reader, &mut state);
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
    fn write_settings_disable_only_foreign_key_checks_for_one_batch() {
        let mut db = ref_db();
        let relaxed = crate::WriteSettings {
            validate_foreign_keys: false,
        };
        db.execute_batch_returning_with_settings(
            Batch::new().with_op(typed("article", "article", Some("target"))),
            BatchReturn::Stats,
            relaxed,
        )
        .unwrap();
        assert_eq!(db.execution_counts.fallback_scans, 0);

        assert!(
            db.execute_batch_returning(
                Batch::new().with_op(typed("other", "article", Some("still-missing"))),
                BatchReturn::Stats,
            )
            .is_err(),
            "default writes must resume foreign-key validation"
        );

        db.execute_batch_returning(
            Batch::new().with_op(typed("target", "person", None)),
            BatchReturn::Stats,
        )
        .unwrap();
        assert!(
            db.execute_batch_returning(
                Batch::new().with_op(delete_typed("target")),
                BatchReturn::Stats,
            )
            .is_err(),
            "reverse-reference bookkeeping must survive relaxed writes"
        );

        let mut db = ref_db();
        db.execute_batch_returning(
            Batch::new()
                .with_op(typed("target", "person", None))
                .with_op(typed("article", "article", Some("target"))),
            BatchReturn::Stats,
        )
        .unwrap();
        db.execute_batch_returning_with_settings(
            Batch::new().with_op(typed("target", "organization", None)),
            BatchReturn::Stats,
            relaxed,
        )
        .unwrap();
        assert_eq!(
            db.execution_counts.index_reads, 1,
            "relaxed target type changes must only read the primary uniqueness index"
        );
        assert_eq!(
            db.execution_counts.visited_rows, 1,
            "relaxed target type changes must not materialize incoming owners"
        );
        assert!(
            db.execute_batch_returning(
                Batch::new().with_op(delete_typed("target")),
                BatchReturn::Stats,
            )
            .is_err(),
            "relaxed type changes must preserve reverse-reference bookkeeping"
        );
    }

    #[test]
    fn indexed_relationship_changes_stay_on_point_batch_path() {
        let mut db = db();
        db.upsert_relationship(RelationType {
            id: "test.chain".into(),
            name: "chain".into(),
            source_collection: DEFAULT_COLLECTION.into(),
            mode: RelationMode::External,
            indexing_mode: RelationIndexingMode::Enabled,
            meta: semantic_data::schema::Meta::default(),
        })
        .unwrap();
        db.upsert_relationship(RelationType {
            id: "test.direct".into(),
            name: "direct".into(),
            source_collection: DEFAULT_COLLECTION.into(),
            mode: RelationMode::External,
            indexing_mode: RelationIndexingMode::Disabled,
            meta: semantic_data::schema::Meta::default(),
        })
        .unwrap();
        let relation = |relation: &str, id: &str, source: &str, target: &str| {
            let mut object = Object::new();
            object.insert("id", Value::String(id.into()));
            object.insert(ATTR_RELATION_RELATION, Value::String(relation.into()));
            object.insert(ATTR_RELATION_FROM, Value::String(source.into()));
            object.insert(ATTR_RELATION_TO, Value::String(target.into()));
            BatchOperation::Upsert {
                collection: DEFAULT_COLLECTION.into(),
                id: id.into(),
                object,
            }
        };
        let endpoint = |id: &str| BatchOperation::Upsert {
            collection: DEFAULT_COLLECTION.into(),
            id: id.into(),
            object: Object::from_iter([("id".into(), Value::String(id.into()))]),
        };
        db.execute_batch_returning(
            Batch::new()
                .with_op(endpoint("a"))
                .with_op(endpoint("b"))
                .with_op(endpoint("c"))
                .with_op(endpoint("d"))
                .with_op(endpoint("x"))
                .with_op(endpoint("y"))
                .with_op(endpoint("z")),
            BatchReturn::Stats,
        )
        .unwrap();
        let error = db
            .execute_batch_returning_bounded_with_settings(
                Batch::new().with_op(relation("test.chain", "bounded", "x", "y")),
                BatchReturn::Stats,
                crate::WriteSettings::default(),
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot update indexed relationship")
        );
        let default_collection = db
            .catalog()
            .collection_by_name(DEFAULT_COLLECTION)
            .unwrap()
            .lid;
        assert!(
            db.storage
                .get_entity(default_collection, "bounded")
                .unwrap()
                .is_none(),
            "bounded rejection must happen before commit"
        );
        db.execute_batch_returning(
            Batch::new()
                .with_op(relation("test.chain", "ab", "a", "b"))
                .with_op(relation("test.chain", "bc", "b", "c"))
                .with_op(relation("test.chain", "aa", "a", "a"))
                .with_op(relation("test.direct", "xy", "x", "y"))
                .with_op(relation("test.direct", "yz", "y", "z")),
            BatchReturn::Stats,
        )
        .unwrap();
        assert_eq!(db.execution_counts.fallback_scans, 0);
        let edges = db
            .storage
            .scan_collection(
                db.catalog()
                    .collection_by_name(RELATION_EDGES_COLLECTION)
                    .unwrap()
                    .lid,
            )
            .unwrap();
        assert!(edges.iter().any(|edge| {
            edge.object
                .get(REL_EDGE_SOURCE_FIELD)
                .and_then(Value::as_str)
                == Some("a")
                && edge
                    .object
                    .get(REL_EDGE_TARGET_FIELD)
                    .and_then(Value::as_str)
                    == Some("c")
                && edge.object.get(REL_EDGE_DEPTH_FIELD) == Some(&Value::U64(2))
        }));
        assert!(edges.iter().any(|edge| {
            edge.object
                .get(REL_EDGE_RELATION_FIELD)
                .and_then(Value::as_str)
                == Some("test.chain")
                && edge
                    .object
                    .get(REL_EDGE_SOURCE_FIELD)
                    .and_then(Value::as_str)
                    == Some("a")
                && edge
                    .object
                    .get(REL_EDGE_TARGET_FIELD)
                    .and_then(Value::as_str)
                    == Some("a")
                && edge.object.get(REL_EDGE_DEPTH_FIELD) == Some(&Value::U64(1))
        }));
        assert!(!edges.iter().any(|edge| {
            edge.object
                .get(REL_EDGE_RELATION_FIELD)
                .and_then(Value::as_str)
                == Some("test.direct")
                && edge
                    .object
                    .get(REL_EDGE_SOURCE_FIELD)
                    .and_then(Value::as_str)
                    == Some("x")
                && edge
                    .object
                    .get(REL_EDGE_TARGET_FIELD)
                    .and_then(Value::as_str)
                    == Some("z")
        }));

        db.execute_batch_returning(
            Batch::new().with_op(relation("test.chain", "bc", "b", "d")),
            BatchReturn::Stats,
        )
        .unwrap();
        assert_eq!(db.execution_counts.fallback_scans, 0);
        let edges = db
            .storage
            .scan_collection(
                db.catalog()
                    .collection_by_name(RELATION_EDGES_COLLECTION)
                    .unwrap()
                    .lid,
            )
            .unwrap();
        assert!(edges.iter().any(|edge| {
            edge.object
                .get(REL_EDGE_SOURCE_FIELD)
                .and_then(Value::as_str)
                == Some("a")
                && edge
                    .object
                    .get(REL_EDGE_TARGET_FIELD)
                    .and_then(Value::as_str)
                    == Some("d")
        }));
        assert!(!edges.iter().any(|edge| {
            edge.object
                .get(REL_EDGE_SOURCE_FIELD)
                .and_then(Value::as_str)
                == Some("a")
                && edge
                    .object
                    .get(REL_EDGE_TARGET_FIELD)
                    .and_then(Value::as_str)
                    == Some("c")
        }));
    }

    #[test]
    fn compact_reverse_reference_backfill_and_predicate_delete() {
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
        // Reopening rebuilt the reverse references, so the predicate delete
        // runs on the point path with one scan of the collection.
        assert_eq!(db.execution_counts.fallback_scans, 0);
        assert_eq!(db.execution_counts.collection_scans, 1);
    }

    #[test]
    fn compact_revision_fence_rejects_stale_reads() {
        let mut db = db();
        let revision = db.storage.current_revision().unwrap();
        db.execute_batch(Batch::new().with_op(upsert("one", "one")))
            .unwrap();
        let catalog = db.catalog();
        let reader = RevisionReader::new(&db.storage, revision).unwrap();
        assert!(!reader.is_snapshot(), "stale revisions must be fenced");
        let mut state = TxState::default();
        let mut view = TxView::new(&catalog, &reader, &mut state);
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
