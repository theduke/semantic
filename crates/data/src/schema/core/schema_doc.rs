use crate::schema::core::type_name::TypeName;
use std::collections::BTreeMap;

/// A complete schema document: named type definitions + optional root and imports.
#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct SchemaDoc {
    pub defs: BTreeMap<TypeName, crate::schema::core::type_def::TypeDef>,
    pub attributes: BTreeMap<String, crate::schema::attribute::attribute_type::AttributeType>,
    pub classes: BTreeMap<String, crate::schema::class::class_type::ClassType>,
    pub contracts: BTreeMap<String, crate::schema::contract::contract::Contract>,
    pub root_module: Option<crate::schema::module::module::Module>,
    pub modules: BTreeMap<String, crate::schema::module::module::Module>,
    pub packages: BTreeMap<String, crate::schema::package::package::Package>,
    pub imports: Vec<crate::schema::core::schema_import::SchemaImport>,
    pub root: Option<crate::schema::core::type_ref::TypeRef>,
    pub version: Option<crate::schema::core::schema_version::SchemaVersion>,
    pub meta: crate::schema::core::meta::Meta,
}
