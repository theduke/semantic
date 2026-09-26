//! Ordered index scans over the binary index key layout.

use std::ops::Bound;

use semantic_data::schema::{IndexKind, IndexSchema as DataIndexSchema, KeyPath};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::catalog::{IndexSchema, LocalCollectionId, LocalIndexId};
use semantic_db_core::embedded::{BoxEntityIdScan, EntityStorage, StorageWriteOp};

use super::{EntityStore, MemoryKvEngine};

fn index(lid: usize, kind: IndexKind) -> IndexSchema {
    IndexSchema {
        lid: LocalIndexId(lid),
        schema: DataIndexSchema {
            id: format!("items.idx{lid}"),
            name: format!("idx{lid}"),
            kind,
            collection: "items".to_string(),
            key_path: KeyPath {
                segments: vec!["kind".to_string()],
            },
            unique: false,
            extra_key_paths: Vec::new(),
            predicate: None,
            analyzer: Default::default(),
        },
        collection: LocalCollectionId(7),
        canonical_field: "kind".to_string(),
        field_id: None,
        attr_id: None,
    }
}

fn string(value: &str) -> Value {
    Value::String(value.to_string())
}

fn ids(scan: BoxEntityIdScan) -> Vec<String> {
    scan.collect::<Result<Vec<_>, _>>().unwrap()
}

/// Store with an equality index (lid 1) over `kind` holding `rows`.
fn store_with(rows: &[(&str, Value)]) -> EntityStore<MemoryKvEngine> {
    let main = index(1, IndexKind::Equality);
    // A neighbouring index must not leak into scans of index 1.
    let other = index(10, IndexKind::Equality);
    let mut ops = vec![
        StorageWriteOp::ResetIndex(main.lid),
        StorageWriteOp::ResetIndex(other.lid),
    ];
    for (id, value) in rows {
        let mut object = Object::new();
        object.insert("kind", value.clone());
        ops.push(StorageWriteOp::IndexEntity {
            index: main.clone(),
            entity_id: id.to_string(),
            object: object.clone(),
        });
        ops.push(StorageWriteOp::IndexEntity {
            index: other.clone(),
            entity_id: format!("other-{id}"),
            object,
        });
    }
    let mut store = EntityStore::new(MemoryKvEngine::new());
    store.apply_batch(&ops).unwrap();
    store
}

/// Ids whose value lies within the bounds, ordered by value and id.
fn expected(rows: &[(&str, Value)], lower: Bound<&Value>, upper: Bound<&Value>) -> Vec<String> {
    use std::ops::RangeBounds as _;

    let mut matching = rows
        .iter()
        .filter(|(_, value)| (lower, upper).contains(value))
        .map(|(id, value)| (value.clone(), id.to_string()))
        .collect::<Vec<_>>();
    matching.sort();
    matching.into_iter().map(|(_, id)| id).collect()
}

#[test]
fn range_scans_return_ids_in_value_then_id_order() {
    let rows = [
        ("e", string("b")),
        ("a", string("ab")),
        ("d", string("ab")),
        ("c", string("ab")),
        ("b", string("a")),
        ("f", string("")),
        ("g", string("a\0")),
        ("h", Value::I64(-3)),
        ("i", Value::I64(10)),
        ("j", Value::I64(2)),
        ("k", Value::Null),
        ("l", Value::from(-0.5f64)),
        ("m", Value::List(vec![string("x")])),
    ];
    let store = store_with(&rows);
    let lid = LocalIndexId(1);
    let mut bounds = vec![Bound::Unbounded];
    for (_, value) in &rows {
        bounds.push(Bound::Included(value));
        bounds.push(Bound::Excluded(value));
    }
    let snapshot = store.snapshot().unwrap();
    for lower in &bounds {
        for upper in &bounds {
            let want = expected(&rows, *lower, *upper);
            let got = ids(EntityStorage::scan_index_range_stream(
                &store, lid, None, *lower, *upper,
            )
            .unwrap());
            assert_eq!(got, want, "{lower:?}..{upper:?}");
            let got = ids(snapshot
                .scan_index_range_stream(lid, None, *lower, *upper)
                .unwrap());
            assert_eq!(got, want, "snapshot {lower:?}..{upper:?}");
        }
    }

    // Integers sort numerically, not by their textual form.
    let (low, high) = (Value::I64(-10), Value::I64(100));
    assert_eq!(
        ids(EntityStorage::scan_index_range_stream(
            &store,
            lid,
            None,
            Bound::Included(&low),
            Bound::Included(&high),
        )
        .unwrap()),
        ["h", "j", "i"]
    );
    // Equal values yield every id, ordered by id.
    let ab = string("ab");
    assert_eq!(
        ids(EntityStorage::scan_index_range_stream(
            &store,
            lid,
            None,
            Bound::Included(&ab),
            Bound::Included(&ab),
        )
        .unwrap()),
        ["a", "c", "d"]
    );
    assert_eq!(
        store.scan_index_value(lid, None, &ab).unwrap(),
        ["a", "c", "d"]
    );
}

#[test]
fn prefix_scans_match_string_prefixes_in_order() {
    let rows = [
        ("1", string("abc")),
        ("2", string("ab")),
        ("3", string("ab\0")),
        ("4", string("a")),
        ("5", string("b")),
        ("6", string("abd")),
        ("7", Value::Bytes(b"ab".to_vec().into())),
        ("8", Value::List(vec![string("ab")])),
    ];
    let store = store_with(&rows);
    let lid = LocalIndexId(1);
    let scan = |prefix: &str| {
        ids(EntityStorage::scan_index_prefix_stream(&store, lid, None, prefix).unwrap())
    };
    assert_eq!(scan("ab"), ["2", "3", "1", "6"]);
    assert_eq!(scan("ab\0"), ["3"]);
    assert_eq!(scan("abc"), ["1"]);
    assert_eq!(scan(""), ["4", "2", "3", "1", "6", "5"]);
    assert!(scan("c").is_empty());
    assert_eq!(
        ids(store
            .snapshot()
            .unwrap()
            .scan_index_prefix_stream(lid, None, "ab")
            .unwrap()),
        ["2", "3", "1", "6"]
    );
}

#[test]
fn path_index_scans_are_scoped_to_the_path() {
    let index = index(2, IndexKind::PathEquality);
    let mut ops = vec![StorageWriteOp::ResetIndex(index.lid)];
    for (id, rank, other) in [("a", 3, 1), ("b", 1, 5), ("c", 2, 2), ("d", 10, 0)] {
        let mut meta = Object::new();
        meta.insert("rank", Value::I64(rank));
        meta.insert("other", Value::I64(other));
        let mut object = Object::new();
        object.insert("meta", Value::Object(meta));
        object.insert("name", string(&format!("name-{id}")));
        ops.push(StorageWriteOp::IndexEntity {
            index: index.clone(),
            entity_id: id.to_string(),
            object,
        });
    }
    let mut store = EntityStore::new(MemoryKvEngine::new());
    store.apply_batch(&ops).unwrap();

    let rank = FieldPath::from_fields(["meta", "rank"]);
    let (low, high) = (Value::I64(2), Value::I64(10));
    assert_eq!(
        ids(EntityStorage::scan_index_range_stream(
            &store,
            index.lid,
            Some(&rank),
            Bound::Included(&low),
            Bound::Excluded(&high),
        )
        .unwrap()),
        ["c", "a"]
    );
    assert_eq!(
        ids(EntityStorage::scan_index_range_stream(
            &store,
            index.lid,
            Some(&rank),
            Bound::Unbounded,
            Bound::Unbounded,
        )
        .unwrap()),
        ["b", "c", "a", "d"]
    );
    let name = FieldPath::from_fields(["name"]);
    assert_eq!(
        ids(
            EntityStorage::scan_index_prefix_stream(&store, index.lid, Some(&name), "name-")
                .unwrap()
        ),
        ["a", "b", "c", "d"]
    );
    assert_eq!(
        store
            .scan_index_value(index.lid, Some(&rank), &Value::I64(10))
            .unwrap(),
        ["d"]
    );
}
