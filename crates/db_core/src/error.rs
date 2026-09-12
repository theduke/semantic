use thiserror::Error;

use crate::catalog::{CatalogError, LocalClassId, LocalCollectionId, LocalRecordTypeId};
use crate::{ObjectNormalizationError, QueryCanonicalizationError};

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    Validation(#[from] crate::ValidationError),
    #[error("unsupported constraint '{kind}'")]
    UnsupportedConstraint { kind: String },
    #[error("batch returning error: {reason:?} ({field:?})")]
    BatchReturn {
        reason: crate::BatchReturnErrorReason,
        field: Option<String>,
    },
    #[error("entity '{id}' already exists in collection '{collection}'")]
    EntityExists { collection: String, id: String },
    #[error("ref field '{field}' in collection '{collection}' points to missing target id '{id}'")]
    ReferenceTargetNotFound {
        collection: String,
        field: String,
        id: String,
    },
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

    #[error("query parameter error: {reason} ({name:?})")]
    QueryParameter {
        reason: String,
        name: Option<String>,
    },

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

impl crate::TransactionError for DbError {
    fn is_conflict(&self) -> bool {
        matches!(self, Self::TransactionConflict(_))
    }

    fn is_retryable(&self) -> bool {
        matches!(self, Self::TransactionConflict(_))
    }
}

impl From<crate::sql::SqlQueryError> for DbError {
    fn from(error: crate::sql::SqlQueryError) -> Self {
        match error {
            crate::sql::SqlQueryError::Parameter { reason, name } => {
                Self::QueryParameter { reason, name }
            }
            other => Self::InvalidQuery(other.to_string()),
        }
    }
}
