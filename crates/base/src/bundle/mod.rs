use std::collections::BTreeMap;

use semantic_data::bundles::directory;
use semantic_data::schema::{Meta, Module, Package};

use crate::{
    migrations,
    schema::{common, notes, web_bookmark},
};

pub const PACKAGE_NAME: &str = "semantic.base";
pub const MODULE_NAME: &str = "base";

pub fn root_module() -> Module {
    let mut attributes = BTreeMap::new();
    for attribute in common::person::attributes() {
        attributes.insert(attribute.id.clone(), attribute);
    }
    for attribute in directory::attributes() {
        attributes.insert(attribute.id.clone(), attribute);
    }
    for attribute in notes::attributes() {
        attributes.insert(attribute.id.clone(), attribute);
    }

    let bookmark_class = web_bookmark::class();
    let person_class = common::person::class();
    let note_class = notes::class();
    let directory_classes = directory::classes();
    let mut classes = BTreeMap::from([
        (person_class.id.clone(), person_class),
        (note_class.id.clone(), note_class),
        (bookmark_class.id.clone(), bookmark_class),
    ]);
    for class in directory_classes {
        classes.insert(class.id.clone(), class);
    }

    Module {
        name: MODULE_NAME.to_string(),
        constants: BTreeMap::new(),
        types: BTreeMap::new(),
        attributes,
        classes,
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
