mod catalog;
mod error;
mod id_map;
mod ids;
mod names;
mod schema;
mod shared;
mod snapshot;

pub use catalog::{
    AUTO_PATH_INDEX_FIELD, AUTO_PATH_INDEX_NAME, Catalog, CatalogBatchOperation, OBJECT_TYPE_FIELD,
    OBJECT_TYPE_INDEX_NAME, PARENT_RELATION_ATTRIBUTE, PARENT_RELATION_FIELD, PRIMARY_ID_FIELD,
    PRIMARY_ID_INDEX_NAME, RELATION_CLASS_ID, RELATION_FROM_ATTRIBUTE, RELATION_TO_ATTRIBUTE,
};
pub use error::CatalogError;
pub use id_map::IdMap;
pub use ids::{
    LocalAttrId, LocalClassId, LocalCollectionId, LocalFieldId, LocalIndexId, LocalPackageId,
    LocalRecordTypeId, LocalRelationId, LocalTypeDefId,
};
pub use names::{
    IMPLICIT_ROOT_PACKAGE, SEMANTIC_PACKAGE, is_special_builtin_field, nameset_for_identifier,
    nameset_for_qualified,
};
pub use schema::{
    AttributeSchema, ClassSchema, CollectionKind, CollectionSchema, IndexSchema, IntegrityMode,
    NameSet, RecordTypeSchema, RelationshipSchema, TypeDefSchema,
};
pub use shared::{CatalogSnapshot, CatalogVersionMismatch, SharedCatalog};
pub use snapshot::{
    CatalogStorageSnapshot, StoredAppliedMigration, StoredAttribute, StoredClass, StoredCollection,
    StoredFieldId, StoredIndex, StoredPackage, StoredRecordType, StoredRelationship, StoredTypeDef,
};
