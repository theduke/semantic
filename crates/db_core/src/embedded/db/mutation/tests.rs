use std::sync::Arc;

use semantic_data::query::BinaryOp;

use super::*;
use crate::embedded::MemoryEntityStorage;
use crate::embedded::db::compact::CompactReply;
use crate::embedded::storage::{CountingEntityStorage, StorageReadCounts};
use crate::{BatchReply, BatchReturn, Expr, Operand, QueryField};

const ITEMS: &str = "bulk_items";

fn field(name: &str) -> Box<Expr> {
    Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
        name,
    ]))))
}

fn literal(value: &str) -> Box<Expr> {
    Box::new(Expr::Operand(Operand::Literal(Value::String(value.into()))))
}

fn compare(op: BinaryOp, name: &str, value: &str) -> Expr {
    Expr::Binary {
        op,
        left: field(name),
        right: literal(value),
    }
}

fn eq(name: &str, value: &str) -> Expr {
    compare(BinaryOp::Eq, name, value)
}

fn projection(names: &[&str]) -> Vec<QueryField> {
    names
        .iter()
        .map(|name| QueryField {
            expr: field(name),
            alias: Some((*name).to_string()),
            wildcard: None,
        })
        .collect()
}

fn item_id(index: usize) -> String {
    format!("item-{index:04}")
}

/// Every 250th row is `rare`, every 333rd is flagged.
fn item(index: usize) -> Object {
    let kind = if index.is_multiple_of(250) {
        "rare"
    } else {
        "common"
    };
    let flag = if index.is_multiple_of(333) {
        "yes"
    } else {
        "no"
    };
    Object::from_iter(
        [
            ("id", item_id(index)),
            ("kind", kind.to_string()),
            ("flag", flag.to_string()),
            ("name", format!("name-{index}")),
        ]
        .map(|(key, value)| (key.to_string(), Value::String(value))),
    )
}

/// Rows of `ITEMS` with a `kind` index and a unique `name` index.
fn populate<S: EntityStorage>(db: &mut EmbeddedDb<S>, rows: usize) {
    let lid = db
        .create_collection(ITEMS, CollectionKind::Polymorphic)
        .unwrap();
    db.create_index("bulk_items_by_kind", lid, "kind", false)
        .unwrap();
    db.create_index("bulk_items_by_name", lid, "name", true)
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

fn items_lid<S: EntityStorage>(db: &EmbeddedDb<S>) -> LocalCollectionId {
    db.catalog().collection_by_name(ITEMS).unwrap().lid
}

fn rows<S: EntityStorage>(db: &EmbeddedDb<S>) -> Vec<EntityRecord> {
    db.collection_rows(items_lid(db)).unwrap()
}

fn set_label(predicate: Expr) -> UpdateQuery {
    UpdateQuery::new()
        .with_collection(ITEMS)
        .with_predicate(predicate)
        .set(FieldPath::from_fields(["label"]), *literal("updated"))
}

#[test]
fn update_by_primary_key_reads_constant_rows_without_scanning() {
    let mut gets = Vec::new();
    for size in [10, 1_000] {
        let (mut db, counts) = counting_db(size);
        let result = db
            .update_where_returning(set_label(eq("id", "item-0005")))
            .unwrap();
        assert_eq!(result.stats.matched, 1);
        assert_eq!(result.stats.affected, 1);
        assert_eq!(db.execution_counts.fallback_scans, 0);
        assert_eq!(counts.collection_scans(), 0);
        assert_eq!(counts.written_ids(items_lid(&db)), vec!["item-0005"]);
        gets.push(counts.entity_gets());
    }
    assert_eq!(gets[0], gets[1], "point reads must not depend on size");
    assert!(gets[0] <= 8, "point reads: {}", gets[0]);
}

#[test]
fn delete_through_equality_index_reads_only_matches() {
    let (mut db, counts) = counting_db(1_000);
    let deleted = db
        .delete_where(
            DeleteQuery::new()
                .with_collection(ITEMS)
                .with_predicate(eq("kind", "rare")),
        )
        .unwrap();
    assert_eq!(deleted, 4);
    assert_eq!(db.execution_counts.fallback_scans, 0);
    assert_eq!(counts.collection_scans(), 0);
    assert!(
        counts.entity_gets() <= 12,
        "reads: {}",
        counts.entity_gets()
    );
    let mut written = counts.written_ids(items_lid(&db));
    written.sort();
    assert_eq!(written, [0, 250, 500, 750].map(item_id));
    assert_eq!(rows(&db).len(), 996);
}

#[test]
fn predicate_without_index_scans_once_and_writes_only_matches() {
    let (mut db, counts) = counting_db(1_000);
    let result = db
        .update_where_returning(set_label(compare(BinaryOp::NotEq, "flag", "no")))
        .unwrap();
    assert_eq!(result.stats.matched, 4);
    assert_eq!(db.execution_counts.fallback_scans, 0);
    assert_eq!(db.execution_counts.collection_scans, 1);
    assert_eq!(counts.collection_scans(), 1);
    assert_eq!(counts.rows_yielded(), 1_000);
    let mut written = counts.written_ids(items_lid(&db));
    written.sort();
    assert_eq!(written, [0, 333, 666, 999].map(item_id));
}

#[test]
fn batch_predicate_operations_see_earlier_operations() {
    let mut db = EmbeddedDb::in_memory();
    populate(&mut db, 20);
    let mut added = item(100);
    added.insert("kind", Value::String("rare".into()));
    let reply = db
        .execute_batch_returning(
            Batch::new()
                .with_op(BatchOperation::Upsert {
                    collection: ITEMS.into(),
                    id: item_id(100),
                    object: added,
                })
                .with_op(BatchOperation::Update {
                    collection: ITEMS.into(),
                    query: set_label(eq("kind", "rare")),
                })
                .with_op(BatchOperation::Delete {
                    collection: ITEMS.into(),
                    query: DeleteQuery::new().with_predicate(eq("label", "updated")),
                }),
            BatchReturn::Stats,
        )
        .unwrap();
    assert_eq!(db.execution_counts.fallback_scans, 0);
    let BatchReply::Stats { stats } = reply else {
        panic!("stats reply expected");
    };
    // item-0000 is the only stored rare row; the upserted row is rare too.
    assert_eq!((stats.upserted, stats.updated, stats.deleted), (1, 2, 2));
    assert!(
        rows(&db)
            .iter()
            .all(|row| row.id != item_id(0) && row.id != item_id(100))
    );
}

/// A database whose point path is disabled by an incomplete reverse
/// reference backfill, so mutations take the dataset path.
fn dataset_db(rows: usize) -> EmbeddedDb<MemoryEntityStorage> {
    let mut db = EmbeddedDb::in_memory();
    populate(&mut db, rows);
    let references = db
        .catalog()
        .collection_by_name("__semantic.reverse_references")
        .unwrap()
        .lid;
    db.storage
        .apply_batch(&[StorageWriteOp::ClearCollection(references)])
        .unwrap();
    db
}

#[test]
fn predicate_mutations_match_the_dataset_path() {
    let mut fast = EmbeddedDb::in_memory();
    populate(&mut fast, 1_000);
    let mut slow = dataset_db(1_000);
    let updates = [
        set_label(eq("kind", "common"))
            .with_limit(Expr::from(3usize))
            .with_returning(projection(&["id", "label"])),
        set_label(compare(BinaryOp::NotEq, "flag", "no"))
            .with_limit(Expr::from(2usize))
            .with_returning(projection(&["id", "kind"])),
        set_label(Expr::InList {
            expr: field("id"),
            list: vec![
                *literal("item-0003"),
                *literal("missing"),
                *literal("item-0001"),
            ],
            negated: false,
        })
        .with_returning(projection(&["id"])),
        set_label(eq("kind", "rare")).with_limit(Expr::from(0usize)),
        UpdateQuery::new()
            .with_collection(ITEMS)
            .with_predicate(eq("kind", "rare"))
            .set(FieldPath::from_fields(["name"]), *literal("taken")),
        UpdateQuery::new()
            .with_collection(ITEMS)
            .with_predicate(eq("id", "item-0002"))
            .set(FieldPath::from_fields(["name"]), *literal("name-7")),
    ];
    for query in updates {
        let expected = slow.update_where_returning(query.clone());
        assert_eq!(slow.execution_counts.fallback_scans, 1);
        let actual = fast.update_where_returning(query);
        assert_eq!(fast.execution_counts.fallback_scans, 0);
        match (expected, actual) {
            (Ok(expected), Ok(actual)) => assert_eq!(expected, actual),
            (Err(expected), Err(actual)) => {
                assert!(matches!(actual, DbError::UniqueViolation { .. }));
                assert_eq!(expected.to_string(), actual.to_string());
            }
            (expected, actual) => panic!("mismatch: {expected:?} vs {actual:?}"),
        }
        assert_eq!(rows(&slow), rows(&fast));
    }
    let deletes = [
        DeleteQuery::new()
            .with_collection(ITEMS)
            .with_predicate(eq("kind", "rare"))
            .with_limit(Expr::from(2usize))
            .with_returning(projection(&["id", "label"])),
        DeleteQuery::new()
            .with_collection(ITEMS)
            .with_predicate(compare(BinaryOp::NotEq, "flag", "no"))
            .with_returning(projection(&["id", "name"])),
        DeleteQuery::new()
            .with_collection(ITEMS)
            .with_predicate(eq("id", "item-0010")),
    ];
    for query in deletes {
        let expected = slow.delete_where_returning(query.clone()).unwrap();
        let actual = fast.delete_where_returning(query).unwrap();
        assert_eq!(fast.execution_counts.fallback_scans, 0);
        assert_eq!(expected, actual);
        assert_eq!(rows(&slow), rows(&fast));
    }
}

#[test]
fn unique_violation_through_predicate_update_is_structured() {
    let mut db = EmbeddedDb::in_memory();
    populate(&mut db, 1_000);
    let revision = db.storage.current_revision().unwrap();
    let error = db
        .update_where_returning(
            UpdateQuery::new()
                .with_collection(ITEMS)
                .with_predicate(eq("id", "item-0002"))
                .set(FieldPath::from_fields(["name"]), *literal("name-7")),
        )
        .unwrap_err();
    let DbError::UniqueViolation {
        collection,
        index,
        field,
        value,
        existing_id,
        id,
    } = error
    else {
        panic!("unique violation expected: {error:?}");
    };
    assert_eq!(
        (collection.as_str(), index.as_str(), field.as_str()),
        (ITEMS, "bulk_items_by_name", "semantic:name")
    );
    assert_eq!(*value, Value::String("name-7".into()));
    assert_eq!((existing_id, id), (item_id(2), item_id(7)));
    assert_eq!(db.storage.current_revision().unwrap(), revision);
}

fn cascade_db() -> EmbeddedDb<MemoryEntityStorage> {
    let mut db = EmbeddedDb::in_memory();
    let mut author = crate::embedded::db::tests::ref_ty("person");
    let TypeKind::Ref(reference) = &mut author.kind else {
        unreachable!("ref type")
    };
    reference.on_delete = semantic_data::schema::OnDelete::Cascade;
    crate::embedded::db::tests::register_ref_schema(&mut db, author);
    let entity = |id: &str, ty: &str, author: Option<&str>| {
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
    db.transact(
        Batch::new()
            .with_op(entity("p1", "person", None))
            .with_op(entity("p2", "person", None))
            .with_op(entity("a1", "article", Some("p1")))
            .with_op(entity("a2", "article", Some("p1")))
            .with_op(entity("a3", "article", Some("p2"))),
    )
    .unwrap();
    db
}

fn remaining_ids<S: EntityStorage>(db: &EmbeddedDb<S>) -> Vec<String> {
    let lid = db
        .catalog()
        .collection_by_name(DEFAULT_COLLECTION)
        .unwrap()
        .lid;
    db.collection_rows(lid)
        .unwrap()
        .into_iter()
        .map(|row| row.id)
        .collect()
}

#[test]
fn predicate_delete_cascades_on_both_paths() {
    let delete = || BatchOperation::Delete {
        collection: DEFAULT_COLLECTION.into(),
        query: DeleteQuery::new().with_predicate(eq("id", "p1")),
    };

    let mut point = cascade_db();
    let reply = point
        .execute_batch_returning(Batch::new().with_op(delete()), BatchReturn::Stats)
        .unwrap();
    assert_eq!(point.execution_counts.fallback_scans, 0);
    assert!(matches!(reply, BatchReply::Stats { stats } if stats.deleted == 3));
    assert_eq!(remaining_ids(&point), ["a3", "p2"]);

    // The dataset path expands cascades once, before building its reply.
    let mut dataset = cascade_db();
    let outcome = dataset.transact(Batch::new().with_op(delete())).unwrap();
    assert_eq!(outcome.stats.deleted, 3);
    assert_eq!(
        outcome.dataset[DEFAULT_COLLECTION]
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["a3", "p2"]
    );
    assert_eq!(remaining_ids(&dataset), ["a3", "p2"]);

    // DELETE ... WHERE reports the directly deleted rows on both paths.
    let mut point = cascade_db();
    let direct = DeleteQuery::new()
        .with_collection(DEFAULT_COLLECTION)
        .with_predicate(eq("id", "p1"));
    assert_eq!(point.delete_where(direct.clone()).unwrap(), 1);
    assert_eq!(remaining_ids(&point), ["a3", "p2"]);
}

#[test]
fn stale_snapshot_conflicts_on_predicate_reads() {
    let mut db = EmbeddedDb::in_memory();
    populate(&mut db, 10);
    let stale = db.storage.current_revision().unwrap();
    // A concurrent writer commits between the snapshot and the commit.
    db.execute_batch(Batch::new().with_op(BatchOperation::Upsert {
        collection: ITEMS.into(),
        id: item_id(50),
        object: item(50),
    }))
    .unwrap();
    let catalog = db.catalog();
    let version = db.catalog.snapshot().version;
    for predicate in [
        eq("id", "item-0001"),
        eq("kind", "rare"),
        compare(BinaryOp::NotEq, "flag", "no"),
    ] {
        let query = set_label(predicate);
        let result = db.run_compact(
            crate::embedded::db::TxScope::new(
                &catalog,
                stale,
                crate::IsolationLevel::ReadCommitted,
            ),
            version,
            crate::WriteSettings::default(),
            false,
            |db, view, _| {
                tx_update(
                    view,
                    &db.query_context(),
                    ITEMS,
                    &query,
                    &DefaultExpressionContext::now(),
                    false,
                )
            },
            |_, _, _, result| Ok(CompactReply::Ready(result)),
        );
        assert!(
            matches!(result, Err(DbError::TransactionConflict(_))),
            "{result:?}"
        );
    }
    assert!(
        rows(&db)
            .iter()
            .all(|row| !row.object.contains_key("label"))
    );
}

#[test]
fn mutation_access_uses_primary_keys_indexes_and_scans() {
    let mut db = EmbeddedDb::in_memory();
    populate(&mut db, 1_000);
    let catalog = db.catalog();
    let collection = catalog.collection_by_name(ITEMS).unwrap();
    let snapshot = db.storage.snapshot().unwrap();
    let access = |predicate: Expr| {
        mutation_access(
            &catalog,
            &db.query_context(),
            &*snapshot,
            collection,
            Some(&predicate),
        )
        .unwrap()
    };
    let both = |left, right| Expr::Binary {
        op: BinaryOp::And,
        left: Box::new(left),
        right: Box::new(right),
    };
    assert_eq!(
        access(both(eq("kind", "rare"), eq("id", "item-0001"))),
        MutationAccess::Ids(BTreeSet::from([item_id(1)]))
    );
    assert!(matches!(
        access(both(compare(BinaryOp::NotEq, "flag", "no"), eq("kind", "rare"))),
        MutationAccess::Index { path: None, value: Value::String(value), .. } if value == "rare"
    ));
    assert_eq!(
        access(compare(BinaryOp::NotEq, "kind", "rare")),
        MutationAccess::Scan
    );
}
