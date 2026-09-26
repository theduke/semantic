//! Interactive transactions ([`Db::begin_transaction`]).
//!
//! Backends without interactive transactions (reported as an `Unsupported`
//! storage error) skip these checks.

use super::*;
use semantic_data::schema::IndexKind;
use semantic_db_core::{QueryPlan, StorageErrorKind, TransactionOptions};

const ITEMS: &str = "suite_tx_items";

fn item(id: &str, name: &str, n: i64) -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String(id.to_string()));
    object.insert("name", Value::String(name.to_string()));
    object.insert("n", Value::I64(n));
    object
}

fn by_name(name: &str) -> SelectQuery {
    SelectQuery::new()
        .with_collection(ITEMS)
        .with_predicate(eq_predicate(
            FieldPath::from_fields(["name"]),
            Value::String(name.to_string()),
        ))
}

fn n_above(n: i64, limit: usize) -> SelectQuery {
    SelectQuery::new()
        .with_collection(ITEMS)
        .with_predicate(Expr::Binary {
            op: BinaryOp::Gt,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["n"])))),
            right: Box::new(Expr::Operand(Operand::Literal(Value::I64(n)))),
        })
        .with_order_by(vec![OrderBy {
            expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["n"]))),
            direction: SortDirection::Asc,
        }])
        .with_limit(limit)
}

async fn setup(db: &Db) {
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertCollection {
                name: ITEMS.into(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::Permissive,
            })
            .with_op(DdlOperation::UpsertIndex {
                name: "suite_tx_name".into(),
                collection: ITEMS.into(),
                field: "name".into(),
                unique: true,
                kind: IndexKind::Equality,
                extra_fields: Vec::new(),
                predicate: None,
                analyzer: Default::default(),
            })
            .with_op(DdlOperation::UpsertIndex {
                name: "suite_tx_n".into(),
                collection: ITEMS.into(),
                field: "n".into(),
                unique: false,
                kind: IndexKind::Range,
                extra_fields: Vec::new(),
                predicate: None,
                analyzer: Default::default(),
            }),
    )
    .await
    .unwrap();
    db.delete_where(DeleteQuery::new().with_collection(ITEMS))
        .await
        .unwrap();
    let mut batch = Batch::new();
    for index in 0..10 {
        let id = format!("item-{index}");
        batch = batch.with_op(BatchOperation::Upsert {
            collection: ITEMS.into(),
            object: item(&id, &format!("name-{index}"), index),
            id,
        });
    }
    db.execute_batch(batch).await.unwrap();
}

async fn begin(db: &Db) -> Box<dyn semantic_db_core::TransactionHandle> {
    db.begin_transaction(TransactionOptions::default())
        .await
        .unwrap()
}

pub async fn test_transactions(db: &Db) {
    match db.begin_transaction(TransactionOptions::default()).await {
        Ok(tx) => tx.rollback().await.unwrap(),
        Err(error) if error.storage_kind() == Some(StorageErrorKind::Unsupported) => return,
        Err(error) => panic!("begin transaction: {error}"),
    }
    setup(db).await;
    test_read_your_writes(db).await;
    test_conflicts_and_rollbacks(db).await;
}

async fn test_read_your_writes(db: &Db) {
    let tx = begin(db).await;
    tx.upsert(ITEMS.into(), "new".into(), item("new", "fresh", 15))
        .await
        .unwrap();
    tx.upsert(
        ITEMS.into(),
        "item-3".into(),
        item("item-3", "renamed", 100),
    )
    .await
    .unwrap();
    assert!(tx.delete(ITEMS.into(), "item-5".into()).await.unwrap());

    assert!(tx.get(ITEMS.into(), "new".into()).await.unwrap().is_some());
    assert!(
        tx.get(ITEMS.into(), "item-5".into())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        row_ids(&tx.select(by_name("fresh").into()).await.unwrap()),
        ["new"]
    );
    assert!(
        tx.select(by_name("name-5").into())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        tx.select(by_name("name-3").into())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        db.plan(by_name("fresh")).await.unwrap(),
        QueryPlan::IndexLookup { .. }
    ));
    assert_eq!(
        row_ids(&tx.select(n_above(6, 5).into()).await.unwrap()),
        ["item-7", "item-8", "item-9", "new", "item-3"]
    );
    assert!(matches!(
        db.plan(n_above(6, 5)).await.unwrap(),
        QueryPlan::IndexRange { ordered: true, .. }
    ));

    // Other readers do not observe uncommitted writes.
    assert!(db.select(by_name("fresh")).await.unwrap().is_empty());
    assert!(db.get(ITEMS, "item-5").await.unwrap().is_some());

    let savepoint = tx.savepoint().await.unwrap();
    tx.upsert(ITEMS.into(), "temp".into(), item("temp", "temp", 50))
        .await
        .unwrap();
    tx.rollback_to(savepoint).await.unwrap();
    assert!(tx.get(ITEMS.into(), "temp".into()).await.unwrap().is_none());

    let commit = tx.commit().await.unwrap();
    assert_eq!(commit.stats.upserted, 2);
    assert_eq!(commit.stats.deleted, 1);
    assert_eq!(
        row_ids(&db.select(by_name("fresh")).await.unwrap()),
        ["new"]
    );
    assert!(db.get(ITEMS, "item-5").await.unwrap().is_none());
    assert!(db.get(ITEMS, "temp").await.unwrap().is_none());
    assert_eq!(
        row_ids(&db.select(n_above(6, 5)).await.unwrap()),
        ["item-7", "item-8", "item-9", "new", "item-3"]
    );
}

async fn test_conflicts_and_rollbacks(db: &Db) {
    let first = begin(db).await;
    let second = begin(db).await;
    first
        .upsert(ITEMS.into(), "first".into(), item("first", "first", 1))
        .await
        .unwrap();
    second
        .upsert(ITEMS.into(), "second".into(), item("second", "second", 2))
        .await
        .unwrap();
    first.commit().await.unwrap();
    assert!(matches!(
        second.commit().await,
        Err(DbError::TransactionConflict(_))
    ));
    assert!(db.get(ITEMS, "first").await.unwrap().is_some());
    assert!(db.get(ITEMS, "second").await.unwrap().is_none());

    let dropped = begin(db).await;
    dropped
        .upsert(
            ITEMS.into(),
            "dropped".into(),
            item("dropped", "dropped", 3),
        )
        .await
        .unwrap();
    drop(dropped);
    let rolled_back = begin(db).await;
    rolled_back
        .delete(ITEMS.into(), "item-0".into())
        .await
        .unwrap();
    rolled_back.rollback().await.unwrap();
    assert!(db.get(ITEMS, "dropped").await.unwrap().is_none());
    assert!(db.get(ITEMS, "item-0").await.unwrap().is_some());

    let duplicate = begin(db).await;
    duplicate
        .upsert(ITEMS.into(), "dup".into(), item("dup", "name-1", 4))
        .await
        .unwrap();
    assert!(matches!(
        duplicate.commit().await,
        Err(DbError::UniqueViolation { .. })
    ));
    assert!(db.get(ITEMS, "dup").await.unwrap().is_none());
}
