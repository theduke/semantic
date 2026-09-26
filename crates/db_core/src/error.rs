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
    /// A write would store two rows with the same value in a unique index.
    #[error("unique index violation on field '{field}' ({existing_id} vs {id})")]
    UniqueViolation {
        collection: String,
        index: String,
        field: String,
        /// Boxed to keep `DbError` small.
        value: Box<semantic_data::value::Value>,
        /// The row already holding `value` (the smaller id of the pair).
        existing_id: String,
        /// The conflicting row.
        id: String,
    },
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
    Storage(#[source] StorageError),

    #[error("transaction conflict: {0}")]
    TransactionConflict(String),

    #[error("transaction retries exhausted after {attempts} attempts")]
    TransactionRetriesExhausted { attempts: u32 },
}

impl DbError {
    /// A storage error of `kind` without an underlying error.
    pub fn storage(kind: StorageErrorKind, message: impl Into<String>) -> Self {
        Self::Storage(StorageError::new(kind, message))
    }

    /// A storage error of `kind` caused by the backend error `source`.
    pub fn storage_with_source(
        kind: StorageErrorKind,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Storage(StorageError::with_source(kind, message, source))
    }

    /// The kind of a storage error, `None` for every other error.
    pub fn storage_kind(&self) -> Option<StorageErrorKind> {
        match self {
            Self::Storage(error) => Some(error.kind),
            _ => None,
        }
    }
}

/// Classification of a [`StorageError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageErrorKind {
    /// An I/O operation of the storage failed (file system, network).
    Io,
    /// Stored data could not be decoded or violates a storage invariant.
    Corruption,
    /// The storage does not support the requested operation.
    Unsupported,
    /// Any other failure reported by the storage backend.
    Backend,
    /// A concurrent writer changed the storage.
    Conflict,
    /// The storage or one of its handles is in a state that does not allow
    /// the operation (poisoned locks, stopped workers, live read handles).
    InvalidState,
}

impl StorageErrorKind {
    /// Stable snake_case name, e.g. for error codes.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Io => "io",
            Self::Corruption => "corruption",
            Self::Unsupported => "unsupported",
            Self::Backend => "backend",
            Self::Conflict => "conflict",
            Self::InvalidState => "invalid_state",
        }
    }
}

impl std::fmt::Display for StorageErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error reported by a storage backend.
///
/// Displays as its message; the backend error, if any, is available through
/// [`std::error::Error::source`].
#[derive(Debug)]
pub struct StorageError {
    pub kind: StorageErrorKind,
    pub message: String,
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl StorageError {
    pub fn new(kind: StorageErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        kind: StorageErrorKind,
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn kind(&self) -> StorageErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

/// Untyped storage failures are [`StorageErrorKind::Backend`] errors.
impl From<String> for StorageError {
    fn from(message: String) -> Self {
        Self::new(StorageErrorKind::Backend, message)
    }
}

impl From<&str> for StorageError {
    fn from(message: &str) -> Self {
        Self::new(StorageErrorKind::Backend, message)
    }
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
            CatalogError::UnstorableType { definition, error } => {
                Self::InvalidQuery(error.in_context(&definition))
            }
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

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn storage_errors_keep_kind_message_and_source() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing file");
        let err = DbError::storage_with_source(StorageErrorKind::Io, "open db: missing file", io);
        assert_eq!(err.storage_kind(), Some(StorageErrorKind::Io));
        assert_eq!(err.to_string(), "storage error: open db: missing file");
        let storage = err.source().expect("storage error source");
        assert_eq!(storage.to_string(), "open db: missing file");
        let io = storage.source().expect("backend error source");
        assert_eq!(
            io.downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::NotFound)
        );
    }

    #[test]
    fn untyped_storage_errors_are_backend_errors() {
        let err = DbError::Storage(format!("code {}", 7).into());
        assert_eq!(err.storage_kind(), Some(StorageErrorKind::Backend));
        assert_eq!(err.to_string(), "storage error: code 7");
        assert!(err.source().unwrap().source().is_none());
        let err = DbError::Storage("plain".into());
        assert_eq!(err.storage_kind(), Some(StorageErrorKind::Backend));
        assert_eq!(DbError::InvalidQuery("x".into()).storage_kind(), None);
        assert_eq!(StorageErrorKind::InvalidState.to_string(), "invalid_state");
    }
}
