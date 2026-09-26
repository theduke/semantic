//! Change feed delivery on redb with concurrent writers.

use std::sync::Arc;

use futures::StreamExt as _;
use semantic_data::query::{Batch, BatchOperation};
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::CollectionKind;
use semantic_db_core::{ChangeFeedItem, ChangeSubscriptionOptions, Db};
use semantic_db_kv::EntityStore;

use super::{DbOpenMode, RedbBackend, RedbDatabase, RedbKvEngine};

const ITEMS: &str = "feed_items";
const WRITES: usize = 20;
const CAPACITY: usize = 4;

async fn write(db: &Db, id: String) {
    let mut object = Object::new();
    object.insert("id", Value::String(id.clone()));
    db.execute_batch(Batch::new().with_op(BatchOperation::Upsert {
        collection: ITEMS.into(),
        id,
        object,
    }))
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_writers_are_delivered_in_revision_order() {
    let dir = tempfile::tempdir().unwrap();
    let engine = RedbKvEngine::open(dir.path().join("feed"), DbOpenMode::AutoCreate).unwrap();
    let mut db = RedbDatabase::open(EntityStore::new(engine))
        .unwrap()
        .with_change_feed_capacity(CAPACITY);
    db.create_collection(ITEMS, CollectionKind::Polymorphic)
        .unwrap();
    let db = Arc::new(Db::new(RedbBackend::new(db)));
    let mut live = db
        .subscribe_changes(ChangeSubscriptionOptions::default())
        .unwrap();
    // Never polled while writing, so it falls behind.
    let mut slow = db
        .subscribe_changes(ChangeSubscriptionOptions::default())
        .unwrap();

    let consumer = tokio::spawn(async move {
        let mut revisions = Vec::new();
        let mut lagged = 0;
        while let Some(item) = live.next().await {
            match item {
                ChangeFeedItem::Event(event) => {
                    revisions.push(event.revision);
                    if event.changes.iter().any(|change| change.id == "done") {
                        break;
                    }
                }
                ChangeFeedItem::Lagged { skipped, .. } => lagged += skipped,
            }
        }
        (revisions, lagged)
    });
    let writers = ["left", "right"].map(|prefix| {
        let db = Arc::clone(&db);
        tokio::spawn(async move {
            for index in 0..WRITES {
                write(&db, format!("{prefix}-{index}")).await;
            }
        })
    });
    for writer in writers {
        writer.await.unwrap();
    }
    write(&db, "done".into()).await;

    let (revisions, lagged) = consumer.await.unwrap();
    assert!(
        revisions.windows(2).all(|pair| pair[0] < pair[1]),
        "revisions must increase strictly: {revisions:?}"
    );
    assert_eq!(revisions.len() as u64 + lagged, 2 * WRITES as u64 + 1);

    let Some(ChangeFeedItem::Lagged {
        skipped,
        resume_revision,
    }) = slow.next().await
    else {
        panic!("the slow subscriber must lag");
    };
    assert_eq!(skipped, (2 * WRITES + 1 - CAPACITY) as u64);
    let mut retained = Vec::new();
    for _ in 0..CAPACITY {
        match slow.next().await {
            Some(ChangeFeedItem::Event(event)) => retained.push(event.revision),
            other => panic!("expected an event, got {other:?}"),
        }
    }
    assert_eq!(retained[0], resume_revision);
    assert_eq!(retained.last(), revisions.last());
}
