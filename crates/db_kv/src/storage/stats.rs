//! Maintained collection row counts and index entry counts.
//!
//! [`EntityStore`] keeps one counter per collection (number of entity keys)
//! and per index (number of index entry keys) under the stats key space
//! (`0x05`, see [`keys`]), as big-endian `u64` values. A missing counter
//! means zero.
//!
//! Counters are updated in the same write transaction as the data: every
//! physical write of [`EntityStore`] compares the final state of each key
//! with its current state, so the created and removed entity and index keys
//! are known exactly (including those removed by `ClearCollection`,
//! `ClearIndex` and `ResetIndex`), and their net deltas are applied to the
//! counters before the transaction commits.
//!
//! Databases written before counters existed have data but no counters.
//! The meta entry `0x01 "stats"` ([`keys::stats_version_key`]) therefore
//! marks the counters as maintained: without it, reads report counts as
//! unknown (`None`) and writes leave counters alone. `prepare_open` calls
//! [`EntityStore::ensure_stats`], which backfills such databases once: it
//! counts keys through one read handle (streaming, without decoding
//! payloads) and writes all counters plus the marker in one write
//! transaction conditioned on the read revision. A concurrent write makes
//! that transaction conflict; the backfill is then deferred to the next
//! open and counts stay unknown (callers fall back to counting keys).
//!
//! Raw engine writes that bypass [`EntityStore`] (for example
//! [`EntityStore::put_raw`] or writes through the engine itself) do not
//! maintain counters.

use std::collections::BTreeMap;

use semantic_db_core::embedded::storage::StorageCommitOutcome;
use semantic_db_core::{DbError, StorageErrorKind};

use super::{EntityStore, KvEngine, KvReadTxn, KvWriteTxn};
use crate::keys;

/// Format version of the stats counters.
pub const STATS_VERSION: u32 = 1;

/// Outcome of [`EntityStore::ensure_stats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsBackfill {
    /// Counters were already maintained; nothing was written.
    UpToDate,
    /// Counters were computed and written.
    Backfilled {
        /// Collections with a non-zero row count.
        collections: usize,
        /// Indexes with a non-zero entry count.
        indexes: usize,
    },
    /// A concurrent write changed the database while counting; the backfill
    /// is retried on the next open.
    Deferred,
}

/// Net counter changes of one write transaction.
#[derive(Debug, Default)]
pub(crate) struct CounterDeltas(BTreeMap<Vec<u8>, i64>);

impl CounterDeltas {
    /// Record that `key` changes from existing (`existed`) to `exists`.
    pub(crate) fn record(&mut self, key: &[u8], existed: bool, exists: bool) {
        let delta = match (existed, exists) {
            (false, true) => 1,
            (true, false) => -1,
            _ => return,
        };
        if let Some(counter) = keys::counter_key_for(key) {
            *self.0.entry(counter).or_default() += delta;
        }
    }

    /// Apply the deltas to the counters, if counters are maintained.
    pub(crate) fn apply(self, txn: &mut dyn KvWriteTxn) -> Result<(), DbError> {
        let deltas = self
            .0
            .into_iter()
            .filter(|(_, delta)| *delta != 0)
            .collect::<Vec<_>>();
        if deltas.is_empty() || !is_maintained(txn.get(&keys::stats_version_key())?)? {
            return Ok(());
        }
        for (key, delta) in deltas {
            let current = decode_counter(txn.get(&key)?)?;
            write_counter(txn, &key, current.saturating_add_signed(delta))?;
        }
        Ok(())
    }
}

/// Read the counter at `counter_key` through `txn`, or `None` when counters
/// are not maintained.
pub(crate) fn read_counter(
    txn: &dyn KvReadTxn,
    counter_key: &[u8],
) -> Result<Option<u64>, DbError> {
    if !is_maintained(txn.get(&keys::stats_version_key())?)? {
        return Ok(None);
    }
    decode_counter(txn.get(counter_key)?).map(Some)
}

impl<E: KvEngine> EntityStore<E> {
    /// Make sure the stats counters are maintained, backfilling them for
    /// databases written before they existed; see the [module docs](self).
    pub fn ensure_stats(&mut self) -> Result<StatsBackfill, DbError> {
        if is_maintained(self.engine.get(&keys::stats_version_key())?)? {
            return Ok(StatsBackfill::UpToDate);
        }
        let (revision, counters) = {
            let txn = self.engine.begin_read()?;
            let mut counters = BTreeMap::<Vec<u8>, u64>::new();
            for tag in [keys::TAG_ENTITY, keys::TAG_INDEX] {
                for entry in txn.scan_prefix_stream(vec![tag])? {
                    let (key, _) = entry?;
                    if let Some(counter) = keys::counter_key_for(&key) {
                        *counters.entry(counter).or_default() += 1;
                    }
                }
            }
            (txn.revision(), counters)
        };

        let mut outcome = StatsBackfill::UpToDate;
        let committed = self.engine.write_with(revision, |txn| {
            if is_maintained(txn.get(&keys::stats_version_key())?)? {
                return Ok(());
            }
            // Drop counters of an earlier, interrupted maintenance.
            for (key, _) in txn.scan_prefix(&[keys::TAG_STATS])? {
                txn.delete(&key)?;
            }
            let mut collections = 0;
            let mut indexes = 0;
            for (key, count) in &counters {
                match key.get(1) {
                    Some(&keys::STATS_COLLECTION_ROWS) => collections += 1,
                    _ => indexes += 1,
                }
                write_counter(txn, key, *count)?;
            }
            txn.put(&keys::stats_version_key(), &STATS_VERSION.to_be_bytes())?;
            outcome = StatsBackfill::Backfilled {
                collections,
                indexes,
            };
            Ok(())
        })?;
        match committed {
            StorageCommitOutcome::Committed { .. } => Ok(outcome),
            StorageCommitOutcome::Conflict { .. } => Ok(StatsBackfill::Deferred),
        }
    }
}

fn is_maintained(marker: Option<Vec<u8>>) -> Result<bool, DbError> {
    let Some(marker) = marker else {
        return Ok(false);
    };
    let version = <[u8; 4]>::try_from(marker.as_slice())
        .map(u32::from_be_bytes)
        .map_err(|_| {
            DbError::Deserialization(format!(
                "invalid stats version entry of {} bytes",
                marker.len()
            ))
        })?;
    if version != STATS_VERSION {
        return Err(DbError::storage(
            StorageErrorKind::Unsupported,
            format!("unsupported stats version {version}; expected {STATS_VERSION}"),
        ));
    }
    Ok(true)
}

fn decode_counter(value: Option<Vec<u8>>) -> Result<u64, DbError> {
    let Some(value) = value else {
        return Ok(0);
    };
    <[u8; 8]>::try_from(value.as_slice())
        .map(u64::from_be_bytes)
        .map_err(|_| {
            DbError::Deserialization(format!("invalid stats counter of {} bytes", value.len()))
        })
}

fn write_counter(txn: &mut dyn KvWriteTxn, key: &[u8], count: u64) -> Result<(), DbError> {
    if count == 0 {
        txn.delete(key)
    } else {
        txn.put(key, &count.to_be_bytes())
    }
}
