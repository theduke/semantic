//! Consistent online backups of a redb database.
//!
//! redb has no online backup API, so a backup copies every key-value pair
//! of every engine table from one read transaction into a fresh database
//! with the same table layout. The meta table, including the engine's
//! revision counter, is copied like any other table, so the copy opens at
//! the revision of the captured state. Writes committed after the read
//! transaction began are not part of the copy, and writers are not blocked
//! while it is written.
//!
//! The copy is written to a temporary file next to the target in batches of
//! non-durable write transactions, made durable by a final commit, and only
//! then moved to the target path.

use std::path::{Path, PathBuf};

use redb::ReadableTable;
use semantic_db_core::embedded::{BackupSource, StorageBackup};
use semantic_db_core::{DbError, StorageErrorKind};

use crate::tables::{ReadTables, RedbTable};
use crate::{RedbReadTxn, storage_err};

/// Entries copied per write transaction.
const BATCH_ENTRIES: usize = 10_000;

impl RedbReadTxn {
    /// Write the state observed by this read handle to a new database at
    /// `path`, which must not exist.
    pub fn backup_to(&self, path: &Path) -> Result<StorageBackup, DbError> {
        if path.exists() {
            return Err(target_exists(path));
        }
        let partial = partial_path(path);
        let result = write_copy(&self.tables, &partial).and_then(|entries| {
            // Re-check: the target may have appeared while copying.
            if path.exists() {
                return Err(target_exists(path));
            }
            std::fs::rename(&partial, path).map_err(storage_err)?;
            Ok(entries)
        });
        match result {
            Ok(entries) => Ok(StorageBackup {
                revision: Some(self.revision),
                entries,
            }),
            Err(error) => {
                let _ = std::fs::remove_file(&partial);
                Err(error)
            }
        }
    }
}

impl BackupSource for RedbReadTxn {
    fn revision(&self) -> Option<u64> {
        Some(self.revision)
    }

    fn write_to(self: Box<Self>, path: &Path) -> Result<StorageBackup, DbError> {
        self.backup_to(path)
    }
}

/// Copy every table of `source` into a new database at `path`, returning
/// the number of copied entries.
fn write_copy(source: &ReadTables, path: &Path) -> Result<u64, DbError> {
    let target = redb::Database::create(path).map_err(storage_err)?;
    let mut copied = 0u64;
    for table in RedbTable::ALL {
        let mut entries = source.table(table)?.iter().map_err(storage_err)?;
        let mut done = false;
        while !done {
            let mut txn = target.begin_write().map_err(storage_err)?;
            txn.set_durability(redb::Durability::None);
            {
                let mut out = txn.open_table(table.definition()).map_err(storage_err)?;
                for _ in 0..BATCH_ENTRIES {
                    let Some(entry) = entries.next() else {
                        done = true;
                        break;
                    };
                    let (key, value) = entry.map_err(storage_err)?;
                    out.insert(key.value(), value.value())
                        .map_err(storage_err)?;
                    copied += 1;
                }
            }
            txn.commit().map_err(storage_err)?;
        }
    }
    // Persist the non-durable commits.
    let mut txn = target.begin_write().map_err(storage_err)?;
    txn.set_durability(redb::Durability::Immediate);
    txn.commit().map_err(storage_err)?;
    Ok(copied)
}

fn partial_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "backup".into());
    path.with_file_name(format!(".{name}.partial-{}", std::process::id()))
}

fn target_exists(path: &Path) -> DbError {
    DbError::storage(
        StorageErrorKind::InvalidState,
        format!("backup target {} already exists", path.display()),
    )
}
