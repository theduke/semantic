mod backend;
pub mod db;
mod schema_store;
pub mod storage;

pub use backend::EmbeddedBackend;
pub use db::EmbeddedDb;
#[cfg(test)]
pub(crate) use storage::MemoryEntityStorage;
pub use storage::{
    BoxEntityIdScan, BoxEntityScan, EntityReadSnapshot, EntityStorage, ForwardingReadSnapshot,
    StorageCommitOutcome, StorageStats, StorageTableStats, StorageTransactionCapabilities,
    StorageWriteOp, StoredEntity, StoredEntityKind, unsupported_ordered_index_scan,
    unsupported_storage_maintenance,
};
