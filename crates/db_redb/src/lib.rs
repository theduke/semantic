use redb::{ReadableTable, TableDefinition};
use semantic_data::schema::DbOpenMode;
use semantic_db_core::embedded::{
    EmbeddedBackend, EmbeddedDb, StorageCommitOutcome, StorageTransactionCapabilities,
};
use semantic_db_core::{DbConfig, DbError};
use semantic_db_kv::{
    BoxKvPrefixScan, EntityStore, KvEngine, KvReadTxn, KvWriteOp, KvWriteTxn, prefix_range_end,
};
use std::path::Path;
use std::sync::Arc;

const KV_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("kv");
const META_REV_KEY: &[u8] = b"__semantic/revision";

pub struct RedbKvEngine {
    db: Arc<redb::Database>,
}

impl std::fmt::Debug for RedbKvEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbKvEngine").finish_non_exhaustive()
    }
}

impl RedbKvEngine {
    pub fn open(path: impl AsRef<Path>, mode: DbOpenMode) -> std::result::Result<Self, DbError> {
        let path = path.as_ref();
        let db = match mode {
            DbOpenMode::OpenExisting => redb::Database::open(path).map_err(storage_err)?,
            DbOpenMode::AutoCreate => {
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).map_err(storage_err)?;
                    }
                }

                if path.exists() {
                    redb::Database::open(path).map_err(storage_err)?
                } else {
                    redb::Database::create(path).map_err(storage_err)?
                }
            }
        };
        {
            let read_txn = db.begin_read().map_err(storage_err)?;
            match read_txn.open_table(KV_TABLE) {
                Ok(_) => {}
                Err(redb::TableError::TableDoesNotExist(_)) => {
                    let write_txn = db.begin_write().map_err(storage_err)?;
                    let _ = write_txn.open_table(KV_TABLE).map_err(storage_err)?;
                    write_txn.commit().map_err(storage_err)?;
                }
                Err(err) => return Err(storage_err(err)),
            }
        }
        Ok(Self { db: Arc::new(db) })
    }
}

impl RedbKvEngine {
    /// Open a read handle backed by one redb read transaction.
    ///
    /// The handle observes the committed state at the time it was opened,
    /// independent of later commits.
    pub fn read_txn(&self) -> Result<RedbReadTxn, DbError> {
        let read_txn = self.db.begin_read().map_err(storage_err)?;
        let table = read_txn.open_table(KV_TABLE).map_err(storage_err)?;
        let revision = read_revision_table(&table)?;
        Ok(RedbReadTxn { table, revision })
    }
}

/// Snapshot read handle over one redb read transaction.
pub struct RedbReadTxn {
    table: redb::ReadOnlyTable<&'static [u8], &'static [u8]>,
    revision: Option<u64>,
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
        self.revision
    }

    fn is_snapshot(&self) -> bool {
        true
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self
            .table
            .get(key)
            .map_err(storage_err)?
            .map(|value| value.value().to_vec()))
    }

    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        let iter = match &end {
            Some(end) if end.as_slice() <= start.as_slice() => {
                return Ok(Box::new(std::iter::empty()));
            }
            Some(end) => self
                .table
                .range::<&[u8]>(start.as_slice()..end.as_slice())
                .map_err(storage_err)?,
            None => self
                .table
                .range::<&[u8]>(start.as_slice()..)
                .map_err(storage_err)?,
        };
        Ok(Box::new(iter.map(|item| {
            let (key, value) = item.map_err(storage_err)?;
            Ok((key.value().to_vec(), value.value().to_vec()))
        })))
    }
}

/// Mutable access to one redb write transaction's table.
struct RedbWriteTxn<'a, 'txn> {
    table: &'a mut redb::Table<'txn, &'static [u8], &'static [u8]>,
    changed: bool,
}

impl KvWriteTxn for RedbWriteTxn<'_, '_> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self
            .table
            .get(key)
            .map_err(storage_err)?
            .map(|value| value.value().to_vec()))
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        let iter = match prefix_range_end(prefix) {
            Some(end) => self
                .table
                .range::<&[u8]>(prefix..end.as_slice())
                .map_err(storage_err)?,
            None => self.table.range::<&[u8]>(prefix..).map_err(storage_err)?,
        };
        iter.map(|item| {
            let (key, value) = item.map_err(storage_err)?;
            Ok((key.value().to_vec(), value.value().to_vec()))
        })
        .collect()
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError> {
        self.table.insert(key, value).map_err(storage_err)?;
        self.changed = true;
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        if self.table.remove(key).map_err(storage_err)?.is_some() {
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

    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError> {
        self.read_txn()?.get(key)
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

    fn scan_prefix(&self, prefix: &[u8]) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.scan_prefix_stream(prefix.to_vec())?.collect()
    }

    fn scan_prefix_at_revision(
        &self,
        prefix: &[u8],
        _revision: u64,
    ) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
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
        let write_txn = self.db.begin_write().map_err(storage_err)?;
        let (actual, next) = {
            let mut table = write_txn.open_table(KV_TABLE).map_err(storage_err)?;
            let actual = read_revision_table(&table)?;
            if let Some(expected) = expected_revision
                && actual != Some(expected)
            {
                return Ok(StorageCommitOutcome::Conflict {
                    expected_revision: Some(expected),
                    actual_revision: actual,
                });
            }
            let mut txn = RedbWriteTxn {
                table: &mut table,
                changed: false,
            };
            // Dropping the uncommitted transaction on error aborts it.
            f(&mut txn)?;
            let next = if txn.changed {
                let revision = actual.unwrap_or(0).saturating_add(1);
                table
                    .insert(META_REV_KEY, revision.to_be_bytes().as_slice())
                    .map_err(storage_err)?;
                Some(revision)
            } else {
                None
            };
            (actual, next)
        };
        match next {
            Some(revision) => {
                write_txn.commit().map_err(storage_err)?;
                Ok(StorageCommitOutcome::Committed {
                    revision: Some(revision),
                })
            }
            None => {
                write_txn.abort().map_err(storage_err)?;
                Ok(StorageCommitOutcome::Committed { revision: actual })
            }
        }
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> std::result::Result<(), DbError> {
        self.write_with(None, |txn| apply_ops(txn, ops)).map(|_| ())
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        StorageTransactionCapabilities {
            conflict_detection: true,
            mvcc: false,
            snapshot_reads: false,
        }
    }

    fn current_revision(&self) -> std::result::Result<Option<u64>, DbError> {
        let read_txn = self.db.begin_read().map_err(storage_err)?;
        let table = read_txn.open_table(KV_TABLE).map_err(storage_err)?;
        read_revision_table(&table)
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> std::result::Result<StorageCommitOutcome, DbError> {
        self.write_with(expected_revision, |txn| apply_ops(txn, ops))
    }
}

fn storage_err(err: impl std::fmt::Display) -> DbError {
    DbError::Storage(err.to_string())
}

fn read_revision_table(
    table: &impl ReadableTable<&'static [u8], &'static [u8]>,
) -> std::result::Result<Option<u64>, DbError> {
    let Some(value) = table.get(META_REV_KEY).map_err(storage_err)? else {
        return Ok(Some(0));
    };
    let bytes = value.value();
    if bytes.len() != 8 {
        return Err(DbError::Storage(
            "invalid revision payload in redb metadata".to_string(),
        ));
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(bytes);
    Ok(Some(u64::from_be_bytes(buf)))
}

pub type RedbDatabase = EmbeddedDb<EntityStore<RedbKvEngine>>;
pub type RedbBackend = EmbeddedBackend<EntityStore<RedbKvEngine>>;

pub fn open_backend(
    path: impl AsRef<Path>,
    mode: DbOpenMode,
) -> std::result::Result<RedbBackend, DbError> {
    open_backend_with_config(path, mode, DbConfig::default())
}

pub fn open_backend_with_config(
    path: impl AsRef<Path>,
    mode: DbOpenMode,
    config: DbConfig,
) -> std::result::Result<RedbBackend, DbError> {
    let engine = RedbKvEngine::open(path, mode)?;
    let db = RedbDatabase::open_with_config(EntityStore::new(engine), config)?;
    Ok(RedbBackend::new(db))
}

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

    /// Entries of an indexed database re-encoded in the legacy textual layout.
    fn legacy_layout_entries() -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut db = semantic_db_kv::open_memory().unwrap();
        let items = db
            .create_collection("items", CollectionKind::Untyped)
            .unwrap();
        db.create_index("by_kind", items, "kind", false).unwrap();
        for (id, kind) in [("one", "music"), ("two", "video"), ("three", "music")] {
            let mut object = Object::new();
            object.insert("id", Value::String(id.into()));
            object.insert("kind", Value::String(kind.into()));
            db.insert("items", id, object).unwrap();
        }
        let path_indexes = db
            .catalog()
            .indexes()
            .filter(|(_, index)| {
                index.schema.kind == semantic_data::schema::IndexKind::PathEquality
            })
            .map(|(lid, _)| lid)
            .collect();
        let (_, store) = db.into_parts();
        semantic_db_kv::keys::legacy::downgrade_entries(
            store.scan_raw_prefix(&[]).unwrap(),
            &path_indexes,
        )
        .unwrap()
    }

    #[test]
    fn redb_legacy_layout_is_migrated_once_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let legacy_revision = {
            let mut engine = RedbKvEngine::open(&path, DbOpenMode::AutoCreate).unwrap();
            let ops = legacy_layout_entries()
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
