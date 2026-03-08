use std::collections::BTreeMap;

/// A package is a set of modules with a distinguished root module.
#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct Package {
    pub name: String,
    /// The root module is the package entry module and is not nested.
    pub root: crate::schema::module::module::Module,
    /// Additional non-root modules in the package.
    pub modules: BTreeMap<String, crate::schema::module::module::Module>,
    /// Package-scoped migrations, each targeted at a module.
    pub migrations: Vec<crate::schema::migration::Migration>,
    pub version: Option<crate::schema::core::schema_version::SchemaVersion>,
    pub meta: crate::schema::core::meta::Meta,
}
