use std::collections::BTreeMap;

use semantic_data::schema::{
    Migration, MigrationCollectionKind, MigrationDdlOperation, MigrationIntegrityMode,
    MigrationOperation, Module, Package,
    core::{type_def::TypeDef, type_kind::TypeKind},
};

use crate::{
    CoreError,
    catalog::{Catalog, CollectionKind, IntegrityMode},
    fresh_catalog_with_core_schema,
};

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct AppliedMigration {
    pub package: String,
    pub migration: Migration,
}

impl AppliedMigration {
    pub fn key(&self) -> String {
        applied_migration_key(&self.package, &self.migration.module, &self.migration.name)
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct PackageRegistrationOutcome {
    pub executed_migrations: Vec<AppliedMigration>,
}

pub fn applied_migration_key(package: &str, module: &str, name: &str) -> String {
    format!("{package}::{module}::{name}")
}

pub fn normalize_package_definition(package: &Package) -> Result<Package, CoreError> {
    let mut package = package.clone();
    normalize_module_type_defs(&mut package.root)?;
    for module in package.modules.values_mut() {
        normalize_module_type_defs(module)?;
    }
    Ok(package)
}

pub fn validate_package_migrations(package: &Package) -> Result<(), CoreError> {
    let package = normalize_package_definition(package)?;
    let modules = package_modules(&package);
    let mut catalog = fresh_catalog_with_core_schema()?;

    for migration in &package.migrations {
        if !modules.contains_key(migration.module.as_str()) {
            return Err(CoreError::new(format!(
                "package '{}' migration '{}::{}' references unknown module '{}'",
                package.name, migration.module, migration.name, migration.module
            )));
        }
        for operation in &migration.operations {
            if let MigrationOperation::Ddl(operation) = operation {
                apply_migration_ddl_operation(&mut catalog, &migration.module, operation)?;
            }
        }
    }

    for module in modules.values() {
        let actual_types = actual_module_types(&catalog, &module.name);
        if module.types != actual_types {
            return Err(CoreError::new(format!(
                "module '{}' type definitions do not match migration result",
                module.name
            )));
        }

        let actual_attributes = actual_module_attributes(&catalog, &module.name);
        if module.attributes != actual_attributes {
            return Err(CoreError::new(format!(
                "module '{}' attributes do not match migration result",
                module.name
            )));
        }

        let actual_classes = actual_module_classes(&catalog, &module.name);
        if module.classes != actual_classes {
            return Err(CoreError::new(format!(
                "module '{}' classes do not match migration result",
                module.name
            )));
        }
    }

    Ok(())
}

pub fn apply_migration_ddl_operation(
    catalog: &mut Catalog,
    module: &str,
    operation: &MigrationDdlOperation,
) -> Result<(), CoreError> {
    match operation {
        MigrationDdlOperation::UpsertAttribute { attribute } => {
            catalog.upsert_attribute_with_module(attribute.clone(), Some(module.to_string()));
        }
        MigrationDdlOperation::DeleteAttribute { id } => {
            let _ = catalog.delete_attribute(id);
        }
        MigrationDdlOperation::UpsertTypeDef { type_def } => {
            let mut type_def = type_def.clone();
            match &type_def.module {
                Some(existing) if existing != module => {
                    return Err(CoreError::new(format!(
                        "type definition '{}' is declared for module '{}' but migration targets module '{}'",
                        type_def.name, existing, module
                    )));
                }
                Some(_) => {}
                None => type_def.module = Some(module.to_string()),
            }
            let _ = catalog.upsert_type_def(type_def);
        }
        MigrationDdlOperation::DeleteTypeDef { name } => {
            let _ = catalog.delete_type_def(name);
        }
        MigrationDdlOperation::UpsertRecordType { id, name, record } => {
            catalog.upsert_record_type_with_module(
                id.clone(),
                name.clone(),
                record.clone(),
                Some(module.to_string()),
            );
        }
        MigrationDdlOperation::DeleteRecordType { id } => {
            let _ = catalog.delete_record_type(id);
        }
        MigrationDdlOperation::UpsertClass { class } => {
            catalog
                .upsert_class_with_module(class.clone(), Some(module.to_string()))
                .map_err(|err| CoreError::new(err.to_string()))?;
        }
        MigrationDdlOperation::DeleteClass { id } => {
            let _ = catalog.delete_class(id);
        }
        MigrationDdlOperation::UpsertCollection {
            name,
            kind,
            integrity_mode,
        } => {
            catalog
                .upsert_collection(
                    name.clone(),
                    migration_collection_kind(*kind),
                    migration_integrity_mode(*integrity_mode),
                )
                .map_err(|err| CoreError::new(err.to_string()))?;
        }
        MigrationDdlOperation::DeleteCollection { name } => {
            let _ = catalog.delete_collection(name);
        }
        MigrationDdlOperation::UpsertIndex {
            name,
            collection,
            field,
            unique,
        } => {
            let Some(collection_schema) = catalog.collection_by_name(collection) else {
                return Err(CoreError::new(format!(
                    "collection '{collection}' not found"
                )));
            };
            let _ = catalog
                .upsert_index(name.clone(), collection_schema.lid, field.clone(), *unique)
                .map_err(|err| CoreError::new(err.to_string()))?;
        }
        MigrationDdlOperation::DeleteIndex { name, collection } => {
            let Some(collection_schema) = catalog.collection_by_name(collection) else {
                return Err(CoreError::new(format!(
                    "collection '{collection}' not found"
                )));
            };
            let _ = catalog.delete_index(collection_schema.lid, name);
        }
        MigrationDdlOperation::UpsertRelationship { relationship } => {
            let _ = catalog
                .upsert_relationship(relationship.clone())
                .map_err(|err| CoreError::new(err.to_string()))?;
        }
        MigrationDdlOperation::DeleteRelationship { id } => {
            let _ = catalog.delete_relationship(id);
        }
        MigrationDdlOperation::SetAutoIndex { enabled } => {
            catalog.set_auto_index_enabled(*enabled);
        }
    }
    Ok(())
}

fn migration_collection_kind(kind: MigrationCollectionKind) -> CollectionKind {
    match kind {
        MigrationCollectionKind::Polymorphic => CollectionKind::Polymorphic,
    }
}

fn migration_integrity_mode(mode: MigrationIntegrityMode) -> IntegrityMode {
    match mode {
        MigrationIntegrityMode::Permissive => IntegrityMode::Permissive,
        MigrationIntegrityMode::StrictRegisteredSchema => IntegrityMode::StrictRegisteredSchema,
    }
}

fn normalize_module_type_defs(module: &mut Module) -> Result<(), CoreError> {
    for type_def in module.types.values_mut() {
        match &type_def.module {
            Some(existing) if existing != &module.name => {
                return Err(CoreError::new(format!(
                    "type definition '{}' is declared for module '{}' but stored in module '{}'",
                    type_def.name, existing, module.name
                )));
            }
            Some(_) => {}
            None => type_def.module = Some(module.name.clone()),
        }
    }
    Ok(())
}

fn package_modules<'a>(package: &'a Package) -> BTreeMap<&'a str, &'a Module> {
    let mut out = BTreeMap::new();
    let _ = out.insert(package.root.name.as_str(), &package.root);
    for module in package.modules.values() {
        let _ = out.insert(module.name.as_str(), module);
    }
    out
}

fn actual_module_types(catalog: &Catalog, module: &str) -> BTreeMap<String, TypeDef> {
    catalog
        .type_defs()
        .filter_map(|(_, type_def)| {
            (type_def.type_def.module.as_deref() == Some(module)
                && !matches!(
                    type_def.type_def.ty.kind,
                    TypeKind::Attribute(_) | TypeKind::Class(_)
                ))
            .then(|| (type_def.type_def.name.clone(), type_def.type_def.clone()))
        })
        .collect()
}

fn actual_module_attributes(
    catalog: &Catalog,
    module: &str,
) -> BTreeMap<String, semantic_data::schema::AttributeType> {
    catalog
        .type_defs()
        .filter_map(|(_, type_def)| {
            if type_def.type_def.module.as_deref() != Some(module) {
                return None;
            }
            match &type_def.type_def.ty.kind {
                TypeKind::Attribute(attribute) => {
                    Some((attribute.id.clone(), attribute.as_ref().clone()))
                }
                _ => None,
            }
        })
        .collect()
}

fn actual_module_classes(
    catalog: &Catalog,
    module: &str,
) -> BTreeMap<String, semantic_data::schema::ClassType> {
    catalog
        .type_defs()
        .filter_map(|(_, type_def)| {
            if type_def.type_def.module.as_deref() != Some(module) {
                return None;
            }
            match &type_def.type_def.ty.kind {
                TypeKind::Class(class) => Some((class.id.clone(), class.clone())),
                _ => None,
            }
        })
        .collect()
}
