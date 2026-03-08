mod backend;
pub mod db;
mod schema_store;
mod spawner;
pub mod storage;

pub use crate::backend::KvBackend;
pub use crate::db::KvDb;
#[cfg(feature = "tokio")]
pub use crate::spawner::TokioSpawner;
pub use crate::spawner::{DefaultKvBackendSpawner, InlineSpawner, KvBackendSpawner};
pub use crate::storage::{
    EntityStore, KvCommitOutcome, KvEngine, KvTransactionCapabilities, KvWriteOp, MemoryKvEngine,
};
pub use semantic_db_core::DbError;
pub use semantic_db_core::catalog::{
    Catalog, CatalogError, CollectionKind, CollectionSchema, LocalAttrId, LocalClassId,
    LocalCollectionId, LocalIndexId, LocalRecordTypeId, LocalTypeDefId, SharedCatalog,
};
pub use semantic_db_core::{
    Assignment, Batch, BatchOperation, BinaryOp, CompareOp, DeleteQuery, DeleteResult, Expr,
    MutationStats, ObjectNormalizationError, Operand, OrderBy, Predicate, Query,
    QueryCanonicalizationError, QueryField, QueryResult, SelectQuery, SortDirection,
    TransactionOptions, UnaryOp, UpdateQuery, UpdateResult,
};
