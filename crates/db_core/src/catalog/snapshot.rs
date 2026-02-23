use semantic_data::schema::{
    attribute::attribute_type::AttributeType, class::class_type::ClassType,
    core::type_def::TypeDef, record::record_type::RecordType,
};

use crate::catalog::{
    LocalAttrId, LocalClassId, LocalCollectionId, LocalFieldId, LocalIndexId, LocalRecordTypeId,
    LocalTypeDefId,
};

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct CatalogStorageSnapshot {
    pub attributes: Vec<StoredAttribute>,
    pub type_defs: Vec<StoredTypeDef>,
    pub record_types: Vec<StoredRecordType>,
    pub classes: Vec<StoredClass>,
    pub collections: Vec<StoredCollection>,
    pub indexes: Vec<StoredIndex>,
    pub next_field_id: usize,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct StoredAttribute {
    pub lid: LocalAttrId,
    pub attribute: AttributeType,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct StoredTypeDef {
    pub lid: LocalTypeDefId,
    pub type_def: TypeDef,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct StoredRecordType {
    pub lid: LocalRecordTypeId,
    pub id: String,
    pub name: String,
    pub record: RecordType,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct StoredClass {
    pub lid: LocalClassId,
    pub class: ClassType,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct StoredCollection {
    pub lid: LocalCollectionId,
    pub name: String,
    pub kind: StoredCollectionKind,
    pub field_ids: Vec<StoredFieldId>,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum StoredCollectionKind {
    Untyped,
    Record { record_type: LocalRecordTypeId },
    Class { class: LocalClassId },
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct StoredFieldId {
    pub field_id: LocalFieldId,
    pub canonical_field: String,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct StoredIndex {
    pub lid: LocalIndexId,
    pub name: String,
    pub collection: LocalCollectionId,
    pub field: String,
    pub unique: bool,
}
