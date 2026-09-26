//! Full-text index entries: one key per distinct token and row.

use semantic_data::query::{BinaryOp, Expr, Operand, TextAnalyzer};
use semantic_data::schema::{IndexKind, IndexSchema as DataIndexSchema, KeyPath};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::catalog::{IndexSchema, LocalCollectionId, LocalIndexId};
use semantic_db_core::embedded::{EntityStorage, StorageWriteOp};

use super::{EntityStore, MemoryKvEngine, index_keys};
use crate::keys::{index_key, index_prefix};

fn full_text(columns: &[&str], analyzer: TextAnalyzer, predicate: Option<Expr>) -> IndexSchema {
    let path = |column: &str| KeyPath {
        segments: vec![column.to_string()],
    };
    IndexSchema {
        lid: LocalIndexId(4),
        schema: DataIndexSchema {
            id: "docs.search".to_string(),
            name: "search".to_string(),
            kind: IndexKind::FullText,
            collection: "docs".to_string(),
            key_path: path(columns[0]),
            unique: false,
            extra_key_paths: columns[1..].iter().map(|column| path(column)).collect(),
            predicate,
            analyzer,
        },
        collection: LocalCollectionId(7),
        canonical_field: columns[0].to_string(),
        field_id: None,
        attr_id: None,
    }
}

fn object(fields: &[(&str, Value)]) -> Object {
    let mut object = Object::new();
    for (field, value) in fields {
        object.insert(*field, value.clone());
    }
    object
}

fn string(value: &str) -> Value {
    Value::String(value.to_string())
}

fn token_keys(index: &IndexSchema, id: &str, tokens: &[&str]) -> Vec<Vec<u8>> {
    let mut keys = tokens
        .iter()
        .map(|token| index_key(index.lid, None, &string(token), id))
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

fn keys_of(index: &IndexSchema, id: &str, object: &Object) -> Vec<Vec<u8>> {
    index_keys(index, id, object).unwrap().into_iter().collect()
}

#[test]
fn one_key_per_distinct_token() {
    let index = full_text(&["body"], TextAnalyzer::default(), None);
    let row = object(&[("body", string("The cat, the HAT; the cat!"))]);
    assert_eq!(
        keys_of(&index, "d1", &row),
        token_keys(&index, "d1", &["cat", "hat", "the"])
    );
    for key in keys_of(&index, "d1", &row) {
        assert!(key.starts_with(&index_prefix(index.lid)));
        assert!(key.ends_with(b"d1"));
    }
}

#[test]
fn composite_columns_and_lists_concatenate_tokens() {
    let index = full_text(&["title", "tags"], TextAnalyzer::default(), None);
    let row = object(&[
        ("title", string("Red fox")),
        (
            "tags",
            Value::List(vec![string("animal"), Value::U64(3), string("red-list")]),
        ),
        ("ignored", string("unindexed words")),
    ]);
    assert_eq!(
        keys_of(&index, "d1", &row),
        token_keys(&index, "d1", &["animal", "fox", "list", "red"])
    );
}

#[test]
fn non_string_values_and_uncovered_rows_have_no_keys() {
    let index = full_text(&["body"], TextAnalyzer::default(), None);
    assert!(keys_of(&index, "d1", &object(&[("body", Value::U64(7))])).is_empty());
    assert!(keys_of(&index, "d1", &object(&[("body", Value::Null)])).is_empty());
    assert!(keys_of(&index, "d1", &object(&[])).is_empty());

    let published = Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
            "status",
        ])))),
        right: Box::new(Expr::Operand(Operand::Literal(string("published")))),
    };
    let partial = full_text(&["body"], TextAnalyzer::default(), Some(published));
    let draft = object(&[("body", string("hello")), ("status", string("draft"))]);
    assert!(keys_of(&partial, "d1", &draft).is_empty());
    let live = object(&[("body", string("hello")), ("status", string("published"))]);
    assert_eq!(
        keys_of(&partial, "d1", &live),
        token_keys(&partial, "d1", &["hello"])
    );
}

#[test]
fn analyzer_options_shape_the_tokens() {
    let analyzer = TextAnalyzer {
        stemming: true,
        min_token_len: 3,
    };
    let index = full_text(&["body"], analyzer, None);
    let row = object(&[("body", string("A dog is running with dogs"))]);
    assert_eq!(
        keys_of(&index, "d1", &row),
        token_keys(&index, "d1", &["dog", "run", "with"])
    );
}

#[test]
fn token_probes_find_the_rows() {
    let index = full_text(&["body"], TextAnalyzer::default(), None);
    let mut store = EntityStore::new(MemoryKvEngine::new());
    let mut ops = vec![StorageWriteOp::ResetIndex(index.lid)];
    for (id, body) in [("a", "red apple"), ("b", "green apple"), ("c", "red car")] {
        ops.push(StorageWriteOp::IndexEntity {
            index: index.clone(),
            entity_id: id.to_string(),
            object: object(&[("body", string(body))]),
        });
    }
    store.apply_batch(&ops).unwrap();
    let probe = |token: &str| {
        EntityStorage::scan_index_value(&store, index.lid, None, &string(token)).unwrap()
    };
    assert_eq!(probe("apple"), ["a", "b"]);
    assert_eq!(probe("red"), ["a", "c"]);
    assert!(probe("blue").is_empty());
    // Tokens are exact keys, not prefixes.
    assert!(probe("appl").is_empty());
    assert_eq!(store.index_keys(index.lid).unwrap().len(), 6);
}
