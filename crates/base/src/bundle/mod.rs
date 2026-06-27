use std::collections::BTreeMap;

use semantic_data::schema::{Meta, Module, Package};

use crate::{migrations, schema::common};

pub const PACKAGE_NAME: &str = "semantic.base";
pub const MODULE_NAME: &str = "base";

pub fn root_module() -> Module {
    let mut attributes = BTreeMap::new();
    for attribute in common::file::attributes()
        .into_iter()
        .chain(common::person::attributes())
    {
        attributes.insert(attribute.id.clone(), attribute);
    }

    let file_class = common::file::class();
    let person_class = common::person::class();

    Module {
        name: MODULE_NAME.to_string(),
        constants: BTreeMap::new(),
        types: BTreeMap::new(),
        attributes,
        classes: BTreeMap::from([
            (file_class.id.clone(), file_class),
            (person_class.id.clone(), person_class),
        ]),
        interfaces: BTreeMap::new(),
        contracts: BTreeMap::new(),
        meta: Meta::default(),
    }
}

pub fn package() -> Package {
    Package {
        name: PACKAGE_NAME.to_string(),
        root: root_module(),
        modules: BTreeMap::new(),
        migrations: migrations::all(),
        version: None,
        meta: Meta::default(),
    }
}
