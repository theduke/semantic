//! Interactive transactions on the embedded database.

use std::sync::atomic::Ordering;

use semantic_data::query::{BinaryOp, SortDirection};
use semantic_data::schema::IndexKind;

use super::*;
use crate::catalog::IndexDefinition;
use crate::embedded::MemoryEntityStorage;
use crate::embedded::storage::{CountingEntityStorage, StorageReadCounts};
use crate::{BatchStats, ConflictPolicy, Expr, Operand, OrderBy, TransactionMetrics};

const ITEMS: &str = "tx_items";

fn item_id(index: usize) -> String {
    format!("item-{index}")
}

fn row(id: &str, name: &str, n: i64) -> Object {
    Object::from_iter([
        ("id".to_string(), Value::String(id.to_string())),
        ("name".to_string(), Value::String(name.to_string())),
        ("n".to_string(), Value::I64(n)),
    ])
}

fn item(index: usize) -> Object {
    row(&item_id(index), &format!("name-{index}"), index as i64)
}

fn field(name: &str) -> Expr {
    Expr::Operand(Operand::Field(FieldPath::from_fields([name])))
}

fn compare(op: BinaryOp, name: &str, value: Value) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(field(name)),
        right: Box::new(Expr::Operand(Operand::Literal(value))),
    }
}

fn by_name(name: &str) -> SelectQuery {
    SelectQuery::new()
        .with_collection(ITEMS)
        .with_predicate(compare(
            BinaryOp::Eq,
            "name",
            Value::String(name.to_string()),
        ))
}

fn n_above(n: i64, direction: SortDirection, limit: usize) -> SelectQuery {
    SelectQuery::new()
        .with_collection(ITEMS)
        .with_predicate(compare(BinaryOp::Gt, "n", Value::I64(n)))
        .with_order_by(vec![OrderBy {
            expr: field("n"),
            direction,
        }])
        .with_limit(limit)
}

fn all() -> SelectQuery {
    SelectQuery::new().with_collection(ITEMS)
}

fn ids(rows: &[Object]) -> Vec<String> {
    rows.iter()
        .map(|row| row.get("id").and_then(Value::as_str).unwrap().to_string())
        .collect()
}

/// `ITEMS` with a unique equality index on `name`, a range index on `n` and
/// rows `0..rows`.
fn populate<S: EntityStorage>(db: &mut EmbeddedDb<S>, rows: usize) {
    let lid = db
        .create_collection(ITEMS, CollectionKind::Polymorphic)
        .unwrap();
    db.create_index("tx_items_name", lid, "name", true).unwrap();
    db.create_index_definition(IndexDefinition {
        name: "tx_items_n".into(),
        collection: lid,
        fields: vec!["n".into()],
        unique: false,
        kind: IndexKind::Range,
        predicate: None,
        analyzer: Default::default(),
    })
    .unwrap();
    db.transact(Batch {
        operations: (0..rows)
            .map(|index| BatchOperation::Upsert {
                collection: ITEMS.into(),
                id: item_id(index),
                object: item(index),
            })
            .collect(),
    })
    .unwrap();
}

fn counting_db(rows: usize) -> (EmbeddedDb<CountingEntityStorage>, Arc<StorageReadCounts>) {
    let (storage, counts) = CountingEntityStorage::new();
    let mut db = EmbeddedDb::new(storage);
    populate(&mut db, rows);
    counts.reset();
    (db, counts)
}

fn memory_db(rows: usize) -> EmbeddedDb<MemoryEntityStorage> {
    let mut db = EmbeddedDb::in_memory();
    populate(&mut db, rows);
    db
}

fn begin<S: EntityStorage>(db: &EmbeddedDb<S>) -> EmbeddedTransaction {
    db.begin(TransactionOptions::default()).unwrap()
}

#[test]
fn statements_read_their_own_writes_through_indexes() {
    let (mut db, counts) = counting_db(10);
    let mut tx = begin(&db);
    tx.upsert(ITEMS, "new", row("new", "fresh", 15)).unwrap();
    tx.upsert(ITEMS, "extra", row("extra", "extra", -1))
        .unwrap();
    tx.upsert(ITEMS, item_id(3), row(&item_id(3), "renamed", 100))
        .unwrap();
    assert!(tx.delete(ITEMS, &item_id(5)).unwrap());
    assert!(!tx.delete(ITEMS, "absent").unwrap());

    let new = tx.get(ITEMS, "new").unwrap().unwrap();
    assert_eq!(new.object.get("n"), Some(&Value::I64(15)));
    assert!(tx.get(ITEMS, &item_id(5)).unwrap().is_none());

    counts.reset();
    assert_eq!(ids(&tx.select(by_name("fresh")).unwrap()), ["new"]);
    assert!(tx.select(by_name("name-5")).unwrap().is_empty());
    assert!(tx.select(by_name("name-3")).unwrap().is_empty());
    assert_eq!(ids(&tx.select(by_name("renamed")).unwrap()), [item_id(3)]);
    assert!(matches!(
        db.explain_query(Query::Select(by_name("fresh")))
            .unwrap()
            .access_path,
        AccessPath::IndexLookup { .. }
    ));

    assert_eq!(
        ids(&tx.select(n_above(6, SortDirection::Asc, 5)).unwrap()),
        [item_id(7), item_id(8), item_id(9), "new".into(), item_id(3)]
    );
    assert_eq!(
        ids(&tx.select(n_above(6, SortDirection::Desc, 2)).unwrap()),
        [item_id(3), "new".to_string()]
    );
    assert!(matches!(
        db.explain_query(Query::Select(n_above(6, SortDirection::Asc, 5)))
            .unwrap()
            .access_path,
        AccessPath::IndexRange { ordered: true, .. }
    ));
    assert_eq!(
        counts.collection_scans.load(Ordering::Relaxed),
        0,
        "indexed statements must not scan the collection"
    );
    // 10 stored rows, two inserted and one deleted.
    assert_eq!(tx.select(all()).unwrap().len(), 11);

    // Other readers do not observe the uncommitted writes.
    let reader = db.owned_reader().unwrap().unwrap();
    assert!(reader.select(by_name("fresh")).unwrap().is_empty());
    assert!(reader.get(ITEMS, &item_id(5)).unwrap().is_some());
    assert!(db.get(ITEMS, "new").unwrap().is_none());

    let commit = tx.commit(&mut db).unwrap();
    assert_eq!(
        commit.stats,
        BatchStats {
            upserted: 3,
            updated: 0,
            deleted: 1,
        }
    );
    assert_eq!(commit.revision, db.storage.current_revision().unwrap());
    assert_eq!(ids(&db.select(by_name("fresh")).unwrap()), ["new"]);
    assert!(db.get(ITEMS, &item_id(5)).unwrap().is_none());
    assert_eq!(
        ids(&db.select(n_above(6, SortDirection::Desc, 2)).unwrap()),
        [item_id(3), "new".to_string()]
    );
    // The reader keeps its snapshot.
    assert!(reader.get(ITEMS, &item_id(5)).unwrap().is_some());
}

#[test]
fn predicate_mutations_see_earlier_statements() {
    let mut db = memory_db(10);
    let mut tx = begin(&db);
    tx.upsert(ITEMS, "new", row("new", "fresh", 15)).unwrap();
    let updated = tx
        .update_where(
            UpdateQuery::new()
                .with_collection(ITEMS)
                .with_predicate(compare(BinaryOp::Eq, "name", Value::String("fresh".into())))
                .set(
                    FieldPath::from_fields(["label"]),
                    Expr::Operand(Operand::Literal(Value::String("set".into()))),
                ),
        )
        .unwrap();
    assert_eq!(updated.stats.affected, 1);
    assert_eq!(
        tx.get(ITEMS, "new").unwrap().unwrap().object.get("label"),
        Some(&Value::String("set".into()))
    );
    let deleted = tx
        .delete_where(
            DeleteQuery::new()
                .with_collection(ITEMS)
                .with_predicate(compare(BinaryOp::Gt, "n", Value::I64(8))),
        )
        .unwrap();
    assert_eq!(deleted.deleted, 2);
    assert!(tx.get(ITEMS, "new").unwrap().is_none());
    assert_eq!(tx.stats().updated, 1);
    assert_eq!(tx.stats().deleted, 2);
    tx.commit(&mut db).unwrap();
    assert_eq!(db.select(all()).unwrap().len(), 9);
}

#[test]
fn savepoints_undo_later_writes() {
    let mut db = memory_db(3);
    let mut tx = begin(&db);
    tx.upsert(ITEMS, "a", row("a", "a", 10)).unwrap();
    let first = tx.savepoint();
    tx.upsert(ITEMS, "b", row("b", "b", 11)).unwrap();
    assert!(tx.delete(ITEMS, &item_id(0)).unwrap());
    tx.upsert(ITEMS, "a", row("a", "a-changed", 20)).unwrap();
    let second = tx.savepoint();
    tx.upsert(ITEMS, "c", row("c", "c", 12)).unwrap();

    tx.rollback_to(first).unwrap();
    assert_eq!(
        tx.get(ITEMS, "a").unwrap().unwrap().object.get("n"),
        Some(&Value::I64(10))
    );
    assert!(tx.get(ITEMS, "b").unwrap().is_none());
    assert!(tx.get(ITEMS, "c").unwrap().is_none());
    assert!(tx.get(ITEMS, &item_id(0)).unwrap().is_some());
    assert_eq!(tx.select(all()).unwrap().len(), 4);
    assert!(
        tx.rollback_to(second).is_err(),
        "later savepoints are released"
    );

    // The savepoint stays usable.
    tx.upsert(ITEMS, "d", row("d", "d", 13)).unwrap();
    tx.rollback_to(first).unwrap();
    assert!(tx.get(ITEMS, "d").unwrap().is_none());
    assert_eq!(tx.stats().upserted, 1);

    tx.commit(&mut db).unwrap();
    assert_eq!(
        ids(&db.select(all()).unwrap()),
        ["a", "item-0", "item-1", "item-2"]
    );
}

#[test]
fn failing_statements_leave_no_partial_writes() {
    let mut db = memory_db(1);
    let mut tx = begin(&db);
    let error = tx
        .execute_batch(
            Batch::new()
                .with_op(BatchOperation::Upsert {
                    collection: ITEMS.into(),
                    id: "ok".into(),
                    object: row("ok", "ok", 1),
                })
                .with_op(BatchOperation::Create {
                    collection: ITEMS.into(),
                    id: item_id(0),
                    object: item(0),
                }),
        )
        .unwrap_err();
    assert!(matches!(error, DbError::EntityExists { .. }), "{error}");
    assert!(tx.get(ITEMS, "ok").unwrap().is_none());
    assert_eq!(tx.stats().upserted, 0);
    tx.create(ITEMS, "created", row("created", "created", 2))
        .unwrap();
    tx.commit(&mut db).unwrap();
    assert!(db.get(ITEMS, "ok").unwrap().is_none());
    assert!(db.get(ITEMS, "created").unwrap().is_some());
}

#[test]
fn rolled_back_and_dropped_transactions_leave_no_trace() {
    let mut db = memory_db(2);
    let revision = db.storage.current_revision().unwrap();
    let mut tx = begin(&db);
    tx.upsert(ITEMS, "x", row("x", "x", 1)).unwrap();
    tx.delete(ITEMS, &item_id(0)).unwrap();
    tx.rollback();

    let mut tx = begin(&db);
    tx.upsert(ITEMS, "y", row("y", "y", 1)).unwrap();
    drop(tx);

    assert_eq!(db.storage.current_revision().unwrap(), revision);
    assert_eq!(ids(&db.select(all()).unwrap()), ["item-0", "item-1"]);

    // A transaction without writes commits nothing.
    let commit = begin(&db).commit(&mut db).unwrap();
    assert_eq!(commit.revision, revision);
    assert_eq!(db.storage.current_revision().unwrap(), revision);
}

#[test]
fn commits_conflict_with_writes_after_begin() {
    let mut db = memory_db(1);
    let mut tx = begin(&db);
    tx.upsert(ITEMS, "x", row("x", "x", 1)).unwrap();
    db.insert(ITEMS, "y", row("y", "y", 2)).unwrap();
    assert!(matches!(
        db.commit_transaction(tx),
        Err(DbError::TransactionConflict(_))
    ));
    assert!(db.get(ITEMS, "x").unwrap().is_none());

    let mut tx = begin(&db);
    tx.upsert(ITEMS, "x", row("x", "x", 1)).unwrap();
    db.create_collection("other", CollectionKind::Polymorphic)
        .unwrap();
    assert!(matches!(
        db.commit_transaction(tx),
        Err(DbError::TransactionConflict(_))
    ));
}

#[test]
fn run_transaction_retries_conflicts() {
    let (mut db, counts) = counting_db(1);
    counts.inject_conflicts.store(1, Ordering::Relaxed);
    let mut attempts = 0;
    let result = db
        .run_transaction(TransactionOptions::default(), |tx| {
            attempts += 1;
            // Every attempt reads its own snapshot.
            assert!(tx.get(ITEMS, "r").unwrap().is_none());
            tx.upsert(ITEMS, "r", row("r", "r", 1))?;
            Ok(attempts)
        })
        .unwrap();
    assert_eq!(result.value, 2);
    assert_eq!(
        result.metrics,
        TransactionMetrics {
            attempts: 2,
            conflicts: 1,
        }
    );
    assert!(db.get(ITEMS, "r").unwrap().is_some());

    counts.inject_conflicts.store(1, Ordering::Relaxed);
    let options = TransactionOptions {
        conflict_policy: ConflictPolicy::Fail,
        ..TransactionOptions::default()
    };
    let error = db
        .run_transaction(options, |tx| tx.upsert(ITEMS, "s", row("s", "s", 2)))
        .unwrap_err();
    assert!(matches!(error, DbError::TransactionConflict(_)));
    assert!(db.get(ITEMS, "s").unwrap().is_none());
}

#[test]
fn commit_validates_unique_indexes_and_references() {
    let mut db = memory_db(2);
    let mut tx = begin(&db);
    tx.upsert(ITEMS, "dup", row("dup", "name-1", 7)).unwrap();
    assert_eq!(tx.select(by_name("name-1")).unwrap().len(), 2);
    assert!(matches!(
        db.commit_transaction(tx),
        Err(DbError::UniqueViolation { .. })
    ));
    assert!(db.get(ITEMS, "dup").unwrap().is_none());

    let mut db = EmbeddedDb::in_memory();
    super::tests::register_ref_schema(&mut db, super::tests::ref_ty("person"));
    let mut tx = begin(&db);
    tx.upsert(
        DEFAULT_COLLECTION,
        "article",
        typed("article", "article", Some("missing")),
    )
    .unwrap();
    let error = db.commit_transaction(tx).unwrap_err();
    assert!(error.to_string().contains("ref field"), "{error}");

    let mut tx = begin(&db);
    tx.upsert(
        DEFAULT_COLLECTION,
        "person",
        typed("person", "person", None),
    )
    .unwrap();
    tx.upsert(
        DEFAULT_COLLECTION,
        "article",
        typed("article", "article", Some("person")),
    )
    .unwrap();
    tx.commit(&mut db).unwrap();
    let mut tx = begin(&db);
    tx.delete(DEFAULT_COLLECTION, "person").unwrap();
    let error = db.commit_transaction(tx).unwrap_err();
    assert!(error.to_string().contains("ref field"), "{error}");
    assert!(db.get(DEFAULT_COLLECTION, "person").unwrap().is_some());
}

fn typed(id: &str, ty: &str, author: Option<&str>) -> Object {
    let mut object = Object::new();
    object.insert("id", id.to_string());
    object.insert("type", format!("local:{ty}"));
    if let Some(author) = author {
        object.insert("author", author.to_string());
    }
    object
}

#[test]
fn deletes_cascade_within_the_transaction() {
    let mut db = EmbeddedDb::in_memory();
    let mut author = super::tests::ref_ty("person");
    let TypeKind::Ref(reference) = &mut author.kind else {
        unreachable!("ref type")
    };
    reference.on_delete = semantic_data::schema::OnDelete::Cascade;
    super::tests::register_ref_schema(&mut db, author);
    let mut tx = begin(&db);
    for (id, ty, author) in [
        ("p1", "person", None),
        ("p2", "person", None),
        ("a1", "article", Some("p1")),
        ("a2", "article", Some("p1")),
        ("a3", "article", Some("p2")),
    ] {
        tx.upsert(DEFAULT_COLLECTION, id, typed(id, ty, author))
            .unwrap();
    }
    tx.commit(&mut db).unwrap();

    let mut tx = begin(&db);
    assert!(tx.delete(DEFAULT_COLLECTION, "p1").unwrap());
    assert!(tx.get(DEFAULT_COLLECTION, "a1").unwrap().is_none());
    assert_eq!(
        ids(&tx
            .select(SelectQuery::new().with_collection(DEFAULT_COLLECTION))
            .unwrap()),
        ["a3", "p2"]
    );
    assert_eq!(tx.stats().deleted, 3);
    let commit = tx.commit(&mut db).unwrap();
    assert_eq!(commit.stats.deleted, 3);
    assert_eq!(
        ids(&db
            .select(SelectQuery::new().with_collection(DEFAULT_COLLECTION))
            .unwrap()),
        ["a3", "p2"]
    );
}

#[test]
fn read_only_transactions_reject_writes() {
    let db = memory_db(1);
    let mut tx = db
        .begin(TransactionOptions {
            read_only: true,
            ..TransactionOptions::default()
        })
        .unwrap();
    assert!(matches!(
        tx.upsert(ITEMS, "x", row("x", "x", 1)),
        Err(DbError::InvalidQuery(_))
    ));
    assert!(tx.delete(ITEMS, &item_id(0)).is_err());
    assert!(tx.get(ITEMS, &item_id(0)).unwrap().is_some());
}

#[test]
fn snapshot_isolation_reads_one_snapshot() {
    let mut db = memory_db(1);
    for isolation in [
        IsolationLevel::ReadCommitted,
        IsolationLevel::RepeatableRead,
        IsolationLevel::Snapshot,
        IsolationLevel::Serializable,
    ] {
        let mut tx = db
            .begin(TransactionOptions {
                isolation,
                ..TransactionOptions::default()
            })
            .unwrap();
        db.insert(ITEMS, "late", row("late", "late", 5)).unwrap();
        assert!(tx.get(ITEMS, "late").unwrap().is_none(), "{isolation:?}");
        assert!(tx.select(by_name("late")).unwrap().is_empty());
        tx.upsert(ITEMS, "mine", row("mine", "mine", 6)).unwrap();
        assert!(matches!(
            db.commit_transaction(tx),
            Err(DbError::TransactionConflict(_))
        ));
        db.delete(ITEMS, "late").unwrap();
    }
}
