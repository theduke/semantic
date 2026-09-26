use semantic_data::schema::lowered::LowerError;
use thiserror::Error;

use crate::catalog::{LocalClassId, LocalCollectionId, LocalRecordTypeId};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CatalogError {
    #[error("collection '{name}' already exists")]
    CollectionAlreadyExists { name: String },

    #[error("collection '{0:?}' not found")]
    UnknownCollection(LocalCollectionId),

    #[error("record type '{0:?}' not found")]
    UnknownRecordType(LocalRecordTypeId),

    #[error("class '{0:?}' not found")]
    UnknownClass(LocalClassId),

    #[error("attribute '{id}' not found")]
    UnknownAttribute { id: String },

    #[error("invalid schema: {0}")]
    InvalidSchema(String),

    /// A data definition whose type has no stored-value representation.
    #[error("invalid schema: {}", .error.in_context(.definition))]
    UnstorableType {
        /// Human-readable definition context, e.g. `attribute 'x:tags'`.
        definition: String,
        error: LowerError,
    },
}
