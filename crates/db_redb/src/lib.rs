use redb::{ReadableTable, TableDefinition};
use semantic_data::schema::DbOpenMode;
use semantic_db_kv::{DbError, KvCommitOutcome, KvEngine, KvTransactionCapabilities, KvWriteOp};
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

    fn scan_prefix(&self, prefix: &[u8]) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        let read_txn = self.db.begin_read().map_err(storage_err)?;
        let table = read_txn.open_table(KV_TABLE).map_err(storage_err)?;
        let iter = table.iter().map_err(storage_err)?;

        let mut out = Vec::new();
        for item in iter {
            let (key_guard, value_guard): (redb::AccessGuard<&[u8]>, redb::AccessGuard<&[u8]>) =
                item.map_err(storage_err)?;
            let key = key_guard.value();
            if key.starts_with(prefix) {
                out.push((key.to_vec(), value_guard.value().to_vec()));
            }
        }

        Ok(out)
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

    fn tx_capabilities(&self) -> KvTransactionCapabilities {
        KvTransactionCapabilities {
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
    ) -> std::result::Result<KvCommitOutcome, DbError> {
        let write_txn = self.db.begin_write().map_err(storage_err)?;
        let next_revision;
        {
            let mut table = write_txn.open_table(KV_TABLE).map_err(storage_err)?;
            let actual = read_revision_table(&table)?;
            if let Some(expected) = expected_revision {
                if actual != Some(expected) {
                    return Ok(KvCommitOutcome::Conflict {
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
        Ok(KvCommitOutcome::Committed {
            revision: next_revision,
        })
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

pub type RedbDatabase = semantic_db_kv::KvDb<RedbKvEngine>;
pub type RedbBackend = semantic_db_kv::KvBackend<RedbKvEngine>;

pub fn open_backend(
    path: impl AsRef<Path>,
    mode: DbOpenMode,
) -> std::result::Result<RedbBackend, DbError> {
    let engine = RedbKvEngine::open(path, mode)?;
    let db = RedbDatabase::open(engine)?;
    Ok(RedbBackend::new(db))
}

#[cfg(test)]
mod tests {
    use semantic_data::value::{Object, Value};
    use semantic_db_core::Db;
    use semantic_db_kv::CollectionKind;

    use super::{DbOpenMode, RedbDatabase, RedbKvEngine, open_backend};

    #[test]
    fn redb_backend_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");

        {
            let engine = RedbKvEngine::open(&path, DbOpenMode::AutoCreate).unwrap();
            let mut db = RedbDatabase::new(engine);
            db.create_collection("items", CollectionKind::Untyped)
                .unwrap();

            let mut obj = Object::new();
            obj.insert("id", Value::String("id1".into()));
            obj.insert("name", Value::String("n".into()));
            db.insert("items", "id1", obj).unwrap();
        }

        {
            let engine = RedbKvEngine::open(path, DbOpenMode::OpenExisting).unwrap();
            let mut db = RedbDatabase::new(engine);
            db.create_collection("items", CollectionKind::Untyped)
                .unwrap();
            let out = db
                .select(semantic_db_core::SelectQuery::new().with_collection("items"))
                .unwrap();
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].get("name"), Some(&Value::String("n".into())));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_redb_backend_testsuite() {
        let dir = tempfile::tempdir().unwrap();
        let backend = open_backend(dir.path().join("db"), DbOpenMode::AutoCreate).unwrap();
        let db = Db::new(backend);

        semantic_db_core::test::test_db(&db).await;
    }
}
