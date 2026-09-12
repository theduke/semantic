use redb::{ReadableTable, TableDefinition};
use semantic_data::schema::DbOpenMode;
use semantic_db_core::embedded::{
    EmbeddedBackend, EmbeddedDb, StorageCommitOutcome, StorageTransactionCapabilities,
};
use semantic_db_core::{DbConfig, DbError};
use semantic_db_kv::{BoxKvPrefixScan, EntityStore, KvEngine, KvWriteOp};
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
            let write_txn = db.begin_write().map_err(storage_err)?;
            let _ = write_txn.open_table(KV_TABLE).map_err(storage_err)?;
            write_txn.commit().map_err(storage_err)?;
        }
        Ok(Self { db: Arc::new(db) })
    }
}

impl KvEngine for RedbKvEngine {
    type PrefixScan = BoxKvPrefixScan;

    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, DbError> {
        let read_txn = self.db.begin_read().map_err(storage_err)?;
        let table = read_txn.open_table(KV_TABLE).map_err(storage_err)?;
        // TODO: prevent cloning?
        let value = table
            .get(key)
            .map_err(storage_err)?
            .map(|v| v.value().to_vec());
        Ok(value)
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> std::result::Result<(), DbError> {
        let write_txn = self.db.begin_write().map_err(storage_err)?;
        {
            let mut table = write_txn.open_table(KV_TABLE).map_err(storage_err)?;
            table
                .insert(key.as_slice(), value.as_slice())
                .map_err(storage_err)?;
        }
        write_txn.commit().map_err(storage_err)?;
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> std::result::Result<(), DbError> {
        let write_txn = self.db.begin_write().map_err(storage_err)?;
        {
            let mut table = write_txn.open_table(KV_TABLE).map_err(storage_err)?;
            let _ = table.remove(key).map_err(storage_err)?;
        }
        write_txn.commit().map_err(storage_err)?;
        Ok(())
    }

    fn scan_prefix_stream(
        &self,
        prefix: Vec<u8>,
    ) -> std::result::Result<Self::PrefixScan, DbError> {
        let read_txn = self.db.begin_read().map_err(storage_err)?;
        let table = read_txn.open_table(KV_TABLE).map_err(storage_err)?;
        let iter = if let Some(end) = prefix_range_end(&prefix) {
            table
                .range::<&[u8]>(prefix.as_slice()..end.as_slice())
                .map_err(storage_err)?
        } else {
            table
                .range::<&[u8]>(prefix.as_slice()..)
                .map_err(storage_err)?
        };

        Ok(Box::new(iter.map(move |item| {
            let (key_guard, value_guard) = item.map_err(storage_err)?;
            let key = key_guard.value();
            if !key.starts_with(&prefix) {
                return Err(DbError::Storage(
                    "redb prefix scan returned key outside requested range".to_string(),
                ));
            }
            Ok((key.to_vec(), value_guard.value().to_vec()))
        })))
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

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> std::result::Result<(), DbError> {
        let write_txn = self.db.begin_write().map_err(storage_err)?;
        {
            let mut table = write_txn.open_table(KV_TABLE).map_err(storage_err)?;
            for op in ops {
                match op {
                    KvWriteOp::Put { key, value } => {
                        table
                            .insert(key.as_slice(), value.as_slice())
                            .map_err(storage_err)?;
                    }
                    KvWriteOp::Delete { key } => {
                        let _ = table.remove(key.as_slice()).map_err(storage_err)?;
                    }
                }
            }
            let revision = read_revision_table(&table)?.unwrap_or(0).saturating_add(1);
            table
                .insert(META_REV_KEY, revision.to_be_bytes().as_slice())
                .map_err(storage_err)?;
        }
        write_txn.commit().map_err(storage_err)?;
        Ok(())
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
        let write_txn = self.db.begin_write().map_err(storage_err)?;
        let next_revision;
        {
            let mut table = write_txn.open_table(KV_TABLE).map_err(storage_err)?;
            let actual = read_revision_table(&table)?;
            if let Some(expected) = expected_revision {
                if actual != Some(expected) {
                    return Ok(StorageCommitOutcome::Conflict {
                        expected_revision: Some(expected),
                        actual_revision: actual,
                    });
                }
            }
            for op in ops {
                match op {
                    KvWriteOp::Put { key, value } => {
                        table
                            .insert(key.as_slice(), value.as_slice())
                            .map_err(storage_err)?;
                    }
                    KvWriteOp::Delete { key } => {
                        let _ = table.remove(key.as_slice()).map_err(storage_err)?;
                    }
                }
            }
            let revision = actual.unwrap_or(0).saturating_add(1);
            table
                .insert(META_REV_KEY, revision.to_be_bytes().as_slice())
                .map_err(storage_err)?;
            next_revision = Some(revision);
        }
        write_txn.commit().map_err(storage_err)?;
        Ok(StorageCommitOutcome::Committed {
            revision: next_revision,
        })
    }
}

fn prefix_range_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    for idx in (0..end.len()).rev() {
        if end[idx] != u8::MAX {
            end[idx] += 1;
            end.truncate(idx + 1);
            return Some(end);
        }
    }
    None
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

    use super::{DbOpenMode, RedbDatabase, RedbKvEngine, open_backend};

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
            assert_eq!(out[0].get("name"), Some(&Value::String("n".into())));
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
            assert_eq!(by_id[0].get("name"), Some(&Value::String("first".into())));

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
