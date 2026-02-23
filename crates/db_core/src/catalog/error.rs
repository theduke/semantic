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
}
