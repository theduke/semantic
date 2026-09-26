//! Maintenance operations on the in-memory test storage.
//!
//! The test storage has no raw index keys, so verification skips the index
//! entry checks here; they are covered by the key-value storages (db_kv,
//! db_redb). Reverse references, relationship data, reindexing and change
//! events are storage independent.

use futures::{FutureExt as _, StreamExt as _};
use semantic_data::query::BinaryOp;
use semantic_data::schema::{RelationIndexingMode, RelationMode};

use super::super::compact::{MARKER, REFERENCES};
use super::*;
use crate::embedded::MemoryEntityStorage;
use crate::{ChangeSubscriptionOptions, Expr, Operand, SelectQuery, VerifyProblemKind as Kind};

fn eq(field: &str, value: &str) -> Expr {
    Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
            field,
        ])))),
        right: Box::new(Expr::Operand(Operand::Literal(Value::String(value.into())))),
    }
}

fn event(id: &str, kind: &str) -> Object {
    Object::from_iter([
        ("id".to_string(), Value::String(id.into())),
        ("kind".to_string(), Value::String(kind.into())),
    ])
}

/// A database with an `events` collection indexed by `kind`.
fn events_db() -> (EmbeddedDb<MemoryEntityStorage>, LocalIndexId) {
    let mut db = EmbeddedDb::in_memory();
    let events = db
        .create_collection("events", CollectionKind::Polymorphic)
        .unwrap();
    db.create_index("events_kind_idx", events, "kind", false)
        .unwrap();
    for (id, kind) in [("e1", "music"), ("e2", "video"), ("e3", "music")] {
        db.insert("events", id, event(id, kind)).unwrap();
    }
    let index = db
        .catalog()
        .find_equality_index(events, "kind")
        .unwrap()
        .lid;
    (db, index)
}

fn music<S: EntityStorage>(db: &EmbeddedDb<S>) -> usize {
    db.select(
        SelectQuery::new()
            .with_collection("events")
            .with_predicate(eq("kind", "music")),
    )
    .unwrap()
    .len()
}

fn kinds(report: &VerifyReport) -> BTreeSet<Kind> {
    report.problems.iter().map(|problem| problem.kind).collect()
}

#[test]
fn reindex_restores_queries_after_index_corruption() {
    let (mut db, index) = events_db();
    assert_eq!(music(&db), 2);
    db.storage.corrupt_index(index);
    assert_eq!(music(&db), 0);

    let report = db
        .reindex(&ReindexTarget::Index {
            collection: "events".into(),
            name: "events_kind_idx".into(),
        })
        .unwrap();
    assert_eq!(report.indexes.len(), 1);
    assert_eq!(report.indexes[0].collection, "events");
    assert_eq!(report.indexes[0].index, "events_kind_idx");
    assert_eq!(report.indexes[0].rows, 3);
    assert_eq!(report.indexes[0].entries, Some(3));
    assert!(report.derived.is_empty());
    assert_eq!(music(&db), 2);
    assert!(!db.storage.index_needs_rebuild(index).unwrap());
}

#[test]
fn reindex_selects_collections_and_everything() {
    let (mut db, index) = events_db();
    db.storage.corrupt_index(index);
    let report = db
        .reindex(&ReindexTarget::Collection("events".into()))
        .unwrap();
    let names = report
        .indexes
        .iter()
        .map(|index| (index.collection.as_str(), index.index.as_str()))
        .collect::<Vec<_>>();
    // Built-in indexes plus the explicit one, all of `events`.
    assert!(names.contains(&("events", "events_kind_idx")), "{names:?}");
    assert!(names.iter().all(|(collection, _)| *collection == "events"));
    assert_eq!(music(&db), 2);

    db.storage.corrupt_index(index);
    let report = db.reindex(&ReindexTarget::All).unwrap();
    assert!(
        report
            .indexes
            .iter()
            .any(|index| index.index == "events_kind_idx")
    );
    assert_eq!(
        report.derived,
        [
            DerivedData::RelationshipEdges,
            DerivedData::ReverseReferences
        ]
    );
    assert_eq!(music(&db), 2);

    let unknown = db.reindex(&ReindexTarget::Index {
        collection: "events".into(),
        name: "missing".into(),
    });
    assert!(matches!(unknown, Err(DbError::InvalidQuery(_))));
    assert!(matches!(
        db.reindex(&ReindexTarget::Collection("missing".into())),
        Err(DbError::UnknownCollectionByName { .. })
    ));
}

#[test]
fn maintenance_publishes_no_change_events() {
    let (mut db, _) = events_db();
    let mut subscription =
        db.subscribe_changes(ChangeSubscriptionOptions::default().with_internal());
    db.reindex(&ReindexTarget::All).unwrap();
    db.repair(&VerifyOptions::all()).unwrap();
    assert!(subscription.next().now_or_never().flatten().is_none());
}

#[test]
fn verify_skips_checks_the_storage_cannot_run() {
    let (mut db, _) = events_db();
    let report = db.verify(&VerifyOptions::all()).unwrap();
    assert!(report.is_ok(), "{report}");
    assert!(report.checked.rows >= 3);
    assert!(
        report
            .skipped
            .iter()
            .any(|skipped| skipped.starts_with("index entries")),
        "{report}"
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|skipped| skipped.starts_with("storage integrity")),
        "{report}"
    );
}

fn article(id: &str, author: &str) -> BatchOperation {
    let mut object = Object::new();
    object.insert("id", id.to_string());
    object.insert("type", "local:article".to_string());
    object.insert("author", author.to_string());
    BatchOperation::Upsert {
        collection: DEFAULT_COLLECTION.into(),
        id: id.into(),
        object,
    }
}

fn person(id: &str) -> BatchOperation {
    let mut object = Object::new();
    object.insert("id", id.to_string());
    object.insert("type", "local:person".to_string());
    BatchOperation::Upsert {
        collection: DEFAULT_COLLECTION.into(),
        id: id.into(),
        object,
    }
}

#[test]
fn verify_finds_and_repair_fixes_reverse_references() {
    let mut db = EmbeddedDb::in_memory();
    super::super::tests::register_ref_schema(&mut db, super::super::tests::ref_ty("person"));
    db.transact(
        Batch::new()
            .with_op(person("p1"))
            .with_op(person("p2"))
            .with_op(article("a1", "p1"))
            .with_op(article("a2", "p2")),
    )
    .unwrap();
    let options = VerifyOptions::all();
    let clean = db.verify(&options).unwrap();
    assert!(clean.is_ok(), "{clean}");
    assert_eq!(clean.checked.reverse_references, 2);

    // Drop one reference and add one no row derives.
    let catalog = db.catalog();
    let references = catalog.collection_by_name(REFERENCES).unwrap().lid;
    let stored = db
        .storage
        .scan_collection(references)
        .unwrap()
        .into_iter()
        .find(|row| row.id != MARKER)
        .unwrap();
    let mut bogus = stored.clone();
    bogus.id = "bogus".into();
    bogus.object.insert("id", "bogus".to_string());
    db.storage
        .apply_batch(&[
            StorageWriteOp::DeleteEntity {
                collection: references,
                entity_id: stored.id.clone(),
            },
            StorageWriteOp::PutEntity(bogus),
        ])
        .unwrap();

    let report = db.verify(&options).unwrap();
    assert_eq!(
        kinds(&report),
        BTreeSet::from([Kind::MissingReverseReference, Kind::StaleReverseReference]),
        "{report}"
    );
    assert!(report.problems.iter().any(|problem| {
        problem.kind == Kind::StaleReverseReference && problem.entity_id.as_deref() == Some("bogus")
    }));

    let repair = db.repair(&options).unwrap();
    assert_eq!(repair.before.problem_count, 2);
    assert!(
        repair
            .rebuilt
            .derived
            .contains(&DerivedData::ReverseReferences)
    );
    assert!(repair.after.is_ok(), "{}", repair.after);
    assert!(db.verify(&options).unwrap().is_ok());
}

#[test]
fn verify_finds_and_repair_fixes_relationship_edges() {
    use semantic_data::attr::{ATTR_RELATION_FROM, ATTR_RELATION_RELATION, ATTR_RELATION_TO};

    let mut db = EmbeddedDb::in_memory();
    db.upsert_relationship(RelationType {
        id: "follows".into(),
        name: "follows".into(),
        source_collection: DEFAULT_COLLECTION.into(),
        mode: RelationMode::External,
        indexing_mode: RelationIndexingMode::Enabled,
        meta: Default::default(),
    })
    .unwrap();
    for id in ["a", "b", "c"] {
        db.insert(
            DEFAULT_COLLECTION,
            id,
            Object::from_iter([("id".to_string(), Value::String(id.into()))]),
        )
        .unwrap();
    }
    for (id, from, to) in [("ab", "a", "b"), ("bc", "b", "c")] {
        let object = Object::from_iter([
            ("id".to_string(), Value::String(id.into())),
            (ATTR_RELATION_FROM.to_string(), Value::String(from.into())),
            (ATTR_RELATION_TO.to_string(), Value::String(to.into())),
            (
                ATTR_RELATION_RELATION.to_string(),
                Value::String("follows".into()),
            ),
        ]);
        db.insert(DEFAULT_COLLECTION, id, object).unwrap();
    }
    let options = VerifyOptions::all();
    let clean = db.verify(&options).unwrap();
    assert!(clean.is_ok(), "{clean}");
    // a->b, b->c and the transitive a->c.
    assert_eq!(clean.checked.relationship_edges, 3);

    let catalog = db.catalog();
    let edges = catalog
        .collection_by_name(RELATION_EDGES_COLLECTION)
        .unwrap()
        .lid;
    let stored = db.storage.scan_collection(edges).unwrap();
    let mut stale = stored[0].clone();
    stale.id = "follows|x|y".into();
    db.storage
        .apply_batch(&[
            StorageWriteOp::DeleteEntity {
                collection: edges,
                entity_id: stored[1].id.clone(),
            },
            StorageWriteOp::PutEntity(stale),
        ])
        .unwrap();

    let report = db.verify(&options).unwrap();
    assert_eq!(
        kinds(&report),
        BTreeSet::from([Kind::MissingRelationshipEdge, Kind::StaleRelationshipEdge]),
        "{report}"
    );
    let repair = db.repair(&options).unwrap();
    assert!(repair.after.is_ok(), "{}", repair.after);
    assert_eq!(repair.after.checked.relationship_edges, 3);
}

#[test]
fn unsupported_storage_operations_report_unsupported() {
    let (mut db, _) = events_db();
    let unsupported = |error: DbError| error.storage_kind() == Some(StorageErrorKind::Unsupported);
    assert!(unsupported(db.compact_storage().unwrap_err()));
    assert!(unsupported(db.rewrite_payloads(10).unwrap_err()));
    let dir = std::env::temp_dir().join("semantic-maintenance-unsupported-backup");
    assert!(unsupported(db.backup(&dir).unwrap_err()));
    assert_eq!(db.storage_stats().unwrap(), Default::default());
}
