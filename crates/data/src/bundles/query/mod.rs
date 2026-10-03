//! Built-in schema definitions for constructing and transporting query ASTs.
//!
//! Install [`package`] through the normal package migration machinery. Current
//! definitions come from the codecs; historical migrations use frozen snapshots.

use std::collections::BTreeMap;

use crate::schema::{Meta, Migration, MigrationDdlOperation, MigrationOperation, Module, Package};

pub use crate::query::semantic::MODULE_NAME;
pub const PACKAGE_NAME: &str = "semantic.query";
pub const V1_MIGRATION_NAME: &str = "001_query_ast";

pub fn root_module() -> Module {
    Module {
        name: MODULE_NAME.into(),
        constants: BTreeMap::new(),
        types: crate::query::semantic::definitions(),
        attributes: BTreeMap::new(),
        classes: BTreeMap::new(),
        interfaces: BTreeMap::new(),
        contracts: BTreeMap::new(),
        meta: Meta::default(),
    }
}

pub fn package() -> Package {
    Package {
        name: PACKAGE_NAME.into(),
        root: root_module(),
        modules: BTreeMap::new(),
        migrations: vec![migration_v1()],
        version: None,
        meta: Meta::default(),
    }
}

/// Frozen initial definition graph. Never replace the snapshot with current
/// definitions: already applied migration definitions must remain unchanged.
pub fn migration_v1() -> Migration {
    let definitions: BTreeMap<String, crate::schema::TypeDef> =
        facet_json::from_str(include_str!("v1-definitions.json"))
            .expect("the frozen query AST schema snapshot must be valid");
    Migration {
        module: MODULE_NAME.into(),
        name: V1_MIGRATION_NAME.into(),
        description: Some("Introduce the complete public query AST value schema.".into()),
        operations: definitions
            .into_values()
            .map(|type_def| {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertTypeDef { type_def })
            })
            .collect(),
        meta: Meta::default(),
    }
}
