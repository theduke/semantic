//! Shared definitions used by independently installable built-in packages.
//!
//! Historical base/filestore migrations assigned these attributes to their own
//! modules. Both packages finish with the same shared definitions so registering
//! them in either order no longer alternates ownership or reference spelling.

use std::collections::BTreeMap;

use crate::attr::{ATTR_DESCRIPTION, ATTR_PARENT, ATTR_TITLE};
use crate::schema::{
    AttributeType, EntityRef, Meta, Migration, MigrationDdlOperation, MigrationOperation, Module,
    StringType, Type, TypeKind,
};

pub const MODULE_NAME: &str = "shared";
pub const MIGRATION_NAME: &str = "010_shared_attribute_ownership";
pub const ATTRIBUTE_IDS: [&str; 3] = [ATTR_TITLE, ATTR_DESCRIPTION, ATTR_PARENT];

pub fn module() -> Module {
    Module {
        name: MODULE_NAME.into(),
        constants: BTreeMap::new(),
        types: BTreeMap::new(),
        attributes: v1_attributes()
            .into_iter()
            .map(|attribute| (attribute.id.clone(), attribute))
            .collect(),
        classes: BTreeMap::new(),
        interfaces: BTreeMap::new(),
        contracts: BTreeMap::new(),
        meta: Meta::default(),
    }
}

/// Frozen migration shared by base and filestore. Future changes require a new
/// migration and must not change this constructor or `v1_attributes`.
pub fn migration_v1() -> Migration {
    Migration {
        module: MODULE_NAME.into(),
        name: MIGRATION_NAME.into(),
        description: Some(
            "Give built-in shared attributes identical definitions and module ownership.".into(),
        ),
        operations: v1_attributes()
            .into_iter()
            .map(|attribute| {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute })
            })
            .collect(),
        meta: Meta::default(),
    }
}

fn v1_attributes() -> Vec<AttributeType> {
    let string = || {
        Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        }))
    };
    [
        ("semantic:title", "title", "Title", string()),
        (
            "semantic:description",
            "description",
            "Description",
            string(),
        ),
        // Equivalent to the historical unrestricted `Ref("id")`, retaining
        // its Restrict delete behavior and using the current intrinsic form.
        (
            "semantic:parent",
            "parent",
            "Parent",
            Type::new(TypeKind::Ref(EntityRef::any())),
        ),
    ]
    .into_iter()
    .map(|(id, name, title, ty)| AttributeType {
        id: id.into(),
        name: name.into(),
        ty,
        constraints: Vec::new(),
        meta: Meta {
            title: Some(title.into()),
            ..Meta::default()
        },
    })
    .collect()
}
