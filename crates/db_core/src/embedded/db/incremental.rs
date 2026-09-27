use super::*;

pub(super) const CONTRIBUTORS: &str = "__semantic.relationship_contributors";
pub(super) const COUNTS: &str = "__semantic.relationship_counts";
/// Marks a complete contributor and count backfill. Version 2 rows are
/// written with index maintenance; databases holding the version 1 marker
/// (rows stored without index entries) are rebuilt when opened.
pub(super) const COMPLETION_MARKER: &str = "backfill:v2";

use super::compact::TxRead;
use crate::batch_return::changes;
use crate::embedded::storage::index_entries_unchanged;

// Length-prefix components because IDs and relation names may contain delimiters.
pub(super) fn key(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| format!("{}:{part}", part.len()))
        .collect()
}

fn count_object(id: &str, count: u64) -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String(id.to_string()));
    object.insert("count", Value::U64(count));
    object
}

/// The edge row of `relation` from `source` to `target`.
pub(super) fn relationship_edge(
    relation: &str,
    source: &str,
    target: &str,
    depth: usize,
) -> (String, Object) {
    let id = format!("{relation}|{source}|{target}");
    let mut object = Object::new();
    object.insert("id", Value::String(id.clone()));
    object.insert(REL_EDGE_RELATION_FIELD, Value::String(relation.to_string()));
    object.insert(REL_EDGE_SOURCE_FIELD, Value::String(source.to_string()));
    object.insert(REL_EDGE_TARGET_FIELD, Value::String(target.to_string()));
    object.insert(REL_EDGE_DEPTH_FIELD, Value::U64(depth as u64));
    object.insert(
        REL_EDGE_SOURCE_KEY_FIELD,
        Value::String(format!("{relation}|{source}")),
    );
    object.insert(
        REL_EDGE_TARGET_KEY_FIELD,
        Value::String(format!("{relation}|{target}")),
    );
    (id, object)
}

/// Append the storage operations turning the rows `before` of `collection`
/// into `after`.
///
/// Unchanged rows produce no operations. Index maintenance is diff-aware:
/// indexes whose indexed values did not change are skipped, the others get
/// one [`StorageWriteOp::ReindexEntity`] so storage writes only the entries
/// that differ.
pub(super) fn push_row_delta(
    catalog: &Catalog,
    collection: LocalCollectionId,
    before: &BTreeMap<String, Object>,
    after: &BTreeMap<String, Object>,
    ops: &mut Vec<StorageWriteOp>,
) {
    for id in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        push_row_change(catalog, collection, id, before.get(id), after.get(id), ops);
    }
}

/// Append the storage operations turning row `id` of `collection` from
/// `old` into `new` (see [`push_row_delta`]).
pub(super) fn push_row_change(
    catalog: &Catalog,
    collection: LocalCollectionId,
    id: &str,
    old: Option<&Object>,
    new: Option<&Object>,
    ops: &mut Vec<StorageWriteOp>,
) {
    if old == new {
        return;
    }
    // Shared by the entity write's index maintenance ops.
    let (old_shared, new_shared) = (old.cloned().map(Arc::new), new.cloned().map(Arc::new));
    match new {
        Some(object) => ops.push(StorageWriteOp::PutEntity(StoredEntity {
            id: id.to_string(),
            collection: collection.0,
            kind: infer_entity_kind(catalog, object),
            object: object.clone(),
        })),
        None => ops.push(StorageWriteOp::DeleteEntity {
            collection,
            entity_id: id.to_string(),
        }),
    }
    for index in catalog.indexes_for_collection(collection) {
        if let (Some(old), Some(new)) = (old, new)
            && index_entries_unchanged(index, old, new)
        {
            continue;
        }
        ops.push(StorageWriteOp::ReindexEntity {
            index: index.clone(),
            entity_id: id.to_string(),
            old: old_shared.clone(),
            new: new_shared.clone(),
        });
    }
}

/// Reset every index of `collection`, before rewriting all its rows.
pub(super) fn push_reset_indexes(
    catalog: &Catalog,
    collection: LocalCollectionId,
    ops: &mut Vec<StorageWriteOp>,
) {
    ops.push(StorageWriteOp::ClearCollection(collection));
    for index in catalog.indexes_for_collection(collection) {
        ops.push(StorageWriteOp::ResetIndex(index.lid));
    }
}

impl<S: EntityStorage> EmbeddedDb<S> {
    pub(super) fn initialize_relationship_contributors(&mut self) -> Result<(), DbError> {
        for name in [CONTRIBUTORS, COUNTS] {
            if self.catalog().collection_by_name(name).is_none() {
                self.create_collection(name, CollectionKind::Polymorphic)?;
            }
            self.mark_collection_internal(name)?;
        }
        let catalog = self.catalog();
        let counts = catalog.collection_by_name(COUNTS).unwrap();
        // Missing, or an older marker whose rows lack index entries.
        if self
            .storage
            .get_entity(counts.lid, COMPLETION_MARKER)?
            .is_none()
        {
            let started = Instant::now();
            tracing::debug!(
                operation = "database_open",
                phase = "relationship_contributor_backfill",
                "Database startup backfill started"
            );
            let revision = self.storage.current_revision()?;
            let mut ops = Vec::new();
            self.rebuild_relationship_edges(&catalog, &BTreeMap::new(), &mut ops)?;
            let write_operations = ops.len();
            match self.storage.apply_batch_conditional(&ops, revision)? {
                StorageCommitOutcome::Committed { .. } => {}
                StorageCommitOutcome::Conflict { .. } => {
                    return Err(DbError::TransactionConflict(
                        "relationship contributor backfill conflicted".into(),
                    ));
                }
            }
            tracing::debug!(
                operation = "database_open",
                phase = "relationship_contributor_backfill",
                elapsed = ?started.elapsed(),
                write_operations,
                "Database startup backfill completed"
            );
        }
        Ok(())
    }

    pub(super) fn rebuild_relationship_contributors(
        &self,
        catalog: &Catalog,
        after: &BTreeMap<String, BTreeMap<String, Object>>,
        ops: &mut Vec<StorageWriteOp>,
    ) -> Result<(), DbError> {
        let (Some(contributors), Some(counts)) = (
            catalog.collection_by_name(CONTRIBUTORS),
            catalog.collection_by_name(COUNTS),
        ) else {
            return Ok(());
        };
        push_reset_indexes(catalog, contributors.lid, ops);
        push_reset_indexes(catalog, counts.lid, ops);
        let mut contributor_rows = BTreeMap::new();
        let mut totals = BTreeMap::<String, u64>::new();
        for (_, schema) in catalog.relationships() {
            let relationship = &schema.relationship;
            if relationship.source_collection == RELATION_EDGES_COLLECTION {
                continue;
            }
            let Some(collection) = catalog.collection_by_name(&relationship.source_collection)
            else {
                continue;
            };
            let rows = match after.get(&collection.name) {
                Some(rows) => rows.clone(),
                None => self
                    .storage
                    .scan_collection(collection.lid)?
                    .into_iter()
                    .map(|row| (row.id, row.object))
                    .collect(),
            };
            for (id, object) in rows {
                if let Some((source, target)) =
                    contribution(catalog, relationship, collection, &id, &object)
                {
                    let contributor = key(&[&relationship.id, &id, &source, &target]);
                    contributor_rows.insert(contributor.clone(), count_object(&contributor, 1));
                    *totals
                        .entry(key(&[&relationship.id, &source, &target]))
                        .or_default() += 1;
                }
            }
        }
        totals.insert(COMPLETION_MARKER.to_string(), 1);
        let count_rows = totals
            .into_iter()
            .map(|(id, count)| {
                let object = count_object(&id, count);
                (id, object)
            })
            .collect();
        push_row_delta(
            catalog,
            contributors.lid,
            &BTreeMap::new(),
            &contributor_rows,
            ops,
        );
        push_row_delta(catalog, counts.lid, &BTreeMap::new(), &count_rows, ops);
        Ok(())
    }

    /// Append the relationship contributor, count and edge writes of the
    /// change from `before` to `after`, reading the stored state at the
    /// current revision. Rebuilds everything when the derived collections
    /// are missing or were cleared by `ops` (DDL preludes).
    pub(super) fn update_relationship_edges(
        &self,
        catalog: &Catalog,
        before: &BTreeMap<String, BTreeMap<String, Object>>,
        after: &BTreeMap<String, BTreeMap<String, Object>>,
        ops: &mut Vec<StorageWriteOp>,
    ) -> Result<(), DbError> {
        let (Some(contributors), Some(_), Some(_)) = (
            catalog.collection_by_name(CONTRIBUTORS),
            catalog.collection_by_name(COUNTS),
            catalog.collection_by_name(RELATION_EDGES_COLLECTION),
        ) else {
            return self.rebuild_relationship_edges(catalog, after, ops);
        };
        // DDL preludes rebuilt contributors using the new catalog; package data
        // migrations require a final rebuild from their post-migration rows.
        if ops
            .iter()
            .any(|op| matches!(op, StorageWriteOp::ClearCollection(id) if *id == contributors.lid))
        {
            return self.rebuild_relationship_edges(catalog, after, ops);
        }
        let reader = RevisionReader::new(&self.storage, self.storage.current_revision()?)?;
        update_relationship_edges(catalog, &reader, &changes(before, after), ops)
    }
}

/// Append the relationship contributor, count and edge writes of `changes`,
/// reading the stored contributor counts and edges through `reader`.
///
/// Contributor, count and edge rows go through [`push_row_change`], so their
/// indexes are maintained like those of any other row.
pub(super) fn update_relationship_edges(
    catalog: &Catalog,
    reader: &dyn TxRead,
    changes: &crate::batch_return::ChangeSet,
    ops: &mut Vec<StorageWriteOp>,
) -> Result<(), DbError> {
    let (Some(contributors), Some(counts), Some(edges)) = (
        catalog.collection_by_name(CONTRIBUTORS),
        catalog.collection_by_name(COUNTS),
        catalog.collection_by_name(RELATION_EDGES_COLLECTION),
    ) else {
        return Err(DbError::storage(
            crate::StorageErrorKind::InvalidState,
            "relationship contributors are not initialized",
        ));
    };
    let mut deltas = BTreeMap::<(String, String, String), i64>::new();
    for ((collection_name, id), change) in changes {
        let Some(collection) = catalog.collection_by_name(collection_name) else {
            continue;
        };
        for (_, schema) in catalog.relationships() {
            let relationship = &schema.relationship;
            if relationship.source_collection != *collection_name {
                continue;
            }
            let old = change
                .before
                .as_ref()
                .and_then(|object| contribution(catalog, relationship, collection, id, object));
            let new = change
                .after
                .as_ref()
                .and_then(|object| contribution(catalog, relationship, collection, id, object));
            if old == new {
                continue;
            }
            for (contribution, delta) in [(old, -1), (new, 1)] {
                let Some((source, target)) = contribution else {
                    continue;
                };
                let contributor = key(&[&relationship.id, id, &source, &target]);
                let row = count_object(&contributor, 1);
                let (old_row, new_row) = if delta < 0 {
                    (Some(&row), None)
                } else {
                    (None, Some(&row))
                };
                push_row_change(
                    catalog,
                    contributors.lid,
                    &contributor,
                    old_row,
                    new_row,
                    ops,
                );
                *deltas
                    .entry((relationship.id.clone(), source, target))
                    .or_default() += delta;
            }
        }
    }
    let mut affected = BTreeSet::new();
    let mut updated_counts = BTreeMap::<(String, String, String), u64>::new();
    for ((relation, source, target), delta) in deltas {
        if delta == 0 {
            continue;
        }
        let id = key(&[&relation, &source, &target]);
        let old = reader
            .get_entity(counts.lid, &id)?
            .and_then(|row| stored_count(&row.object))
            .unwrap_or(0);
        let new = old.checked_add_signed(delta).ok_or_else(|| {
            DbError::Storage(format!("invalid relationship contributor count for {id}").into())
        })?;
        updated_counts.insert((relation.clone(), source.clone(), target.clone()), new);
        if (old == 0) != (new == 0) {
            let relationship = &catalog.relationship_by_id(&relation).unwrap().relationship;
            if relationship.indexing_mode == RelationIndexingMode::Enabled {
                affected.insert(relation);
            } else {
                let (edge_id, edge) = relationship_edge(&relation, &source, &target, 1);
                push_row_change(
                    catalog,
                    edges.lid,
                    &edge_id,
                    (old > 0).then_some(&edge),
                    (new > 0).then_some(&edge),
                    ops,
                );
            }
        }
        let old_row = (old > 0).then(|| count_object(&id, old));
        let new_row = (new > 0).then(|| count_object(&id, new));
        push_row_change(
            catalog,
            counts.lid,
            &id,
            old_row.as_ref(),
            new_row.as_ref(),
            ops,
        );
    }
    if affected.is_empty() {
        return Ok(());
    }
    let mut old_edges = BTreeMap::new();
    reader.scan_collection(edges.lid, &mut |row| {
        if row
            .object
            .get(REL_EDGE_RELATION_FIELD)
            .and_then(Value::as_str)
            .is_some_and(|relation| affected.contains(relation))
        {
            old_edges.insert(row.id, row.object);
        }
        Ok(())
    })?;
    let new_edges =
        compute_indexed_edges_from_counts(catalog, reader, counts.lid, &affected, &updated_counts)?;
    push_row_delta(catalog, edges.lid, &old_edges, &new_edges, ops);
    Ok(())
}

fn stored_count(object: &Object) -> Option<u64> {
    match object.get("count") {
        Some(Value::U64(count)) => Some(*count),
        _ => None,
    }
}

fn compute_indexed_edges_from_counts(
    catalog: &Catalog,
    reader: &dyn TxRead,
    counts: LocalCollectionId,
    affected: &BTreeSet<String>,
    updated: &BTreeMap<(String, String, String), u64>,
) -> Result<BTreeMap<String, Object>, DbError> {
    let mut direct = BTreeMap::<String, BTreeSet<(String, String)>>::new();
    reader.scan_collection(counts, &mut |row| {
        if row.id == COMPLETION_MARKER {
            return Ok(());
        }
        let Some(parts) = parse_key(&row.id, 3) else {
            return Ok(());
        };
        let key = (parts[0].clone(), parts[1].clone(), parts[2].clone());
        if !affected.contains(&key.0) || updated.contains_key(&key) {
            return Ok(());
        }
        if stored_count(&row.object).is_some_and(|count| count > 0) {
            direct.entry(key.0).or_default().insert((key.1, key.2));
        }
        Ok(())
    })?;
    for ((relation, source, target), count) in updated {
        if !affected.contains(relation) {
            continue;
        }
        let edges = direct.entry(relation.clone()).or_default();
        if *count == 0 {
            edges.remove(&(source.clone(), target.clone()));
        } else {
            edges.insert((source.clone(), target.clone()));
        }
    }

    let mut out = BTreeMap::new();
    for (relation, direct) in direct {
        let mut adjacency = BTreeMap::<String, Vec<String>>::new();
        for (source, target) in &direct {
            adjacency
                .entry(source.clone())
                .or_default()
                .push(target.clone());
            let (id, object) = relationship_edge(&relation, source, target, 1);
            out.insert(id, object);
        }
        for source in adjacency.keys() {
            let mut queue = std::collections::VecDeque::from([(source.clone(), 0usize)]);
            let mut seen = BTreeMap::from([(source.clone(), 0usize)]);
            while let Some((node, depth)) = queue.pop_front() {
                let Some(targets) = adjacency.get(&node) else {
                    continue;
                };
                for target in targets {
                    let next_depth = depth.saturating_add(1);
                    if seen
                        .get(target)
                        .is_none_or(|existing| next_depth < *existing)
                    {
                        seen.insert(target.clone(), next_depth);
                        queue.push_back((target.clone(), next_depth));
                        let (id, object) = relationship_edge(&relation, source, target, next_depth);
                        match out
                            .get(&id)
                            .and_then(|object: &Object| object.get(REL_EDGE_DEPTH_FIELD))
                        {
                            Some(Value::U64(existing)) if *existing <= next_depth as u64 => {}
                            _ => {
                                out.insert(id, object);
                            }
                        }
                    }
                }
            }
        }
    }
    // Ensure affected relationship definitions still exist. This also makes
    // malformed contributor state fail close to its source.
    for relation in affected {
        if catalog.relationship_by_id(relation).is_none() {
            return Err(DbError::Storage(
                format!("relationship contributor references unknown relationship '{relation}'")
                    .into(),
            ));
        }
    }
    Ok(out)
}

/// The `(source, target)` pair `object` (row `id` of `collection`)
/// contributes to `relationship`, if any.
pub(super) fn contribution(
    catalog: &Catalog,
    relationship: &RelationType,
    collection: &CollectionSchema,
    id: &str,
    object: &Object,
) -> Option<(String, String)> {
    let (source, target) = match &relationship.mode {
        RelationMode::Embedded { attribute } => {
            let field = catalog
                .attribute_by_id(attribute)
                .map(|attr| attr.attribute.id.as_str())
                .unwrap_or_else(|| collection.canonical_field_name(attribute));
            (id.to_string(), object.get(field)?.as_str()?.to_string())
        }
        RelationMode::External => {
            let discriminator = external_relation_field_name(
                catalog,
                collection,
                object,
                "relation",
                ATTR_RELATION_RELATION,
            );
            if object
                .get(&discriminator)
                .is_some_and(|value| value.as_str() != Some(relationship.id.as_str()))
            {
                return None;
            }
            (
                external_relation_field_value(
                    catalog,
                    collection,
                    object,
                    "from",
                    ATTR_RELATION_FROM,
                )?,
                external_relation_field_value(catalog, collection, object, "to", ATTR_RELATION_TO)?,
            )
        }
    };
    if source.is_empty() || target.is_empty() {
        None
    } else {
        Some((source, target))
    }
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
            && let Some(field) = catalog.class_field_for_alias(class_ids[0], alias)
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
    let field =
        external_relation_field_name(catalog, source_collection, object, alias, fallback_attr);
    object
        .get(&field)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

pub(super) fn parse_key(value: &str, expected: usize) -> Option<Vec<String>> {
    let bytes = value.as_bytes();
    let mut offset = 0usize;
    let mut out = Vec::with_capacity(expected);
    while offset < bytes.len() && out.len() < expected {
        let colon = bytes[offset..].iter().position(|byte| *byte == b':')? + offset;
        let length = value[offset..colon].parse::<usize>().ok()?;
        let start = colon + 1;
        let end = start.checked_add(length)?;
        let part = value.get(start..end)?;
        out.push(part.to_string());
        offset = end;
    }
    (offset == bytes.len() && out.len() == expected).then_some(out)
}
