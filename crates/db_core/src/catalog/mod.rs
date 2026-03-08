mod catalog;
mod error;
mod id_map;
mod ids;
mod schema;
mod shared;
mod snapshot;

pub use catalog::{
    AUTO_PATH_INDEX_FIELD, AUTO_PATH_INDEX_NAME, Catalog, OBJECT_TYPE_FIELD,
    OBJECT_TYPE_INDEX_NAME, PRIMARY_ID_FIELD, PRIMARY_ID_INDEX_NAME,
};
pub use error::CatalogError;
pub use id_map::IdMap;
pub use ids::{
    LocalAttrId, LocalClassId, LocalCollectionId, LocalFieldId, LocalIndexId, LocalRecordTypeId,
    LocalTypeDefId,
};
pub use schema::{
    AttributeSchema, ClassSchema, CollectionKind, CollectionSchema, IndexSchema, RecordTypeSchema,
    TypeDefSchema,
};
pub use shared::{CatalogSnapshot, CatalogVersionMismatch, SharedCatalog};
pub use snapshot::{
    CatalogStorageSnapshot, StoredAttribute, StoredClass, StoredCollection, StoredCollectionKind,
    StoredFieldId, StoredIndex, StoredRecordType, StoredTypeDef,
};
