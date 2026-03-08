use std::path::Path;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use redb::{ReadableTable, TableDefinition};
use semantic_data::value::Object;
use semantic_db_core::catalog::{Catalog, CollectionKind, LocalCollectionId};
use semantic_db_core::{
    Backend, Batch, BatchOutcome, DeleteQuery, EntityRecord, MutationStats, Query, QueryExplain,
    QueryPlan, QueryResult, UpdateQuery,
};
use semantic_db_kv::{DbError, KvCommitOutcome, KvEngine, KvTransactionCapabilities, KvWriteOp};

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
    pub fn open(path: impl AsRef<Path>) -> std::result::Result<Self, DbError> {
        let db = redb::Database::create(path).map_err(storage_err)?;
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

pub struct RedbBackend {
    db: Arc<RwLock<RedbDatabase>>,
}

impl RedbBackend {
    pub fn open(path: impl AsRef<Path>) -> std::result::Result<Self, DbError> {
        let engine = RedbKvEngine::open(path)?;
        let db = RedbDatabase::open(engine)?;
        Ok(Self::new(db))
    }

    pub fn new(db: RedbDatabase) -> Self {
        Self {
            db: Arc::new(RwLock::new(db)),
        }
    }

    async fn with_db_read<T, F>(&self, call: F) -> std::result::Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&RedbDatabase) -> std::result::Result<T, DbError> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || {
            let db = db
                .read()
                .map_err(|_| DbError::Storage("redb backend rwlock poisoned".to_string()))?;
            call(&db)
        })
        .await
        .map_err(|err| DbError::Storage(format!("redb blocking task failed: {err}")))?
    }

    async fn with_db_write<T, F>(&self, call: F) -> std::result::Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&mut RedbDatabase) -> std::result::Result<T, DbError> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || {
            let mut db = db
                .write()
                .map_err(|_| DbError::Storage("redb backend rwlock poisoned".to_string()))?;
            call(&mut db)
        })
        .await
        .map_err(|err| DbError::Storage(format!("redb blocking task failed: {err}")))?
    }
}

#[async_trait]
impl Backend for RedbBackend {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        self.with_db_read(|db| Ok(db.catalog())).await
    }

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        self.with_db_write(move |db| db.create_collection(name, kind))
            .await
    }

    async fn insert(
        &self,
        collection: String,
        id: String,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        self.with_db_write(move |db| db.insert(&collection, id, object))
            .await
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        self.with_db_read(move |db| db.get(&collection, &id)).await
    }

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError> {
        self.with_db_write(move |db| db.delete(&collection, &id))
            .await
    }

    async fn query(
        &self,
        collection: String,
        query: Query,
    ) -> std::result::Result<QueryResult, DbError> {
        match query {
            Query::Select(select) => {
                self.with_db_read(move |db| db.select(&collection, select).map(QueryResult::Select))
                    .await
            }
            query => {
                self.with_db_write(move |db| db.query(&collection, query))
                    .await
            }
        }
    }

    async fn explain_query(
        &self,
        collection: String,
        query: Query,
    ) -> std::result::Result<QueryExplain, DbError> {
        self.with_db_read(move |db| db.explain_query(&collection, query))
            .await
    }

    async fn plan_query(
        &self,
        collection: String,
        query: Query,
    ) -> std::result::Result<QueryPlan, DbError> {
        self.with_db_read(move |db| db.plan_query(&collection, query))
            .await
    }

    async fn update_where(
        &self,
        collection: String,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        self.with_db_write(move |db| db.update_where(&collection, query))
            .await
    }

    async fn delete_where(
        &self,
        collection: String,
        query: DeleteQuery,
    ) -> std::result::Result<usize, DbError> {
        self.with_db_write(move |db| db.delete_where(&collection, query))
            .await
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        self.with_db_write(move |db| db.execute_batch(batch)).await
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::value::{Object, Value};
    use semantic_db_kv::CollectionKind;

    use super::{RedbDatabase, RedbKvEngine};

    #[test]
    fn redb_backend_roundtrip() {
        let path = std::env::temp_dir().join(format!(
            "semantic-redb-test-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        {
            let engine = RedbKvEngine::open(&path).unwrap();
            let mut db = RedbDatabase::new(engine);
            db.create_collection("items", CollectionKind::Untyped)
                .unwrap();

            let mut obj = Object::new();
            obj.insert("id", Value::String("id1".into()));
            obj.insert("name", Value::String("n".into()));
            db.insert("items", "id1", obj).unwrap();
        }

        {
            let engine = RedbKvEngine::open(&path).unwrap();
            let mut db = RedbDatabase::new(engine);
            db.create_collection("items", CollectionKind::Untyped)
                .unwrap();
            let out = db
                .select("items", semantic_db_core::SelectQuery::new())
                .unwrap();
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].get("name"), Some(&Value::String("n".into())));
        }

        let _ = std::fs::remove_file(path);
    }
}
