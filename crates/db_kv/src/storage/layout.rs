//! Versioned key layout of [`EntityStore`] and its on-open migration.
//!
//! The layout version is stored under the meta key `0x01 "format"`
//! ([`keys::layout_version_key`]) as a big-endian `u32`:
//!
//! - version 1: the legacy textual layout ([`keys::legacy`]). Legacy
//!   databases do not store a version; a database without the version key
//!   but with keys in the legacy entity (`c/`) or index (`i/`) spaces is
//!   version 1.
//! - version 2: the binary, order-preserving layout ([`keys`]).
//!
//! [`EntityStore::migrate_layout`] runs before the catalog is loaded (via
//! `EntityStorage::prepare_open`). It stamps new databases with the current
//! version and migrates legacy databases in one engine write transaction:
//! entity keys are rewritten in place (payloads are unchanged), and legacy
//! index entries and their format markers are dropped, so the database's
//! `index_needs_rebuild` check rebuilds every index under the new layout.
//!
//! A single transaction keeps the migration atomic and needs no resumable
//! progress marker: an interrupted migration leaves the legacy database
//! untouched and is simply retried on the next open. The cost is that the
//! write transaction holds all legacy entities (engines without native write
//! transactions buffer every rewritten entity in memory), which is
//! acceptable for embedded databases. Databases already at the current
//! version are checked with one read and are not written to.

use semantic_db_core::DbError;

use super::{EntityStore, KvEngine, KvWriteTxn};
use crate::keys::{self, legacy};

/// Layout version of the legacy textual key layout.
pub const LAYOUT_VERSION_LEGACY: u32 = 1;
/// Layout version of the binary key layout written by this crate.
pub const LAYOUT_VERSION_CURRENT: u32 = 2;

/// Outcome of [`EntityStore::migrate_layout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMigration {
    /// The database already uses the current layout; nothing was written.
    UpToDate,
    /// A database without legacy data was stamped with the current version.
    Initialized,
    /// A legacy database was migrated.
    Migrated {
        /// Entity keys rewritten to the current layout.
        entities: usize,
        /// Legacy index entries and format markers dropped.
        dropped_index_keys: usize,
    },
}

pub(crate) fn encode_layout_version(version: u32) -> Vec<u8> {
    version.to_be_bytes().to_vec()
}

pub(crate) fn decode_layout_version(bytes: &[u8]) -> Result<u32, DbError> {
    let bytes: [u8; 4] = bytes.try_into().map_err(|_| {
        DbError::Deserialization(format!(
            "invalid storage layout version entry of {} bytes",
            bytes.len()
        ))
    })?;
    Ok(u32::from_be_bytes(bytes))
}

/// Whether a database stamped with `version` must be migrated.
fn needs_migration(version: Option<u32>) -> Result<bool, DbError> {
    match version {
        Some(LAYOUT_VERSION_CURRENT) => Ok(false),
        None | Some(LAYOUT_VERSION_LEGACY) => Ok(true),
        Some(version) if version > LAYOUT_VERSION_CURRENT => Err(DbError::Storage(format!(
            "database storage layout version {version} is newer than the supported version \
             {LAYOUT_VERSION_CURRENT}; upgrade the application to open it"
        ))),
        Some(version) => Err(DbError::Storage(format!(
            "unknown database storage layout version {version}"
        ))),
    }
}

impl<E: KvEngine> EntityStore<E> {
    /// Stored layout version, or `None` when the database is not stamped
    /// (a new or a legacy database).
    pub fn layout_version(&self) -> Result<Option<u32>, DbError> {
        self.engine
            .get(&keys::layout_version_key())?
            .map(|bytes| decode_layout_version(&bytes))
            .transpose()
    }

    /// Bring the database to the current key layout; see the
    /// [module docs](self).
    pub fn migrate_layout(&mut self) -> Result<LayoutMigration, DbError> {
        if !needs_migration(self.layout_version()?)? {
            return Ok(LayoutMigration::UpToDate);
        }
        let mut outcome = LayoutMigration::UpToDate;
        self.engine.write_with(None, |txn| {
            outcome = migrate_in(txn)?;
            Ok(())
        })?;
        Ok(outcome)
    }
}

fn migrate_in(txn: &mut dyn KvWriteTxn) -> Result<LayoutMigration, DbError> {
    let version_key = keys::layout_version_key();
    let version = txn
        .get(&version_key)?
        .map(|bytes| decode_layout_version(&bytes))
        .transpose()?;
    // Re-check inside the write transaction in case another process
    // migrated the database concurrently.
    if !needs_migration(version)? {
        return Ok(LayoutMigration::UpToDate);
    }

    let legacy_entities = txn.scan_prefix(legacy::ENTITY_SPACE)?;
    let legacy_indexes = txn.scan_prefix(legacy::INDEX_SPACE)?;
    let outcome = if legacy_entities.is_empty() && legacy_indexes.is_empty() {
        LayoutMigration::Initialized
    } else {
        LayoutMigration::Migrated {
            entities: legacy_entities.len(),
            dropped_index_keys: legacy_indexes.len(),
        }
    };
    for (key, payload) in legacy_entities {
        let (collection, id) = legacy::parse_entity_key(&key).ok_or_else(|| {
            DbError::Storage(format!(
                "cannot migrate unrecognized legacy entity key {:?}",
                String::from_utf8_lossy(&key)
            ))
        })?;
        txn.put(&keys::entity_key(collection, id), &payload)?;
        txn.delete(&key)?;
    }
    for (key, _) in legacy_indexes {
        txn.delete(&key)?;
    }
    txn.put(&version_key, &encode_layout_version(LAYOUT_VERSION_CURRENT))?;
    Ok(outcome)
}
