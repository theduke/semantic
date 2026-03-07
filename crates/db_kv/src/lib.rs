mod backend;
pub mod db;
mod schema_store;
pub mod storage;

pub use crate::backend::KvBackend;
pub use crate::db::Database;
pub use crate::storage::{
    EntityStore, KvCommitOutcome, KvEngine, KvTransactionCapabilities, KvWriteOp, MemoryKvEngine,
};
pub use semantic_db_core::DbError;
pub use semantic_db_core::catalog::{
    Catalog, CatalogError, CollectionKind, CollectionSchema, LocalAttrId, LocalClassId,
    LocalCollectionId, LocalIndexId, LocalRecordTypeId, LocalTypeDefId, SharedCatalog,
};
pub use semantic_db_core::{
    Assignment, Batch, BatchOperation, BinaryOp, CompareOp, DeleteQuery, Expr, MutationStats,
    ObjectNormalizationError, Operand, OrderBy, Predicate, QueryCanonicalizationError, QueryField,
    SelectQuery, SortDirection, TransactionOptions, UnaryOp, UpdateQuery,
};
