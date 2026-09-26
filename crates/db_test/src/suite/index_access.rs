//! Range, composite, partial, ordered and index-only index access.
//!
//! Differential checks: every query runs against a collection with indexes
//! and against an identical collection without any, over seeded random data
//! with nulls, missing fields, mixed value types and duplicates. Results
//! must be equal (for ordered queries: equal sort keys, since ties may come
//! out in a different order).

use super::*;
use semantic_data::schema::IndexKind;
use semantic_db_core::QueryPlan;

const INDEXED: &str = "suite_index_access_indexed";
const PLAIN: &str = "suite_index_access_plain";
const ROWS: usize = 240;

/// Deterministic xorshift generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

fn random_row(rng: &mut Rng, index: usize) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(format!("r{index:04}")));
    let n = match rng.below(20) {
        0..=10 => Some(Value::I64(rng.below(40) as i64 - 10)),
        11 | 12 => Some(Value::U64(rng.below(30))),
        13 => Some(Value::String(
            ["a", "x", "zz"][rng.below(3) as usize].to_string(),
        )),
        14 => Some(Value::Null),
        15 => Some(Value::Bool(rng.below(2) == 0)),
        16 => Some(Value::F64((rng.below(20) as f64 / 2.0).into())),
        _ => None,
    };
    if let Some(n) = n {
        row.insert("n", n);
    }
    let s = match rng.below(20) {
        0..=14 => Some(Value::String(format!(
            "{}{}",
            [
                "", "a", "ab", "abc", "abd", "b", "ba", "bb", "c", "ab%", "a_b"
            ][rng.below(11) as usize],
            if rng.below(2) == 0 {
                String::new()
            } else {
                rng.below(10).to_string()
            }
        ))),
        15 => Some(Value::Null),
        16 => Some(Value::I64(rng.below(5) as i64)),
        _ => None,
    };
    if let Some(s) = s {
        row.insert("s", s);
    }
    match rng.below(10) {
        0 => {}
        1 => {
            row.insert("owner", Value::Null);
        }
        _ => {
            row.insert(
                "owner",
                Value::String(["a", "b", "c", "d"][rng.below(4) as usize].to_string()),
            );
        }
    }
    if rng.below(5) != 0 {
        row.insert(
            "status",
            Value::String(["open", "closed"][rng.below(2) as usize].to_string()),
        );
    }
    row.insert("note", Value::String(format!("row {index}")));
    row
}

fn index_op(name: &str, fields: &[&str], kind: IndexKind, predicate: Option<Expr>) -> DdlOperation {
    DdlOperation::UpsertIndex {
        name: name.to_string(),
        collection: INDEXED.to_string(),
        field: fields[0].to_string(),
        unique: false,
        kind,
        extra_fields: fields[1..].iter().map(ToString::to_string).collect(),
        predicate,
    }
}

fn status_is_open() -> Expr {
    Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
            "status",
        ])))),
        right: Box::new(Expr::Operand(Operand::Literal(Value::String(
            "open".to_string(),
        )))),
    }
}

async fn setup(db: &Db) {
    let mut ddl = DdlBatch::new();
    for collection in [INDEXED, PLAIN] {
        ddl = ddl.with_op(DdlOperation::UpsertCollection {
            name: collection.into(),
            kind: DdlCollectionKind::Polymorphic,
            integrity_mode: IntegrityMode::Permissive,
        });
    }
    for op in [
        index_op("by_n", &["n"], IndexKind::Range, None),
        index_op("by_s", &["s"], IndexKind::Range, None),
        index_op("by_owner_n", &["owner", "n"], IndexKind::Range, None),
        index_op("open_n", &["n"], IndexKind::Range, Some(status_is_open())),
        index_op(
            "by_owner_status",
            &["owner", "status"],
            IndexKind::Equality,
            None,
        ),
        index_op("by_owner", &["owner"], IndexKind::Equality, None),
    ] {
        ddl = ddl.with_op(op);
    }
    db.execute_ddl(ddl).await.unwrap();

    let mut rng = Rng(0x5eed_1dea_c0ff_ee01);
    let rows = (0..ROWS)
        .map(|index| random_row(&mut rng, index))
        .collect::<Vec<_>>();
    // Index half of the rows before the data exists (maintained on write)
    // and half after (backfilled) by writing in two batches.
    for chunk in rows.chunks(ROWS / 2) {
        let mut batch = Batch::new();
        for row in chunk {
            let id = row.get("id").and_then(Value::as_str).unwrap().to_string();
            for collection in [INDEXED, PLAIN] {
                batch = batch.with_op(BatchOperation::Upsert {
                    collection: collection.into(),
                    id: id.clone(),
                    object: row.clone(),
                });
            }
        }
        db.execute_batch(batch).await.unwrap();
    }
}

async fn run(db: &Db, query: &str, collection: &str) -> Vec<Object> {
    let sql = query.replace("$T", collection);
    match db.query_text(TextQueryFormat::Sql, sql.clone()).await {
        Ok(QueryResult::Select(rows)) => rows,
        other => panic!("{sql}: {other:?}"),
    }
}

async fn plan(db: &Db, query: &str) -> QueryPlan {
    db.plan(QueryInput::Text {
        format: TextQueryFormat::Sql,
        query: query.replace("$T", INDEXED),
        params: Default::default(),
    })
    .await
    .unwrap()
}

fn sorted_by_id(mut rows: Vec<Object>) -> Vec<Object> {
    rows.sort_by(|a, b| a.get("id").cmp(&b.get("id")));
    rows
}

fn sort_keys(rows: &[Object], fields: &[&str]) -> Vec<Vec<Option<Value>>> {
    rows.iter()
        .map(|row| fields.iter().map(|field| row.get(field).cloned()).collect())
        .collect()
}

pub async fn test_index_access(db: &Db) {
    setup(db).await;
    test_differential_unordered(db).await;
    test_differential_ordered(db).await;
    test_index_access_plans(db).await;
    test_writes_keep_indexes_consistent(db).await;
    test_package_composite_index(db).await;
}

async fn test_differential_unordered(db: &Db) {
    let predicates = [
        "n > 5",
        "n >= 5",
        "n < 3",
        "n <= 3",
        "n = 7",
        "n BETWEEN 2 AND 8",
        "n BETWEEN 8 AND 2",
        "n > 2 AND n < 9",
        "n > 2 AND n >= 4 AND n < 20 AND n <= 9",
        "n IN (1, 3, 5, 100)",
        "n > 'a'",
        "n < 'zzz'",
        "n = 'x'",
        "n IS NULL",
        "n > 3 OR owner = 'a'",
        "s LIKE 'ab%'",
        "s LIKE 'a_%'",
        "s LIKE 'ab'",
        "s LIKE 'ab%%'",
        "s LIKE 'ab%1'",
        "s >= 'b'",
        "s < 'abc'",
        "s BETWEEN 'a' AND 'b'",
        "s ~ '^ab'",
        "s ~ '^ab*'",
        "owner = 'a' AND n > 3",
        "owner = 'b' AND n BETWEEN 1 AND 10",
        "owner IN ('a', 'c') AND n < 5",
        "owner = 'a'",
        "owner = 'a' AND n = 4",
        "owner IN ('a', 'b', 'zz')",
        "owner = 'a' AND status = 'open'",
        "owner IN ('a', 'b') AND status IN ('open', 'closed')",
        "owner = 'a' AND s LIKE 'b%'",
        "status = 'open' AND n > 3",
        "status = 'open' AND n < 100 AND owner = 'b'",
        "status = 'open' AND s LIKE 'a%'",
        "n > 3 AND s LIKE 'b%'",
    ];
    for predicate in predicates {
        for select in ["*", "id, n", "id, s", "owner, n, id"] {
            let query = format!("SELECT {select} FROM $T WHERE {predicate}");
            let indexed = sorted_by_id(run(db, &query, INDEXED).await);
            let plain = sorted_by_id(run(db, &query, PLAIN).await);
            assert_eq!(indexed, plain, "{query} ({:?})", plan(db, &query).await);
        }
    }
}

async fn test_differential_ordered(db: &Db) {
    let queries: [(&str, &[&str]); 13] = [
        ("SELECT * FROM $T ORDER BY n LIMIT 5", &["n"]),
        ("SELECT * FROM $T ORDER BY n DESC LIMIT 5", &["n"]),
        (
            "SELECT * FROM $T WHERE n > 3 ORDER BY n LIMIT 4 OFFSET 2",
            &["n"],
        ),
        (
            "SELECT * FROM $T WHERE n >= -100 ORDER BY n DESC LIMIT 9",
            &["n"],
        ),
        (
            "SELECT * FROM $T WHERE owner = 'a' ORDER BY n DESC LIMIT 3",
            &["n"],
        ),
        ("SELECT * FROM $T WHERE n > 3 ORDER BY n", &["n"]),
        ("SELECT * FROM $T WHERE n > 3 ORDER BY n DESC", &["n"]),
        ("SELECT * FROM $T ORDER BY s LIMIT 7", &["s"]),
        (
            "SELECT * FROM $T WHERE s >= 'b' ORDER BY s DESC LIMIT 6",
            &["s"],
        ),
        ("SELECT * FROM $T WHERE owner = 'b' ORDER BY n", &["n"]),
        (
            "SELECT * FROM $T WHERE status = 'open' AND n >= 0 ORDER BY n DESC LIMIT 5",
            &["n"],
        ),
        (
            "SELECT * FROM $T WHERE owner IN ('a', 'b') ORDER BY owner, n LIMIT 8",
            &["owner", "n"],
        ),
        (
            "SELECT id, n FROM $T WHERE n > 3 ORDER BY n LIMIT 5",
            &["n"],
        ),
    ];
    for (query, keys) in queries {
        let indexed = run(db, query, INDEXED).await;
        let plain = run(db, query, PLAIN).await;
        assert_eq!(
            sort_keys(&indexed, keys),
            sort_keys(&plain, keys),
            "{query} ({:?})",
            plan(db, query).await
        );
        // The rows themselves must match the predicate: compare them with
        // the unlimited, unordered result of the plain collection.
        let unlimited = query.split(" ORDER BY ").next().unwrap().to_string();
        let candidates = run(db, &unlimited, PLAIN).await;
        for row in &indexed {
            assert!(candidates.contains(row), "{query}: unexpected row {row:?}");
        }
    }
}

async fn test_index_access_plans(db: &Db) {
    let expect_range = |plan: QueryPlan, index: &str, ordered: bool, descending: bool| match plan {
        QueryPlan::IndexRange {
            index_name,
            ordered: actual_ordered,
            descending: actual_descending,
            ..
        } => {
            assert_eq!(index_name, index);
            assert_eq!(actual_ordered, ordered);
            assert_eq!(actual_descending, descending);
        }
        other => panic!("expected index scan of {index}, got {other:?}"),
    };
    expect_range(
        plan(db, "SELECT * FROM $T WHERE n > 25").await,
        "by_n",
        false,
        false,
    );
    expect_range(
        plan(db, "SELECT * FROM $T WHERE s LIKE 'abc%'").await,
        "by_s",
        false,
        false,
    );
    expect_range(
        plan(
            db,
            "SELECT * FROM $T WHERE owner = 'a' AND n BETWEEN 1 AND 2",
        )
        .await,
        "by_owner_n",
        false,
        false,
    );
    expect_range(
        plan(db, "SELECT * FROM $T WHERE n > 20 ORDER BY n DESC LIMIT 2").await,
        "by_n",
        true,
        true,
    );
    // Keyset pagination: the next page is one ordered range scan.
    expect_range(
        plan(db, "SELECT id, n FROM $T WHERE n > 7 ORDER BY n LIMIT 10").await,
        "by_n",
        true,
        false,
    );
    assert!(
        matches!(
            plan(db, "SELECT * FROM $T WHERE note > 'row 5'").await,
            QueryPlan::FullScan { .. }
        ),
        "unindexed fields scan"
    );
}

/// Updates and deletes move and remove entries of every index kind.
async fn test_writes_keep_indexes_consistent(db: &Db) {
    for collection in [INDEXED, PLAIN] {
        let update = format!(
            "UPDATE {collection} SET status = 'open', n = 1000 WHERE owner = 'c' AND n < 0"
        );
        db.query_text(TextQueryFormat::Sql, update).await.unwrap();
        let delete = format!("DELETE FROM {collection} WHERE s LIKE 'ba%'");
        db.query_text(TextQueryFormat::Sql, delete).await.unwrap();
    }
    for predicate in [
        "n = 1000",
        "status = 'open' AND n > 100",
        "owner = 'c' AND n >= 1000",
        "s LIKE 'b%'",
        "n < 0",
    ] {
        let query = format!("SELECT id, n, s, owner, status FROM $T WHERE {predicate}");
        let indexed = sorted_by_id(run(db, &query, INDEXED).await);
        let plain = sorted_by_id(run(db, &query, PLAIN).await);
        assert_eq!(indexed, plain, "{query}");
    }
}

/// A package migration creates a unique composite range index.
async fn test_package_composite_index(db: &Db) {
    let collection = "suite_index_access_package";
    let package = Package {
        name: "suite.index_access".to_string(),
        root: Module {
            name: "index_access".to_string(),
            constants: BTreeMap::new(),
            types: BTreeMap::new(),
            attributes: BTreeMap::new(),
            classes: BTreeMap::new(),
            interfaces: BTreeMap::new(),
            contracts: BTreeMap::new(),
            meta: Meta::default(),
        },
        modules: BTreeMap::new(),
        migrations: vec![Migration {
            module: "index_access".to_string(),
            name: "001_init".to_string(),
            description: None,
            operations: vec![
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertCollection {
                    name: collection.to_string(),
                    kind: MigrationCollectionKind::Polymorphic,
                    integrity_mode: MigrationIntegrityMode::Permissive,
                }),
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertIndex {
                    name: "by_owner_slot".to_string(),
                    collection: collection.to_string(),
                    field: "owner".to_string(),
                    unique: true,
                    kind: IndexKind::Range,
                    extra_fields: vec!["slot".to_string()],
                    predicate: None,
                }),
            ],
            meta: Meta::default(),
        }],
        version: None,
        meta: Meta::default(),
    };
    db.upsert_package(package).await.unwrap();
    let row = |id: &str, owner: &str, slot: i64| {
        let mut object = Object::new();
        object.insert("id", Value::String(id.to_string()));
        object.insert("owner", Value::String(owner.to_string()));
        object.insert("slot", Value::I64(slot));
        BatchOperation::Upsert {
            collection: collection.to_string(),
            id: id.to_string(),
            object,
        }
    };
    db.execute_batch(
        Batch::new()
            .with_op(row("a1", "a", 1))
            .with_op(row("a2", "a", 2))
            .with_op(row("b1", "b", 1)),
    )
    .await
    .unwrap();
    let err = db
        .execute_batch(Batch::new().with_op(row("a1-again", "a", 1)))
        .await
        .unwrap_err();
    assert!(matches!(err, DbError::UniqueViolation { .. }), "{err:?}");

    let query = format!("SELECT id FROM {collection} WHERE owner = 'a' AND slot >= 2");
    let QueryResult::Select(rows) = db
        .query_text(TextQueryFormat::Sql, query.clone())
        .await
        .unwrap()
    else {
        panic!("select result expected");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("id"), Some(&Value::String("a2".to_string())));
    let plan = db
        .plan(QueryInput::Text {
            format: TextQueryFormat::Sql,
            query,
            params: Default::default(),
        })
        .await
        .unwrap();
    assert!(
        matches!(&plan, QueryPlan::IndexRange { index_name, .. } if index_name == "by_owner_slot"),
        "{plan:?}"
    );
}
