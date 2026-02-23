use thiserror::Error;

use semantic_db_core::catalog::{CatalogError, LocalClassId, LocalCollectionId, LocalRecordTypeId};
use semantic_db_core::{ObjectNormalizationError, QueryCanonicalizationError};

pub type Result<T, E = DbError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("collection '{name}' already exists")]
    CollectionAlreadyExists { name: String },

    #[error("collection '{name}' not found")]
    UnknownCollectionByName { name: String },

    #[error("collection '{0:?}' not found")]
    UnknownCollection(LocalCollectionId),

    #[error("record type '{0:?}' not found")]
    UnknownRecordType(LocalRecordTypeId),

    #[error("class '{0:?}' not found")]
    UnknownClass(LocalClassId),

    #[error("attribute '{id}' not found")]
    UnknownAttribute { id: String },

    #[error("entity '{id}' not found in collection '{collection}'")]
    EntityNotFound { collection: String, id: String },

    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error(transparent)]
    QueryCanonicalization(#[from] QueryCanonicalizationError),

    #[error(transparent)]
    ObjectNormalization(#[from] ObjectNormalizationError),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("deserialization error: {0}")]
    Deserialization(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("transaction conflict: {0}")]
    TransactionConflict(String),

    #[error("transaction retries exhausted after {attempts} attempts")]
    TransactionRetriesExhausted { attempts: u32 },
}

impl From<CatalogError> for DbError {
    fn from(value: CatalogError) -> Self {
        match value {
            CatalogError::CollectionAlreadyExists { name } => {
                Self::CollectionAlreadyExists { name }
            }
            CatalogError::UnknownCollection(id) => Self::UnknownCollection(id),
            CatalogError::UnknownRecordType(id) => Self::UnknownRecordType(id),
            CatalogError::UnknownClass(id) => Self::UnknownClass(id),
            CatalogError::UnknownAttribute { id } => Self::UnknownAttribute { id },
            CatalogError::InvalidSchema(msg) => Self::InvalidQuery(msg),
        }
    }
}

impl semantic_db_core::TransactionError for DbError {
    fn is_conflict(&self) -> bool {
        matches!(self, Self::TransactionConflict(_))
    }

    fn is_retryable(&self) -> bool {
        matches!(self, Self::TransactionConflict(_))
    }
}
