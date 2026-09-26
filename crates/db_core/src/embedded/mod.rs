mod backend;
pub mod db;
mod schema_store;
pub mod storage;

pub use backend::EmbeddedBackend;
pub use db::{DbReader, EmbeddedDb, EmbeddedTransaction};
#[cfg(test)]
pub(crate) use storage::MemoryEntityStorage;
pub use storage::{
    BackupSource, BoxCheckedEntityScan, BoxEntityIdScan, BoxEntityScan, BoxIndexEntryScan,
    BoxIndexKeyScan, CheckedEntity, CheckedEntityScanItem, EntityReadSnapshot, EntityStorage,
    ForwardingReadSnapshot, IndexEntryScanItem, IndexKeyEntry, IndexKeyScanItem,
    PayloadRewriteBatch, StorageBackup, StorageCommitOutcome, StorageStats, StorageTableStats,
    StorageTransactionCapabilities, StorageWriteOp, StoredEntity, StoredEntityKind,
    unsupported_ordered_index_scan, unsupported_storage_maintenance,
};
