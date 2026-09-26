//! Read-only consistency checks of one committed database state.
//!
//! The verifier streams every collection once from one storage snapshot and
//! compares the data derived from the rows with what is stored:
//!
//! - Index entries: every entry a row derives must exist (point lookups),
//!   and every stored entry must be derived by its row. Stale entries are
//!   found by comparing the number of stored entries with the number of
//!   derived ones; only when they differ is the index scanned entry by entry.
//! - Reverse references and relationship contributors: the same scheme,
//!   with point reads of the expected rows and a detailed scan only on a
//!   count mismatch.
//! - Relationship counts and edges: the direct `(source, target)` pairs of
//!   each relationship are tallied during the row pass (memory proportional
//!   to the number of distinct pairs, not to the rows), and the expected
//!   edges are derived from them like the write path does.
//! - Maintained counters are compared with the counted rows and entries.
//! - Unique indexes are checked for rows sharing a key value (memory
//!   proportional to the rows of the collection being checked).
//!
//! Besides the report, the verifier produces a [`RepairPlan`] naming what a
//! repair must rebuild.

use std::collections::HashMap;

use super::super::compact::{MARKER, REFERENCES, reference_rows, resolved_references};
use super::super::incremental::{COMPLETION_MARKER, CONTRIBUTORS, COUNTS, key, parse_key};
use super::*;
use crate::catalog::IndexSchema;
use crate::embedded::BoxIndexKeyScan;
use crate::{
    StorageIntegrityCheck, VerifyOptions, VerifyProblem, VerifyProblemKind as Kind, VerifyReport,
};

/// What a repair must rebuild to fix the problems of a verify.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RepairPlan {
    pub(crate) indexes: BTreeSet<LocalIndexId>,
    pub(crate) reverse_references: bool,
    pub(crate) relationship_edges: bool,
    pub(crate) stats: bool,
}

/// Entities whose derived index keys are cached by a stale-entry scan.
const STALE_SCAN_CACHE: usize = 4096;

/// Running tally of expected derived items and those found missing.
#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    expected: u64,
    missing: u64,
}

impl Tally {
    /// Whether `stored` items can only match when none is stale.
    fn has_stale(&self, stored: u64) -> bool {
        stored != self.expected - self.missing
    }
}

pub(crate) struct Verifier<'r, S> {
    catalog: &'r Catalog,
    snapshot: &'r dyn EntityReadSnapshot,
    options: VerifyOptions,
    report: VerifyReport,
    plan: RepairPlan,
    /// Tallies of the built indexes being checked.
    indexes: BTreeMap<LocalIndexId, Tally>,
    references: Option<(LocalCollectionId, Tally)>,
    contributors: Option<(LocalCollectionId, Tally)>,
    /// Contributions per relationship and `(source, target)` pair.
    direct: BTreeMap<String, BTreeMap<(String, String), u64>>,
    /// Key values seen (and the first row holding them) per checked unique
    /// index of the collection being checked.
    unique_values: BTreeMap<LocalIndexId, BTreeMap<Value, String>>,
    storage: std::marker::PhantomData<fn() -> S>,
}

impl<'r, S: EntityStorage> Verifier<'r, S> {
    pub(crate) fn new(
        catalog: &'r Catalog,
        snapshot: &'r dyn EntityReadSnapshot,
        options: VerifyOptions,
    ) -> Self {
        Self {
            catalog,
            snapshot,
            options,
            report: VerifyReport::default(),
            plan: RepairPlan::default(),
            indexes: BTreeMap::new(),
            references: None,
            contributors: None,
            direct: BTreeMap::new(),
            unique_values: BTreeMap::new(),
            storage: std::marker::PhantomData,
        }
    }

    /// Record the outcome of the physical integrity check.
    pub(crate) fn integrity(&mut self, check: StorageIntegrityCheck) {
        match check {
            StorageIntegrityCheck::Intact => {}
            StorageIntegrityCheck::Repaired => self.problem(
                Kind::StorageIntegrity,
                None,
                None,
                None,
                "the storage found and repaired physical damage".into(),
            ),
            StorageIntegrityCheck::Corrupt(detail) => {
                self.problem(Kind::StorageIntegrity, None, None, None, detail)
            }
            StorageIntegrityCheck::Skipped(reason) => self
                .report
                .skipped
                .push(format!("storage integrity: {reason}")),
        }
    }

    pub(crate) fn run(mut self) -> Result<(VerifyReport, RepairPlan), DbError> {
        let started = std::time::Instant::now();
        let snapshot = self.snapshot;
        let revision = snapshot.revision()?;
        self.report.revision = revision;
        let fence = !snapshot.is_consistent();

        self.prepare_indexes()?;
        self.prepare_derived()?;
        for (_, collection) in self.catalog.collections() {
            self.check_collection(collection)?;
        }
        let indexes = self.indexes.keys().copied().collect::<Vec<_>>();
        for lid in indexes {
            self.check_index_entries(lid)?;
        }
        if self.options.check_reverse_references {
            self.check_reverse_references()?;
        }
        if self.options.check_relationship_edges {
            self.check_relationship_edges()?;
        }

        if fence && snapshot.revision()? != revision {
            return Err(DbError::TransactionConflict(
                "database changed during verification".into(),
            ));
        }
        self.report.duration = started.elapsed();
        Ok((self.report, self.plan))
    }

    fn problem(
        &mut self,
        kind: Kind,
        collection: Option<&str>,
        index: Option<&str>,
        entity_id: Option<&str>,
        detail: String,
    ) {
        self.report.problem_count += 1;
        if self.report.problems.len() < self.options.max_problems {
            self.report.problems.push(VerifyProblem {
                kind,
                collection: collection.map(str::to_string),
                index: index.map(str::to_string),
                entity_id: entity_id.map(str::to_string),
                detail,
            });
        }
    }

    fn index_problem(&mut self, kind: Kind, index: &IndexSchema, id: Option<&str>, detail: String) {
        self.plan.indexes.insert(index.lid);
        self.problem(
            kind,
            Some(&index.schema.collection),
            Some(&index.schema.name),
            id,
            detail,
        );
    }

    /// Select the indexes whose entries are checked; unbuilt indexes are
    /// problems themselves.
    fn prepare_indexes(&mut self) -> Result<(), DbError> {
        if !self.options.check_indexes && !self.options.check_stats {
            return Ok(());
        }
        let catalog = self.catalog;
        let mut supported = true;
        for (lid, index) in catalog.indexes() {
            if self.snapshot.index_needs_rebuild(lid)? {
                if self.options.check_indexes {
                    self.index_problem(
                        Kind::IndexNotBuilt,
                        index,
                        None,
                        "index is not marked as built".into(),
                    );
                }
                continue;
            }
            if supported {
                if let Err(error) = self.snapshot.scan_index_keys(index) {
                    if error.storage_kind() != Some(StorageErrorKind::Unsupported) {
                        return Err(error);
                    }
                    self.report
                        .skipped
                        .push(format!("index entries and index counters: {error}"));
                    supported = false;
                }
            }
            if supported {
                self.indexes.insert(lid, Tally::default());
            }
        }
        Ok(())
    }

    fn prepare_derived(&mut self) -> Result<(), DbError> {
        let catalog = self.catalog;
        if self.options.check_reverse_references {
            match catalog.collection_by_name(REFERENCES) {
                Some(references) => {
                    self.references = Some((references.lid, Tally::default()));
                    if self.snapshot.get_entity(references.lid, MARKER)?.is_none() {
                        self.plan.reverse_references = true;
                        self.problem(
                            Kind::DerivedDataNotInitialized,
                            Some(REFERENCES),
                            None,
                            None,
                            "reverse references were never backfilled".into(),
                        );
                    }
                }
                None => self
                    .report
                    .skipped
                    .push("reverse references: not maintained by this database".into()),
            }
        }
        if self.options.check_relationship_edges {
            match (
                catalog.collection_by_name(CONTRIBUTORS),
                catalog.collection_by_name(COUNTS),
            ) {
                (Some(contributors), Some(counts)) => {
                    self.contributors = Some((contributors.lid, Tally::default()));
                    if self
                        .snapshot
                        .get_entity(counts.lid, COMPLETION_MARKER)?
                        .is_none()
                    {
                        self.plan.relationship_edges = true;
                        self.problem(
                            Kind::DerivedDataNotInitialized,
                            Some(COUNTS),
                            None,
                            None,
                            "relationship contributors were never backfilled".into(),
                        );
                    }
                }
                _ => self
                    .report
                    .skipped
                    .push("relationship contributors: not maintained by this database".into()),
            }
        }
        Ok(())
    }

    /// Relationships whose contributions come from `collection`.
    fn relationships_of(&self, collection: &CollectionSchema) -> Vec<&'r RelationType> {
        if !self.options.check_relationship_edges || collection.name == RELATION_EDGES_COLLECTION {
            return Vec::new();
        }
        self.catalog
            .relationships()
            .map(|(_, schema)| &schema.relationship)
            .filter(|relationship| relationship.source_collection == collection.name)
            .collect()
    }

    fn check_collection(&mut self, collection: &CollectionSchema) -> Result<(), DbError> {
        let catalog = self.catalog;
        let snapshot = self.snapshot;
        let indexes = catalog
            .indexes_for_collection(collection.lid)
            .filter(|index| self.options.check_indexes && self.indexes.contains_key(&index.lid))
            .collect::<Vec<_>>();
        let references = self
            .references
            .filter(|_| collection.name != REFERENCES)
            .map(|(lid, _)| lid);
        let relationships = self.relationships_of(collection);
        let scan_rows = self.options.check_payloads
            || !indexes.is_empty()
            || references.is_some()
            || !relationships.is_empty();

        self.report.checked.collections += 1;
        let rows = if scan_rows {
            let mut rows = 0;
            for item in snapshot.scan_collection_checked(collection.lid)? {
                let (id, entity) = item?;
                rows += 1;
                match entity {
                    Ok(entity) => {
                        self.check_row_indexes(&indexes, &entity)?;
                        if let Some(references) = references {
                            self.check_row_references(collection, references, &entity)?;
                        }
                        self.tally_contributions(collection, &relationships, &entity)?;
                    }
                    Err(error) => {
                        if self.options.check_payloads {
                            self.problem(
                                Kind::CorruptPayload,
                                Some(&collection.name),
                                None,
                                Some(&id),
                                error.to_string(),
                            );
                        }
                    }
                }
            }
            rows
        } else if self.options.check_stats {
            snapshot.count_collection_entities(collection.lid)?
        } else {
            return Ok(());
        };
        self.report.checked.rows += rows;
        self.unique_values.clear();

        if self.options.check_stats
            && let Some(counter) = snapshot.collection_row_count(collection.lid)?
        {
            self.report.checked.counters += 1;
            if counter != rows {
                self.plan.stats = true;
                self.problem(
                    Kind::WrongRowCount,
                    Some(&collection.name),
                    None,
                    None,
                    format!("row counter is {counter}, but the collection has {rows} rows"),
                );
            }
        }
        Ok(())
    }

    fn check_row_indexes(
        &mut self,
        indexes: &[&IndexSchema],
        entity: &StoredEntity,
    ) -> Result<(), DbError> {
        for index in indexes {
            let keys = self
                .snapshot
                .index_keys_for(index, &entity.id, &entity.object)?;
            let mut missing = 0;
            for key in &keys {
                if !self.snapshot.contains_index_key(index.lid, key)? {
                    missing += 1;
                    self.index_problem(
                        Kind::MissingIndexEntry,
                        index,
                        Some(&entity.id),
                        format!("missing entry {}", key_hex(key)),
                    );
                }
            }
            let tally = self.indexes.get_mut(&index.lid).expect("checked index");
            tally.expected += keys.len() as u64;
            tally.missing += missing;
            self.check_unique_value(index, entity);
        }
        Ok(())
    }

    /// Report `entity` when an earlier row holds its key value in the
    /// unique `index`. Not part of the repair plan: rebuilding the index
    /// cannot remove duplicate rows.
    fn check_unique_value(&mut self, index: &IndexSchema, entity: &StoredEntity) {
        if !index.schema.unique || !index.schema.kind.is_value_index() {
            return;
        }
        let Some(value) = index.key_value(&entity.object) else {
            return;
        };
        let seen = self.unique_values.entry(index.lid).or_default();
        let Some(existing) = seen.get(&value) else {
            seen.insert(value, entity.id.clone());
            return;
        };
        let detail = format!("row {existing:?} holds the same value {value:?}");
        self.problem(
            Kind::DuplicateUniqueValue,
            Some(&index.schema.collection),
            Some(&index.schema.name),
            Some(&entity.id),
            detail,
        );
    }

    fn check_row_references(
        &mut self,
        collection: &CollectionSchema,
        references: LocalCollectionId,
        entity: &StoredEntity,
    ) -> Result<(), DbError> {
        let owner = (collection.name.clone(), entity.id.clone());
        let expected = reference_rows(resolved_references(self.catalog, &owner, &entity.object));
        let mut missing = 0;
        for (id, object) in &expected {
            let detail = match self.snapshot.get_entity(references, id)? {
                None => "reverse reference is missing",
                Some(stored) if stored.object != *object => "reverse reference differs",
                Some(_) => continue,
            };
            missing += 1;
            self.plan.reverse_references = true;
            self.problem(
                Kind::MissingReverseReference,
                Some(&collection.name),
                None,
                Some(&entity.id),
                format!("{detail}: {id}"),
            );
        }
        let tally = &mut self.references.as_mut().expect("checked references").1;
        tally.expected += expected.len() as u64;
        tally.missing += missing;
        Ok(())
    }

    fn tally_contributions(
        &mut self,
        collection: &CollectionSchema,
        relationships: &[&RelationType],
        entity: &StoredEntity,
    ) -> Result<(), DbError> {
        for relationship in relationships {
            let Some((source, target)) = super::super::incremental::contribution(
                self.catalog,
                relationship,
                collection,
                &entity.id,
                &entity.object,
            ) else {
                continue;
            };
            if let Some((contributors, _)) = self.contributors {
                let id = key(&[&relationship.id, &entity.id, &source, &target]);
                let missing = self.snapshot.get_entity(contributors, &id)?.is_none();
                if missing {
                    self.plan.relationship_edges = true;
                    self.problem(
                        Kind::MissingRelationshipContributor,
                        Some(&collection.name),
                        None,
                        Some(&entity.id),
                        format!("missing contributor {id}"),
                    );
                }
                let tally = &mut self.contributors.as_mut().expect("checked").1;
                tally.expected += 1;
                tally.missing += u64::from(missing);
            }
            *self
                .direct
                .entry(relationship.id.clone())
                .or_default()
                .entry((source, target))
                .or_default() += 1;
        }
        Ok(())
    }

    /// Count the stored entries of a checked index, find stale entries when
    /// the count does not match, and compare the entry counter.
    fn check_index_entries(&mut self, lid: LocalIndexId) -> Result<(), DbError> {
        let index = self.catalog.index_by_lid(lid).expect("catalog index");
        let tally = self.indexes[&lid];
        let stored = count_index_keys(self.snapshot.scan_index_keys(index)?)?;
        self.report.checked.indexes += 1;
        self.report.checked.index_entries += stored;
        if self.options.check_indexes && tally.has_stale(stored) {
            self.find_stale_index_entries(index)?;
        }
        if self.options.check_stats
            && let Some(counter) = self.snapshot.index_entry_count(lid)?
        {
            self.report.checked.counters += 1;
            if counter != stored {
                self.plan.stats = true;
                self.index_problem(
                    Kind::WrongIndexEntryCount,
                    index,
                    None,
                    format!("entry counter is {counter}, but the index has {stored} entries"),
                );
            }
        }
        Ok(())
    }

    fn find_stale_index_entries(&mut self, index: &IndexSchema) -> Result<(), DbError> {
        let snapshot = self.snapshot;
        // Keys the rows derive, per entity; `None` for missing rows.
        let mut derived = HashMap::<String, Option<BTreeSet<Vec<u8>>>>::new();
        for entry in snapshot.scan_index_keys(index)? {
            let entry = entry?;
            let Some(id) = entry.entity_id else {
                self.index_problem(
                    Kind::StaleIndexEntry,
                    index,
                    None,
                    format!("malformed entry {}", key_hex(&entry.key)),
                );
                continue;
            };
            if !derived.contains_key(&id) {
                if derived.len() >= STALE_SCAN_CACHE {
                    derived.clear();
                }
                let keys = match snapshot.get_entity(index.collection, &id) {
                    Ok(Some(row)) => Some(
                        snapshot
                            .index_keys_for(index, &row.id, &row.object)?
                            .into_iter()
                            .collect(),
                    ),
                    Ok(None) => None,
                    // Undecodable rows are reported by the payload check.
                    Err(_) => continue,
                };
                derived.insert(id.clone(), keys);
            }
            let detail = match &derived[&id] {
                None => "entry of a missing row",
                Some(keys) if !keys.contains(&entry.key) => "entry the row does not derive",
                Some(_) => continue,
            };
            self.index_problem(
                Kind::StaleIndexEntry,
                index,
                Some(&id),
                format!("{detail}: {}", key_hex(&entry.key)),
            );
        }
        Ok(())
    }

    fn check_reverse_references(&mut self) -> Result<(), DbError> {
        let Some((references, tally)) = self.references else {
            return Ok(());
        };
        let snapshot = self.snapshot;
        let marker = u64::from(snapshot.get_entity(references, MARKER)?.is_some());
        let stored = snapshot.count_collection_entities(references)? - marker;
        self.report.checked.reverse_references += stored;
        if !tally.has_stale(stored) {
            return Ok(());
        }
        for row in snapshot.scan_collection_stream(references)? {
            let row = row?;
            if row.id == MARKER {
                continue;
            }
            let owner = (
                row.object.get("collection").and_then(Value::as_str),
                row.object.get("owner").and_then(Value::as_str),
            );
            let derived = match owner {
                (Some(collection), Some(owner)) => {
                    match self.catalog.collection_by_name(collection) {
                        Some(schema) => match snapshot.get_entity(schema.lid, owner) {
                            Ok(Some(entity)) => {
                                let key = (collection.to_string(), owner.to_string());
                                reference_rows(resolved_references(
                                    self.catalog,
                                    &key,
                                    &entity.object,
                                ))
                                .remove(&row.id)
                                .is_some_and(|expected| expected == row.object)
                            }
                            Ok(None) => false,
                            Err(_) => continue,
                        },
                        None => false,
                    }
                }
                _ => false,
            };
            if !derived {
                self.plan.reverse_references = true;
                self.problem(
                    Kind::StaleReverseReference,
                    Some(REFERENCES),
                    None,
                    Some(&row.id),
                    "no row derives this reverse reference".into(),
                );
            }
        }
        Ok(())
    }

    fn check_relationship_edges(&mut self) -> Result<(), DbError> {
        if let Some((contributors, tally)) = self.contributors {
            let stored = self.snapshot.count_collection_entities(contributors)?;
            if tally.has_stale(stored) {
                self.find_stale_contributors(contributors)?;
            }
            self.check_relationship_counts()?;
        }
        self.check_edges()
    }

    fn find_stale_contributors(&mut self, contributors: LocalCollectionId) -> Result<(), DbError> {
        let catalog = self.catalog;
        let snapshot = self.snapshot;
        for row in snapshot.scan_collection_stream(contributors)? {
            let row = row?;
            let derived = match parse_key(&row.id, 4).as_deref() {
                Some([relation, id, source, target]) => {
                    let source_collection = catalog
                        .relationship_by_id(relation)
                        .map(|schema| &schema.relationship)
                        .and_then(|relationship| {
                            catalog
                                .collection_by_name(&relationship.source_collection)
                                .map(|collection| (relationship, collection))
                        });
                    match source_collection {
                        Some((relationship, collection)) => {
                            match snapshot.get_entity(collection.lid, id) {
                                Ok(Some(entity)) => {
                                    super::super::incremental::contribution(
                                        catalog,
                                        relationship,
                                        collection,
                                        id,
                                        &entity.object,
                                    ) == Some((source.clone(), target.clone()))
                                }
                                Ok(None) => false,
                                Err(_) => continue,
                            }
                        }
                        None => false,
                    }
                }
                _ => false,
            };
            if !derived {
                self.plan.relationship_edges = true;
                self.problem(
                    Kind::StaleRelationshipContributor,
                    Some(CONTRIBUTORS),
                    None,
                    Some(&row.id),
                    "no row derives this contributor".into(),
                );
            }
        }
        Ok(())
    }

    fn check_relationship_counts(&mut self) -> Result<(), DbError> {
        let Some(counts) = self.catalog.collection_by_name(COUNTS) else {
            return Ok(());
        };
        let snapshot = self.snapshot;
        let mut problems = Vec::new();
        for (relation, pairs) in &self.direct {
            for ((source, target), expected) in pairs {
                let id = key(&[relation, source, target]);
                let stored = snapshot
                    .get_entity(counts.lid, &id)?
                    .and_then(|row| stored_count(&row.object));
                if stored != Some(*expected) {
                    problems.push((
                        id,
                        format!("count is {stored:?}, but {expected} rows contribute"),
                    ));
                }
            }
        }
        for row in snapshot.scan_collection_stream(counts.lid)? {
            let row = row?;
            if row.id == COMPLETION_MARKER {
                continue;
            }
            let expected = parse_key(&row.id, 3).and_then(|parts| {
                self.direct
                    .get(&parts[0])?
                    .get(&(parts[1].clone(), parts[2].clone()))
            });
            if expected.is_none() {
                problems.push((row.id, "count of a pair no row contributes".into()));
            }
        }
        for (id, detail) in problems {
            self.plan.relationship_edges = true;
            self.problem(
                Kind::WrongRelationshipCount,
                Some(COUNTS),
                None,
                Some(&id),
                detail,
            );
        }
        Ok(())
    }

    fn check_edges(&mut self) -> Result<(), DbError> {
        let catalog = self.catalog;
        let snapshot = self.snapshot;
        let Some(edges) = catalog.collection_by_name(RELATION_EDGES_COLLECTION) else {
            return Ok(());
        };
        let mut expected = BTreeSet::new();
        let mut problems = Vec::new();
        for (_, schema) in catalog.relationships() {
            let relationship = &schema.relationship;
            if relationship.source_collection == RELATION_EDGES_COLLECTION
                || catalog
                    .collection_by_name(&relationship.source_collection)
                    .is_none()
            {
                continue;
            }
            let pairs = self
                .direct
                .get(&relationship.id)
                .map(|pairs| pairs.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for (id, object) in EmbeddedDb::<S>::edges_from_direct(relationship, &pairs) {
                let detail = match snapshot.get_entity(edges.lid, &id)? {
                    None => "edge is missing",
                    Some(stored) if stored.object != object => "edge differs",
                    Some(_) => {
                        expected.insert(id);
                        continue;
                    }
                };
                problems.push((Kind::MissingRelationshipEdge, detail, id.clone()));
                expected.insert(id);
            }
        }
        for row in snapshot.scan_collection_stream(edges.lid)? {
            let row = row?;
            self.report.checked.relationship_edges += 1;
            if !expected.contains(&row.id) {
                problems.push((
                    Kind::StaleRelationshipEdge,
                    "no relationship derives this edge",
                    row.id,
                ));
            }
        }
        for (kind, detail, id) in problems {
            self.plan.relationship_edges = true;
            self.problem(
                kind,
                Some(RELATION_EDGES_COLLECTION),
                None,
                Some(&id),
                detail.into(),
            );
        }
        Ok(())
    }
}

fn stored_count(object: &Object) -> Option<u64> {
    match object.get("count") {
        Some(Value::U64(count)) => Some(*count),
        _ => None,
    }
}

fn count_index_keys(scan: BoxIndexKeyScan) -> Result<u64, DbError> {
    let mut count = 0;
    for entry in scan {
        entry?;
        count += 1;
    }
    Ok(count)
}

/// Hex rendering of a storage key, shortened for reports.
fn key_hex(key: &[u8]) -> String {
    const MAX: usize = 48;
    let mut out = String::from("0x");
    for byte in key.iter().take(MAX) {
        out.push_str(&format!("{byte:02x}"));
    }
    if key.len() > MAX {
        out.push_str("...");
    }
    out
}
