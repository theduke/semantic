//! Change feed events of the embedded database.

use futures::{FutureExt as _, StreamExt as _};
use semantic_data::query::{BinaryOp, Expr as DataExpr, Operand as DataOperand};
use semantic_data::schema::{
    Meta, Migration, MigrationCollectionKind, MigrationDdlOperation, MigrationIntegrityMode, Module,
};

use super::*;
use crate::embedded::MemoryEntityStorage;
use crate::{
    BatchReturn, ChangeEvent, ChangeFeedItem, ChangeKind, ChangeSource, ChangeSubscription,
    ChangeSubscriptionOptions, Expr, Operand,
};

const NOTES: &str = "feed_notes";
const TASKS: &str = "feed_tasks";
const PACKAGE_ROWS: &str = "feed_package_rows";

fn note(id: &str, title: &str) -> Object {
    Object::from_iter([
        ("id".to_string(), Value::String(id.to_string())),
        ("label".to_string(), Value::String(title.to_string())),
    ])
}

fn upsert(collection: &str, id: &str, title: &str) -> BatchOperation {
    BatchOperation::Upsert {
        collection: collection.into(),
        id: id.into(),
        object: note(id, title),
    }
}

fn delete(collection: &str, id: &str) -> BatchOperation {
    BatchOperation::DeleteById {
        collection: collection.into(),
        id: id.into(),
    }
}

fn id_is(id: &str) -> Expr {
    Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
            "id",
        ])))),
        right: Box::new(Expr::Operand(Operand::Literal(Value::String(id.into())))),
    }
}

fn db() -> EmbeddedDb<MemoryEntityStorage> {
    let mut db = EmbeddedDb::in_memory();
    for name in [NOTES, TASKS] {
        db.create_collection(name, CollectionKind::Polymorphic)
            .unwrap();
    }
    db
}

fn revision<S: EntityStorage>(db: &EmbeddedDb<S>) -> u64 {
    db.current_revision().unwrap().unwrap()
}

/// The next item if one is ready.
fn next(subscription: &mut ChangeSubscription) -> Option<ChangeFeedItem> {
    subscription.next().now_or_never().flatten()
}

fn event(subscription: &mut ChangeSubscription) -> ChangeEvent {
    match next(subscription) {
        Some(ChangeFeedItem::Event(event)) => event,
        other => panic!("expected an event, got {other:?}"),
    }
}

/// All ready events.
fn events(subscription: &mut ChangeSubscription) -> Vec<ChangeEvent> {
    std::iter::from_fn(|| next(subscription))
        .map(|item| match item {
            ChangeFeedItem::Event(event) => event,
            other => panic!("expected an event, got {other:?}"),
        })
        .collect()
}

fn kinds(event: &ChangeEvent) -> Vec<(&str, &str, ChangeKind)> {
    event
        .changes
        .iter()
        .map(|change| (change.collection.as_str(), change.id.as_str(), change.kind))
        .collect()
}

fn title(row: &Option<Object>) -> Option<&str> {
    row.as_ref()?.get("label")?.as_str()
}

#[test]
fn one_event_per_batch_on_both_write_paths() {
    for dataset_path in [false, true] {
        let mut db = db();
        let mut changes =
            db.subscribe_changes(ChangeSubscriptionOptions::default().with_payloads());
        let write = |db: &mut EmbeddedDb<_>, batch: Batch| {
            if dataset_path {
                db.execute_batch(batch).map(drop)
            } else {
                db.execute_batch_returning(batch, BatchReturn::Stats)
                    .map(drop)
            }
        };

        write(
            &mut db,
            Batch::new()
                .with_op(upsert(NOTES, "a", "a1"))
                .with_op(upsert(NOTES, "b", "b1")),
        )
        .unwrap();
        let created = event(&mut changes);
        assert_eq!(created.source, ChangeSource::Batch);
        assert_eq!(created.revision, revision(&db));
        assert!(!created.catalog_changed);
        assert_eq!(created.catalog_version, db.catalog.snapshot().version);
        assert_eq!(
            kinds(&created),
            [
                (NOTES, "a", ChangeKind::Created),
                (NOTES, "b", ChangeKind::Created)
            ]
        );
        assert_eq!(created.changes[0].before, None);
        assert_eq!(title(&created.changes[0].after), Some("a1"));

        write(
            &mut db,
            Batch::new()
                .with_op(upsert(NOTES, "a", "a2"))
                .with_op(delete(NOTES, "b")),
        )
        .unwrap();
        let updated = event(&mut changes);
        assert!(updated.revision > created.revision);
        assert_eq!(
            kinds(&updated),
            [
                (NOTES, "a", ChangeKind::Updated),
                (NOTES, "b", ChangeKind::Deleted)
            ]
        );
        assert_eq!(title(&updated.changes[0].before), Some("a1"));
        assert_eq!(title(&updated.changes[0].after), Some("a2"));
        assert_eq!(title(&updated.changes[1].before), Some("b1"));
        assert_eq!(updated.changes[1].after, None);

        // Rewriting a row unchanged changes nothing.
        write(&mut db, Batch::new().with_op(upsert(NOTES, "a", "a2"))).unwrap();
        assert_eq!(next(&mut changes), None, "dataset path: {dataset_path}");
    }
}

#[test]
fn predicate_mutations_list_every_row_including_cascades() {
    let mut db = EmbeddedDb::in_memory();
    let mut author = super::tests::ref_ty("person");
    let TypeKind::Ref(reference) = &mut author.kind else {
        unreachable!("ref type")
    };
    reference.on_delete = semantic_data::schema::OnDelete::Cascade;
    super::tests::register_ref_schema(&mut db, author);
    let typed = |id: &str, ty: &str, author: Option<&str>| {
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
    };
    db.execute_batch(
        Batch::new()
            .with_op(typed("p1", "person", None))
            .with_op(typed("p2", "person", None))
            .with_op(typed("a1", "article", Some("p1")))
            .with_op(typed("a2", "article", Some("p1")))
            .with_op(typed("a3", "article", Some("p2"))),
    )
    .unwrap();
    let mut changes = db.subscribe_changes(ChangeSubscriptionOptions::default());

    db.update_where(
        UpdateQuery::new()
            .with_collection(DEFAULT_COLLECTION)
            .with_predicate(id_is("a3"))
            .set(
                FieldPath::from_fields(["author"]),
                Expr::Operand(Operand::Literal(Value::String("p1".into()))),
            ),
    )
    .unwrap();
    assert_eq!(
        kinds(&event(&mut changes)),
        [(DEFAULT_COLLECTION, "a3", ChangeKind::Updated)]
    );

    db.delete_where(
        DeleteQuery::new()
            .with_collection(DEFAULT_COLLECTION)
            .with_predicate(id_is("p1")),
    )
    .unwrap();
    let deleted = event(&mut changes);
    assert_eq!(deleted.source, ChangeSource::Batch);
    assert_eq!(
        kinds(&deleted),
        ["a1", "a2", "a3", "p1"].map(|id| (DEFAULT_COLLECTION, id, ChangeKind::Deleted))
    );
    assert_eq!(next(&mut changes), None);
}

#[test]
fn interactive_commit_emits_net_changes_once() {
    let mut db = db();
    db.execute_batch(Batch::new().with_op(upsert(NOTES, "keep", "k")))
        .unwrap();
    let mut changes = db.subscribe_changes(ChangeSubscriptionOptions::default().with_payloads());

    let mut tx = db.begin(TransactionOptions::default()).unwrap();
    tx.upsert(NOTES, "a", note("a", "1")).unwrap();
    tx.upsert(NOTES, "a", note("a", "2")).unwrap();
    tx.upsert(TASKS, "temp", note("temp", "t")).unwrap();
    assert!(tx.delete(TASKS, "temp").unwrap());
    assert!(tx.delete(NOTES, "keep").unwrap());
    assert_eq!(
        next(&mut changes),
        None,
        "nothing is published before commit"
    );
    let commit = tx.commit(&mut db).unwrap();

    let committed = event(&mut changes);
    assert_eq!(committed.source, ChangeSource::Transaction);
    assert_eq!(Some(committed.revision), commit.revision);
    assert_eq!(
        kinds(&committed),
        [
            (NOTES, "a", ChangeKind::Created),
            (NOTES, "keep", ChangeKind::Deleted)
        ]
    );
    assert_eq!(title(&committed.changes[0].after), Some("2"));
    assert_eq!(next(&mut changes), None);
}

#[test]
fn failed_and_conflicted_commits_emit_nothing() {
    let mut db = db();
    db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertIndex {
        name: "feed_title".into(),
        collection: NOTES.into(),
        field: "label".into(),
        unique: true,
        kind: IndexKind::Equality,
        extra_fields: Vec::new(),
        predicate: None,
    }))
    .unwrap();
    db.execute_batch(Batch::new().with_op(upsert(NOTES, "a", "x")))
        .unwrap();
    let mut changes = db.subscribe_changes(ChangeSubscriptionOptions::default());

    for dataset_path in [false, true] {
        let batch = Batch::new().with_op(upsert(NOTES, "b", "x"));
        let result = if dataset_path {
            db.execute_batch(batch).map(drop)
        } else {
            db.execute_batch_returning(batch, BatchReturn::Stats)
                .map(drop)
        };
        result.unwrap_err();
    }
    assert_eq!(next(&mut changes), None);

    let mut first = db.begin(TransactionOptions::default()).unwrap();
    let mut second = db.begin(TransactionOptions::default()).unwrap();
    first.upsert(NOTES, "c", note("c", "c")).unwrap();
    second.upsert(NOTES, "d", note("d", "d")).unwrap();
    first.commit(&mut db).unwrap();
    assert_eq!(
        kinds(&event(&mut changes)),
        [(NOTES, "c", ChangeKind::Created)]
    );
    assert!(matches!(
        second.commit(&mut db),
        Err(DbError::TransactionConflict(_))
    ));
    assert_eq!(next(&mut changes), None);
}

fn feed_package(update: bool) -> Package {
    let mut migrations = vec![Migration {
        module: "feed".into(),
        name: "001_rows".into(),
        description: None,
        operations: vec![
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertCollection {
                name: PACKAGE_ROWS.into(),
                kind: MigrationCollectionKind::Polymorphic,
                integrity_mode: MigrationIntegrityMode::Permissive,
            }),
            MigrationOperation::Insert {
                collection: PACKAGE_ROWS.into(),
                id: "seed".into(),
                object: note("seed", "v1"),
            },
        ],
        meta: Meta::default(),
    }];
    if update {
        migrations.push(Migration {
            module: "feed".into(),
            name: "002_retitle".into(),
            description: None,
            operations: vec![MigrationOperation::Update {
                query: semantic_data::query::UpdateQuery::new()
                    .with_collection(PACKAGE_ROWS)
                    .with_predicate(DataExpr::Binary {
                        op: BinaryOp::Eq,
                        left: Box::new(DataExpr::Operand(DataOperand::Field(
                            FieldPath::from_fields(["id"]),
                        ))),
                        right: Box::new(DataExpr::Operand(DataOperand::Literal(Value::String(
                            "seed".into(),
                        )))),
                    })
                    .set(
                        FieldPath::from_fields(["label"]),
                        DataExpr::Operand(DataOperand::Literal(Value::String("v2".into()))),
                    ),
            }],
            meta: Meta::default(),
        });
    }
    Package {
        name: "shared.feed".into(),
        root: Module {
            name: "feed".into(),
            constants: BTreeMap::new(),
            types: BTreeMap::new(),
            attributes: BTreeMap::new(),
            classes: BTreeMap::new(),
            interfaces: BTreeMap::new(),
            contracts: BTreeMap::new(),
            meta: Meta::default(),
        },
        modules: BTreeMap::new(),
        migrations,
        version: None,
        meta: Meta::default(),
    }
}

#[test]
fn ddl_and_migrations_report_catalog_changes() {
    let mut db = db();
    let mut all = db.subscribe_changes(ChangeSubscriptionOptions::default().with_payloads());
    let mut rows =
        db.subscribe_changes(ChangeSubscriptionOptions::default().with_collections([PACKAGE_ROWS]));

    db.create_collection("feed_other", CollectionKind::Polymorphic)
        .unwrap();
    let ddl = event(&mut all);
    assert_eq!(ddl.source, ChangeSource::Ddl);
    assert!(ddl.catalog_changed);
    assert!(ddl.changes.is_empty());
    assert_eq!(ddl.catalog_version, db.catalog.snapshot().version);

    db.upsert_package(feed_package(false)).unwrap();
    let created = event(&mut all);
    assert_eq!(created.source, ChangeSource::Migration);
    assert!(created.catalog_changed);
    assert_eq!(created.catalog_version, db.catalog.snapshot().version);
    assert_eq!(
        kinds(&created),
        [(PACKAGE_ROWS, "seed", ChangeKind::Created)]
    );

    db.upsert_package(feed_package(true)).unwrap();
    let updated = event(&mut all);
    assert_eq!(updated.source, ChangeSource::Migration);
    assert_eq!(
        kinds(&updated),
        [(PACKAGE_ROWS, "seed", ChangeKind::Updated)]
    );
    assert_eq!(title(&updated.changes[0].before), Some("v1"));
    assert_eq!(title(&updated.changes[0].after), Some("v2"));
    assert_eq!(next(&mut all), None);

    // Filtered subscriptions skip catalog-only commits.
    let filtered = events(&mut rows);
    assert_eq!(
        filtered
            .iter()
            .map(|event| event.revision)
            .collect::<Vec<_>>(),
        [created.revision, updated.revision]
    );
}

#[test]
fn internal_collections_are_excluded_by_default() {
    let mut db = db();
    let mut default = db.subscribe_changes(ChangeSubscriptionOptions::default());
    let mut internal = db.subscribe_changes(ChangeSubscriptionOptions::default().with_internal());
    db.activate_validation().unwrap();

    assert!(
        events(&mut default)
            .iter()
            .all(|event| event.source == ChangeSource::Ddl && event.changes.is_empty())
    );
    let maintenance = events(&mut internal)
        .into_iter()
        .filter(|event| event.source == ChangeSource::Maintenance)
        .collect::<Vec<_>>();
    assert_eq!(maintenance.len(), 1);
    assert_eq!(
        kinds(&maintenance[0]),
        [(validation::STATE, validation::ACTIVE, ChangeKind::Created)]
    );
}

#[test]
fn filters_payloads_and_multiple_subscribers() {
    let mut db = db();
    let mut first = db.subscribe_changes(ChangeSubscriptionOptions::default().with_payloads());
    let mut second = db.subscribe_changes(ChangeSubscriptionOptions::default().with_payloads());
    let mut notes =
        db.subscribe_changes(ChangeSubscriptionOptions::default().with_collections([NOTES]));

    db.execute_batch(
        Batch::new()
            .with_op(upsert(NOTES, "n", "note"))
            .with_op(upsert(TASKS, "t", "task")),
    )
    .unwrap();
    db.execute_batch(Batch::new().with_op(upsert(TASKS, "u", "task")))
        .unwrap();

    let received = events(&mut first);
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].changes.len(), 2);
    assert_eq!(title(&received[0].changes[1].after), Some("task"));
    assert_eq!(events(&mut second), received);

    let received = events(&mut notes);
    assert_eq!(received.len(), 1);
    assert_eq!(kinds(&received[0]), [(NOTES, "n", ChangeKind::Created)]);
    assert_eq!(received[0].changes[0].before, None);
    assert_eq!(received[0].changes[0].after, None, "payloads not requested");
}

#[test]
fn lagging_subscriber_receives_lagged() {
    let mut db = db().with_change_feed_capacity(2);
    let mut changes = db.subscribe_changes(ChangeSubscriptionOptions::default());
    let mut revisions = Vec::new();
    for index in 0..5 {
        db.execute_batch(Batch::new().with_op(upsert(NOTES, &format!("n{index}"), "t")))
            .unwrap();
        revisions.push(revision(&db));
    }
    assert_eq!(
        next(&mut changes),
        Some(ChangeFeedItem::Lagged {
            skipped: 3,
            resume_revision: revisions[3]
        })
    );
    assert_eq!(event(&mut changes).revision, revisions[3]);
    assert_eq!(event(&mut changes).revision, revisions[4]);
    assert_eq!(next(&mut changes), None);
}

#[test]
fn dropped_subscriptions_stop_delivery_without_affecting_writes() {
    let mut db = db();
    let dropped = db.subscribe_changes(ChangeSubscriptionOptions::default());
    assert_eq!(db.change_feed().subscriber_count(), 1);
    drop(dropped);
    assert_eq!(db.change_feed().subscriber_count(), 0);
    db.execute_batch(Batch::new().with_op(upsert(NOTES, "a", "t")))
        .unwrap();

    let mut later = db.subscribe_changes(ChangeSubscriptionOptions::default());
    assert_eq!(next(&mut later), None, "no history is replayed");
    db.execute_batch(Batch::new().with_op(upsert(NOTES, "b", "t")))
        .unwrap();
    assert_eq!(
        kinds(&event(&mut later)),
        [(NOTES, "b", ChangeKind::Created)]
    );
    assert!(db.get(NOTES, "a").unwrap().is_some());
}
