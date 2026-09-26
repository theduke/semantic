//! redb storage engine for the embedded Semantic database.
//!
//! [`RedbKvEngine`] implements [`KvEngine`] on a redb database file. The
//! flat logical key space is stored in one redb table per key space; see
//! [`tables`] for the layout and the migration of the legacy single-table
//! layout. [`RedbOptions`] configures the page cache and commit durability.

use redb::{ReadableTable, ReadableTableMetadata};
use semantic_data::schema::DbOpenMode;
use semantic_db_core::embedded::{
    EmbeddedBackend, EmbeddedDb, StorageCommitOutcome, StorageTableStats,
    StorageTransactionCapabilities,
};
use semantic_db_core::{DbConfig, DbError};
use semantic_db_kv::{
    BoxKvPrefixScan, EntityStore, KvEngine, KvEngineStats, KvMaintenance, KvReadTxn, KvWriteOp,
    KvWriteTxn, prefix_range_end,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod options;
pub mod tables;

pub use options::{DEFAULT_CACHE_SIZE, RedbDurability, RedbOptions};
pub use tables::RedbTable;

use tables::{ReadTables, WriteTables, read_revision, split_range, write_revision};

pub struct RedbKvEngine {
    db: redb::Database,
    path: PathBuf,
    options: RedbOptions,
    /// Whether commits since the last durable commit may not be persisted.
    unpersisted: bool,
    /// Held by every read handle and scan, so maintenance can refuse to run
    /// while redb read transactions are alive.
    readers: Arc<()>,
}

impl std::fmt::Debug for RedbKvEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbKvEngine")
            .field("path", &self.path)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl RedbKvEngine {
    /// Open the database at `path` with the default [`RedbOptions`].
    pub fn open(path: impl AsRef<Path>, mode: DbOpenMode) -> Result<Self, DbError> {
        Self::open_with_options(path, mode, RedbOptions::default())
    }

    /// Open the database at `path`.
    ///
    /// Creates missing engine tables and migrates the legacy single-table
    /// layout (see [`tables`]) before returning.
    pub fn open_with_options(
        path: impl AsRef<Path>,
        mode: DbOpenMode,
        options: RedbOptions,
    ) -> Result<Self, DbError> {
        let path = path.as_ref();
        let builder = options.builder();
        let db = match mode {
            DbOpenMode::OpenExisting => builder.open(path).map_err(storage_err)?,
            DbOpenMode::AutoCreate => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent).map_err(storage_err)?;
                }

                if path.exists() {
                    builder.open(path).map_err(storage_err)?
                } else {
                    builder.create(path).map_err(storage_err)?
                }
            }
        };
        tables::prepare_tables(&db)?;
        Ok(Self {
            db,
            path: path.to_path_buf(),
            options,
            unpersisted: false,
            readers: Arc::new(()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn options(&self) -> &RedbOptions {
        &self.options
    }

    /// Open a read handle backed by one redb read transaction.
    ///
    /// The handle observes the committed state at the time it was opened,
    /// independent of later commits.
    pub fn read_txn(&self) -> Result<RedbReadTxn, DbError> {
        let tables = ReadTables::new(self.db.begin_read().map_err(storage_err)?);
        let revision = read_revision(tables.table(RedbTable::Meta)?)?;
        Ok(RedbReadTxn {
            tables,
            revision,
            reader: Arc::clone(&self.readers),
        })
    }

    /// Persist all commits made with a durability below
    /// [`RedbDurability::Immediate`]. Also runs when the engine is dropped.
    pub fn flush(&mut self) -> Result<(), DbError> {
        if !self.unpersisted {
            return Ok(());
        }
        let mut txn = self.db.begin_write().map_err(storage_err)?;
        txn.set_durability(redb::Durability::Immediate);
        txn.commit().map_err(storage_err)?;
        self.unpersisted = false;
        Ok(())
    }

    fn ensure_no_readers(&self, operation: &str) -> Result<(), DbError> {
        if Arc::strong_count(&self.readers) > 1 {
            return Err(DbError::Storage(format!(
                "cannot run {operation} while read transactions are open"
            )));
        }
        Ok(())
    }
}

impl Drop for RedbKvEngine {
    fn drop(&mut self) {
        // redb also commits durably on drop; flushing here keeps the
        // guarantee independent of that implementation detail.
        let _ = self.flush();
    }
}

/// Snapshot read handle over one redb read transaction.
pub struct RedbReadTxn {
    tables: ReadTables,
    revision: u64,
    /// Registers the handle with the engine's live-reader count.
    reader: Arc<()>,
}

impl std::fmt::Debug for RedbReadTxn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbReadTxn")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

impl KvReadTxn for RedbReadTxn {
    fn revision(&self) -> Option<u64> {
        Some(self.revision)
    }

    fn is_snapshot(&self) -> bool {
        true
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self
            .tables
            .table(RedbTable::for_key(key))?
            .get(key)
            .map_err(storage_err)?
            .map(|value| value.value().to_vec()))
    }

    /// Scans spanning several tables concatenate the per-table scans in key
    /// order (see [`tables::split_range`]).
    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        let mut ranges = Vec::new();
        for (table, start, end) in split_range(&start, end.as_deref()) {
            let table = self.tables.table(table)?;
            let range = match &end {
                Some(end) => table.range::<&[u8]>(start.as_slice()..end.as_slice()),
                None => table.range::<&[u8]>(start.as_slice()..),
            };
            ranges.push(range.map_err(storage_err)?);
        }
        // The scan outlives this handle; it keeps the reader registration
        // alive until it is dropped.
        let reader = Arc::clone(&self.reader);
        Ok(Box::new(ranges.into_iter().flatten().map(move |item| {
            let _registered = &reader;
            let (key, value) = item.map_err(storage_err)?;
            Ok((key.value().to_vec(), value.value().to_vec()))
        })))
    }
}

/// Mutable access to the tables of one redb write transaction.
struct RedbWriteTxn<'a, 'txn> {
    tables: &'a mut WriteTables<'txn>,
    changed: bool,
}

impl KvWriteTxn for RedbWriteTxn<'_, '_> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self
            .tables
            .table(RedbTable::for_key(key))
            .get(key)
            .map_err(storage_err)?
            .map(|value| value.value().to_vec()))
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        let end = prefix_range_end(prefix);
        let mut entries = Vec::new();
        for (table, start, end) in split_range(prefix, end.as_deref()) {
            let table = self.tables.table(table);
            let range = match &end {
                Some(end) => table.range::<&[u8]>(start.as_slice()..end.as_slice()),
                None => table.range::<&[u8]>(start.as_slice()..),
            };
            for item in range.map_err(storage_err)? {
                let (key, value) = item.map_err(storage_err)?;
                entries.push((key.value().to_vec(), value.value().to_vec()));
            }
        }
        Ok(entries)
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError> {
        self.tables
            .table_mut(RedbTable::for_key(key))
            .insert(key, value)
            .map_err(storage_err)?;
        self.changed = true;
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        if self
            .tables
            .table_mut(RedbTable::for_key(key))
            .remove(key)
            .map_err(storage_err)?
            .is_some()
        {
            self.changed = true;
        }
        Ok(())
    }
}

fn apply_ops(txn: &mut dyn KvWriteTxn, ops: &[KvWriteOp]) -> Result<(), DbError> {
    for op in ops {
        match op {
            KvWriteOp::Put { key, value } => txn.put(key, value)?,
            KvWriteOp::Delete { key } => txn.delete(key)?,
        }
    }
    Ok(())
}

impl KvEngine for RedbKvEngine {
    type PrefixScan = BoxKvPrefixScan;

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        self.read_txn()?.get(key)
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), DbError> {
        self.write_batch(&[KvWriteOp::Put { key, value }])
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.write_batch(&[KvWriteOp::Delete { key: key.to_vec() }])
    }

    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<Self::PrefixScan, DbError> {
        self.read_txn()?.scan_prefix_stream(prefix)
    }

    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        self.read_txn()?.scan_range_stream(start, end)
    }

    fn begin_read(&self) -> Result<Box<dyn KvReadTxn + '_>, DbError> {
        Ok(Box::new(self.read_txn()?))
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.scan_prefix_stream(prefix.to_vec())?.collect()
    }

    fn scan_prefix_at_revision(
        &self,
        prefix: &[u8],
        _revision: u64,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.scan_prefix(prefix)
    }

    fn write_with<F>(
        &mut self,
        expected_revision: Option<u64>,
        f: F,
    ) -> Result<StorageCommitOutcome, DbError>
    where
        F: FnOnce(&mut dyn KvWriteTxn) -> Result<(), DbError>,
    {
        let mut write_txn = self.db.begin_write().map_err(storage_err)?;
        write_txn.set_durability(self.options.durability.into());
        write_txn.set_quick_repair(self.options.quick_repair);
        let (actual, next) = {
            let mut tables = WriteTables::open(&write_txn)?;
            let actual = read_revision(tables.table(RedbTable::Meta))?;
            if let Some(expected) = expected_revision
                && actual != expected
            {
                return Ok(StorageCommitOutcome::Conflict {
                    expected_revision: Some(expected),
                    actual_revision: Some(actual),
                });
            }
            let mut txn = RedbWriteTxn {
                tables: &mut tables,
                changed: false,
            };
            // Dropping the uncommitted transaction on error aborts it.
            f(&mut txn)?;
            let next = if txn.changed {
                let revision = actual.saturating_add(1);
                write_revision(tables.table_mut(RedbTable::Meta), revision)?;
                Some(revision)
            } else {
                None
            };
            (actual, next)
        };
        match next {
            Some(revision) => {
                write_txn.commit().map_err(storage_err)?;
                if self.options.durability != RedbDurability::Immediate {
                    self.unpersisted = true;
                }
                Ok(StorageCommitOutcome::Committed {
                    revision: Some(revision),
                })
            }
            None => {
                write_txn.abort().map_err(storage_err)?;
                Ok(StorageCommitOutcome::Committed {
                    revision: Some(actual),
                })
            }
        }
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> Result<(), DbError> {
        self.write_with(None, |txn| apply_ops(txn, ops)).map(|_| ())
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        StorageTransactionCapabilities {
            conflict_detection: true,
            mvcc: false,
            snapshot_reads: false,
        }
    }

    fn current_revision(&self) -> Result<Option<u64>, DbError> {
        Ok(self.read_txn()?.revision())
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        self.write_with(expected_revision, |txn| apply_ops(txn, ops))
    }
}

impl KvMaintenance for RedbKvEngine {
    /// Compact the database file (see [`redb::Database::compact`]).
    ///
    /// Fails while read transactions or scans are open.
    fn compact(&mut self) -> Result<bool, DbError> {
        self.ensure_no_readers("compaction")?;
        self.flush()?;
        self.db.compact().map_err(storage_err)
    }

    /// Check and repair the database file (see
    /// [`redb::Database::check_integrity`]).
    ///
    /// Fails while read transactions or scans are open.
    fn check_integrity(&mut self) -> Result<bool, DbError> {
        self.ensure_no_readers("integrity checks")?;
        self.flush()?;
        self.db.check_integrity().map_err(storage_err)
    }

    /// Entry counts of the engine tables (physical entries, including the
    /// revision counter in `meta`), redb page statistics, and the file size.
    ///
    /// Reading the page statistics briefly opens a write transaction.
    fn stats(&self) -> Result<KvEngineStats, DbError> {
        let read = ReadTables::new(self.db.begin_read().map_err(storage_err)?);
        let mut tables = Vec::with_capacity(RedbTable::ALL.len());
        for table in RedbTable::ALL {
            tables.push(StorageTableStats {
                name: table.name().to_string(),
                entries: read.table(table)?.len().map_err(storage_err)?,
            });
        }
        drop(read);
        let db_stats = {
            let txn = self.db.begin_write().map_err(storage_err)?;
            let stats = txn.stats().map_err(storage_err)?;
            txn.abort().map_err(storage_err)?;
            stats
        };
        let file_size = std::fs::metadata(&self.path).map_err(storage_err)?.len();
        Ok(KvEngineStats {
            file_size_bytes: Some(file_size),
            entries: Some(tables.iter().map(|table| table.entries).sum()),
            tables,
            allocated_bytes: Some(db_stats.allocated_pages() * db_stats.page_size() as u64),
            stored_bytes: Some(db_stats.stored_bytes()),
            fragmented_bytes: Some(db_stats.fragmented_bytes()),
        })
    }
}

pub(crate) fn storage_err(err: impl std::fmt::Display) -> DbError {
    DbError::Storage(err.to_string())
}

pub type RedbDatabase = EmbeddedDb<EntityStore<RedbKvEngine>>;
pub type RedbBackend = EmbeddedBackend<EntityStore<RedbKvEngine>>;

pub fn open_backend(path: impl AsRef<Path>, mode: DbOpenMode) -> Result<RedbBackend, DbError> {
    open_backend_with_config(path, mode, DbConfig::default())
}

pub fn open_backend_with_config(
    path: impl AsRef<Path>,
    mode: DbOpenMode,
    config: DbConfig,
) -> Result<RedbBackend, DbError> {
    open_backend_with_options(path, mode, RedbOptions::default(), config)
}

pub fn open_backend_with_options(
    path: impl AsRef<Path>,
    mode: DbOpenMode,
    options: RedbOptions,
    config: DbConfig,
) -> Result<RedbBackend, DbError> {
    let engine = RedbKvEngine::open_with_options(path, mode, options)?;
    let db = RedbDatabase::open_with_config(EntityStore::new(engine), config)?;
    Ok(RedbBackend::new(db))
}

#[cfg(test)]
mod engine_tests;

#[cfg(test)]
mod tests {
    use semantic_data::query::BinaryOp;
    use semantic_data::value::{FieldPath, Object, Value};
    use semantic_db_core::catalog::CollectionKind;
    use semantic_db_core::{Db, Expr, Operand, SelectQuery};

    use semantic_db_core::DbError;
    use semantic_db_core::catalog::LocalCollectionId;
    use semantic_db_core::embedded::{
        EntityStorage, StorageCommitOutcome, StorageWriteOp, StoredEntity, StoredEntityKind,
    };
    use semantic_db_kv::{BoxKvPrefixScan, KvEngine, KvReadTxn, KvWriteOp};

    use super::{DbOpenMode, RedbDatabase, RedbKvEngine, open_backend};

    fn put(key: &[u8], value: &[u8]) -> KvWriteOp {
        KvWriteOp::Put {
            key: key.to_vec(),
            value: value.to_vec(),
        }
    }

    fn keys(scan: BoxKvPrefixScan) -> Vec<Vec<u8>> {
        scan.map(|item| item.unwrap().0).collect()
    }

    fn seeded_engine(dir: &tempfile::TempDir) -> RedbKvEngine {
        let mut engine = RedbKvEngine::open(dir.path().join("kv"), DbOpenMode::AutoCreate).unwrap();
        engine
            .write_batch(&[
                put(b"a", b"1"),
                put(b"b", b"2"),
                put(b"b/1", b"3"),
                put(b"c", b"4"),
            ])
            .unwrap();
        engine
    }

    #[test]
    fn redb_read_txn_is_isolated_from_later_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = seeded_engine(&dir);
        let revision = engine.current_revision().unwrap();
        let read = engine.read_txn().unwrap();
        engine
            .write_batch(&[put(b"a", b"changed"), put(b"b/2", b"new")])
            .unwrap();
        engine.delete(b"c").unwrap();

        assert!(read.is_snapshot());
        assert_eq!(read.revision(), revision);
        assert_eq!(read.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(read.get(b"c").unwrap(), Some(b"4".to_vec()));
        assert_eq!(
            keys(read.scan_prefix_stream(b"b".to_vec()).unwrap()),
            vec![b"b".to_vec(), b"b/1".to_vec()]
        );

        let fresh = engine.begin_read().unwrap();
        assert_eq!(fresh.get(b"a").unwrap(), Some(b"changed".to_vec()));
        assert_eq!(fresh.get(b"c").unwrap(), None);
        assert_eq!(fresh.revision(), engine.current_revision().unwrap());
        assert_ne!(fresh.revision(), revision);
    }

    #[test]
    fn redb_range_scans_respect_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let engine = seeded_engine(&dir);
        let read = engine.begin_read().unwrap();
        assert_eq!(
            keys(
                read.scan_range_stream(b"b".to_vec(), Some(b"c".to_vec()))
                    .unwrap()
            ),
            vec![b"b".to_vec(), b"b/1".to_vec()]
        );
        assert_eq!(
            keys(
                read.scan_range_stream(b"b/".to_vec(), Some(b"c/".to_vec()))
                    .unwrap()
            ),
            vec![b"b/1".to_vec(), b"c".to_vec()]
        );
        assert!(
            keys(
                read.scan_range_stream(b"c".to_vec(), Some(b"b".to_vec()))
                    .unwrap()
            )
            .is_empty()
        );
        assert_eq!(
            keys(
                engine
                    .scan_range_stream(b"a/".to_vec(), Some(b"d".to_vec()))
                    .unwrap()
            ),
            vec![b"b".to_vec(), b"b/1".to_vec(), b"c".to_vec()]
        );
    }

    #[test]
    fn redb_write_txn_skips_no_ops_and_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = seeded_engine(&dir);
        let revision = engine.current_revision().unwrap();

        let outcome = engine
            .write_with(revision, |txn| {
                assert_eq!(txn.get(b"a")?, Some(b"1".to_vec()));
                txn.delete(b"missing")
            })
            .unwrap();
        assert_eq!(outcome, StorageCommitOutcome::Committed { revision });
        engine.write_batch(&[]).unwrap();
        assert_eq!(engine.current_revision().unwrap(), revision);

        assert!(
            engine
                .write_with(revision, |txn| {
                    txn.put(b"a", b"lost")?;
                    Err(DbError::Storage("abort".into()))
                })
                .is_err()
        );
        assert_eq!(engine.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(engine.current_revision().unwrap(), revision);

        let outcome = engine
            .write_with(revision, |txn| {
                txn.put(b"b/2", b"x")?;
                assert_eq!(
                    txn.scan_prefix(b"b/")?,
                    vec![
                        (b"b/1".to_vec(), b"3".to_vec()),
                        (b"b/2".to_vec(), b"x".to_vec())
                    ]
                );
                Ok(())
            })
            .unwrap();
        let next = revision.map(|revision| revision + 1);
        assert_eq!(outcome, StorageCommitOutcome::Committed { revision: next });
        assert_eq!(
            engine.write_with(revision, |_| Ok(())).unwrap(),
            StorageCommitOutcome::Conflict {
                expected_revision: revision,
                actual_revision: next,
            }
        );
    }

    #[test]
    fn redb_unchanged_entity_batch_preserves_revision() {
        let dir = tempfile::tempdir().unwrap();
        let engine = RedbKvEngine::open(dir.path().join("kv"), DbOpenMode::AutoCreate).unwrap();
        let mut store = semantic_db_kv::EntityStore::new(engine);
        let mut object = Object::new();
        object.insert("id", Value::String("one".into()));
        let ops = [StorageWriteOp::PutEntity(StoredEntity {
            collection: 1,
            kind: StoredEntityKind::Untyped,
            id: "one".into(),
            object,
        })];
        let revision = store.current_revision().unwrap();
        let StorageCommitOutcome::Committed { revision: written } =
            store.apply_batch_conditional(&ops, revision).unwrap()
        else {
            panic!("unexpected conflict");
        };
        assert_ne!(written, revision);
        assert_eq!(
            store.apply_batch_conditional(&ops, written).unwrap(),
            StorageCommitOutcome::Committed { revision: written }
        );
        assert_eq!(store.current_revision().unwrap(), written);
        let snapshot = store.snapshot().unwrap();
        assert!(snapshot.is_consistent());
        assert_eq!(snapshot.revision().unwrap(), written);
        assert!(
            snapshot
                .get_entity(LocalCollectionId(1), "one")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn redb_reopen_and_unchanged_package_preserve_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let package = semantic_data::filestore::package();
        let revision;
        let expected_catalog;
        {
            let engine = RedbKvEngine::open(&path, DbOpenMode::AutoCreate).unwrap();
            let mut db = RedbDatabase::open(semantic_db_kv::EntityStore::new(engine)).unwrap();
            db.upsert_package(package.clone()).unwrap();
            expected_catalog = semantic_db_core::catalog::Catalog::from_storage_snapshot(
                db.catalog().to_storage_snapshot(),
            )
            .unwrap()
            .to_storage_snapshot();
            let (_, storage) = db.into_parts();
            revision = storage.current_revision().unwrap();
        }
        let engine = RedbKvEngine::open(&path, DbOpenMode::OpenExisting).unwrap();
        let mut db = RedbDatabase::open(semantic_db_kv::EntityStore::new(engine)).unwrap();
        assert_eq!(db.catalog().to_storage_snapshot(), expected_catalog);
        assert!(
            db.upsert_package(package)
                .unwrap()
                .executed_migrations
                .is_empty()
        );
        // Typedef metadata is persisted independently of its class projection.
        assert_eq!(
            db.catalog()
                .type_def_by_name("semantic:filestore:file")
                .unwrap()
                .type_def
                .module
                .as_deref(),
            Some("filestore")
        );
        let (_, storage) = db.into_parts();
        assert_eq!(storage.current_revision().unwrap(), revision);
    }

    #[test]
    fn redb_legacy_layout_is_migrated_once_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let legacy_revision = {
            let mut engine = RedbKvEngine::open(&path, DbOpenMode::AutoCreate).unwrap();
            let ops = crate::engine_tests::legacy_layout_entries()
                .into_iter()
                .map(|(key, value)| KvWriteOp::Put { key, value })
                .collect::<Vec<_>>();
            engine.write_batch(&ops).unwrap();
            engine.current_revision().unwrap()
        };

        let mut revisions = Vec::new();
        for _ in 0..3 {
            let engine = RedbKvEngine::open(&path, DbOpenMode::OpenExisting).unwrap();
            let db = RedbDatabase::open(semantic_db_kv::EntityStore::new(engine)).unwrap();
            assert_eq!(
                db.get("items", "two").unwrap().unwrap().object.get("kind"),
                Some(&Value::String("video".into()))
            );
            let rows = db
                .select(
                    SelectQuery::new()
                        .with_collection("items")
                        .with_predicate(eq_predicate("kind", "music")),
                )
                .unwrap();
            let mut ids = rows
                .iter()
                .map(|row| row.get("id").cloned().unwrap())
                .collect::<Vec<_>>();
            ids.sort();
            assert_eq!(
                ids,
                [Value::String("one".into()), Value::String("three".into())]
            );
            let catalog = db.catalog();
            let items = catalog.collection_by_name("items").unwrap().lid;
            let by_kind = catalog.find_equality_index(items, "kind").unwrap().lid;
            let (_, store) = db.into_parts();
            assert_eq!(
                store.layout_version().unwrap(),
                Some(semantic_db_kv::LAYOUT_VERSION_CURRENT)
            );
            assert!(store.scan_raw_prefix(b"c/").unwrap().is_empty());
            assert!(store.scan_raw_prefix(b"i/").unwrap().is_empty());
            assert!(!store.index_needs_rebuild(by_kind).unwrap());
            assert_eq!(
                store
                    .scan_index_value(by_kind, None, &Value::String("music".into()))
                    .unwrap(),
                ["one", "three"]
            );
            revisions.push(store.current_revision().unwrap());
        }
        // The first open migrates; later opens do not write.
        assert_ne!(revisions[0], legacy_revision);
        assert_eq!(revisions[1], revisions[0]);
        assert_eq!(revisions[2], revisions[0]);
    }

    #[test]
    fn redb_backend_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");

        {
            let engine = RedbKvEngine::open(&path, DbOpenMode::AutoCreate).unwrap();
            let mut db = RedbDatabase::new(semantic_db_kv::EntityStore::new(engine));
            db.create_collection("items", CollectionKind::Polymorphic)
                .unwrap();

            let mut obj = Object::new();
            obj.insert("id", Value::String("id1".into()));
            obj.insert("name", Value::String("n".into()));
            db.insert("items", "id1", obj).unwrap();
        }

        {
            let engine = RedbKvEngine::open(path, DbOpenMode::OpenExisting).unwrap();
            let mut db = RedbDatabase::new(semantic_db_kv::EntityStore::new(engine));
            db.create_collection("items", CollectionKind::Polymorphic)
                .unwrap();
            let out = db
                .select(SelectQuery::new().with_collection("items"))
                .unwrap();
            assert_eq!(out.len(), 1);
            // Polymorphic collections canonicalize `name` to the core `semantic:name`
            // attribute, and select returns qualified field names by default.
            assert_eq!(
                out[0].get("semantic:name"),
                Some(&Value::String("n".into()))
            );
        }
    }

    #[test]
    fn redb_reopen_preserves_builtin_id_and_type_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let expected_indexes;

        {
            let engine = RedbKvEngine::open(&path, DbOpenMode::AutoCreate).unwrap();
            let mut db = RedbDatabase::open(semantic_db_kv::EntityStore::new(engine)).unwrap();
            db.create_collection("items", CollectionKind::Polymorphic)
                .unwrap();

            let mut first = Object::new();
            first.insert("id", Value::String("item1".into()));
            first.insert("type", Value::String("semantic:test:item".into()));
            first.insert("name", Value::String("first".into()));
            db.insert("items", "item1", first).unwrap();

            let mut second = Object::new();
            second.insert("id", Value::String("item2".into()));
            second.insert("type", Value::String("semantic:test:other".into()));
            second.insert("name", Value::String("second".into()));
            db.insert("items", "item2", second).unwrap();
            expected_indexes = db
                .catalog()
                .indexes()
                .map(|(lid, index)| (lid, index.clone()))
                .collect::<Vec<_>>();
        }

        {
            let engine = RedbKvEngine::open(&path, DbOpenMode::OpenExisting).unwrap();
            let db = RedbDatabase::open(semantic_db_kv::EntityStore::new(engine)).unwrap();
            let catalog = db.catalog();
            assert_eq!(catalog.indexes().count(), expected_indexes.len());
            for (lid, expected) in &expected_indexes {
                let index = catalog.index_by_lid(*lid).unwrap();
                assert_eq!(index.lid, *lid);
                assert_eq!(index.collection, expected.collection);
                assert_eq!(index.schema.id, expected.schema.id);
            }

            let by_id = db
                .select(
                    SelectQuery::new()
                        .with_collection("items")
                        .with_predicate(eq_predicate("id", "item1")),
                )
                .unwrap();
            assert_eq!(by_id.len(), 1);
            assert_eq!(
                by_id[0].get("semantic:name"),
                Some(&Value::String("first".into()))
            );

            let by_type = db
                .select(
                    SelectQuery::new()
                        .with_collection("items")
                        .with_predicate(eq_predicate("type", "semantic:test:item")),
                )
                .unwrap();
            assert_eq!(by_type.len(), 1);
            assert_eq!(by_type[0].get("id"), Some(&Value::String("item1".into())));
        }
    }

    fn eq_predicate(field: &str, value: &str) -> Expr {
        Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                field,
            ])))),
            right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                value.to_string(),
            )))),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_redb_backend_testsuite() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open_backend(dir.path().join("db"), DbOpenMode::AutoCreate).unwrap();
        let db = Db::new(backend);

        semantic_db_test::suite::test_db(&db).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn recursive_validation_backend_parity_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("validation");
        let db = Db::new(open_backend(&path, DbOpenMode::AutoCreate).unwrap());
        semantic_db_test::suite::test_validation(&db).await;
        drop(db);
        let db = Db::new(open_backend(&path, DbOpenMode::AutoCreate).unwrap());
        assert!(matches!(
            db.delete(None::<&str>, "target").await.unwrap_err(),
            semantic_db_core::DbError::Validation(_)
        ));
    }
}
