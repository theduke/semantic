//! Ordered scans of equality and range indexes.

use semantic_db_core::catalog::LocalIndexId;
use semantic_db_core::embedded::BoxIndexEntryScan;
use semantic_db_core::{DbError, IndexEntry, IndexScan, StorageErrorKind};

use super::KvReadTxn;
use crate::keys::{index_prefix, index_scan_range, memcmp};

/// Entries of `index` within `scan.range`, read through `txn` in key order
/// (descending with `scan.reverse`).
pub(super) fn scan_index_entries(
    txn: &dyn KvReadTxn,
    index: LocalIndexId,
    scan: &IndexScan,
) -> Result<BoxIndexEntryScan, DbError> {
    let Some((start, end)) = index_scan_range(index, &scan.range) else {
        return Ok(Box::new(std::iter::empty()));
    };
    let keys = if scan.reverse {
        txn.scan_range_rev_stream(start, Some(end))?
    } else {
        txn.scan_range_stream(start, Some(end))?
    };
    let offset = index_prefix(index).len();
    let with_keys = scan.with_keys;
    Ok(Box::new(keys.map(move |item| {
        let (key, _) = item?;
        let token = key
            .get(offset..)
            .ok_or_else(|| corrupt_entry("truncated key"))?;
        let (value, len) = if with_keys {
            let (value, len) = memcmp::decode_prefix(token)?;
            (Some(value), len)
        } else {
            (None, memcmp::encoded_len(token)?)
        };
        let id = std::str::from_utf8(&token[len..])
            .map_err(|_| corrupt_entry("entity id is not UTF-8"))?;
        Ok(IndexEntry {
            id: id.to_string(),
            key: value,
        })
    })))
}

fn corrupt_entry(message: &str) -> DbError {
    DbError::storage(
        StorageErrorKind::Corruption,
        format!("invalid index entry key: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use std::ops::Bound;

    use semantic_data::query::{BinaryOp, Expr, Operand};
    use semantic_data::schema::{IndexKind, IndexSchema as DataIndexSchema, KeyPath};
    use semantic_data::value::{FieldPath, Object, Value};
    use semantic_db_core::catalog::{IndexSchema, LocalCollectionId, LocalIndexId};
    use semantic_db_core::embedded::{EntityStorage, StorageWriteOp};
    use semantic_db_core::{IndexColumnRange, IndexScan, IndexScanRange};

    use crate::keys::index_key;
    use crate::storage::{EntityStore, MemoryKvEngine};

    fn index(kind: IndexKind, columns: &[&str], predicate: Option<Expr>) -> IndexSchema {
        let key_path = |column: &str| KeyPath {
            segments: vec![column.to_string()],
        };
        IndexSchema {
            lid: LocalIndexId(4),
            schema: DataIndexSchema {
                id: "items.idx".to_string(),
                name: "idx".to_string(),
                kind,
                collection: "items".to_string(),
                key_path: key_path(columns[0]),
                unique: false,
                extra_key_paths: columns[1..].iter().map(|column| key_path(column)).collect(),
                predicate,
                analyzer: Default::default(),
            },
            collection: LocalCollectionId(7),
            canonical_field: columns[0].to_string(),
            field_id: None,
            attr_id: None,
        }
    }

    fn row(fields: &[(&str, Value)]) -> Object {
        fields
            .iter()
            .map(|(field, value)| (field.to_string(), value.clone()))
            .collect()
    }

    fn s(value: &str) -> Value {
        Value::String(value.to_string())
    }

    fn store(index: &IndexSchema, rows: &[(&str, Object)]) -> EntityStore<MemoryKvEngine> {
        let mut ops = vec![StorageWriteOp::ResetIndex(index.lid)];
        ops.extend(rows.iter().map(|(id, object)| StorageWriteOp::IndexEntity {
            index: index.clone(),
            entity_id: id.to_string(),
            object: object.clone(),
        }));
        let mut store = EntityStore::new(MemoryKvEngine::new());
        store.apply_batch(&ops).unwrap();
        store
    }

    fn scan(
        store: &EntityStore<MemoryKvEngine>,
        range: IndexScanRange,
        reverse: bool,
    ) -> Vec<(String, Option<Value>)> {
        let snapshot = store.snapshot().unwrap();
        let scan = IndexScan {
            range,
            reverse,
            with_keys: true,
        };
        snapshot
            .scan_index_entries(LocalIndexId(4), &scan)
            .unwrap()
            .map(|entry| entry.map(|entry| (entry.id, entry.key)))
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn range_indexes_share_the_equality_key_derivation() {
        let range = index(IndexKind::Range, &["n"], None);
        let object = row(&[("n", Value::I64(3))]);
        let key = range.key_value(&object).unwrap();
        assert_eq!(key, Value::I64(3));
        let equality = index(IndexKind::Equality, &["n"], None);
        assert_eq!(equality.key_value(&object), Some(key.clone()));
        assert_eq!(
            index_key(range.lid, None, &key, "a"),
            index_key(equality.lid, None, &Value::I64(3), "a")
        );
        assert_eq!(range.key_value(&row(&[("m", Value::I64(3))])), None);
    }

    #[test]
    fn composite_keys_frame_the_column_values() {
        let composite = index(IndexKind::Range, &["owner", "n"], None);
        assert_eq!(
            composite.key_value(&row(&[("owner", s("a")), ("n", Value::I64(2))])),
            Some(Value::List(vec![s("a"), Value::I64(2)]))
        );
        // Missing trailing columns are keyed as `Void`; rows without the
        // leading column have no entry.
        assert_eq!(
            composite.key_value(&row(&[("owner", s("a"))])),
            Some(Value::List(vec![s("a"), Value::Void]))
        );
        assert_eq!(composite.key_value(&row(&[("n", Value::I64(2))])), None);
        assert_eq!(
            composite.key_columns(Value::List(vec![s("a"), Value::Void])),
            Some(vec![Some(s("a")), None])
        );
    }

    #[test]
    fn partial_indexes_skip_rows_outside_the_predicate() {
        let predicate = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                "status",
            ])))),
            right: Box::new(Expr::Operand(Operand::Literal(s("open")))),
        };
        let partial = index(IndexKind::Range, &["n"], Some(predicate));
        let rows = [
            ("a", row(&[("n", Value::I64(1)), ("status", s("open"))])),
            ("b", row(&[("n", Value::I64(2)), ("status", s("closed"))])),
            ("c", row(&[("n", Value::I64(3))])),
            ("d", row(&[("status", s("open"))])),
        ];
        let store = store(&partial, &rows);
        let all = IndexScanRange::single(IndexColumnRange::all());
        assert_eq!(
            scan(&store, all, false),
            [("a".to_string(), Some(Value::I64(1)))]
        );
    }

    #[test]
    fn composite_scans_fix_leading_columns_and_iterate_both_ways() {
        let composite = index(IndexKind::Range, &["owner", "n"], None);
        let rows = [
            ("a1", row(&[("owner", s("a")), ("n", Value::I64(1))])),
            ("a2", row(&[("owner", s("a")), ("n", Value::I64(2))])),
            ("a3", row(&[("owner", s("a")), ("n", Value::I64(3))])),
            ("a-", row(&[("owner", s("a"))])),
            ("ab", row(&[("owner", s("ab")), ("n", Value::I64(2))])),
            ("b2", row(&[("owner", s("b")), ("n", Value::I64(2))])),
            ("x", row(&[("n", Value::I64(2))])),
        ];
        let store = store(&composite, &rows);
        let ids = |entries: Vec<(String, Option<Value>)>| {
            entries.into_iter().map(|(id, _)| id).collect::<Vec<_>>()
        };
        let owner_a = |column| IndexScanRange {
            composite: true,
            prefix: vec![s("a")],
            column,
        };
        assert_eq!(
            ids(scan(&store, owner_a(IndexColumnRange::all()), false)),
            ["a-", "a1", "a2", "a3"]
        );
        let at_least_two = owner_a(IndexColumnRange::Bounds {
            lower: Bound::Included(Value::I64(2)),
            upper: Bound::Unbounded,
        });
        assert_eq!(ids(scan(&store, at_least_two.clone(), false)), ["a2", "a3"]);
        assert_eq!(ids(scan(&store, at_least_two, true)), ["a3", "a2"]);
        let below_three = owner_a(IndexColumnRange::Bounds {
            lower: Bound::Excluded(Value::Void),
            upper: Bound::Excluded(Value::I64(3)),
        });
        assert_eq!(
            scan(&store, below_three, true),
            [
                (
                    "a2".to_string(),
                    Some(Value::List(vec![s("a"), Value::I64(2)]))
                ),
                (
                    "a1".to_string(),
                    Some(Value::List(vec![s("a"), Value::I64(1)]))
                ),
            ]
        );
        let prefix = IndexScanRange {
            composite: true,
            prefix: Vec::new(),
            column: IndexColumnRange::StringPrefix("a".to_string()),
        };
        assert_eq!(
            ids(scan(&store, prefix, false)),
            ["a-", "a1", "a2", "a3", "ab"]
        );
        let exact = IndexScanRange {
            composite: true,
            prefix: vec![s("b"), Value::I64(2)],
            column: IndexColumnRange::all(),
        };
        assert_eq!(ids(scan(&store, exact, false)), ["b2"]);
        // Equality probes (unique checks) use the full key value.
        let probe = store
            .snapshot()
            .unwrap()
            .scan_index_value(
                composite.lid,
                None,
                &Value::List(vec![s("a"), Value::I64(2)]),
            )
            .unwrap();
        assert_eq!(probe, ["a2"]);
    }
}
