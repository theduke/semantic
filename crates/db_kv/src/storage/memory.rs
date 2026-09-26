use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::Arc;

use semantic_db_core::embedded::{StorageCommitOutcome, StorageTransactionCapabilities};
use semantic_db_core::{DbError, StorageErrorKind};

use super::{BoxKvPrefixScan, KvEngine, KvReadTxn, KvWriteOp, KvWriteTxn, prefix_range_end};

type KvMap = BTreeMap<Vec<u8>, Vec<u8>>;

/// In-memory key-value engine.
///
/// The map is shared copy-on-write: read handles and MVCC snapshots hold a
/// reference to the state they observe, and the next write clones the map
/// only while such a reference is alive.
#[derive(Debug, Clone, Default)]
pub struct MemoryKvEngine {
    map: Arc<KvMap>,
    revision: u64,
    mvcc_snapshots: Option<BTreeMap<u64, Arc<KvMap>>>,
}

/// Snapshot read handle over a [`MemoryKvEngine`] state.
#[derive(Debug, Clone)]
pub struct MemoryKvSnapshot {
    map: Arc<KvMap>,
    revision: u64,
}

impl MemoryKvEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_mvcc(enable_mvcc: bool) -> Self {
        let mut engine = Self::default();
        if enable_mvcc {
            engine.mvcc_snapshots = Some(BTreeMap::new());
            engine.capture_snapshot();
        }
        engine
    }

    /// Take an owned snapshot of the current state.
    pub fn read_snapshot(&self) -> MemoryKvSnapshot {
        MemoryKvSnapshot {
            map: self.map.clone(),
            revision: self.revision,
        }
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.capture_snapshot();
    }

    fn capture_snapshot(&mut self) {
        if let Some(snapshots) = &mut self.mvcc_snapshots {
            snapshots.insert(self.revision, self.map.clone());
            if snapshots.len() > 64 {
                let keep_from = snapshots
                    .keys()
                    .rev()
                    .nth(63)
                    .copied()
                    .unwrap_or(self.revision);
                snapshots.retain(|rev, _| *rev >= keep_from);
            }
        }
    }

    fn apply(map: &mut KvMap, ops: &[KvWriteOp]) {
        for op in ops {
            match op {
                KvWriteOp::Put { key, value } => {
                    map.insert(key.clone(), value.clone());
                }
                KvWriteOp::Delete { key } => {
                    map.remove(key);
                }
            }
        }
    }
}

impl KvReadTxn for MemoryKvSnapshot {
    fn revision(&self) -> Option<u64> {
        Some(self.revision)
    }

    fn is_snapshot(&self) -> bool {
        true
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self.map.get(key).cloned())
    }

    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        Ok(Box::new(RangeScan::new(self.map.clone(), start, end)))
    }
}

/// Lazy ordered scan over a shared map.
///
/// Each step seeks past the previously returned key, so the scan holds no
/// borrow of the map and only visits keys inside the range.
struct RangeScan {
    map: Arc<KvMap>,
    lower: Bound<Vec<u8>>,
    end: Option<Vec<u8>>,
}

impl RangeScan {
    fn new(map: Arc<KvMap>, start: Vec<u8>, end: Option<Vec<u8>>) -> Self {
        Self {
            map,
            lower: Bound::Included(start),
            end,
        }
    }
}

impl Iterator for RangeScan {
    type Item = super::KvScanItem;

    fn next(&mut self) -> Option<Self::Item> {
        let lower = match &self.lower {
            Bound::Included(key) => Bound::Included(key.as_slice()),
            Bound::Excluded(key) => Bound::Excluded(key.as_slice()),
            Bound::Unbounded => Bound::Unbounded,
        };
        let (key, value) = self
            .map
            .range::<[u8], _>((lower, Bound::Unbounded))
            .next()?;
        if self
            .end
            .as_ref()
            .is_some_and(|end| key.as_slice() >= end.as_slice())
        {
            return None;
        }
        let item = (key.clone(), value.clone());
        self.lower = Bound::Excluded(key.clone());
        Some(Ok(item))
    }
}

/// Write transaction applied in place, with an undo log for rollback.
struct MemoryWriteTxn<'a> {
    map: &'a mut KvMap,
    undo: Vec<(Vec<u8>, Option<Vec<u8>>)>,
}

impl MemoryWriteTxn<'_> {
    fn rollback(self) {
        for (key, previous) in self.undo.into_iter().rev() {
            match previous {
                Some(value) => self.map.insert(key, value),
                None => self.map.remove(&key),
            };
        }
    }
}

impl KvWriteTxn for MemoryWriteTxn<'_> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self.map.get(key).cloned())
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        let end = prefix_range_end(prefix);
        let upper = end.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
        Ok(self
            .map
            .range::<[u8], _>((Bound::Included(prefix), upper))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError> {
        let previous = self.map.insert(key.to_vec(), value.to_vec());
        self.undo.push((key.to_vec(), previous));
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        if let Some(previous) = self.map.remove(key) {
            self.undo.push((key.to_vec(), Some(previous)));
        }
        Ok(())
    }
}

impl KvEngine for MemoryKvEngine {
    type PrefixScan = BoxKvPrefixScan;

    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError> {
        Ok(self.map.get(key).cloned())
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError> {
        Arc::make_mut(&mut self.map).insert(key, value);
        self.bump_revision();
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> std::result::Result<(), DbError> {
        Arc::make_mut(&mut self.map).remove(key);
        self.bump_revision();
        Ok(())
    }

    fn scan_prefix_stream(
        &self,
        prefix: Vec<u8>,
    ) -> std::result::Result<Self::PrefixScan, DbError> {
        let end = prefix_range_end(&prefix);
        self.scan_range_stream(prefix, end)
    }

    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        Ok(Box::new(RangeScan::new(self.map.clone(), start, end)))
    }

    fn begin_read(&self) -> Result<Box<dyn KvReadTxn + '_>, DbError> {
        Ok(Box::new(self.read_snapshot()))
    }

    fn begin_read_owned(&self) -> Result<Option<Box<dyn KvReadTxn>>, DbError> {
        Ok(Some(Box::new(self.read_snapshot())))
    }

    fn write_with<F>(
        &mut self,
        expected_revision: Option<u64>,
        f: F,
    ) -> Result<StorageCommitOutcome, DbError>
    where
        F: FnOnce(&mut dyn KvWriteTxn) -> Result<(), DbError>,
    {
        if let Some(expected) = expected_revision
            && expected != self.revision
        {
            return Ok(StorageCommitOutcome::Conflict {
                expected_revision: Some(expected),
                actual_revision: Some(self.revision),
            });
        }
        // Copy-on-write: clones only while a read handle shares the map.
        let mut txn = MemoryWriteTxn {
            map: Arc::make_mut(&mut self.map),
            undo: Vec::new(),
        };
        if let Err(err) = f(&mut txn) {
            txn.rollback();
            return Err(err);
        }
        if !txn.undo.is_empty() {
            self.bump_revision();
        }
        Ok(StorageCommitOutcome::Committed {
            revision: Some(self.revision),
        })
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> std::result::Result<(), DbError> {
        if ops.is_empty() {
            return Ok(());
        }
        Self::apply(Arc::make_mut(&mut self.map), ops);
        self.bump_revision();
        Ok(())
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        StorageTransactionCapabilities {
            conflict_detection: true,
            mvcc: self.mvcc_snapshots.is_some(),
            snapshot_reads: self.mvcc_snapshots.is_some(),
        }
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        Ok(Some(self.revision))
    }

    fn scan_prefix_at_revision_stream(
        &self,
        prefix: Vec<u8>,
        revision: u64,
    ) -> std::result::Result<Self::PrefixScan, DbError> {
        if revision == self.revision {
            return self.scan_prefix_stream(prefix);
        }
        let Some(snapshots) = &self.mvcc_snapshots else {
            return Err(DbError::storage(
                StorageErrorKind::Unsupported,
                "snapshot reads require an mvcc-enabled engine",
            ));
        };
        let Some(snapshot) = snapshots.get(&revision) else {
            return Err(DbError::storage(
                StorageErrorKind::InvalidState,
                format!("mvcc snapshot for revision {revision} not available"),
            ));
        };
        let end = prefix_range_end(&prefix);
        Ok(Box::new(RangeScan::new(snapshot.clone(), prefix, end)))
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        let actual = Some(self.revision);
        if let Some(expected) = expected_revision
            && actual != Some(expected)
        {
            return Ok(StorageCommitOutcome::Conflict {
                expected_revision: Some(expected),
                actual_revision: actual,
            });
        }
        self.write_batch(ops)?;
        Ok(StorageCommitOutcome::Committed {
            revision: Some(self.revision),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{KvEngine, KvWriteOp, MemoryKvEngine, StorageCommitOutcome};

    #[test]
    fn conditional_write_reports_conflict() {
        let mut engine = MemoryKvEngine::new();
        engine
            .write_batch(&[KvWriteOp::Put {
                key: b"k".to_vec(),
                value: b"v1".to_vec(),
            }])
            .unwrap();

        let out = engine
            .write_batch_conditional(
                &[KvWriteOp::Put {
                    key: b"k".to_vec(),
                    value: b"v2".to_vec(),
                }],
                Some(0),
            )
            .unwrap();
        assert_eq!(
            out,
            StorageCommitOutcome::Conflict {
                expected_revision: Some(0),
                actual_revision: Some(1),
            }
        );
    }

    #[test]
    fn mvcc_snapshot_read_returns_historic_state() {
        let mut engine = MemoryKvEngine::with_mvcc(true);
        engine
            .write_batch(&[KvWriteOp::Put {
                key: b"pref/a".to_vec(),
                value: b"one".to_vec(),
            }])
            .unwrap();
        let rev1 = engine.current_revision().unwrap().unwrap();

        engine
            .write_batch(&[KvWriteOp::Put {
                key: b"pref/a".to_vec(),
                value: b"two".to_vec(),
            }])
            .unwrap();

        let past = engine.scan_prefix_at_revision(b"pref/", rev1).unwrap();
        assert_eq!(past.len(), 1);
        assert_eq!(past[0].1, b"one".to_vec());
    }
}
