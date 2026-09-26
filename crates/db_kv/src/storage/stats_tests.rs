//! Maintained collection row counts and index entry counts.

use semantic_data::query::{AggregateOp, BinaryOp};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::catalog::{CollectionKind, LocalCollectionId, LocalIndexId};
use semantic_db_core::embedded::{EmbeddedDb, EntityStorage, StorageWriteOp, StoredEntity};
use semantic_db_core::{
    Batch, BatchOperation, DeleteQuery, Expr, FunctionArg, Operand, QueryField, SelectQuery,
    UpdateQuery,
};

use super::stats::StatsBackfill;
use super::{EntityStore, KvEngine, MemoryKvEngine};
use crate::keys;

type Db = EmbeddedDb<EntityStore<MemoryKvEngine>>;

fn object(id: &str, kind: &str) -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String(id.to_string()));
    object.insert("kind", Value::String(kind.to_string()));
    object
}

fn upsert(id: &str, kind: &str) -> BatchOperation {
    BatchOperation::Upsert {
        collection: "items".to_string(),
        id: id.to_string(),
        object: object(id, kind),
    }
}

fn batch(ops: impl IntoIterator<Item = BatchOperation>) -> Batch {
    ops.into_iter().fold(Batch::new(), Batch::with_op)
}

fn kind_is(kind: &str) -> Expr {
    Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
            "kind",
        ])))),
        right: Box::new(Expr::Operand(Operand::Literal(Value::String(
            kind.to_string(),
        )))),
    }
}

/// A database with an `items` collection indexed by `kind`.
fn items_db() -> (Db, LocalCollectionId, LocalIndexId) {
    let mut db = crate::open_memory().unwrap();
    let items = db
        .create_collection("items", CollectionKind::Untyped)
        .unwrap();
    db.create_index("by_kind", items, "kind", false).unwrap();
    let index = by_kind(&db);
    (db, items, index)
}

fn by_kind(db: &Db) -> LocalIndexId {
    db.catalog()
        .indexes()
        .find(|(_, index)| index.schema.name == "by_kind")
        .map(|(lid, _)| lid)
        .unwrap()
}

/// Assert the maintained counts, and that they match the stored keys.
fn assert_counts(
    store: &EntityStore<MemoryKvEngine>,
    collection: LocalCollectionId,
    index: LocalIndexId,
    rows: u64,
    entries: u64,
) {
    let counted_rows = store.count_collection_entities(collection).unwrap();
    let counted_entries = store.index_keys(index).unwrap().len() as u64;
    assert_eq!((counted_rows, counted_entries), (rows, entries));
    assert_eq!(store.collection_row_count(collection).unwrap(), Some(rows));
    assert_eq!(store.index_entry_count(index).unwrap(), Some(entries));
    let snapshot = store.snapshot().unwrap();
    assert_eq!(
        snapshot.collection_row_count(collection).unwrap(),
        Some(rows)
    );
    assert_eq!(snapshot.index_entry_count(index).unwrap(), Some(entries));
}

fn count_star(db: &Db) -> Value {
    let rows = db
        .select(
            SelectQuery::new()
                .with_collection("items")
                .with_projection(vec![QueryField {
                    expr: Box::new(Expr::Aggregate {
                        op: AggregateOp::Count,
                        distinct: false,
                        arg: Box::new(FunctionArg::Wildcard),
                    }),
                    alias: Some("n".to_string()),
                    wildcard: None,
                }]),
        )
        .unwrap();
    rows[0].get("n").cloned().unwrap()
}

#[test]
fn counters_follow_compact_and_dataset_writes() {
    let (mut db, items, index) = items_db();
    assert_counts(db.storage(), items, index, 0, 0);

    // Compact path: inserts.
    db.transact(batch([
        upsert("one", "music"),
        upsert("two", "video"),
        upsert("three", "music"),
    ]))
    .unwrap();
    assert_counts(db.storage(), items, index, 3, 3);

    // Replacing a row keeps the row count; its index entry is replaced.
    db.transact(batch([upsert("two", "music")])).unwrap();
    assert_counts(db.storage(), items, index, 3, 3);

    // Dataset path: updates and deletes.
    db.update_where(
        UpdateQuery::new()
            .with_collection("items")
            .with_predicate(kind_is("music"))
            .set(
                FieldPath::from_fields(["kind"]),
                Expr::Operand(Operand::Literal(Value::String("audio".to_string()))),
            ),
    )
    .unwrap();
    assert_counts(db.storage(), items, index, 3, 3);
    db.delete("items", "one").unwrap();
    assert_counts(db.storage(), items, index, 2, 2);
    db.delete_where(
        DeleteQuery::new()
            .with_collection("items")
            .with_predicate(kind_is("audio")),
    )
    .unwrap();
    assert_counts(db.storage(), items, index, 0, 0);
    assert_eq!(count_star(&db), Value::I64(0));

    db.transact(batch([upsert("four", "text")])).unwrap();
    assert_counts(db.storage(), items, index, 1, 1);
    assert_eq!(count_star(&db), Value::I64(1));
}

#[test]
fn index_counters_follow_index_build_clear_and_reset() {
    let mut db = crate::open_memory().unwrap();
    let items = db
        .create_collection("items", CollectionKind::Untyped)
        .unwrap();
    db.transact(batch([upsert("one", "music"), upsert("two", "video")]))
        .unwrap();
    // Building an index over existing rows counts its entries.
    db.create_index("by_kind", items, "kind", false).unwrap();
    let index = by_kind(&db);
    let (_, mut store) = db.into_parts();
    assert_counts(&store, items, index, 2, 2);

    let entity = |id: &str, kind: &str| StoredEntity {
        collection: items.0,
        kind: semantic_db_core::embedded::StoredEntityKind::Untyped,
        id: id.to_string(),
        object: object(id, kind),
    };
    let schema = semantic_db_core::catalog::IndexSchema {
        lid: index,
        schema: semantic_data::schema::IndexSchema {
            id: "items.by_kind".to_string(),
            name: "by_kind".to_string(),
            kind: semantic_data::schema::IndexKind::Equality,
            collection: "items".to_string(),
            key_path: semantic_data::schema::KeyPath {
                segments: vec!["kind".to_string()],
            },
            unique: false,
            extra_key_paths: Vec::new(),
            predicate: None,
            analyzer: Default::default(),
        },
        collection: items,
        canonical_field: "kind".to_string(),
        field_id: None,
        attr_id: None,
    };

    // Reset and rebuild with one entry in one batch.
    store
        .apply_batch(&[
            StorageWriteOp::ResetIndex(index),
            StorageWriteOp::IndexEntity {
                index: schema.clone(),
                entity_id: "one".to_string(),
                object: object("one", "music"),
            },
        ])
        .unwrap();
    assert_counts(&store, items, index, 2, 1);

    store
        .apply_batch(&[StorageWriteOp::ClearIndex(index)])
        .unwrap();
    assert_counts(&store, items, index, 2, 0);

    // A put staged before a clear in the same batch is not counted.
    store
        .apply_batch(&[
            StorageWriteOp::PutEntity(entity("three", "text")),
            StorageWriteOp::ClearCollection(items),
            StorageWriteOp::PutEntity(entity("four", "text")),
        ])
        .unwrap();
    assert_counts(&store, items, index, 1, 0);

    // The typed raw helpers maintain counters too.
    store.put_entity(&entity("five", "text")).unwrap();
    store
        .put_index_entry(index, &Value::String("text".to_string()), "five")
        .unwrap();
    assert_counts(&store, items, index, 2, 1);
    store.delete_entity(items, "five").unwrap();
    store
        .delete_index_entry(index, &Value::String("text".to_string()), "five")
        .unwrap();
    assert_counts(&store, items, index, 1, 0);
}

/// Remove every stats key and the stats marker through the engine,
/// producing a database as written before counters were maintained.
fn strip_stats(store: &mut EntityStore<MemoryKvEngine>) {
    let keys = store
        .scan_raw_prefix(&[keys::TAG_STATS])
        .unwrap()
        .into_iter()
        .map(|(key, _)| key)
        .chain([keys::stats_version_key()])
        .collect::<Vec<_>>();
    for key in keys {
        store.engine_mut().delete(&key).unwrap();
    }
}

#[test]
fn legacy_database_without_counters_is_counted_and_backfilled_on_open() {
    let (mut db, items, index) = items_db();
    db.transact(batch([
        upsert("one", "music"),
        upsert("two", "video"),
        upsert("three", "music"),
    ]))
    .unwrap();
    let (_, mut store) = db.into_parts();
    strip_stats(&mut store);
    assert!(
        store
            .scan_raw_prefix(&[keys::TAG_STATS])
            .unwrap()
            .is_empty()
    );

    // Unknown counts; writes do not create partial counters.
    assert_eq!(store.collection_row_count(items).unwrap(), None);
    assert_eq!(store.index_entry_count(index).unwrap(), None);
    store
        .apply_batch(&[StorageWriteOp::DeleteEntity {
            collection: items,
            entity_id: "three".to_string(),
        }])
        .unwrap();
    assert!(
        store
            .scan_raw_prefix(&[keys::TAG_STATS])
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.count_collection_entities(items).unwrap(), 2);

    // Opening backfills the counters in one transaction.
    let db = Db::open(store).unwrap();
    // The storage-level delete above left `three`'s index entry behind.
    assert_counts(db.storage(), items, index, 2, 3);
    assert_eq!(count_star(&db), Value::I64(2));

    let (_, mut store) = db.into_parts();
    assert_eq!(store.ensure_stats().unwrap(), StatsBackfill::UpToDate);
    strip_stats(&mut store);
    assert_eq!(
        store.ensure_stats().unwrap(),
        StatsBackfill::Backfilled {
            collections: store
                .scan_raw_prefix(&[keys::TAG_STATS, keys::STATS_COLLECTION_ROWS])
                .unwrap()
                .len(),
            indexes: store
                .scan_raw_prefix(&[keys::TAG_STATS, keys::STATS_INDEX_ENTRIES])
                .unwrap()
                .len(),
        }
    );
    assert_counts(&store, items, index, 2, 3);
}
