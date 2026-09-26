//! Execution of [`PhysicalIndexScan`](crate::PhysicalIndexScan)s over the
//! storage's ordered index scans.
//!
//! Entries are read range by range in key order (reversed for descending
//! scans). Rows are point-read per entry, lazily when the reader owns its
//! snapshot, so a `LIMIT` above an ordered scan stops reading early.
//! Index-only scans build rows from the decoded keys (key columns plus the
//! id) and only point-read rows whose key does not decode losslessly.

use super::*;
use crate::embedded::storage::BoxIndexEntryScan;

type EntityRows = Box<dyn Iterator<Item = Result<StoredEntity, crate::CoreError>> + Send>;

/// How an index scan turns entries into rows.
struct RowSource {
    collection: LocalCollectionId,
    index: crate::catalog::IndexSchema,
    id_field: String,
    index_only: bool,
}

impl RowSource {
    /// The row of `entry` built from its key, when the scan is index-only
    /// and the key decodes losslessly.
    fn row_from_key(&self, entry: &crate::IndexEntry) -> Option<StoredEntity> {
        if !self.index_only {
            return None;
        }
        let key = entry.key.clone()?;
        if !crate::key_decodes_losslessly(&key) {
            return None;
        }
        let mut object = Object::new();
        object.insert(self.id_field.clone(), Value::String(entry.id.clone()));
        for (column, value) in self.index.columns().zip(self.index.key_columns(key)?) {
            if let Some(value) = value {
                object.insert(column.to_string(), value);
            }
        }
        Some(StoredEntity {
            collection: self.collection.0,
            // Row views only read the object.
            kind: StoredEntityKind::Untyped,
            id: entry.id.clone(),
            object,
        })
    }

    fn row(
        &self,
        entry: crate::IndexEntry,
        read: impl FnOnce(LocalCollectionId, &str) -> Result<Option<StoredEntity>, DbError>,
    ) -> Result<Option<StoredEntity>, crate::CoreError> {
        if let Some(row) = self.row_from_key(&entry) {
            return Ok(Some(row));
        }
        read(self.collection, &entry.id).map_err(|err| crate::CoreError::new(err.to_string()))
    }
}

impl EmbeddedPhysicalDataSource<'_> {
    /// Rows of `scan` read through the index, or `None` when the index or
    /// ordered scans are unavailable (the caller then falls back to a
    /// filtered scan).
    pub(super) fn index_range_scan(
        &self,
        scan: &crate::PhysicalIndexScan,
    ) -> Result<Option<EmbeddedCollectionScan>, crate::CoreError> {
        let collection = self
            .resolve_collection(&scan.source)
            .map_err(|err| crate::CoreError::new(err.to_string()))?;
        let Some(index) = self.catalog.index_by_lid(scan.index).filter(|index| {
            index.collection == collection.lid && index.schema.kind.is_value_index()
        }) else {
            return Ok(None);
        };
        let Some(entries) = self.open_entry_scans(index, scan)? else {
            return Ok(None);
        };
        // Without residual filtering every entry yields a row, so a limited
        // scan never needs more entries than the limit.
        let entries: BoxIndexEntryScan = match (scan.limit_hint, &scan.residual_predicate) {
            (Some(limit), None) => Box::new(entries.take(limit)),
            _ => entries,
        };
        let source = RowSource {
            collection: collection.lid,
            index: index.clone(),
            id_field: collection
                .canonical_field_name(crate::catalog::PRIMARY_ID_FIELD)
                .to_string(),
            index_only: scan.index_only,
        };
        let rows: EntityRows = match self.reader.shared() {
            Some(snapshot) => Box::new(entries.filter_map(move |entry| {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => return Some(Err(crate::CoreError::new(err.to_string()))),
                };
                source
                    .row(entry, |collection, id| snapshot.get_entity(collection, id))
                    .transpose()
            })),
            None => {
                let mut rows = Vec::new();
                for entry in entries {
                    let entry = entry.map_err(|err| crate::CoreError::new(err.to_string()))?;
                    if let Some(row) = source.row(entry, |collection, id| {
                        self.reader.get_entity(collection, id)
                    })? {
                        rows.push(Ok(row));
                    }
                }
                Box::new(rows.into_iter())
            }
        };
        Ok(Some(self.row_views(collection, rows)?))
    }

    /// The concatenated entry scans of `scan`'s ranges, or `None` when the
    /// storage has no ordered index scans.
    fn open_entry_scans(
        &self,
        index: &crate::catalog::IndexSchema,
        scan: &crate::PhysicalIndexScan,
    ) -> Result<Option<BoxIndexEntryScan>, crate::CoreError> {
        let reverse = scan.direction == semantic_data::query::SortDirection::Desc;
        let mut ranges = scan.ranges.clone();
        if reverse {
            ranges.reverse();
        }
        let mut scans = Vec::with_capacity(ranges.len());
        for range in ranges {
            let request = crate::IndexScan {
                range,
                reverse,
                with_keys: scan.index_only,
            };
            match self.reader.scan_index_entries(index.lid, &request) {
                Ok(entries) => scans.push(entries),
                Err(err) if err.storage_kind() == Some(crate::StorageErrorKind::Unsupported) => {
                    return Ok(None);
                }
                Err(err) => return Err(crate::CoreError::new(err.to_string())),
            }
        }
        Ok(Some(Box::new(scans.into_iter().flatten())))
    }
}

#[cfg(all(test, feature = "sql"))]
mod tests;
