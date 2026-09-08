use semantic_db_core::DbError;
use semantic_db_core::embedded::{StorageCommitOutcome, StorageTransactionCapabilities};
use semantic_db_kv::{BoxKvPrefixScan, KvEngine, KvWriteOp, MemoryKvEngine};

use crate::event;
use crate::{EventId, LogStore};

/// A write-ahead, memory-resident key-value engine.
#[derive(Debug)]
pub struct LogEngine<S: LogStore> {
    store: S,
    memory: MemoryKvEngine,
    last_event: Option<EventId>,
    poisoned: bool,
}

impl<S: LogStore> LogEngine<S> {
    pub fn open(store: S) -> std::result::Result<Self, DbError> {
        let mut ids = store.event_ids(EventId::FIRST)?;
        ids.sort_unstable();
        let mut memory = MemoryKvEngine::new();
        let mut expected = EventId::FIRST;
        let mut last_event = None;
        for id in ids {
            if id != expected {
                return Err(DbError::Storage(format!(
                    "WAL event sequence gap: expected {}, found {}",
                    expected.get(),
                    id.get()
                )));
            }
            let bytes = store.read_event(id)?.ok_or_else(|| {
                DbError::Storage(format!("WAL event {} disappeared during replay", id.get()))
            })?;
            let operations = event::decode(id, &bytes)?;
            memory.write_batch(&operations)?;
            last_event = Some(id);
            expected = id.next()?;
        }
        Ok(Self {
            store,
            memory,
            last_event,
            poisoned: false,
        })
    }

    pub fn event_count(&self) -> u64 {
        self.last_event.map_or(0, EventId::get)
    }

    fn ensure_healthy(&self) -> std::result::Result<(), DbError> {
        if self.poisoned {
            Err(DbError::Storage(
                "WAL engine is poisoned after an uncertain commit; reopen it".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    fn commit(
        &mut self,
        operations: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        self.ensure_healthy()?;
        let actual_revision = self.memory.current_revision()?;
        if let Some(expected) = expected_revision
            && actual_revision != Some(expected)
        {
            return Ok(StorageCommitOutcome::Conflict {
                expected_revision: Some(expected),
                actual_revision,
            });
        }
        if operations.is_empty() {
            return Ok(StorageCommitOutcome::Committed {
                revision: actual_revision,
            });
        }
        let id = match self.last_event {
            Some(id) => id.next()?,
            None => EventId::FIRST,
        };
        let bytes = event::encode(id, operations)?;
        if let Err(err) = self.store.append_event(id, bytes) {
            self.poisoned = true;
            return Err(err);
        }
        if let Err(err) = self.memory.write_batch(operations) {
            self.poisoned = true;
            return Err(DbError::Storage(format!(
                "WAL event {} committed but cache apply failed: {err}",
                id.get()
            )));
        }
        self.last_event = Some(id);
        Ok(StorageCommitOutcome::Committed {
            revision: self.memory.current_revision()?,
        })
    }
}

impl<S: LogStore> KvEngine for LogEngine<S> {
    type PrefixScan = BoxKvPrefixScan;

    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError> {
        self.ensure_healthy()?;
        self.memory.get(key)
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError> {
        self.write_batch(&[KvWriteOp::Put { key, value }])
    }

    fn delete(&mut self, key: &[u8]) -> std::result::Result<(), DbError> {
        self.write_batch(&[KvWriteOp::Delete { key: key.to_vec() }])
    }

    fn scan_prefix_stream(
        &self,
        prefix: Vec<u8>,
    ) -> std::result::Result<Self::PrefixScan, DbError> {
        self.ensure_healthy()?;
        self.memory.scan_prefix_stream(prefix)
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.memory.tx_capabilities()
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        self.ensure_healthy()?;
        self.memory.current_revision()
    }

    fn scan_prefix_at_revision_stream(
        &self,
        prefix: Vec<u8>,
        revision: u64,
    ) -> std::result::Result<Self::PrefixScan, DbError> {
        self.ensure_healthy()?;
        self.memory.scan_prefix_at_revision_stream(prefix, revision)
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        self.commit(ops, expected_revision)
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> std::result::Result<(), DbError> {
        match self.commit(ops, None)? {
            StorageCommitOutcome::Committed { .. } => Ok(()),
            StorageCommitOutcome::Conflict { .. } => Err(DbError::Storage(
                "unexpected unconditional WAL commit conflict".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, btree_map::Entry};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Debug, Default)]
    struct State {
        events: BTreeMap<EventId, Vec<u8>>,
        listed_ids: Option<Vec<EventId>>,
        fail_after_append: bool,
    }

    #[derive(Clone, Debug, Default)]
    struct MemoryLogStore(Arc<Mutex<State>>);

    impl LogStore for MemoryLogStore {
        fn event_ids(&self, from: EventId) -> std::result::Result<Vec<EventId>, DbError> {
            let state = self.0.lock().unwrap();
            Ok(state
                .listed_ids
                .clone()
                .unwrap_or_else(|| state.events.keys().copied().collect())
                .into_iter()
                .filter(|id| *id >= from)
                .collect())
        }

        fn read_event(&self, id: EventId) -> std::result::Result<Option<Vec<u8>>, DbError> {
            Ok(self.0.lock().unwrap().events.get(&id).cloned())
        }

        fn append_event(
            &mut self,
            id: EventId,
            bytes: Vec<u8>,
        ) -> std::result::Result<(), DbError> {
            let mut state = self.0.lock().unwrap();
            match state.events.entry(id) {
                Entry::Vacant(entry) => {
                    entry.insert(bytes);
                }
                Entry::Occupied(_) => {
                    return Err(DbError::Storage("duplicate event".to_string()));
                }
            }
            if state.fail_after_append {
                return Err(DbError::Storage("uncertain append".to_string()));
            }
            Ok(())
        }
    }

    #[test]
    fn committed_batches_replay_in_order() {
        let store = MemoryLogStore::default();
        let mut engine = LogEngine::open(store.clone()).unwrap();
        engine
            .write_batch(&[
                KvWriteOp::Put {
                    key: b"a".to_vec(),
                    value: b"first".to_vec(),
                },
                KvWriteOp::Put {
                    key: b"a".to_vec(),
                    value: b"second".to_vec(),
                },
            ])
            .unwrap();
        engine
            .write_batch(&[KvWriteOp::Delete { key: b"a".to_vec() }])
            .unwrap();
        assert_eq!(engine.event_count(), 2);

        let reopened = LogEngine::open(store).unwrap();
        assert_eq!(reopened.event_count(), 2);
        assert_eq!(reopened.get(b"a").unwrap(), None);
        assert_eq!(reopened.current_revision().unwrap(), Some(2));
    }

    #[test]
    fn conflicts_and_empty_batches_do_not_append() {
        let store = MemoryLogStore::default();
        let mut engine = LogEngine::open(store.clone()).unwrap();
        assert!(matches!(
            engine.write_batch_conditional(&[KvWriteOp::Delete { key: vec![] }], Some(7)),
            Ok(StorageCommitOutcome::Conflict { .. })
        ));
        engine.write_batch(&[]).unwrap();
        assert!(store.0.lock().unwrap().events.is_empty());
    }

    #[test]
    fn duplicate_append_preserves_original_event() {
        let mut store = MemoryLogStore::default();
        store
            .append_event(EventId::FIRST, b"original".to_vec())
            .unwrap();
        assert!(
            store
                .append_event(EventId::FIRST, b"replacement".to_vec())
                .is_err()
        );
        assert_eq!(
            store.read_event(EventId::FIRST).unwrap(),
            Some(b"original".to_vec())
        );
    }

    #[test]
    fn uncertain_append_poisons_instance_and_reopen_recovers() {
        let store = MemoryLogStore::default();
        store.0.lock().unwrap().fail_after_append = true;
        let mut engine = LogEngine::open(store.clone()).unwrap();
        assert!(engine.put(b"key".to_vec(), b"value".to_vec()).is_err());
        assert!(engine.get(b"key").is_err());
        drop(engine);

        store.0.lock().unwrap().fail_after_append = false;
        let reopened = LogEngine::open(store).unwrap();
        assert_eq!(reopened.get(b"key").unwrap(), Some(b"value".to_vec()));
    }

    #[test]
    fn replay_rejects_gaps_and_corruption() {
        let gap = MemoryLogStore::default();
        gap.0.lock().unwrap().events.insert(
            EventId::new(2).unwrap(),
            event::encode(
                EventId::new(2).unwrap(),
                &[KvWriteOp::Delete { key: vec![] }],
            )
            .unwrap(),
        );
        assert!(LogEngine::open(gap).is_err());

        let corrupt = MemoryLogStore::default();
        corrupt
            .0
            .lock()
            .unwrap()
            .events
            .insert(EventId::FIRST, b"not an event".to_vec());
        assert!(LogEngine::open(corrupt).is_err());
    }

    #[test]
    fn replay_rejects_missing_objects_and_duplicate_listings() {
        let missing = MemoryLogStore::default();
        missing.0.lock().unwrap().listed_ids = Some(vec![EventId::FIRST]);
        assert!(LogEngine::open(missing).is_err());

        let duplicate = MemoryLogStore::default();
        duplicate.0.lock().unwrap().events.insert(
            EventId::FIRST,
            event::encode(EventId::FIRST, &[KvWriteOp::Delete { key: vec![] }]).unwrap(),
        );
        duplicate.0.lock().unwrap().listed_ids = Some(vec![EventId::FIRST, EventId::FIRST]);
        assert!(LogEngine::open(duplicate).is_err());
    }
}
