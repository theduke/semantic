use std::collections::BTreeMap;

use semantic_db_core::DbError;

use super::{KvCommitOutcome, KvEngine, KvTransactionCapabilities, KvWriteOp};

#[derive(Debug, Clone, Default)]
pub struct MemoryKvEngine {
    map: BTreeMap<Vec<u8>, Vec<u8>>,
    revision: u64,
    mvcc_snapshots: Option<BTreeMap<u64, BTreeMap<Vec<u8>, Vec<u8>>>>,
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
}

impl KvEngine for MemoryKvEngine {
    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError> {
        Ok(self.map.get(key).cloned())
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError> {
        self.map.insert(key, value);
        self.bump_revision();
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> std::result::Result<(), DbError> {
        self.map.remove(key);
        self.bump_revision();
        Ok(())
    }

    fn scan_prefix(&self, prefix: &[u8]) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        Ok(self
            .map
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> std::result::Result<(), DbError> {
        for op in ops {
            match op {
                KvWriteOp::Put { key, value } => {
                    self.map.insert(key.clone(), value.clone());
                }
                KvWriteOp::Delete { key } => {
                    self.map.remove(key);
                }
            }
        }
        if !ops.is_empty() {
            self.bump_revision();
        }
        Ok(())
    }

    fn tx_capabilities(&self) -> KvTransactionCapabilities {
        KvTransactionCapabilities {
            conflict_detection: true,
            mvcc: self.mvcc_snapshots.is_some(),
            snapshot_reads: self.mvcc_snapshots.is_some(),
        }
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        Ok(Some(self.revision))
    }

    fn scan_prefix_at_revision(
        &self,
        prefix: &[u8],
        revision: u64,
    ) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        if revision == self.revision {
            return self.scan_prefix(prefix);
        }
        let Some(snapshots) = &self.mvcc_snapshots else {
            return Err(DbError::Storage(
                "snapshot reads require an mvcc-enabled engine".to_string(),
            ));
        };
        let Some(snapshot) = snapshots.get(&revision) else {
            return Err(DbError::Storage(format!(
                "mvcc snapshot for revision {revision} not available"
            )));
        };
        Ok(snapshot
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<KvCommitOutcome, DbError> {
        let actual = Some(self.revision);
        if let Some(expected) = expected_revision
            && actual != Some(expected)
        {
            return Ok(KvCommitOutcome::Conflict {
                expected_revision: Some(expected),
                actual_revision: actual,
            });
        }
        self.write_batch(ops)?;
        Ok(KvCommitOutcome::Committed {
            revision: Some(self.revision),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{KvCommitOutcome, KvEngine, KvWriteOp, MemoryKvEngine};

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
            KvCommitOutcome::Conflict {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn memory_backend_testsuite() {
        use crate::{KvBackend, KvDb};
        use semantic_db_core::Db;

        let engine = MemoryKvEngine::new();
        let db = KvDb::open(engine).unwrap();
        let backend = KvBackend::new(db);
        let db = Db::new(backend);

        semantic_db_test::suite::test_db(&db).await;
    }
}
