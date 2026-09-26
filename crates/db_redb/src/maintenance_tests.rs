//! Maintenance operations on redb files: backups, compaction, physical
//! integrity checks and repairs of index entries removed below the store.

use std::collections::BTreeSet;
use std::path::Path;

use semantic_data::query::BinaryOp;
use semantic_data::schema::DbOpenMode;
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::catalog::CollectionKind;
use semantic_db_core::embedded::EntityStorage as _;
use semantic_db_core::embedded::db::write_backup;
use semantic_db_core::{
    Db, Expr, Operand, SelectQuery, StorageErrorKind, TransactionOptions, VerifyOptions,
    VerifyProblemKind as Kind,
};
use semantic_db_kv::{EntityStore, KvEngine as _, keys};

use crate::{RedbDatabase, RedbKvEngine};

fn open(path: &Path, mode: DbOpenMode) -> RedbDatabase {
    RedbDatabase::open(EntityStore::new(RedbKvEngine::open(path, mode).unwrap())).unwrap()
}

fn event(id: &str, kind: &str) -> Object {
    Object::from_iter([
        ("id".to_string(), Value::String(id.into())),
        ("kind".to_string(), Value::String(kind.into())),
    ])
}

fn seeded(path: &Path) -> RedbDatabase {
    let mut db = open(path, DbOpenMode::AutoCreate);
    let events = db
        .create_collection("events", CollectionKind::Polymorphic)
        .unwrap();
    db.create_index("events_kind_idx", events, "kind", false)
        .unwrap();
    for (id, kind) in [("e1", "music"), ("e2", "video"), ("e3", "music")] {
        db.insert("events", id, event(id, kind)).unwrap();
    }
    db
}

fn music(db: &RedbDatabase) -> BTreeSet<String> {
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

#[test]
fn backup_copies_the_state_at_its_start() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = seeded(&dir.path().join("db.redb"));
    let source = db.backup_source().unwrap();
    let revision = source.revision();
    // Committed after the backup started: not part of the copy.
    db.insert("events", "e4", event("e4", "music")).unwrap();
    db.delete("events", "e1").unwrap();

    let target = dir.path().join("backup").join("copy.redb");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let report = write_backup(source, &target).unwrap();
    assert_eq!(report.revision, revision);
    assert!(report.entries > 0);
    assert!(report.bytes.is_some_and(|bytes| bytes > 0));
    assert_eq!(
        std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
        1,
        "no partial file is left behind"
    );

    let mut copy = open(&target, DbOpenMode::OpenExisting);
    assert_eq!(copy.current_revision().unwrap(), revision);
    assert!(copy.get("events", "e1").unwrap().is_some());
    assert!(copy.get("events", "e4").unwrap().is_none());
    assert_eq!(
        music(&copy),
        BTreeSet::from(["e1".to_string(), "e3".to_string()])
    );
    assert_eq!(
        copy.catalog().collection_by_name("events").unwrap().lid,
        db.catalog().collection_by_name("events").unwrap().lid
    );
    let report = copy.verify(&VerifyOptions::all()).unwrap();
    assert!(report.is_ok(), "{report}");

    // Backups never overwrite files.
    let error = db.backup(&target).unwrap_err();
    assert_eq!(error.storage_kind(), Some(StorageErrorKind::InvalidState));
}

#[tokio::test(flavor = "multi_thread")]
async fn backend_maintenance_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.redb");
    drop(seeded(&path));
    let db = Db::new(crate::open_backend(&path, DbOpenMode::OpenExisting).unwrap());

    let verify = db.verify(VerifyOptions::all()).await.unwrap();
    assert!(verify.is_ok(), "{verify}");
    assert!(verify.skipped.is_empty(), "{verify}");

    let stats = db.storage_stats().await.unwrap();
    assert!(stats.file_size_bytes.is_some());
    let compact = db.compact_storage().await.unwrap();
    assert!(compact.after.file_size_bytes.is_some());

    let export = db.export_snapshot().await.unwrap();
    assert_eq!(
        export.revision,
        db.verify(VerifyOptions::none()).await.unwrap().revision
    );
    assert!(export.catalog.collection_by_name("events").is_some());
    let rows = futures::StreamExt::collect::<Vec<_>>(export.stream).await;
    let events = rows
        .iter()
        .filter(|row| row.as_ref().unwrap().collection == "events")
        .count();
    assert_eq!(events, 3);

    // An open transaction snapshot blocks compaction and integrity checks.
    let transaction = db
        .begin_transaction(TransactionOptions::default())
        .await
        .unwrap();
    let error = db.compact_storage().await.unwrap_err();
    assert_eq!(error.storage_kind(), Some(StorageErrorKind::InvalidState));
    assert!(error.to_string().contains("retry"), "{error}");
    let integrity = db.verify(VerifyOptions::all()).await.unwrap();
    assert!(
        integrity
            .skipped
            .iter()
            .any(|skipped| skipped.contains("retry")),
        "{integrity}"
    );
    transaction.rollback().await.unwrap();

    let rewrite = db.rewrite_payloads(1).await.unwrap();
    assert_eq!(rewrite.rewritten, 0);
    assert!(rewrite.scanned > 0);

    let backup = db.backup(dir.path().join("copy.redb")).await.unwrap();
    assert_eq!(backup.revision, export.revision);
    let repair = db.repair(VerifyOptions::all()).await.unwrap();
    assert!(repair.after.is_ok(), "{repair}");
}

#[test]
fn repair_restores_index_entries_deleted_in_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.redb");
    let db = seeded(&path);
    let index = db
        .catalog()
        .find_equality_index(
            db.catalog().collection_by_name("events").unwrap().lid,
            "kind",
        )
        .unwrap()
        .lid;
    let (_, mut store) = db.into_parts();
    store
        .engine_mut()
        .delete(&keys::index_key(
            index,
            None,
            &Value::String("music".into()),
            "e1",
        ))
        .unwrap();
    drop(store);

    let mut db = open(&path, DbOpenMode::OpenExisting);
    assert_eq!(music(&db), BTreeSet::from(["e3".to_string()]));
    let report = db.verify(&VerifyOptions::all()).unwrap();
    assert_eq!(
        report
            .problems
            .iter()
            .map(|problem| problem.kind)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([Kind::MissingIndexEntry, Kind::WrongIndexEntryCount]),
        "{report}"
    );
    let repair = db.repair(&VerifyOptions::all()).unwrap();
    assert!(repair.after.is_ok(), "{repair}");
    assert_eq!(
        music(&db),
        BTreeSet::from(["e1".to_string(), "e3".to_string()])
    );
    assert_eq!(db.storage().index_entry_count(index).unwrap(), Some(3));
}
