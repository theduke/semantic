use crate::query::{DeleteQuery, UpdateQuery};
use crate::value::Object;

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct Migration {
    pub module: String,
    pub name: String,
    pub description: Option<String>,
    pub operations: Vec<MigrationOperation>,
    pub meta: crate::schema::core::meta::Meta,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum MigrationCollectionKind {
    Polymorphic,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum MigrationIntegrityMode {
    Permissive,
    StrictRegisteredSchema,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum MigrationDdlOperation {
    UpsertAttribute {
        attribute: crate::schema::attribute::attribute_type::AttributeType,
    },
    DeleteAttribute {
        id: String,
    },
    UpsertTypeDef {
        type_def: crate::schema::core::type_def::TypeDef,
    },
    DeleteTypeDef {
        name: crate::schema::core::type_name::TypeName,
    },
    UpsertRecordType {
        id: String,
        name: String,
        record: crate::schema::record::record_type::RecordType,
    },
    DeleteRecordType {
        id: String,
    },
    UpsertClass {
        class: crate::schema::class::class_type::ClassType,
    },
    DeleteClass {
        id: String,
    },
    UpsertCollection {
        name: String,
        kind: MigrationCollectionKind,
        integrity_mode: MigrationIntegrityMode,
    },
    DeleteCollection {
        name: String,
    },
    UpsertIndex {
        name: String,
        collection: String,
        field: String,
        unique: bool,
    },
    DeleteIndex {
        name: String,
        collection: String,
    },
    UpsertRelationship {
        relationship: crate::schema::relation::relation_type::RelationType,
    },
    DeleteRelationship {
        id: String,
    },
    SetAutoIndex {
        enabled: bool,
    },
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum MigrationOperation {
    Ddl(MigrationDdlOperation),
    Insert {
        collection: String,
        id: String,
        object: Object,
    },
    Update {
        query: UpdateQuery,
    },
    Delete {
        query: DeleteQuery,
    },
}
