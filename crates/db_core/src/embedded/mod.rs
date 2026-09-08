mod backend;
pub mod db;
mod schema_store;
pub mod storage;

pub use backend::EmbeddedBackend;
pub use db::EmbeddedDb;
#[cfg(test)]
pub(crate) use storage::MemoryEntityStorage;
pub use storage::{
    BoxEntityIdScan, BoxEntityScan, EntityStorage, StorageCommitOutcome,
    StorageTransactionCapabilities, StorageWriteOp, StoredEntity, StoredEntityKind,
};
