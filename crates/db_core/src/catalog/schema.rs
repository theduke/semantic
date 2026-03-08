use std::collections::BTreeMap;

use fnv::FnvHashMap;
use semantic_data::schema::{
    IndexSchema as DataIndexSchema, attribute::attribute_type::AttributeType,
    class::class_type::ClassType, core::type_def::TypeDef, core::type_node::Type,
    record::record_type::RecordType,
};

use crate::catalog::{
    LocalAttrId, LocalClassId, LocalCollectionId, LocalFieldId, LocalIndexId, LocalRecordTypeId,
    LocalRelationId, LocalTypeDefId,
};

#[derive(Debug, Clone)]
pub struct AttributeSchema {
    pub lid: LocalAttrId,
    pub attribute: AttributeType,
}

#[derive(Debug, Clone)]
pub struct RecordTypeSchema {
    pub lid: LocalRecordTypeId,
    pub id: String,
    pub name: String,
    pub record: RecordType,
}

#[derive(Debug, Clone)]
pub struct TypeDefSchema {
    pub lid: LocalTypeDefId,
    pub type_def: TypeDef,
}

#[derive(Debug, Clone)]
pub struct ClassSchema {
    pub lid: LocalClassId,
    pub class: ClassType,
    pub attributes: BTreeMap<String, LocalAttrId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CollectionKind {
    Polymorphic,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum IntegrityMode {
    Permissive,
    StrictRegisteredSchema,
}

#[derive(Debug, Clone)]
pub struct CollectionSchema {
    pub lid: LocalCollectionId,
    pub name: String,
    pub kind: CollectionKind,
    pub integrity_mode: IntegrityMode,

    field_aliases: FnvHashMap<String, String>,
    field_types: FnvHashMap<String, Type>,
    field_ids: FnvHashMap<String, LocalFieldId>,
    field_names_by_id: FnvHashMap<LocalFieldId, String>,
    attr_by_field_id: FnvHashMap<LocalFieldId, LocalAttrId>,
    closed_fields: bool,
}

impl CollectionSchema {
    pub(crate) fn new(
        lid: LocalCollectionId,
        name: String,
        kind: CollectionKind,
        integrity_mode: IntegrityMode,
        field_aliases: FnvHashMap<String, String>,
        field_types: FnvHashMap<String, Type>,
        field_ids: FnvHashMap<String, LocalFieldId>,
        field_names_by_id: FnvHashMap<LocalFieldId, String>,
        attr_by_field_id: FnvHashMap<LocalFieldId, LocalAttrId>,
        closed_fields: bool,
    ) -> Self {
        Self {
            lid,
            name,
            kind,
            integrity_mode,
            field_aliases,
            field_types,
            field_ids,
            field_names_by_id,
            attr_by_field_id,
            closed_fields,
        }
    }

    pub fn canonical_field_name<'a>(&'a self, field: &'a str) -> &'a str {
        self.field_aliases
            .get(field)
            .map(String::as_str)
            .unwrap_or(field)
    }

    pub fn field_type(&self, canonical_field: &str) -> Option<&Type> {
        self.field_types.get(canonical_field)
    }

    pub fn field_id(&self, field_name_or_alias: &str) -> Option<LocalFieldId> {
        let canonical = self.canonical_field_name(field_name_or_alias);
        self.field_ids.get(canonical).copied()
    }

    pub fn field_name_by_id(&self, field_id: LocalFieldId) -> Option<&str> {
        self.field_names_by_id.get(&field_id).map(String::as_str)
    }

    pub fn attr_for_field_id(&self, field_id: LocalFieldId) -> Option<LocalAttrId> {
        self.attr_by_field_id.get(&field_id).copied()
    }

    pub fn fields(&self) -> impl Iterator<Item = (LocalFieldId, &str)> {
        self.field_names_by_id
            .iter()
            .map(|(field_id, name)| (*field_id, name.as_str()))
    }

    pub fn is_closed_field_set(&self) -> bool {
        self.closed_fields
    }

    pub fn knows_field(&self, canonical_field: &str) -> bool {
        self.field_types.contains_key(canonical_field)
    }
}

#[derive(Debug, Clone)]
pub struct IndexSchema {
    pub lid: LocalIndexId,
    pub schema: DataIndexSchema,
    pub collection: LocalCollectionId,
    pub canonical_field: String,
    pub field_id: Option<LocalFieldId>,
    pub attr_id: Option<LocalAttrId>,
}

#[derive(Debug, Clone)]
pub struct RelationshipSchema {
    pub lid: LocalRelationId,
    pub relationship: semantic_data::schema::RelationType,
}
