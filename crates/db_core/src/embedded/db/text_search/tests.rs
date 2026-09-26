//! Planning and execution of full-text index probes on the embedded
//! database.

use std::sync::Arc;

use semantic_data::query::{TextAnalyzer, TextMatchMode};
use semantic_data::schema::IndexKind;

use super::*;
use crate::catalog::{CollectionKind, IndexDefinition};
use crate::embedded::storage::{CountingEntityStorage, StorageReadCounts};
use crate::{AccessPath, Expr, Operand, Query};

const DOCS: &str = "docs";
/// Same rows as [`DOCS`], without indexes.
const PLAIN: &str = "plain_docs";
const WORDS: [&str; 8] = [
    "apple", "Banana", "cherry", "date", "elder", "fig", "grape", "Äpfel",
];

/// Deterministic xorshift generator.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound
    }

    fn words(&mut self, max: u64) -> Vec<&'static str> {
        (0..self.below(max + 1))
            .map(|_| WORDS[self.below(WORDS.len() as u64) as usize])
            .collect()
    }

    fn text(&mut self, max: u64) -> String {
        let separators = [" ", ", ", "-", "! ", "_"];
        let mut text = String::new();
        for word in self.words(max) {
            if !text.is_empty() {
                text.push_str(separators[self.below(separators.len() as u64) as usize]);
            }
            text.push_str(word);
        }
        text
    }
}

/// Row `index` with random `title`, `body` (sometimes missing or not a
/// string) and `tags` (a list of strings).
fn random_doc(rng: &mut Rng, index: usize) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(format!("d{index:04}")));
    row.insert("title", Value::String(rng.text(3)));
    match rng.below(6) {
        0 => {}
        1 => {
            row.insert("body", Value::U64(rng.below(10)));
        }
        _ => {
            row.insert("body", Value::String(rng.text(6)));
        }
    }
    if rng.below(2) == 0 {
        row.insert(
            "tags",
            Value::List(
                rng.words(3)
                    .into_iter()
                    .map(|word| Value::String(word.to_string()))
                    .collect(),
            ),
        );
    }
    row.insert("n", Value::U64(rng.below(4)));
    row
}

fn full_text(
    collection: LocalCollectionId,
    fields: &[&str],
    analyzer: TextAnalyzer,
) -> IndexDefinition {
    IndexDefinition {
        name: "search".to_string(),
        collection,
        fields: fields.iter().map(ToString::to_string).collect(),
        unique: false,
        kind: IndexKind::FullText,
        predicate: None,
        analyzer,
    }
}

/// [`DOCS`] with a full-text index over `title`, `body` and `tags`, and
/// [`PLAIN`] with the same `rows` random documents.
fn populate<S: EntityStorage>(db: &mut EmbeddedDb<S>, rows: usize) {
    let docs = db
        .create_collection(DOCS, CollectionKind::Polymorphic)
        .unwrap();
    db.create_collection(PLAIN, CollectionKind::Polymorphic)
        .unwrap();
    db.create_index_definition(full_text(
        docs,
        &["title", "body", "tags"],
        TextAnalyzer::default(),
    ))
    .unwrap();
    let mut rng = Rng(0x7e47_5ea2_c0ff_ee01);
    let mut operations = Vec::new();
    for index in 0..rows {
        let row = random_doc(&mut rng, index);
        let id = row.get("id").and_then(Value::as_str).unwrap().to_string();
        for collection in [DOCS, PLAIN] {
            operations.push(BatchOperation::Upsert {
                collection: collection.into(),
                id: id.clone(),
                object: row.clone(),
            });
        }
    }
    db.transact(Batch { operations }).unwrap();
}

fn counting_db(rows: usize) -> (EmbeddedDb<CountingEntityStorage>, Arc<StorageReadCounts>) {
    let (storage, counts) = CountingEntityStorage::new();
    let mut db = EmbeddedDb::new(storage);
    populate(&mut db, rows);
    counts.reset();
    (db, counts)
}

fn select(query: &str) -> SelectQuery {
    match crate::sql::parse_sql_query(query, Default::default())
        .unwrap()
        .query
    {
        Query::Select(select) => select,
        other => panic!("not a select: {other:?}"),
    }
}

/// `SELECT id FROM {collection} WHERE {predicate}`.
fn select_where(collection: &str, predicate: Expr) -> SelectQuery {
    let mut query = select(&format!("SELECT id FROM {collection}"));
    query.predicate = Some(predicate);
    query
}

fn field(name: &str) -> Expr {
    Expr::Operand(Operand::Field(FieldPath::from_fields([name])))
}

fn text_match(fields: &[&str], query: &str, mode: TextMatchMode, analyzer: TextAnalyzer) -> Expr {
    Expr::TextMatch {
        exprs: fields.iter().map(|name| field(name)).collect(),
        query: Box::new(Expr::Operand(Operand::Literal(Value::String(
            query.to_string(),
        )))),
        mode,
        analyzer,
    }
}

fn access<S: EntityStorage>(db: &EmbeddedDb<S>, query: SelectQuery) -> AccessPath {
    db.explain_query(Query::Select(query)).unwrap().access_path
}

fn sorted_ids(rows: &[Object]) -> Vec<String> {
    let mut ids = rows
        .iter()
        .map(|row| row.get("id").and_then(Value::as_str).unwrap().to_string())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

#[test]
fn text_matches_on_the_indexed_fields_use_the_index() {
    let (db, _) = counting_db(50);
    let cases = [
        (
            "SELECT id FROM docs WHERE text_match(title, body, tags, 'Apple, cherry apple')",
            Some((vec!["apple", "cherry"], TextMatchMode::All)),
        ),
        (
            "SELECT id FROM docs WHERE text_match_any(tags, title, body, 'fig grape') AND n = 1",
            Some((vec!["fig", "grape"], TextMatchMode::Any)),
        ),
        (
            "SELECT id FROM docs WHERE MATCH (title, body, tags) AGAINST ('date')",
            Some((vec!["date"], TextMatchMode::All)),
        ),
        (
            "SELECT id FROM docs WHERE MATCH (title, body, tags) AGAINST ('date elder' IN NATURAL LANGUAGE MODE)",
            Some((vec!["date", "elder"], TextMatchMode::Any)),
        ),
        // Only some of the indexed fields: the index holds other tokens.
        ("SELECT id FROM docs WHERE text_match(title, 'apple')", None),
        // No tokens.
        (
            "SELECT id FROM docs WHERE text_match(title, body, tags, ' -- ')",
            None,
        ),
        // Not a conjunct.
        (
            "SELECT id FROM docs WHERE NOT text_match(title, body, tags, 'apple')",
            None,
        ),
        (
            "SELECT id FROM docs WHERE text_match(title, body, tags, 'apple') OR n = 1",
            None,
        ),
        // No full-text index.
        (
            "SELECT id FROM plain_docs WHERE text_match(title, body, tags, 'apple')",
            None,
        ),
    ];
    for (query, expected) in cases {
        let path = access(&db, select(query));
        let actual = match &path {
            AccessPath::TextSearch {
                index_name,
                fields,
                tokens,
                mode,
            } => {
                assert_eq!(index_name, "search");
                // `title` is canonicalized to the core title attribute.
                assert_eq!(fields.len(), 3, "{fields:?}");
                assert!(fields.ends_with(&["body".into(), "tags".into()]));
                Some((tokens.iter().map(String::as_str).collect::<Vec<_>>(), *mode))
            }
            _ => None,
        };
        assert_eq!(actual, expected, "{query}: {path:?}");
    }

    // Text matches with a different analyzer are not answered by the index.
    let stemmed = TextAnalyzer {
        stemming: true,
        ..Default::default()
    };
    let query = select_where(
        DOCS,
        text_match(
            &["title", "body", "tags"],
            "apple",
            TextMatchMode::All,
            stemmed,
        ),
    );
    assert!(!matches!(access(&db, query), AccessPath::TextSearch { .. }));
}

#[test]
fn indexed_text_matches_equal_scan_filters() {
    let (db, counts) = counting_db(300);
    let mut rng = Rng(0xfeed_beef_0bad_cafe);
    for round in 0..120 {
        let mode = if rng.below(2) == 0 {
            TextMatchMode::All
        } else {
            TextMatchMode::Any
        };
        let query = rng.text(3);
        let predicate = |collection_fields: &[&str]| {
            let matched = text_match(collection_fields, &query, mode, TextAnalyzer::default());
            if round % 3 == 0 {
                Expr::Binary {
                    op: semantic_data::query::BinaryOp::And,
                    left: Box::new(matched),
                    right: Box::new(Expr::Binary {
                        op: semantic_data::query::BinaryOp::Lt,
                        left: Box::new(field("n")),
                        right: Box::new(Expr::Operand(Operand::Literal(Value::U64(2)))),
                    }),
                }
            } else {
                matched
            }
        };
        let fields = ["body", "tags", "title"];
        let indexed = select_where(DOCS, predicate(&fields));
        let tokens = TextAnalyzer::default().query_tokens(&query);
        assert_eq!(
            matches!(access(&db, indexed.clone()), AccessPath::TextSearch { .. }),
            !tokens.is_empty(),
            "{query:?}"
        );

        counts.reset();
        let rows = db.select(indexed).unwrap();
        if !tokens.is_empty() {
            assert_eq!(counts.collection_scans(), 0, "{query:?}");
            if round % 3 != 0 {
                // Exactly the matching rows are read.
                assert_eq!(counts.entity_gets(), rows.len(), "{query:?}");
            }
        }
        let expected = db.select(select_where(PLAIN, predicate(&fields))).unwrap();
        assert_eq!(
            sorted_ids(&rows),
            sorted_ids(&expected),
            "{mode:?} {query:?}"
        );
    }
}

#[test]
fn analyzer_options_apply_to_index_and_matches() {
    let mut db = EmbeddedDb::new(crate::embedded::MemoryEntityStorage::new());
    let docs = db
        .create_collection(DOCS, CollectionKind::Polymorphic)
        .unwrap();
    let analyzer = TextAnalyzer {
        stemming: true,
        min_token_len: 3,
    };
    db.create_index_definition(full_text(docs, &["body"], analyzer))
        .unwrap();
    let rows = [
        ("a", "The dogs are running"),
        ("b", "a dog ran"),
        ("c", "cats jumped"),
    ];
    db.transact(Batch {
        operations: rows
            .iter()
            .map(|(id, body)| BatchOperation::Upsert {
                collection: DOCS.into(),
                id: id.to_string(),
                object: Object::from_iter([
                    ("id".to_string(), Value::String(id.to_string())),
                    ("body".to_string(), Value::String(body.to_string())),
                ]),
            })
            .collect(),
    })
    .unwrap();

    let search = |query: &str, mode| {
        let query = select_where(DOCS, text_match(&["body"], query, mode, analyzer));
        assert!(matches!(
            access(&db, query.clone()),
            AccessPath::TextSearch { .. }
        ));
        sorted_ids(&db.select(query).unwrap())
    };
    assert_eq!(search("dog", TextMatchMode::All), ["a", "b"]);
    assert_eq!(search("runs dogs", TextMatchMode::All), ["a"]);
    // `a` is below the minimum token length on both sides.
    assert_eq!(search("a jumping cat", TextMatchMode::All), ["c"]);
    assert_eq!(search("run cat", TextMatchMode::Any), ["a", "c"]);

    // The analyzer survives a reopen.
    let reopened = EmbeddedDb::open(db.storage().clone()).unwrap();
    let catalog = reopened.catalog();
    let index = catalog
        .indexes_for_collection(docs)
        .find(|index| index.schema.name == "search")
        .unwrap();
    assert_eq!(index.schema.kind, IndexKind::FullText);
    assert_eq!(index.schema.analyzer, analyzer);
    let query = select_where(
        DOCS,
        text_match(&["body"], "dogs", TextMatchMode::All, analyzer),
    );
    assert!(matches!(
        access(&reopened, query.clone()),
        AccessPath::TextSearch { .. }
    ));
    assert_eq!(sorted_ids(&reopened.select(query).unwrap()), ["a", "b"]);
}

#[test]
fn updates_and_deletes_change_the_matches() {
    let (mut db, _) = counting_db(0);
    let doc = |id: &str, title: &str| BatchOperation::Upsert {
        collection: DOCS.into(),
        id: id.to_string(),
        object: Object::from_iter([
            ("id".to_string(), Value::String(id.to_string())),
            ("title".to_string(), Value::String(title.to_string())),
        ]),
    };
    db.transact(Batch {
        operations: vec![doc("x", "old words"), doc("y", "other words")],
    })
    .unwrap();
    let search = |db: &EmbeddedDb<CountingEntityStorage>, query: &str| {
        sorted_ids(
            &db.select(select(&format!(
                "SELECT id FROM docs WHERE text_match(title, body, tags, '{query}')"
            )))
            .unwrap(),
        )
    };
    assert_eq!(search(&db, "words"), ["x", "y"]);
    db.transact(Batch {
        operations: vec![doc("x", "new text")],
    })
    .unwrap();
    assert!(search(&db, "old").is_empty());
    assert_eq!(search(&db, "new"), ["x"]);
    assert_eq!(search(&db, "words"), ["y"]);
    db.transact(Batch {
        operations: vec![BatchOperation::DeleteById {
            collection: DOCS.into(),
            id: "y".into(),
        }],
    })
    .unwrap();
    assert!(search(&db, "words").is_empty());
}

#[test]
fn invalid_full_text_definitions_are_rejected() {
    let mut db = EmbeddedDb::new(crate::embedded::MemoryEntityStorage::new());
    let docs = db
        .create_collection(DOCS, CollectionKind::Polymorphic)
        .unwrap();
    let unique = IndexDefinition {
        unique: true,
        ..full_text(docs, &["body"], TextAnalyzer::default())
    };
    assert!(db.create_index_definition(unique).is_err());
    let analyzed_equality = IndexDefinition {
        kind: IndexKind::Equality,
        ..full_text(
            docs,
            &["body"],
            TextAnalyzer {
                stemming: true,
                ..Default::default()
            },
        )
    };
    assert!(db.create_index_definition(analyzed_equality).is_err());
}

#[test]
fn opening_a_database_from_before_maintained_full_text_indexes_rebuilds_them() {
    let mut db = EmbeddedDb::new(crate::embedded::MemoryEntityStorage::new());
    populate(&mut db, 30);
    let search = db
        .catalog()
        .indexes()
        .find(|(_, index)| index.schema.name == "search")
        .map(|(lid, _)| lid)
        .unwrap();
    let query = "SELECT id FROM docs WHERE text_match(title, body, tags, 'apple')";
    let expected = sorted_ids(&db.select(select(query)).unwrap());
    assert!(!expected.is_empty());

    // Earlier versions registered full-text indexes without entries and
    // without the full-text core migration.
    let mut storage = db.storage().clone();
    let mut snapshot = db.catalog().to_storage_snapshot();
    snapshot
        .applied_migrations
        .retain(|item| item.applied.migration.name != crate::ddl::FULL_TEXT_INDEXES_MIGRATION);
    let legacy = Catalog::from_storage_snapshot(snapshot).unwrap();
    let ops = catalog_write_ops(&storage, &legacy).unwrap();
    storage.apply_batch(&ops).unwrap();
    storage.drop_index_entries(search);
    assert!(!storage.index_needs_rebuild(search).unwrap());

    let reopened = EmbeddedDb::open(storage).unwrap();
    assert!(
        reopened
            .catalog()
            .applied_migration("semantic", "core", crate::ddl::FULL_TEXT_INDEXES_MIGRATION)
            .is_some()
    );
    assert!(matches!(
        access(&reopened, select(query)),
        AccessPath::TextSearch { .. }
    ));
    assert_eq!(
        sorted_ids(&reopened.select(select(query)).unwrap()),
        expected
    );
}
