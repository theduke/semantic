//! Planning and execution of range, composite, partial, ordered and
//! index-only index scans on the embedded database.

use std::sync::Arc;

use semantic_data::query::BinaryOp;
use semantic_data::schema::IndexKind;

use super::*;
use crate::catalog::{CollectionKind, IndexDefinition};
use crate::embedded::storage::{CountingEntityStorage, StorageReadCounts};
use crate::{AccessPath, Expr, Operand, Query};

const ITEMS: &str = "range_items";

fn item_id(index: usize) -> String {
    format!("item-{index:04}")
}

/// Row `index`: `n` = index, `s` = "name-{index:04}", `owner` cycles
/// through `a`, `b`, `c`, and every fourth row is `open`.
fn item(index: usize) -> Object {
    Object::from_iter([
        ("id".to_string(), Value::String(item_id(index))),
        ("n".to_string(), Value::I64(index as i64)),
        ("s".to_string(), Value::String(format!("name-{index:04}"))),
        (
            "owner".to_string(),
            Value::String(["a", "b", "c"][index % 3].to_string()),
        ),
        (
            "status".to_string(),
            Value::String(
                if index.is_multiple_of(4) {
                    "open"
                } else {
                    "closed"
                }
                .to_string(),
            ),
        ),
        ("note".to_string(), Value::String(format!("note {index}"))),
    ])
}

fn eq_expr(field: &str, value: Value) -> semantic_data::query::Expr {
    semantic_data::query::Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(semantic_data::query::Expr::Operand(
            semantic_data::query::Operand::Field(FieldPath::from_fields([field])),
        )),
        right: Box::new(semantic_data::query::Expr::Operand(
            semantic_data::query::Operand::Literal(value),
        )),
    }
}

fn definition(
    lid: LocalCollectionId,
    name: &str,
    fields: &[&str],
    kind: IndexKind,
) -> IndexDefinition {
    IndexDefinition {
        name: name.to_string(),
        collection: lid,
        fields: fields.iter().map(ToString::to_string).collect(),
        unique: false,
        kind,
        predicate: None,
        analyzer: Default::default(),
    }
}

/// `ITEMS` with range indexes on `n` and `s`, a composite range index on
/// `(owner, n)`, a partial range index on `n` of `open` rows and an
/// equality index on `status`.
fn populate<S: EntityStorage>(db: &mut EmbeddedDb<S>, rows: usize) -> LocalCollectionId {
    let lid = db
        .create_collection(ITEMS, CollectionKind::Polymorphic)
        .unwrap();
    for definition in [
        definition(lid, "by_n", &["n"], IndexKind::Range),
        definition(lid, "by_s", &["s"], IndexKind::Range),
        definition(lid, "by_owner_n", &["owner", "n"], IndexKind::Range),
        IndexDefinition {
            predicate: Some(eq_expr("status", Value::String("open".into()))),
            ..definition(lid, "open_by_n", &["n"], IndexKind::Range)
        },
        definition(lid, "by_status", &["status"], IndexKind::Equality),
    ] {
        db.create_index_definition(definition).unwrap();
    }
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
    lid
}

fn counting_db(rows: usize) -> (EmbeddedDb<CountingEntityStorage>, Arc<StorageReadCounts>) {
    let (storage, counts) = CountingEntityStorage::new();
    let mut db = EmbeddedDb::new(storage);
    populate(&mut db, rows);
    counts.reset();
    (db, counts)
}

fn sql(query: &str) -> Query {
    crate::sql::parse_sql_query(&query.replace("$T", ITEMS), Default::default())
        .unwrap()
        .query
}

fn select(query: &str) -> SelectQuery {
    match sql(query) {
        Query::Select(select) => select,
        other => panic!("not a select: {other:?}"),
    }
}

fn access<S: EntityStorage>(db: &EmbeddedDb<S>, query: &str) -> AccessPath {
    db.explain_query(sql(query)).unwrap().access_path
}

fn index_name(path: &AccessPath) -> Option<&str> {
    match path {
        AccessPath::IndexRange { index_name, .. } => Some(index_name),
        _ => None,
    }
}

fn ids(rows: &[Object]) -> Vec<String> {
    rows.iter()
        .map(|row| row.get("id").and_then(Value::as_str).unwrap().to_string())
        .collect()
}

#[test]
fn predicate_shapes_choose_the_expected_index() {
    let (db, _) = counting_db(200);
    let cases = [
        ("SELECT * FROM $T WHERE n > 150", Some("by_n"), 1),
        ("SELECT * FROM $T WHERE n >= 10 AND n < 20", Some("by_n"), 1),
        ("SELECT * FROM $T WHERE n BETWEEN 3 AND 7", Some("by_n"), 1),
        ("SELECT * FROM $T WHERE n IN (1, 5, 9)", Some("by_n"), 3),
        ("SELECT * FROM $T WHERE s LIKE 'name-01%'", Some("by_s"), 1),
        ("SELECT * FROM $T WHERE s ~ '^name-01'", Some("by_s"), 1),
        (
            "SELECT * FROM $T WHERE owner = 'a' AND n > 150",
            Some("by_owner_n"),
            1,
        ),
        (
            "SELECT * FROM $T WHERE owner IN ('a', 'b') AND n < 10",
            Some("by_owner_n"),
            2,
        ),
        (
            "SELECT * FROM $T WHERE status = 'open' AND n > 100",
            Some("open_by_n"),
            1,
        ),
        // Not implied by the query: the partial index is not eligible.
        (
            "SELECT * FROM $T WHERE status = 'closed' AND n > 190",
            Some("by_n"),
            1,
        ),
        ("SELECT * FROM $T WHERE n > 190", Some("by_n"), 1),
        // Equality indexes serve `IN` probes but no ranges.
        (
            "SELECT * FROM $T WHERE status IN ('open', 'x')",
            Some("by_status"),
            2,
        ),
        ("SELECT * FROM $T WHERE note > 'note 5'", None, 0),
        ("SELECT * FROM $T WHERE s LIKE '%01'", None, 0),
    ];
    for (query, expected, ranges) in cases {
        let path = access(&db, query);
        assert_eq!(index_name(&path), expected, "{query}: {path:?}");
        if let AccessPath::IndexRange { ranges: actual, .. } = path {
            assert_eq!(actual, ranges, "{query}");
        }
    }
    // A single equality on an equality index is left to the plain lookup.
    assert_eq!(
        index_name(&access(&db, "SELECT * FROM $T WHERE status = 'open'")),
        None
    );
}

#[test]
fn partial_index_requires_every_predicate_conjunct() {
    let (db, _) = counting_db(200);
    for query in [
        "SELECT * FROM $T WHERE n > 100",
        "SELECT * FROM $T WHERE status = 'open' OR n > 100",
        "SELECT * FROM $T WHERE status != 'open' AND n > 100",
    ] {
        assert_ne!(
            index_name(&access(&db, query)),
            Some("open_by_n"),
            "{query}"
        );
    }
}

#[test]
fn range_prefix_and_probe_scans_read_no_collection() {
    let (db, counts) = counting_db(300);
    let cases = [
        (
            "SELECT * FROM $T WHERE n >= 10 AND n < 13",
            vec![10, 11, 12],
        ),
        ("SELECT * FROM $T WHERE n IN (7, 3, 250)", vec![3, 7, 250]),
        (
            "SELECT * FROM $T WHERE s LIKE 'name-012%'",
            (120..130).collect(),
        ),
        (
            "SELECT * FROM $T WHERE owner = 'b' AND n BETWEEN 3 AND 12",
            vec![4, 7, 10],
        ),
        (
            "SELECT * FROM $T WHERE status = 'open' AND n BETWEEN 280 AND 290",
            vec![280, 284, 288],
        ),
    ];
    for (query, expected) in cases {
        counts.reset();
        let mut rows = db.select(select(query)).unwrap();
        rows.sort_by(|a, b| a.get("id").cmp(&b.get("id")));
        assert_eq!(
            ids(&rows),
            expected.into_iter().map(item_id).collect::<Vec<_>>(),
            "{query}"
        );
        assert_eq!(counts.collection_scans(), 0, "{query}");
        assert_eq!(counts.entity_gets(), rows.len(), "{query}");
    }
}

#[test]
fn ordered_scans_replace_the_sort_and_stop_at_the_limit() {
    let (db, counts) = counting_db(500);
    let query = "SELECT * FROM $T ORDER BY n DESC LIMIT 3";
    let path = access(&db, query);
    assert!(
        matches!(
            path,
            AccessPath::IndexRange {
                ordered: true,
                descending: true,
                ..
            }
        ),
        "{path:?}"
    );
    let physical = db.explain_query(sql(query)).unwrap().physical;
    assert!(!format!("{physical:?}").contains("TopN"), "{physical:?}");

    counts.reset();
    let rows = db.select(select(query)).unwrap();
    assert_eq!(ids(&rows), [499, 498, 497].map(item_id));
    assert_eq!(counts.collection_scans(), 0);
    assert!(counts.entity_gets() <= 3, "reads: {}", counts.entity_gets());

    counts.reset();
    let rows = db
        .select(select("SELECT * FROM $T ORDER BY s LIMIT 2 OFFSET 5"))
        .unwrap();
    assert_eq!(ids(&rows), [5, 6].map(item_id));
    assert_eq!(counts.collection_scans(), 0);
    assert!(counts.entity_gets() <= 7, "reads: {}", counts.entity_gets());

    // Composite: equality on the leading column, ordered by the next one.
    counts.reset();
    let rows = db
        .select(select(
            "SELECT * FROM $T WHERE owner = 'c' ORDER BY n DESC LIMIT 2",
        ))
        .unwrap();
    assert_eq!(ids(&rows), [497, 494].map(item_id));
    assert_eq!(counts.collection_scans(), 0);
    assert!(counts.entity_gets() <= 2, "reads: {}", counts.entity_gets());
}

#[test]
fn ordered_scans_without_limit_need_a_range_on_the_index() {
    let (db, counts) = counting_db(100);
    // Without a limit the ordering is only taken from an index the
    // predicate already scans.
    let path = access(&db, "SELECT * FROM $T WHERE n > 90 ORDER BY n DESC");
    assert!(
        matches!(path, AccessPath::IndexRange { ordered: true, .. }),
        "{path:?}"
    );
    counts.reset();
    let rows = db
        .select(select("SELECT * FROM $T WHERE n > 95 ORDER BY n DESC"))
        .unwrap();
    assert_eq!(ids(&rows), [99, 98, 97, 96].map(item_id));
    assert_eq!(counts.collection_scans(), 0);

    assert_eq!(
        access(&db, "SELECT * FROM $T ORDER BY n"),
        AccessPath::FullScan
    );
}

#[test]
fn ordered_scans_require_every_row_to_have_an_entry() {
    let (mut db, _) = counting_db(50);
    let mut row = item(1_000);
    row.remove("n");
    db.transact(Batch::new().with_op(BatchOperation::Upsert {
        collection: ITEMS.into(),
        id: item_id(1_000),
        object: row,
    }))
    .unwrap();
    // A row without `n` sorts first but has no entry in `by_n`.
    let query = "SELECT * FROM $T ORDER BY n LIMIT 2";
    assert_eq!(access(&db, query), AccessPath::FullScan);
    let rows = db.select(select(query)).unwrap();
    assert_eq!(ids(&rows), [item_id(1_000), item_id(0)]);
    // A range on the column excludes rows without it.
    let path = access(&db, "SELECT * FROM $T WHERE n >= 0 ORDER BY n LIMIT 2");
    assert!(
        matches!(path, AccessPath::IndexRange { ordered: true, .. }),
        "{path:?}"
    );
}

#[test]
fn keyset_pagination_reads_one_page_per_query() {
    let (db, counts) = counting_db(1_000);
    let page = |after: Option<i64>| {
        let predicate = after.map_or(String::new(), |last| format!("WHERE n > {last}"));
        format!("SELECT id, n FROM $T {predicate} ORDER BY n LIMIT 25")
    };
    let mut last = None;
    let mut seen = Vec::new();
    for _ in 0..4 {
        counts.reset();
        let query = page(last);
        let path = access(&db, &query);
        assert!(
            matches!(path, AccessPath::IndexRange { ordered: true, .. }),
            "{query}: {path:?}"
        );
        let rows = db.select(select(&query)).unwrap();
        assert_eq!(rows.len(), 25);
        assert_eq!(counts.collection_scans(), 0);
        seen.extend(ids(&rows));
        last = rows.last().and_then(|row| match row.get("n") {
            Some(Value::I64(n)) => Some(*n),
            _ => None,
        });
    }
    assert_eq!(seen, (0..100).map(item_id).collect::<Vec<_>>());
}

#[test]
fn index_only_projections_read_no_rows() {
    let (db, counts) = counting_db(300);
    let query = "SELECT id, n FROM $T WHERE n >= 100 AND n < 104";
    let path = access(&db, query);
    assert!(
        matches!(
            path,
            AccessPath::IndexRange {
                index_only: true,
                ..
            }
        ),
        "{path:?}"
    );
    counts.reset();
    let rows = db.select(select(query)).unwrap();
    assert_eq!(
        rows,
        (100..104)
            .map(|index| Object::from_iter([
                ("id".to_string(), Value::String(item_id(index))),
                ("n".to_string(), Value::I64(index as i64)),
            ]))
            .collect::<Vec<_>>()
    );
    assert_eq!(counts.collection_scans(), 0);
    assert_eq!(counts.index_entry_scans(), 1);
    assert_eq!(counts.entity_gets(), 0);

    // Composite keys cover both columns, also for ordered, limited scans.
    counts.reset();
    let rows = db
        .select(select(
            "SELECT owner, n FROM $T WHERE owner = 'a' ORDER BY n DESC LIMIT 2",
        ))
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get("n"), Some(&Value::I64(297)));
    assert_eq!(counts.entity_gets(), 0);
    assert_eq!(counts.collection_scans(), 0);

    // Other fields need the stored rows.
    let path = access(&db, "SELECT id, n, note FROM $T WHERE n >= 100 AND n < 104");
    assert!(
        matches!(
            path,
            AccessPath::IndexRange {
                index_only: false,
                ..
            }
        ),
        "{path:?}"
    );
}

#[test]
fn predicate_mutations_locate_rows_through_ranges() {
    let (mut db, counts) = counting_db(500);
    let deleted = db
        .delete_where(
            DeleteQuery::new()
                .with_collection(ITEMS)
                .with_predicate(Expr::Binary {
                    op: BinaryOp::Gte,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["n"])))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::I64(495)))),
                }),
        )
        .unwrap();
    assert_eq!(deleted, 5);
    assert_eq!(counts.collection_scans(), 0);
    assert_eq!(
        db.collection_rows(db.catalog().collection_by_name(ITEMS).unwrap().lid)
            .unwrap()
            .len(),
        495
    );
    let rows = db.select(select("SELECT * FROM $T WHERE n > 490")).unwrap();
    assert_eq!(rows.len(), 4);
}

#[test]
fn unique_composite_and_partial_indexes_are_enforced() {
    let mut db = EmbeddedDb::in_memory();
    let lid = db
        .create_collection("unique_items", CollectionKind::Polymorphic)
        .unwrap();
    db.create_index_definition(IndexDefinition {
        unique: true,
        ..definition(lid, "owner_slot", &["owner", "slot"], IndexKind::Equality)
    })
    .unwrap();
    db.create_index_definition(IndexDefinition {
        unique: true,
        predicate: Some(eq_expr("status", Value::String("active".into()))),
        ..definition(lid, "active_name", &["name"], IndexKind::Range)
    })
    .unwrap();
    let row = |id: &str, owner: &str, slot: i64, name: &str, status: &str| {
        Object::from_iter([
            ("id".to_string(), Value::String(id.into())),
            ("owner".to_string(), Value::String(owner.into())),
            ("slot".to_string(), Value::I64(slot)),
            ("name".to_string(), Value::String(name.into())),
            ("status".to_string(), Value::String(status.into())),
        ])
    };
    let upsert = |id: &str, object: Object| BatchOperation::Upsert {
        collection: "unique_items".into(),
        id: id.into(),
        object,
    };
    db.transact(
        Batch::new()
            .with_op(upsert("a", row("a", "x", 1, "one", "active")))
            .with_op(upsert("b", row("b", "x", 2, "one", "archived")))
            .with_op(upsert("c", row("c", "y", 1, "two", "active"))),
    )
    .unwrap();
    // Same tuple, point write path.
    let err = db
        .transact(Batch::new().with_op(upsert("d", row("d", "x", 1, "three", "active"))))
        .unwrap_err();
    assert!(matches!(err, DbError::UniqueViolation { .. }), "{err:?}");
    // Same name among active rows.
    let err = db
        .transact(Batch::new().with_op(upsert("d", row("d", "z", 1, "one", "active"))))
        .unwrap_err();
    assert!(matches!(err, DbError::UniqueViolation { .. }), "{err:?}");
    // Same name outside the partial index is allowed.
    db.transact(Batch::new().with_op(upsert("d", row("d", "z", 1, "one", "archived"))))
        .unwrap();
    // Dataset path (predicate update): moving `c` onto `a`'s tuple fails.
    let err = db
        .update_where_returning(
            UpdateQuery::new()
                .with_collection("unique_items")
                .with_predicate(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "owner",
                    ])))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String("y".into())))),
                })
                .set(
                    FieldPath::from_fields(["owner"]),
                    Expr::Operand(Operand::Literal(Value::String("x".into()))),
                ),
        )
        .unwrap_err();
    assert!(matches!(err, DbError::UniqueViolation { .. }), "{err:?}");
    // Activating `d` duplicates the active name `one`.
    let err = db
        .update_where_returning(
            UpdateQuery::new()
                .with_collection("unique_items")
                .with_predicate(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "owner",
                    ])))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String("z".into())))),
                })
                .set(
                    FieldPath::from_fields(["status"]),
                    Expr::Operand(Operand::Literal(Value::String("active".into()))),
                ),
        )
        .unwrap_err();
    assert!(matches!(err, DbError::UniqueViolation { .. }), "{err:?}");
}

#[test]
fn creating_an_index_over_duplicates_fails() {
    let mut db = EmbeddedDb::in_memory();
    let lid = db
        .create_collection("dupes", CollectionKind::Polymorphic)
        .unwrap();
    let object = |id: &str| {
        Object::from_iter([
            ("id".to_string(), Value::String(id.into())),
            ("a".to_string(), Value::I64(1)),
            ("b".to_string(), Value::I64(2)),
        ])
    };
    db.transact(
        Batch::new()
            .with_op(BatchOperation::Upsert {
                collection: "dupes".into(),
                id: "x".into(),
                object: object("x"),
            })
            .with_op(BatchOperation::Upsert {
                collection: "dupes".into(),
                id: "y".into(),
                object: object("y"),
            }),
    )
    .unwrap();
    db.create_index_definition(definition(lid, "ab", &["a", "b"], IndexKind::Range))
        .unwrap();
    let rows = db
        .select(select("SELECT id FROM dupes WHERE a = 1 AND b >= 2"))
        .unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn composite_and_partial_definitions_survive_reopen() {
    let mut db = EmbeddedDb::new(crate::embedded::MemoryEntityStorage::new());
    populate(&mut db, 20);
    let reopened = EmbeddedDb::open(db.storage().clone()).unwrap();
    let catalog = reopened.catalog();
    let collection = catalog.collection_by_name(ITEMS).unwrap();
    let indexes = catalog
        .indexes_for_collection(collection.lid)
        .map(|index| (index.schema.name.clone(), index.clone()))
        .collect::<BTreeMap<_, _>>();
    let composite = &indexes["by_owner_n"];
    assert_eq!(composite.columns().collect::<Vec<_>>(), ["owner", "n"]);
    assert_eq!(composite.schema.kind, IndexKind::Range);
    let partial = &indexes["open_by_n"];
    assert_eq!(
        partial.schema.predicate,
        Some(eq_expr("status", Value::String("open".into())))
    );
    let rows = reopened
        .select(select(
            "SELECT id FROM $T WHERE status = 'open' AND n >= 16",
        ))
        .unwrap();
    assert_eq!(ids(&rows), [item_id(16)]);
}

#[test]
fn opening_a_database_from_before_maintained_range_indexes_rebuilds_them() {
    let mut db = EmbeddedDb::new(crate::embedded::MemoryEntityStorage::new());
    populate(&mut db, 40);
    let by_n = db
        .catalog()
        .indexes()
        .find(|(_, index)| index.schema.name == "by_n")
        .map(|(lid, _)| lid)
        .unwrap();

    // Earlier versions registered range indexes without entries and
    // without the index definitions core migration.
    let mut storage = db.storage().clone();
    let mut snapshot = db.catalog().to_storage_snapshot();
    snapshot
        .applied_migrations
        .retain(|item| item.applied.migration.name != crate::ddl::INDEX_DEFINITIONS_MIGRATION);
    let legacy = Catalog::from_storage_snapshot(snapshot).unwrap();
    let ops = catalog_write_ops(&storage, &legacy).unwrap();
    storage.apply_batch(&ops).unwrap();
    storage.drop_index_entries(by_n);
    assert!(!storage.index_needs_rebuild(by_n).unwrap());

    let reopened = EmbeddedDb::open(storage).unwrap();
    assert!(
        reopened
            .catalog()
            .applied_migration("semantic", "core", crate::ddl::INDEX_DEFINITIONS_MIGRATION)
            .is_some()
    );
    let query = "SELECT id FROM $T WHERE n >= 38";
    assert_eq!(index_name(&access(&reopened, query)), Some("by_n"));
    let rows = reopened.select(select(query)).unwrap();
    assert_eq!(ids(&rows), [item_id(38), item_id(39)]);
}
