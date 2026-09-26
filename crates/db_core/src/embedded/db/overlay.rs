//! Queries inside interactive transactions: a storage snapshot with the
//! transaction's uncommitted writes layered on top.
//!
//! [`OverlaySnapshot`] implements [`EntityReadSnapshot`], so the regular
//! query executor ([`DbReader`]) runs unchanged over it. Semantics per read:
//!
//! - Point reads return the written row, `None` for a row deleted by the
//!   transaction, and otherwise the snapshot row.
//! - Collection scans skip every snapshot row the transaction wrote and
//!   merge in the written rows that still exist, in id order.
//! - Index lookups and ordered index scans take the snapshot's entries, drop
//!   every id the transaction wrote in the index's collection (deleted rows,
//!   and updated rows whose key may have changed), then add the written rows
//!   whose entry matches: the index key
//!   ([`IndexSchema::key_value`](crate::catalog::IndexSchema::key_value),
//!   honouring partial index predicates) equals the value or lies in the
//!   scanned range, or for path-equality indexes the value at the path
//!   equals the looked-up value. Entries stay in (key, id) order, so ordered
//!   scans, `LIMIT` and index-only reads see the transaction's rows in
//!   place. Residual predicates are applied by the executor as usual.
//! - Maintained row counts are adjusted by the transaction's inserts and
//!   deletes; index entry counts (planner statistics only) are those of the
//!   snapshot.
//!
//! Derived data maintained at commit (reverse references and relationship
//! edges) is not part of the overlay: relationship predicates observe the
//! snapshot's committed edges.

use std::ops::Bound;

use super::*;
use crate::batch_return::EntityKey;
use crate::embedded::storage::{BoxEntityIdScan, BoxEntityScan, BoxIndexEntryScan};

type WrittenRows = BTreeMap<String, Option<Object>>;

/// A storage snapshot with a transaction's writes layered on top.
pub(crate) struct OverlaySnapshot {
    base: Arc<dyn EntityReadSnapshot>,
    catalog: Arc<Catalog>,
    /// Rows written by the transaction by collection and id (`None`:
    /// deleted).
    rows: BTreeMap<LocalCollectionId, WrittenRows>,
}

impl OverlaySnapshot {
    pub(crate) fn new(
        base: Arc<dyn EntityReadSnapshot>,
        catalog: Arc<Catalog>,
        overlay: &BTreeMap<EntityKey, Option<Object>>,
    ) -> Self {
        let mut rows = BTreeMap::<LocalCollectionId, WrittenRows>::new();
        for ((collection, id), row) in overlay {
            if let Some(collection) = catalog.collection_by_name(collection) {
                rows.entry(collection.lid)
                    .or_default()
                    .insert(id.clone(), row.clone());
            }
        }
        Self {
            base,
            catalog,
            rows,
        }
    }

    /// The written rows of the collection of `index`, with the index, when
    /// the transaction wrote any.
    fn index_rows(
        &self,
        index: LocalIndexId,
    ) -> Option<(&crate::catalog::IndexSchema, &WrittenRows)> {
        let index = self.catalog.index_by_lid(index)?;
        Some((index, self.rows.get(&index.collection)?))
    }
}

fn stored(collection: LocalCollectionId, id: &str, object: Object) -> StoredEntity {
    StoredEntity {
        collection: collection.0,
        kind: StoredEntityKind::Untyped,
        id: id.to_string(),
        object,
    }
}

/// The value of `object` at `path`.
fn value_at_path<'o>(object: &'o Object, path: &FieldPath) -> Option<&'o Value> {
    let mut segments = path.segments().iter();
    let mut value = match segments.next()? {
        PathSegment::Field(field) => object.get(field)?,
        PathSegment::Index(_) => return None,
    };
    for segment in segments {
        value = match (segment, value) {
            (PathSegment::Field(field), Value::Object(object)) => object.get(field)?,
            (PathSegment::Index(index), Value::List(items)) => items.get(*index)?,
            _ => return None,
        };
    }
    Some(value)
}

/// Merge `extra` into the ordered `base` stream; `before(a, b)` orders two
/// items. Errors of `base` pass through in place.
fn merge_ordered<T: Send + 'static>(
    base: impl Iterator<Item = Result<T, DbError>> + Send + 'static,
    extra: Vec<T>,
    before: impl Fn(&T, &T) -> bool + Send + 'static,
) -> Box<dyn Iterator<Item = Result<T, DbError>> + Send> {
    let mut base = base.peekable();
    let mut extra = extra.into_iter().peekable();
    Box::new(std::iter::from_fn(move || {
        let take_extra = match (base.peek(), extra.peek()) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(Err(_)), Some(_)) => false,
            (Some(Ok(stored)), Some(written)) => before(written, stored),
        };
        if take_extra {
            extra.next().map(Ok)
        } else {
            base.next()
        }
    }))
}

impl EntityReadSnapshot for OverlaySnapshot {
    fn revision(&self) -> Result<Option<u64>, DbError> {
        self.base.revision()
    }

    fn is_consistent(&self) -> bool {
        self.base.is_consistent()
    }

    fn get_entity(
        &self,
        collection: LocalCollectionId,
        id: &str,
    ) -> Result<Option<StoredEntity>, DbError> {
        match self.rows.get(&collection).and_then(|rows| rows.get(id)) {
            Some(row) => Ok(row.clone().map(|object| stored(collection, id, object))),
            None => self.base.get_entity(collection, id),
        }
    }

    fn scan_collection_stream(
        &self,
        collection: LocalCollectionId,
    ) -> Result<BoxEntityScan, DbError> {
        let base = self.base.scan_collection_stream(collection)?;
        let Some(rows) = self.rows.get(&collection) else {
            return Ok(base);
        };
        let written = rows.keys().cloned().collect::<BTreeSet<_>>();
        let base = base.filter(move |entity| {
            entity
                .as_ref()
                .map_or(true, |entity| !written.contains(&entity.id))
        });
        let extra = rows
            .iter()
            .filter_map(|(id, row)| Some(stored(collection, id, row.clone()?)))
            .collect();
        Ok(merge_ordered(base, extra, |a: &StoredEntity, b| {
            a.id < b.id
        }))
    }

    fn collection_row_count(&self, collection: LocalCollectionId) -> Result<Option<u64>, DbError> {
        let Some(count) = self.base.collection_row_count(collection)? else {
            return Ok(None);
        };
        let Some(rows) = self.rows.get(&collection) else {
            return Ok(Some(count));
        };
        let mut count = count as i128;
        for (id, row) in rows {
            let stored = self.base.get_entity(collection, id)?.is_some();
            count += i128::from(row.is_some()) - i128::from(stored);
        }
        Ok(Some(count.max(0) as u64))
    }

    fn index_entry_count(&self, index: LocalIndexId) -> Result<Option<u64>, DbError> {
        self.base.index_entry_count(index)
    }

    fn scan_index_value_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        value: &Value,
    ) -> Result<BoxEntityIdScan, DbError> {
        let base = self.base.scan_index_value_stream(index, path, value)?;
        let Some((schema, rows)) = self.index_rows(index) else {
            return Ok(base);
        };
        let written = rows.keys().cloned().collect::<BTreeSet<_>>();
        let base = base.filter(move |id| id.as_ref().map_or(true, |id| !written.contains(id)));
        let extra = rows
            .iter()
            .filter(|(_, row)| {
                row.as_ref().is_some_and(|row| match path {
                    Some(path) => value_at_path(row, path) == Some(value),
                    None => schema.key_value(row).as_ref() == Some(value),
                })
            })
            .map(|(id, _)| id.clone())
            .collect();
        Ok(merge_ordered(base, extra, |a: &String, b| a < b))
    }

    fn scan_index_range_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        lower: Bound<&Value>,
        upper: Bound<&Value>,
    ) -> Result<BoxEntityIdScan, DbError> {
        match (path, self.index_rows(index)) {
            (_, None) => return self.base.scan_index_range_stream(index, path, lower, upper),
            // Ordered path-index entries of written rows are not merged.
            (Some(_), Some(_)) => return Err(crate::embedded::unsupported_ordered_index_scan()),
            (None, Some(_)) => {}
        }
        let column = crate::IndexColumnRange::Bounds {
            lower: lower.cloned(),
            upper: upper.cloned(),
        };
        self.scan_index_ids(index, crate::IndexScanRange::single(column))
    }

    fn scan_index_prefix_stream(
        &self,
        index: LocalIndexId,
        path: Option<&FieldPath>,
        prefix: &str,
    ) -> Result<BoxEntityIdScan, DbError> {
        match (path, self.index_rows(index)) {
            (_, None) => return self.base.scan_index_prefix_stream(index, path, prefix),
            (Some(_), Some(_)) => return Err(crate::embedded::unsupported_ordered_index_scan()),
            (None, Some(_)) => {}
        }
        let column = crate::IndexColumnRange::StringPrefix(prefix.to_string());
        self.scan_index_ids(index, crate::IndexScanRange::single(column))
    }

    fn scan_index_entries(
        &self,
        index: LocalIndexId,
        scan: &crate::IndexScan,
    ) -> Result<BoxIndexEntryScan, DbError> {
        let Some((schema, rows)) = self.index_rows(index) else {
            return self.base.scan_index_entries(index, scan);
        };
        // Merging needs the keys of the snapshot's entries.
        let base = self.base.scan_index_entries(
            index,
            &crate::IndexScan {
                with_keys: true,
                ..scan.clone()
            },
        )?;
        let written = rows.keys().cloned().collect::<BTreeSet<_>>();
        let base = base.filter(move |entry| {
            entry
                .as_ref()
                .map_or(true, |entry| !written.contains(&entry.id))
        });
        let mut extra = rows
            .iter()
            .filter_map(|(id, row)| {
                let key = schema.key_value(row.as_ref()?)?;
                scan.range.contains(&key).then(|| crate::IndexEntry {
                    id: id.clone(),
                    key: Some(key),
                })
            })
            .collect::<Vec<_>>();
        let order = |a: &crate::IndexEntry, b: &crate::IndexEntry| {
            (a.key.as_ref(), &a.id).cmp(&(b.key.as_ref(), &b.id))
        };
        let reverse = scan.reverse;
        extra.sort_by(|a, b| {
            let ordering = order(a, b);
            if reverse {
                ordering.reverse()
            } else {
                ordering
            }
        });
        let merged = merge_ordered(base, extra, move |a, b| {
            let ordering = order(a, b);
            if reverse {
                ordering.is_gt()
            } else {
                ordering.is_lt()
            }
        });
        if scan.with_keys {
            return Ok(merged);
        }
        Ok(Box::new(merged.map(|entry| {
            entry.map(|entry| crate::IndexEntry { key: None, ..entry })
        })))
    }

    fn index_needs_rebuild(&self, index: LocalIndexId) -> Result<bool, DbError> {
        self.base.index_needs_rebuild(index)
    }
}

impl OverlaySnapshot {
    /// Ids of the entries of the value index `index` within `range`, in key
    /// order.
    fn scan_index_ids(
        &self,
        index: LocalIndexId,
        range: crate::IndexScanRange,
    ) -> Result<BoxEntityIdScan, DbError> {
        let entries = self.scan_index_entries(
            index,
            &crate::IndexScan {
                range,
                reverse: false,
                with_keys: false,
            },
        )?;
        Ok(Box::new(entries.map(|entry| entry.map(|entry| entry.id))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_order_and_passes_errors_through() {
        let base = vec![
            Ok(1),
            Ok(4),
            Err(DbError::InvalidQuery("boom".into())),
            Ok(9),
        ];
        let merged = merge_ordered(base.into_iter(), vec![0, 5, 10], |a, b| a < b)
            .map(|item| item.map_err(|err| err.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(
            merged,
            vec![
                Ok(0),
                Ok(1),
                Ok(4),
                Err("invalid query: boom".to_string()),
                Ok(5),
                Ok(9),
                Ok(10)
            ]
        );
    }

    #[test]
    fn values_at_nested_paths() {
        let object = Object::from_iter([(
            "a".to_string(),
            Value::List(vec![Value::Object(Object::from_iter([(
                "b".to_string(),
                Value::I64(1),
            )]))]),
        )]);
        let mut path = FieldPath::new();
        path.push_field("a".to_string());
        path.push_index(0);
        path.push_field("b".to_string());
        assert_eq!(value_at_path(&object, &path), Some(&Value::I64(1)));
        path.push_index(3);
        assert_eq!(value_at_path(&object, &path), None);
    }
}
