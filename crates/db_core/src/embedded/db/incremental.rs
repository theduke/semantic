use super::*;

const CONTRIBUTORS: &str = "__semantic.relationship_contributors";
const COUNTS: &str = "__semantic.relationship_counts";
const COMPLETION_MARKER: &str = "backfill:v1";

use crate::batch_return::changes;

// Length-prefix components because IDs and relation names may contain delimiters.
fn key(parts: &[&str]) -> String {
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

impl<S: EntityStorage> EmbeddedDb<S> {
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
    pub(super) fn push_row_delta(
        &self,
        catalog: &Catalog,
        collection: LocalCollectionId,
        before: &BTreeMap<String, Object>,
        after: &BTreeMap<String, Object>,
        ops: &mut Vec<StorageWriteOp>,
    ) -> Result<(), DbError> {
        for id in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
            let old = before.get(id);
            let new = after.get(id);
            if old == new {
                continue;
            }
            if let Some(object) = old {
                for index in catalog.indexes_for_collection(collection) {
                    ops.push(StorageWriteOp::UnindexEntity {
                        index: index.clone(),
                        entity_id: id.clone(),
                        object: object.clone(),
                    });
                }
            }
            if let Some(object) = new {
                ops.push(StorageWriteOp::PutEntity(StoredEntity {
                    id: id.clone(),
                    collection: collection.0,
                    kind: infer_entity_kind(catalog, object),
                    object: object.clone(),
                }));
                for index in catalog.indexes_for_collection(collection) {
                    self.push_index_ops(ops, index, id, object)?;
                }
            } else {
                ops.push(StorageWriteOp::DeleteEntity {
                    collection,
                    entity_id: id.clone(),
                });
            }
        }
        Ok(())
    }

    pub(super) fn initialize_relationship_contributors(&mut self) -> Result<(), DbError> {
        for name in [CONTRIBUTORS, COUNTS] {
            if self.catalog().collection_by_name(name).is_none() {
                self.create_collection(name, CollectionKind::Polymorphic)?;
            }
            self.mark_collection_internal(name)?;
        }
        let catalog = self.catalog();
        let counts = catalog.collection_by_name(COUNTS).unwrap();
        if self
            .storage
            .get_entity(counts.lid, COMPLETION_MARKER)?
            .is_none()
        {
            let revision = self.storage.current_revision()?;
            let mut ops = Vec::new();
            self.rebuild_relationship_edges(&catalog, &BTreeMap::new(), &mut ops)?;
            match self.storage.apply_batch_conditional(&ops, revision)? {
                StorageCommitOutcome::Committed { .. } => {}
                StorageCommitOutcome::Conflict { .. } => {
                    return Err(DbError::TransactionConflict(
                        "relationship contributor backfill conflicted".into(),
                    ));
                }
            }
        }
        Ok(())
    }

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
                let discriminator = Self::external_relation_field_name(
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
                    Self::external_relation_field_value(
                        catalog,
                        collection,
                        object,
                        "from",
                        ATTR_RELATION_FROM,
                    )?,
                    Self::external_relation_field_value(
                        catalog,
                        collection,
                        object,
                        "to",
                        ATTR_RELATION_TO,
                    )?,
                )
            }
        };
        if source.is_empty() || target.is_empty() {
            None
        } else {
            Some((source, target))
        }
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
        ops.push(StorageWriteOp::ClearCollection(contributors.lid));
        ops.push(StorageWriteOp::ClearCollection(counts.lid));
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
                    Self::contribution(catalog, relationship, collection, &id, &object)
                {
                    let contributor = key(&[&relationship.id, &id, &source, &target]);
                    ops.push(StorageWriteOp::PutEntity(StoredEntity {
                        id: contributor.clone(),
                        collection: contributors.lid.0,
                        kind: StoredEntityKind::Untyped,
                        object: count_object(&contributor, 1),
                    }));
                    *totals
                        .entry(key(&[&relationship.id, &source, &target]))
                        .or_default() += 1;
                }
            }
        }
        totals.insert(COMPLETION_MARKER.to_string(), 1);
        for (id, count) in totals {
            ops.push(StorageWriteOp::PutEntity(StoredEntity {
                id: id.clone(),
                collection: counts.lid.0,
                kind: StoredEntityKind::Untyped,
                object: count_object(&id, count),
            }));
        }
        Ok(())
    }

    pub(super) fn update_relationship_edges(
        &self,
        catalog: &Catalog,
        before: &BTreeMap<String, BTreeMap<String, Object>>,
        after: &BTreeMap<String, BTreeMap<String, Object>>,
        ops: &mut Vec<StorageWriteOp>,
    ) -> Result<(), DbError> {
        let (Some(contributors), Some(counts), Some(edges)) = (
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
        let changes = changes(before, after);
        let mut deltas = BTreeMap::<(String, String, String), i64>::new();
        for ((collection_name, id), change) in changes {
            let Some(collection) = catalog.collection_by_name(&collection_name) else {
                continue;
            };
            for (_, schema) in catalog.relationships() {
                let relationship = &schema.relationship;
                if relationship.source_collection != collection_name {
                    continue;
                }
                let old = change.before.as_ref().and_then(|object| {
                    Self::contribution(catalog, relationship, collection, &id, object)
                });
                let new = change.after.as_ref().and_then(|object| {
                    Self::contribution(catalog, relationship, collection, &id, object)
                });
                if old == new {
                    continue;
                }
                for (contribution, delta) in [(old, -1), (new, 1)] {
                    let Some((source, target)) = contribution else {
                        continue;
                    };
                    let contributor = key(&[&relationship.id, &id, &source, &target]);
                    if delta < 0 {
                        ops.push(StorageWriteOp::DeleteEntity {
                            collection: contributors.lid,
                            entity_id: contributor,
                        });
                    } else {
                        ops.push(StorageWriteOp::PutEntity(StoredEntity {
                            id: contributor.clone(),
                            collection: contributors.lid.0,
                            kind: StoredEntityKind::Untyped,
                            object: count_object(&contributor, 1),
                        }));
                    }
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
            let old = self
                .storage
                .get_entity(counts.lid, &id)?
                .and_then(|row| match row.object.get("count") {
                    Some(Value::U64(count)) => Some(*count),
                    _ => None,
                })
                .unwrap_or(0);
            let new = old.checked_add_signed(delta).ok_or_else(|| {
                DbError::Storage(format!("invalid relationship contributor count for {id}"))
            })?;
            updated_counts.insert((relation.clone(), source.clone(), target.clone()), new);
            if (old == 0) != (new == 0) {
                let relationship = &catalog.relationship_by_id(&relation).unwrap().relationship;
                if relationship.indexing_mode == RelationIndexingMode::Enabled {
                    affected.insert(relation);
                } else {
                    let (edge_id, edge) = Self::relationship_edge(&relation, &source, &target, 1);
                    let before = if old > 0 {
                        BTreeMap::from([(edge_id.clone(), edge.clone())])
                    } else {
                        BTreeMap::new()
                    };
                    let after = if new > 0 {
                        BTreeMap::from([(edge_id, edge)])
                    } else {
                        BTreeMap::new()
                    };
                    self.push_row_delta(catalog, edges.lid, &before, &after, ops)?;
                }
            }
            if new == 0 {
                ops.push(StorageWriteOp::DeleteEntity {
                    collection: counts.lid,
                    entity_id: id,
                });
            } else {
                ops.push(StorageWriteOp::PutEntity(StoredEntity {
                    id: id.clone(),
                    collection: counts.lid.0,
                    kind: StoredEntityKind::Untyped,
                    object: count_object(&id, new),
                }));
            }
        }
        if affected.is_empty() {
            return Ok(());
        }
        let old_edges = self
            .storage
            .scan_collection(edges.lid)?
            .into_iter()
            .filter(|row| {
                row.object
                    .get(REL_EDGE_RELATION_FIELD)
                    .and_then(Value::as_str)
                    .is_some_and(|relation| affected.contains(relation))
            })
            .map(|row| (row.id, row.object))
            .collect();
        let new_edges = self.compute_indexed_edges_from_counts(
            catalog,
            counts.lid,
            &affected,
            &updated_counts,
        )?;
        self.push_row_delta(catalog, edges.lid, &old_edges, &new_edges, ops)
    }

    fn compute_indexed_edges_from_counts(
        &self,
        catalog: &Catalog,
        counts: LocalCollectionId,
        affected: &BTreeSet<String>,
        updated: &BTreeMap<(String, String, String), u64>,
    ) -> Result<BTreeMap<String, Object>, DbError> {
        let mut direct = BTreeMap::<String, BTreeSet<(String, String)>>::new();
        for row in self.storage.scan_collection_stream(counts)? {
            let row = row?;
            if row.id == COMPLETION_MARKER {
                continue;
            }
            let Some(parts) = parse_key(&row.id, 3) else {
                continue;
            };
            let key = (parts[0].clone(), parts[1].clone(), parts[2].clone());
            if !affected.contains(&key.0) || updated.contains_key(&key) {
                continue;
            }
            let count = row.object.get("count").and_then(|value| match value {
                Value::U64(value) => Some(*value),
                _ => None,
            });
            if count.is_some_and(|count| count > 0) {
                direct.entry(key.0).or_default().insert((key.1, key.2));
            }
        }
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
                let (id, object) = Self::relationship_edge(&relation, source, target, 1);
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
                            let (id, object) =
                                Self::relationship_edge(&relation, source, target, next_depth);
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
                return Err(DbError::Storage(format!(
                    "relationship contributor references unknown relationship '{relation}'"
                )));
            }
        }
        Ok(out)
    }
}

fn parse_key(value: &str, expected: usize) -> Option<Vec<String>> {
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
