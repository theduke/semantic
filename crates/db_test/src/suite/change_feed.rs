//! Change feed ([`Db::subscribe_changes`]).
//!
//! Backends without a change feed (reported as an `Unsupported` storage
//! error) skip these checks. Events are published before a write returns,
//! so every check reads the events that are ready right after the write.

use futures::{FutureExt as _, StreamExt as _};
use semantic_db_core::{
    ChangeEvent, ChangeFeedItem, ChangeKind, ChangeSource, ChangeStream, ChangeSubscriptionOptions,
    StorageErrorKind, TransactionOptions,
};

use super::*;

const ITEMS: &str = "suite_feed_items";
const OTHER: &str = "suite_feed_other";

fn item(id: &str, label: &str) -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String(id.to_string()));
    object.insert("label", Value::String(label.to_string()));
    object
}

fn upsert(collection: &str, id: &str, label: &str) -> BatchOperation {
    BatchOperation::Upsert {
        collection: collection.into(),
        id: id.into(),
        object: item(id, label),
    }
}

fn subscribe(db: &Db, options: ChangeSubscriptionOptions) -> ChangeStream {
    db.subscribe_changes(options).unwrap()
}

/// All events that are ready.
fn ready_events(stream: &mut ChangeStream) -> Vec<ChangeEvent> {
    std::iter::from_fn(|| stream.next().now_or_never().flatten())
        .map(|item| match item {
            ChangeFeedItem::Event(event) => event,
            other => panic!("expected an event, got {other:?}"),
        })
        .collect()
}

fn single_event(stream: &mut ChangeStream) -> ChangeEvent {
    let mut events = ready_events(stream);
    assert_eq!(events.len(), 1, "expected one event: {events:?}");
    events.remove(0)
}

fn kinds(event: &ChangeEvent) -> Vec<(&str, ChangeKind)> {
    event
        .changes
        .iter()
        .map(|change| (change.id.as_str(), change.kind))
        .collect()
}

fn label(row: &Option<Object>) -> Option<&str> {
    row.as_ref()?.get("label")?.as_str()
}

pub async fn test_change_feed(db: &Db) {
    match db.subscribe_changes(ChangeSubscriptionOptions::default()) {
        Ok(_) => {}
        Err(error) if error.storage_kind() == Some(StorageErrorKind::Unsupported) => return,
        Err(error) => panic!("subscribe to changes: {error}"),
    }
    setup(db).await;
    test_writes(db).await;
    test_transaction_commits(db).await;
    test_filters_and_subscribers(db).await;
    test_concurrent_writers(db).await;
}

async fn setup(db: &Db) {
    let mut ddl = DdlBatch::new();
    for name in [ITEMS, OTHER] {
        ddl = ddl.with_op(DdlOperation::UpsertCollection {
            name: name.into(),
            kind: DdlCollectionKind::Polymorphic,
            integrity_mode: IntegrityMode::Permissive,
        });
    }
    let ddl = ddl.with_op(DdlOperation::UpsertIndex {
        name: "suite_feed_label".into(),
        collection: ITEMS.into(),
        field: "label".into(),
        unique: true,
        kind: semantic_data::schema::IndexKind::Equality,
        extra_fields: Vec::new(),
        predicate: None,
        analyzer: Default::default(),
    });
    let mut stream = subscribe(db, ChangeSubscriptionOptions::default());
    db.execute_ddl(ddl).await.unwrap();
    let event = single_event(&mut stream);
    assert_eq!(event.source, ChangeSource::Ddl);
    assert!(event.catalog_changed);
}

async fn test_writes(db: &Db) {
    let mut stream = subscribe(db, ChangeSubscriptionOptions::default().with_payloads());

    db.execute_batch(
        Batch::new()
            .with_op(upsert(ITEMS, "a", "a1"))
            .with_op(upsert(ITEMS, "b", "b1")),
    )
    .await
    .unwrap();
    let created = single_event(&mut stream);
    assert_eq!(created.source, ChangeSource::Batch);
    assert!(!created.catalog_changed);
    assert_eq!(
        kinds(&created),
        [("a", ChangeKind::Created), ("b", ChangeKind::Created)]
    );
    assert_eq!(label(&created.changes[0].after), Some("a1"));

    db.update_where(
        UpdateQuery::new()
            .with_collection(ITEMS)
            .with_predicate(eq_predicate(
                FieldPath::from_fields(["id"]),
                Value::String("a".into()),
            ))
            .set(
                FieldPath::from_fields(["label"]),
                Expr::Operand(Operand::Literal(Value::String("a2".into()))),
            ),
    )
    .await
    .unwrap();
    let updated = single_event(&mut stream);
    assert!(updated.revision > created.revision);
    assert_eq!(kinds(&updated), [("a", ChangeKind::Updated)]);
    assert_eq!(label(&updated.changes[0].before), Some("a1"));
    assert_eq!(label(&updated.changes[0].after), Some("a2"));

    db.delete_where(
        DeleteQuery::new()
            .with_collection(ITEMS)
            .with_predicate(eq_predicate(
                FieldPath::from_fields(["id"]),
                Value::String("b".into()),
            )),
    )
    .await
    .unwrap();
    let deleted = single_event(&mut stream);
    assert!(deleted.revision > updated.revision);
    assert_eq!(kinds(&deleted), [("b", ChangeKind::Deleted)]);
    assert_eq!(deleted.changes[0].after, None);

    // A unique violation fails the write and publishes nothing.
    db.execute_batch(Batch::new().with_op(upsert(ITEMS, "c", "a2")))
        .await
        .unwrap_err();
    assert!(ready_events(&mut stream).is_empty());
}

async fn test_transaction_commits(db: &Db) {
    let begin = || db.begin_transaction(TransactionOptions::default());
    match begin().await {
        Ok(tx) => tx.rollback().await.unwrap(),
        Err(error) if error.storage_kind() == Some(StorageErrorKind::Unsupported) => return,
        Err(error) => panic!("begin transaction: {error}"),
    }
    let mut stream = subscribe(db, ChangeSubscriptionOptions::default());

    let tx = begin().await.unwrap();
    tx.upsert(ITEMS.into(), "tx".into(), item("tx", "tx"))
        .await
        .unwrap();
    tx.upsert(ITEMS.into(), "temp".into(), item("temp", "temp"))
        .await
        .unwrap();
    assert!(tx.delete(ITEMS.into(), "temp".into()).await.unwrap());
    assert!(tx.delete(ITEMS.into(), "a".into()).await.unwrap());
    let commit = tx.commit().await.unwrap();
    let event = single_event(&mut stream);
    assert_eq!(event.source, ChangeSource::Transaction);
    assert_eq!(Some(event.revision), commit.revision);
    assert_eq!(
        kinds(&event),
        [("a", ChangeKind::Deleted), ("tx", ChangeKind::Created)]
    );

    let first = begin().await.unwrap();
    let second = begin().await.unwrap();
    first
        .upsert(ITEMS.into(), "first".into(), item("first", "first"))
        .await
        .unwrap();
    second
        .upsert(ITEMS.into(), "second".into(), item("second", "second"))
        .await
        .unwrap();
    first.commit().await.unwrap();
    assert_eq!(
        kinds(&single_event(&mut stream)),
        [("first", ChangeKind::Created)]
    );
    assert!(matches!(
        second.commit().await,
        Err(DbError::TransactionConflict(_))
    ));
    assert!(ready_events(&mut stream).is_empty());
}

async fn test_filters_and_subscribers(db: &Db) {
    let mut first = subscribe(db, ChangeSubscriptionOptions::default().with_payloads());
    let mut second = subscribe(db, ChangeSubscriptionOptions::default().with_payloads());
    let mut other = subscribe(
        db,
        ChangeSubscriptionOptions::default().with_collections([OTHER]),
    );
    let dropped = subscribe(db, ChangeSubscriptionOptions::default());
    drop(dropped);

    db.execute_batch(
        Batch::new()
            .with_op(upsert(ITEMS, "f1", "f1"))
            .with_op(upsert(OTHER, "o1", "o1")),
    )
    .await
    .unwrap();
    db.execute_batch(Batch::new().with_op(upsert(ITEMS, "f2", "f2")))
        .await
        .unwrap();

    let events = ready_events(&mut first);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].changes.len(), 2);
    assert_eq!(ready_events(&mut second), events);

    let events = ready_events(&mut other);
    assert_eq!(events.len(), 1);
    assert_eq!(kinds(&events[0]), [("o1", ChangeKind::Created)]);
    assert_eq!(events[0].changes[0].after, None, "payloads not requested");
}

async fn test_concurrent_writers(db: &Db) {
    const WRITES: usize = 10;
    let mut stream = subscribe(db, ChangeSubscriptionOptions::default());
    let writer = |prefix: &'static str| async move {
        for index in 0..WRITES {
            let id = format!("{prefix}-{index}");
            db.execute_batch(Batch::new().with_op(upsert(OTHER, &id, &id)))
                .await
                .unwrap();
        }
    };
    futures::join!(writer("left"), writer("right"));

    let events = ready_events(&mut stream);
    assert_eq!(events.len(), 2 * WRITES);
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].revision < pair[1].revision),
        "revisions must increase strictly"
    );
    let ids = events
        .iter()
        .flat_map(|event| event.changes.iter().map(|change| change.id.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 2 * WRITES);
}
