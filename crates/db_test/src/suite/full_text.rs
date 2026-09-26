//! Full-text indexes created by a package migration.
//!
//! Differential checks: text matches run against a collection with
//! full-text indexes and against an identical collection without any, over
//! seeded random documents (several fields, lists of strings, missing and
//! non-string values). Results must be equal and the indexed queries must
//! be planned as full-text probes. Idempotent, so it can run again after a
//! reopen.

use super::*;
use semantic_data::query::{Query, TextAnalyzer, TextMatchMode};
use semantic_data::schema::IndexKind;
use semantic_db_core::QueryPlan;

const INDEXED: &str = "suite_full_text_indexed";
const PLAIN: &str = "suite_full_text_plain";
const ROWS: usize = 160;
const WORDS: [&str; 7] = ["red", "Green", "blue", "fox", "dogs", "running", "Über"];
/// Analyzer of the stemmed `body` index.
const STEMMED: TextAnalyzer = TextAnalyzer {
    stemming: true,
    min_token_len: 2,
};

/// Deterministic xorshift generator.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound
    }

    fn text(&mut self, max: u64) -> String {
        (0..self.below(max + 1))
            .map(|_| WORDS[self.below(WORDS.len() as u64) as usize])
            .collect::<Vec<_>>()
            .join(["", " ", ", ", "-"][self.below(4) as usize])
    }
}

fn random_doc(rng: &mut Rng, index: usize) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(format!("t{index:04}")));
    row.insert("headline", Value::String(rng.text(3)));
    match rng.below(5) {
        0 => {}
        1 => {
            row.insert("body", Value::I64(rng.below(9) as i64));
        }
        _ => {
            row.insert("body", Value::String(rng.text(5)));
        }
    }
    if rng.below(2) == 0 {
        row.insert(
            "labels",
            Value::List(
                (0..rng.below(3))
                    .map(|_| Value::String(rng.text(2)))
                    .collect(),
            ),
        );
    }
    row
}

fn full_text_index(name: &str, fields: &[&str], analyzer: TextAnalyzer) -> MigrationOperation {
    MigrationOperation::Ddl(MigrationDdlOperation::UpsertIndex {
        name: name.to_string(),
        collection: INDEXED.to_string(),
        field: fields[0].to_string(),
        unique: false,
        kind: IndexKind::FullText,
        extra_fields: fields[1..].iter().map(ToString::to_string).collect(),
        predicate: None,
        analyzer,
    })
}

fn package() -> Package {
    let collection = |name: &str| {
        MigrationOperation::Ddl(MigrationDdlOperation::UpsertCollection {
            name: name.to_string(),
            kind: MigrationCollectionKind::Polymorphic,
            integrity_mode: MigrationIntegrityMode::Permissive,
        })
    };
    Package {
        name: "suite.full_text".to_string(),
        root: Module {
            name: "full_text".to_string(),
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
            module: "full_text".to_string(),
            name: "001_init".to_string(),
            description: None,
            operations: vec![
                collection(INDEXED),
                collection(PLAIN),
                full_text_index(
                    "search",
                    &["headline", "body", "labels"],
                    TextAnalyzer::default(),
                ),
                full_text_index("body_stemmed", &["body"], STEMMED),
            ],
            meta: Meta::default(),
        }],
        version: None,
        meta: Meta::default(),
    }
}

async fn setup(db: &Db) {
    db.upsert_package(package()).await.unwrap();
    let mut rng = Rng(0x0f00_d5ee_d0ff_1ce5);
    let rows = (0..ROWS)
        .map(|index| random_doc(&mut rng, index))
        .collect::<Vec<_>>();
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

fn text_match(fields: &[&str], query: &str, mode: TextMatchMode, analyzer: TextAnalyzer) -> Expr {
    Expr::TextMatch {
        exprs: fields
            .iter()
            .map(|field| Expr::Operand(Operand::Field(FieldPath::from_fields([*field]))))
            .collect(),
        query: Box::new(Expr::Operand(Operand::Literal(Value::String(
            query.to_string(),
        )))),
        mode,
        analyzer,
    }
}

fn select(collection: &str, predicate: Expr) -> SelectQuery {
    SelectQuery::new()
        .with_collection(collection)
        .with_predicate(predicate)
}

fn sorted_ids(rows: &[Object]) -> Vec<String> {
    let mut ids = rows
        .iter()
        .map(|row| row.get("id").and_then(Value::as_str).unwrap().to_string())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

async fn check(db: &Db, fields: &[&str], query: &str, mode: TextMatchMode, analyzer: TextAnalyzer) {
    let predicate = text_match(fields, query, mode, analyzer);
    let indexed = select(INDEXED, predicate.clone());
    let plan = db
        .plan(QueryInput::Ast(Query::Select(indexed.clone())))
        .await
        .unwrap();
    let has_tokens = !analyzer.query_tokens(query).is_empty();
    assert_eq!(
        matches!(plan, QueryPlan::TextSearch { .. }),
        has_tokens,
        "{query:?}: {plan:?}"
    );
    let rows = db.select(indexed).await.unwrap();
    let expected = db.select(select(PLAIN, predicate)).await.unwrap();
    assert_eq!(
        sorted_ids(&rows),
        sorted_ids(&expected),
        "{fields:?} {mode:?} {query:?}"
    );
}

pub async fn test_full_text(db: &Db) {
    setup(db).await;
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for _ in 0..40 {
        let query = rng.text(3);
        for mode in [TextMatchMode::All, TextMatchMode::Any] {
            check(
                db,
                &["labels", "headline", "body"],
                &query,
                mode,
                TextAnalyzer::default(),
            )
            .await;
            check(db, &["body"], &query, mode, STEMMED).await;
        }
    }

    // SQL surface.
    let sql = format!("SELECT id FROM {INDEXED} WHERE text_match(headline, body, labels, 'fox')");
    let plan = db
        .plan(QueryInput::Text {
            format: TextQueryFormat::Sql,
            query: sql.clone(),
            params: Default::default(),
        })
        .await
        .unwrap();
    assert!(
        matches!(&plan, QueryPlan::TextSearch { index_name, tokens, .. }
            if index_name == "search" && tokens == &["fox"]),
        "{plan:?}"
    );
    let QueryResult::Select(rows) = db.query_text(TextQueryFormat::Sql, sql).await.unwrap() else {
        panic!("select result expected");
    };
    let expected = db
        .select(select(
            PLAIN,
            text_match(
                &["headline", "body", "labels"],
                "fox",
                TextMatchMode::All,
                TextAnalyzer::default(),
            ),
        ))
        .await
        .unwrap();
    assert_eq!(sorted_ids(&rows), sorted_ids(&expected));
    assert!(!rows.is_empty());
}
