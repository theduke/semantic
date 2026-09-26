//! Maintenance operations over the key-value entity store: verification of
//! raw index entries, counters and payloads, repairs, reindexing and
//! batched payload rewrites.

use std::collections::BTreeSet;

use semantic_data::query::BinaryOp;
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::catalog::{CollectionKind, LocalCollectionId, LocalIndexId};
use semantic_db_core::embedded::{EmbeddedDb, EntityStorage};
use semantic_db_core::{
    Expr, Operand, ReindexTarget, SelectQuery, VerifyOptions, VerifyProblemKind as Kind,
    VerifyReport,
};

use super::entity_codec::{ENTITY_FORMAT_VERSION_V2_COMPACT, EntityPayloadFormat, payload_version};
use super::{EntityStore, KvEngine, MemoryKvEngine};
use crate::keys;

type Db = EmbeddedDb<EntityStore<MemoryKvEngine>>;

struct Fixture {
    db: Db,
    events: LocalCollectionId,
    kind: LocalIndexId,
}

fn event(id: &str, kind: &str) -> Object {
    Object::from_iter([
        ("id".to_string(), Value::String(id.into())),
        ("kind".to_string(), Value::String(kind.into())),
    ])
}

fn fixture_with(store: EntityStore<MemoryKvEngine>) -> Fixture {
    let mut db = EmbeddedDb::open(store).unwrap();
    let events = db
        .create_collection("events", CollectionKind::Polymorphic)
        .unwrap();
    db.create_index("events_kind_idx", events, "kind", false)
        .unwrap();
    for (id, kind) in [("e1", "music"), ("e2", "video"), ("e3", "music")] {
        db.insert("events", id, event(id, kind)).unwrap();
    }
    db.insert("events", "e2", event("e2", "talk")).unwrap();
    db.insert("events", "e4", event("e4", "gone")).unwrap();
    db.delete("events", "e4").unwrap();
    let kind = db
        .catalog()
        .find_equality_index(events, "kind")
        .unwrap()
        .lid;
    Fixture { db, events, kind }
}

fn fixture() -> Fixture {
    fixture_with(EntityStore::new(MemoryKvEngine::new()))
}

/// Modify the raw store below the database and reopen it.
fn tamper(db: Db, change: impl FnOnce(&mut EntityStore<MemoryKvEngine>)) -> Db {
    let (_, mut store) = db.into_parts();
    change(&mut store);
    EmbeddedDb::open(store).unwrap()
}

fn kinds(report: &VerifyReport) -> BTreeSet<Kind> {
    report.problems.iter().map(|problem| problem.kind).collect()
}

fn music(db: &Db) -> BTreeSet<String> {
    db.select(
        SelectQuery::new()
            .with_collection("events")
            .with_predicate(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "kind",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                    "music".into(),
                )))),
            }),
    )
    .unwrap()
    .into_iter()
    .map(|row| row.get("id").unwrap().as_str().unwrap().to_string())
    .collect()
}

fn kind_key(index: LocalIndexId, kind: &str, id: &str) -> Vec<u8> {
    keys::index_key(index, None, &Value::String(kind.into()), id)
}

#[test]
fn verify_is_clean_after_regular_writes() {
    let Fixture { mut db, .. } = fixture();
    let report = db.verify(&VerifyOptions::all()).unwrap();
    assert!(report.is_ok(), "{report}");
    assert!(report.checked.rows >= 3);
    assert!(report.checked.index_entries >= 3);
    assert!(report.checked.counters > 0);
    // The memory engine has no physical integrity check.
    assert_eq!(report.skipped.len(), 1, "{report}");
}

#[test]
fn verify_finds_missing_and_stale_index_entries_and_repair_fixes_them() {
    let Fixture { db, kind, .. } = fixture();
    let db = tamper(db, |store| {
        let engine = store.engine_mut();
        engine.delete(&kind_key(kind, "music", "e1")).unwrap();
        engine.delete(&kind_key(kind, "talk", "e2")).unwrap();
        // An entry of a deleted row and one its row does not derive.
        engine
            .put(kind_key(kind, "gone", "e4"), Vec::new())
            .unwrap();
    });
    assert_eq!(music(&db), BTreeSet::from(["e3".to_string()]));

    let mut db = db;
    let options = VerifyOptions::all();
    let report = db.verify(&options).unwrap();
    assert_eq!(
        kinds(&report),
        BTreeSet::from([
            Kind::MissingIndexEntry,
            Kind::StaleIndexEntry,
            Kind::WrongIndexEntryCount
        ]),
        "{report}"
    );
    let missing = report
        .problems
        .iter()
        .filter(|problem| problem.kind == Kind::MissingIndexEntry)
        .map(|problem| problem.entity_id.clone().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(missing, BTreeSet::from(["e1".into(), "e2".into()]));
    let stale = report
        .problems
        .iter()
        .find(|problem| problem.kind == Kind::StaleIndexEntry)
        .unwrap();
    assert_eq!(stale.entity_id.as_deref(), Some("e4"));
    assert_eq!(stale.index.as_deref(), Some("events_kind_idx"));

    let repair = db.repair(&options).unwrap();
    assert_eq!(repair.rebuilt.indexes.len(), 1, "{repair}");
    assert_eq!(repair.rebuilt.indexes[0].entries, Some(3));
    assert!(repair.after.is_ok(), "{}", repair.after);
    assert_eq!(
        music(&db),
        BTreeSet::from(["e1".to_string(), "e3".to_string()])
    );
}

#[test]
fn reindex_recounts_index_entries() {
    let Fixture { db, kind, .. } = fixture();
    let mut db = tamper(db, |store| {
        store
            .engine_mut()
            .delete(&kind_key(kind, "music", "e1"))
            .unwrap();
    });
    // The raw delete bypassed the counters.
    assert_eq!(db.storage().index_entry_count(kind).unwrap(), Some(3));
    assert_eq!(music(&db).len(), 1);

    let report = db
        .reindex(&ReindexTarget::Collection("events".into()))
        .unwrap();
    assert!(
        report
            .indexes
            .iter()
            .any(|index| index.index == "events_kind_idx")
    );
    assert_eq!(db.storage().index_entry_count(kind).unwrap(), Some(3));
    assert_eq!(music(&db).len(), 2);
    assert!(db.verify(&VerifyOptions::all()).unwrap().is_ok());

    // `All` rebuilds everything and leaves a clean database.
    let report = db.reindex(&ReindexTarget::All).unwrap();
    assert!(!report.derived.is_empty());
    assert!(db.verify(&VerifyOptions::all()).unwrap().is_ok());
}

#[test]
fn verify_finds_wrong_row_counters_and_repair_recounts_them() {
    let Fixture { db, events, .. } = fixture();
    let mut db = tamper(db, |store| {
        store
            .put_raw(
                keys::collection_rows_key(events),
                99u64.to_be_bytes().to_vec(),
            )
            .unwrap();
    });
    let options = VerifyOptions::all();
    let report = db.verify(&options).unwrap();
    assert_eq!(
        kinds(&report),
        BTreeSet::from([Kind::WrongRowCount]),
        "{report}"
    );
    assert_eq!(report.problems[0].collection.as_deref(), Some("events"));

    let repair = db.repair(&options).unwrap();
    assert!(repair.after.is_ok(), "{}", repair.after);
    assert_eq!(db.storage().collection_row_count(events).unwrap(), Some(3));
}

#[test]
fn verify_reports_corrupt_payloads_with_their_entity() {
    let Fixture { db, events, .. } = fixture();
    let mut db = tamper(db, |store| {
        store
            .put_raw(keys::entity_key(events, "e2"), vec![0xff, 0xff, 1, 2, 3])
            .unwrap();
    });
    let options = VerifyOptions::all();
    let report = db.verify(&options).unwrap();
    assert_eq!(
        kinds(&report),
        BTreeSet::from([Kind::CorruptPayload]),
        "{report}"
    );
    assert_eq!(report.problems[0].entity_id.as_deref(), Some("e2"));

    // Payloads cannot be repaired; the problem remains.
    let repair = db.repair(&options).unwrap();
    assert_eq!(kinds(&repair.after), BTreeSet::from([Kind::CorruptPayload]));
}

#[test]
fn rewrite_payloads_upgrades_legacy_rows_in_batches() {
    let Fixture { db, events, .. } = fixture_with(EntityStore::with_payload_format(
        MemoryKvEngine::new(),
        EntityPayloadFormat::SelfContained,
    ));
    let (_, store) = db.into_parts();
    let store = EntityStore::with_payload_format(store.into_inner(), EntityPayloadFormat::Compact);
    let mut db = EmbeddedDb::open(store).unwrap();
    let total = db
        .storage()
        .scan_raw_prefix(&[keys::TAG_ENTITY])
        .unwrap()
        .len() as u64;

    let report = db.rewrite_payloads(2).unwrap();
    assert_eq!(report.scanned, total);
    assert_eq!(
        report.rewritten, total,
        "every row was written in version 1"
    );
    assert_eq!(report.batches, total.div_ceil(2));
    for (_, payload) in db.storage().scan_raw_prefix(&[keys::TAG_ENTITY]).unwrap() {
        assert_eq!(
            payload_version(&payload).unwrap(),
            ENTITY_FORMAT_VERSION_V2_COMPACT
        );
    }
    assert_eq!(music(&db).len(), 2);
    assert_eq!(db.storage().collection_row_count(events).unwrap(), Some(3));
    assert!(db.verify(&VerifyOptions::all()).unwrap().is_ok());

    let again = db.rewrite_payloads(2).unwrap();
    assert_eq!(
        (again.scanned, again.rewritten, again.batches),
        (total, 0, 0)
    );
}
