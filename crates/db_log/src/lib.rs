//! WAL-backed embedded database storage.
//!
//! Each committed key-value batch is stored as one immutable, versioned event
//! before it is applied to an in-memory cache. [`LogFsLogStore`] provides a
//! durable local backend, while [`ObjStoreLogStore`] supports object stores.
//! State is memory-resident and rebuilt by replaying the WAL when opened. WAL
//! compaction and migration from the historical FactorDB format are not
//! currently supported.
//!
//! The CLI and server provider accept local URIs such as
//! `logfs:/absolute/path/database.log`. Library callers can use
//! [`open_backend`] or construct an [`ObjStoreLogStore`] and pass it to
//! [`open_backend_from_store`]. ObjStore durability and atomic-create behavior
//! depend on the selected provider, and one writer per WAL namespace is
//! supported. Replay cost grows with WAL size, and existing collection-wide
//! rewrites can produce correspondingly large events.

mod engine;
mod event;
mod store;

pub use engine::LogEngine;
pub use event::EventId;
pub use store::{LogFsLogStore, LogStore, ObjStoreLogStore};

use std::path::Path;

use semantic_data::schema::DbOpenMode;
use semantic_db_core::embedded::{EmbeddedBackend, EmbeddedDb};
use semantic_db_core::{DbConfig, DbError};
use semantic_db_kv::EntityStore;

pub type LogDatabase<S> = EmbeddedDb<EntityStore<LogEngine<S>>>;
pub type LogBackend<S> = EmbeddedBackend<EntityStore<LogEngine<S>>>;
pub type LogFsDatabase = LogDatabase<LogFsLogStore>;
pub type LogFsBackend = LogBackend<LogFsLogStore>;

/// Open a local logfs-backed database.
pub fn open_backend(
    path: impl AsRef<Path>,
    mode: DbOpenMode,
) -> std::result::Result<LogFsBackend, DbError> {
    open_backend_with_config(path, mode, DbConfig::default())
}

/// Open a local logfs-backed database with explicit database configuration.
pub fn open_backend_with_config(
    path: impl AsRef<Path>,
    mode: DbOpenMode,
    config: DbConfig,
) -> std::result::Result<LogFsBackend, DbError> {
    let store = LogFsLogStore::open(path, mode)?;
    open_backend_from_store_with_config(store, config)
}

/// Build a backend from an arbitrary log store.
pub fn open_backend_from_store<S: LogStore>(
    store: S,
) -> std::result::Result<LogBackend<S>, DbError> {
    open_backend_from_store_with_config(store, DbConfig::default())
}

/// Build a backend from an arbitrary log store and database configuration.
pub fn open_backend_from_store_with_config<S: LogStore>(
    store: S,
    config: DbConfig,
) -> std::result::Result<LogBackend<S>, DbError> {
    let engine = LogEngine::open(store)?;
    let db = LogDatabase::open_with_config(EntityStore::new(engine), config)?;
    Ok(LogBackend::new(db))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use objstore_fs::{FsObjStore, FsObjStoreConfig};
    use semantic_data::value::{Object, Value};
    use semantic_db_core::Db;
    use semantic_db_core::catalog::CollectionKind;

    use super::*;

    #[test]
    fn logfs_database_recovers_catalog_and_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("database.log");
        {
            let store = LogFsLogStore::open(&path, DbOpenMode::AutoCreate).unwrap();
            let engine = LogEngine::open(store).unwrap();
            let mut db = LogFsDatabase::open(EntityStore::new(engine)).unwrap();
            db.create_collection("items", CollectionKind::Untyped)
                .unwrap();
            let mut object = Object::new();
            object.insert("id", Value::String("one".to_string()));
            object.insert("name", Value::String("persisted".to_string()));
            db.insert("items", "one", object).unwrap();
        }
        {
            let store = LogFsLogStore::open(&path, DbOpenMode::OpenExisting).unwrap();
            let engine = LogEngine::open(store).unwrap();
            let db = LogFsDatabase::open(EntityStore::new(engine)).unwrap();
            let object = db.get("items", "one").unwrap().unwrap();
            assert_eq!(
                object.object.get("name"),
                Some(&Value::String("persisted".to_string()))
            );
        }
    }

    #[test]
    fn logfs_open_modes_and_writer_lock_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("database.log");
        assert!(LogFsLogStore::open(&path, DbOpenMode::OpenExisting).is_err());
        let first = LogFsLogStore::open(&path, DbOpenMode::AutoCreate).unwrap();
        assert!(LogFsLogStore::open(&path, DbOpenMode::OpenExisting).is_err());
        drop(first);
        LogFsLogStore::open(&path, DbOpenMode::OpenExisting).unwrap();
    }

    #[test]
    fn objstore_adapter_works_without_an_async_context() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(FsObjStore::new(FsObjStoreConfig::new(dir.path().to_path_buf())).unwrap());
        let mut log = ObjStoreLogStore::new(store).unwrap();
        log.append_event(EventId::FIRST, b"event".to_vec()).unwrap();
        assert_eq!(log.event_ids(EventId::FIRST).unwrap(), vec![EventId::FIRST]);
        assert_eq!(
            log.read_event(EventId::FIRST).unwrap(),
            Some(b"event".to_vec())
        );
        assert!(
            log.append_event(EventId::FIRST, b"replacement".to_vec())
                .is_err()
        );
        assert_eq!(
            log.read_event(EventId::FIRST).unwrap(),
            Some(b"event".to_vec())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn objstore_adapter_works_on_current_thread_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(FsObjStore::new(FsObjStoreConfig::new(dir.path().to_path_buf())).unwrap());
        let mut log = ObjStoreLogStore::new(store).unwrap();
        log.append_event(EventId::FIRST, b"event".to_vec()).unwrap();
        assert_eq!(
            log.read_event(EventId::FIRST).unwrap(),
            Some(b"event".to_vec())
        );
    }

    #[test]
    fn objstore_database_replays_from_individual_objects() {
        let dir = tempfile::tempdir().unwrap();
        {
            let object_store =
                Arc::new(FsObjStore::new(FsObjStoreConfig::new(dir.path().to_path_buf())).unwrap());
            let log_store = ObjStoreLogStore::new(object_store).unwrap();
            let engine = LogEngine::open(log_store).unwrap();
            let mut db = LogDatabase::open(EntityStore::new(engine)).unwrap();
            db.create_collection("items", CollectionKind::Untyped)
                .unwrap();
            let mut object = Object::new();
            object.insert("id", Value::String("one".to_string()));
            object.insert("name", Value::String("persisted".to_string()));
            db.insert("items", "one", object).unwrap();
        }
        {
            let object_store =
                Arc::new(FsObjStore::new(FsObjStoreConfig::new(dir.path().to_path_buf())).unwrap());
            let log_store = ObjStoreLogStore::new(object_store).unwrap();
            let engine = LogEngine::open(log_store).unwrap();
            let db = LogDatabase::open(EntityStore::new(engine)).unwrap();
            let object = db.get("items", "one").unwrap().unwrap();
            assert_eq!(
                object.object.get("name"),
                Some(&Value::String("persisted".to_string()))
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn logfs_backend_passes_shared_suite() {
        let dir = tempfile::tempdir().unwrap();
        let backend =
            open_backend(dir.path().join("database.log"), DbOpenMode::AutoCreate).unwrap();
        semantic_db_test::suite::test_db(&Db::new(backend)).await;
    }
}
