use std::collections::BTreeMap;

use semantic_data::schema::{
    Migration, MigrationCollectionKind, MigrationDdlOperation, MigrationIntegrityMode,
    MigrationOperation, Module, Package,
    core::{type_def::TypeDef, type_kind::TypeKind},
};

use crate::{
    CoreError,
    catalog::{
        Catalog, CatalogBatchOperation, CollectionKind, IntegrityMode, nameset_for_identifier,
    },
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
    normalize_module(&mut package.root)?;
    for module in package.modules.values_mut() {
        normalize_module(module)?;
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
        apply_migration_ddl_batch(
            &mut catalog,
            &migration.module,
            migration
                .operations
                .iter()
                .filter_map(|operation| match operation {
                    MigrationOperation::Ddl(operation) => Some(operation),
                    _ => None,
                }),
        )?;
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
    apply_migration_ddl_batch(catalog, module, std::iter::once(operation))
}

pub fn apply_migration_ddl_batch<'a>(
    catalog: &mut Catalog,
    module: &str,
    operations: impl IntoIterator<Item = &'a MigrationDdlOperation>,
) -> Result<(), CoreError> {
    let catalog_ops = operations
        .into_iter()
        .map(|operation| migration_catalog_batch_operation(module, operation))
        .collect::<Result<Vec<_>, _>>()?;
    catalog
        .apply_batch(&catalog_ops)
        .map_err(|err| CoreError::new(err.to_string()))
}

fn migration_collection_kind(kind: MigrationCollectionKind) -> CollectionKind {
    match kind {
        MigrationCollectionKind::Untyped => CollectionKind::Untyped,
        MigrationCollectionKind::Schema => CollectionKind::Schema,
        MigrationCollectionKind::Polymorphic => CollectionKind::Schema,
    }
}

fn migration_integrity_mode(mode: MigrationIntegrityMode) -> IntegrityMode {
    match mode {
        MigrationIntegrityMode::Permissive => IntegrityMode::Permissive,
        MigrationIntegrityMode::StrictRegisteredSchema => IntegrityMode::StrictRegisteredSchema,
    }
}

fn migration_catalog_batch_operation(
    module: &str,
    operation: &MigrationDdlOperation,
) -> Result<CatalogBatchOperation, CoreError> {
    match operation {
        MigrationDdlOperation::UpsertAttribute { attribute } => {
            Ok(CatalogBatchOperation::UpsertAttribute {
                attribute: attribute.clone(),
                module: Some(module.to_string()),
            })
        }
        MigrationDdlOperation::DeleteAttribute { id } => {
            Ok(CatalogBatchOperation::DeleteAttribute { id: id.clone() })
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
            Ok(CatalogBatchOperation::UpsertTypeDef { type_def })
        }
        MigrationDdlOperation::DeleteTypeDef { name } => {
            Ok(CatalogBatchOperation::DeleteTypeDef { name: name.clone() })
        }
        MigrationDdlOperation::UpsertRecordType { id, name, record } => {
            Ok(CatalogBatchOperation::UpsertRecordType {
                id: id.clone(),
                name: name.clone(),
                record: record.clone(),
                module: Some(module.to_string()),
            })
        }
        MigrationDdlOperation::DeleteRecordType { id } => {
            Ok(CatalogBatchOperation::DeleteRecordType { id: id.clone() })
        }
        MigrationDdlOperation::UpsertClass { class } => Ok(CatalogBatchOperation::UpsertClass {
            class: class.clone(),
            module: Some(module.to_string()),
        }),
        MigrationDdlOperation::DeleteClass { id } => {
            Ok(CatalogBatchOperation::DeleteClass { id: id.clone() })
        }
        MigrationDdlOperation::UpsertCollection {
            name,
            kind,
            integrity_mode,
        } => Ok(CatalogBatchOperation::UpsertCollection {
            name: name.clone(),
            kind: migration_collection_kind(*kind),
            integrity_mode: migration_integrity_mode(*integrity_mode),
        }),
        MigrationDdlOperation::DeleteCollection { name } => {
            Ok(CatalogBatchOperation::DeleteCollection { name: name.clone() })
        }
        MigrationDdlOperation::UpsertIndex {
            name,
            collection,
            field,
            unique,
        } => Ok(CatalogBatchOperation::UpsertIndex {
            name: name.clone(),
            collection: collection.clone(),
            field: field.clone(),
            unique: *unique,
        }),
        MigrationDdlOperation::DeleteIndex { name, collection } => {
            Ok(CatalogBatchOperation::DeleteIndex {
                name: name.clone(),
                collection: collection.clone(),
            })
        }
        MigrationDdlOperation::UpsertRelationship { relationship } => {
            Ok(CatalogBatchOperation::UpsertRelationship {
                relationship: relationship.clone(),
            })
        }
        MigrationDdlOperation::DeleteRelationship { id } => {
            Ok(CatalogBatchOperation::DeleteRelationship { id: id.clone() })
        }
        MigrationDdlOperation::SetAutoIndex { enabled } => {
            Ok(CatalogBatchOperation::SetAutoIndex { enabled: *enabled })
        }
    }
}

fn normalize_module(module: &mut Module) -> Result<(), CoreError> {
    normalize_module_type_defs(module)?;
    normalize_module_attributes(module);
    normalize_module_classes(module);
    Ok(())
}

fn normalize_module_type_defs(module: &mut Module) -> Result<(), CoreError> {
    let mut normalized = BTreeMap::new();
    for (_, mut type_def) in std::mem::take(&mut module.types) {
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
        type_def = normalize_type_def(type_def, &module.name);
        normalized.insert(type_def.name.clone(), type_def);
    }
    module.types = normalized;
    Ok(())
}

fn normalize_module_attributes(module: &mut Module) {
    let mut normalized = BTreeMap::new();
    for (_, mut attribute) in std::mem::take(&mut module.attributes) {
        attribute.id = nameset_for_identifier(&attribute.id, Some(&module.name)).qualified_name;
        attribute.ty = normalize_type(attribute.ty, &module.name);
        normalized.insert(attribute.id.clone(), attribute);
    }
    module.attributes = normalized;
}

fn normalize_module_classes(module: &mut Module) {
    let mut normalized = BTreeMap::new();
    for (_, mut class) in std::mem::take(&mut module.classes) {
        class.id = nameset_for_identifier(&class.id, Some(&module.name)).qualified_name;
        if let Some(inherits) = class.inherits.as_mut() {
            inherits.id = nameset_for_identifier(&inherits.id, Some(&module.name)).qualified_name;
        }
        for ext in &mut class.extends {
            ext.id = nameset_for_identifier(&ext.id, Some(&module.name)).qualified_name;
        }
        for class_attr in class.attributes.values_mut() {
            class_attr.attribute.id =
                nameset_for_identifier(&class_attr.attribute.id, Some(&module.name)).qualified_name;
        }
        normalized.insert(class.id.clone(), class);
    }
    module.classes = normalized;
}

fn normalize_type_def(mut type_def: TypeDef, module: &str) -> TypeDef {
    type_def.name = nameset_for_identifier(&type_def.name, Some(module)).qualified_name;
    type_def.ty = normalize_type(type_def.ty, module);
    type_def
}

fn normalize_type(
    mut ty: semantic_data::schema::Type,
    module: &str,
) -> semantic_data::schema::Type {
    use semantic_data::schema::core::type_kind::TypeKind;

    ty.kind = match ty.kind {
        TypeKind::Optional(mut optional) => {
            optional.inner = Box::new(normalize_type(*optional.inner, module));
            TypeKind::Optional(optional)
        }
        TypeKind::Array(mut array) => {
            array.items = Box::new(normalize_type(*array.items, module));
            TypeKind::Array(array)
        }
        TypeKind::List(mut list) => {
            list.items = Box::new(normalize_type(*list.items, module));
            TypeKind::List(list)
        }
        TypeKind::Tuple(mut tuple) => {
            tuple.items = tuple
                .items
                .into_iter()
                .map(|item| normalize_type(item, module))
                .collect();
            tuple.rest = tuple
                .rest
                .map(|rest| Box::new(normalize_type(*rest, module)));
            TypeKind::Tuple(tuple)
        }
        TypeKind::Map(mut map) => {
            map.keys = Box::new(normalize_type(*map.keys, module));
            map.values = Box::new(normalize_type(*map.values, module));
            TypeKind::Map(map)
        }
        TypeKind::Ref(mut type_ref) => {
            type_ref.name = nameset_for_identifier(&type_ref.name, Some(module)).qualified_name;
            TypeKind::Ref(type_ref)
        }
        kind => kind,
    };
    ty
}

fn package_modules(package: &Package) -> BTreeMap<&str, &Module> {
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
        .filter(|&(_, type_def)| {
            type_def.type_def.module.as_deref() == Some(module)
                && !matches!(
                    type_def.type_def.ty.kind,
                    TypeKind::Attribute(_) | TypeKind::Class(_)
                )
        })
        .map(|(_, type_def)| (type_def.type_def.name.clone(), type_def.type_def.clone()))
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semantic_data::schema::{
        AttributeRef, ClassAttribute, Migration, MigrationOperation, Module,
        collections::list_type::ListType,
        core::{meta::Meta, type_node::Type, type_ref::TypeRef, visibility::Visibility},
        package::package::Package,
        record::{field::Field, record_type::RecordType},
    };

    use super::*;

    #[test]
    fn validate_package_migrations_supports_recursive_schema_batches() {
        let package = Package {
            name: "suite.tree".to_string(),
            root: Module {
                name: "tree".to_string(),
                constants: BTreeMap::new(),
                types: BTreeMap::from([(
                    "suite.tree.payload_type".to_string(),
                    recursive_payload_type_def(Some("tree".to_string())),
                )]),
                attributes: BTreeMap::from([(
                    "suite.tree.payload".to_string(),
                    recursive_payload_attribute(),
                )]),
                classes: BTreeMap::from([("suite.tree.node".to_string(), recursive_node_class())]),
                interfaces: BTreeMap::new(),
                contracts: BTreeMap::new(),
                meta: Meta::default(),
            },
            modules: BTreeMap::new(),
            migrations: vec![Migration {
                module: "tree".to_string(),
                name: "001_init".to_string(),
                description: None,
                operations: vec![
                    MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass {
                        class: recursive_node_class(),
                    }),
                    MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                        attribute: recursive_payload_attribute(),
                    }),
                    MigrationOperation::Ddl(MigrationDdlOperation::UpsertTypeDef {
                        type_def: recursive_payload_type_def(Some("tree".to_string())),
                    }),
                ],
                meta: Meta::default(),
            }],
            version: None,
            meta: Meta::default(),
        };

        validate_package_migrations(&package).unwrap();
    }

    fn recursive_node_class() -> semantic_data::schema::ClassType {
        semantic_data::schema::ClassType {
            id: "suite.tree.node".to_string(),
            name: "TreeNode".to_string(),
            inherits: None,
            extends: vec![],
            attributes: BTreeMap::from([(
                "payload".to_string(),
                ClassAttribute {
                    attribute: AttributeRef {
                        id: "suite.tree.payload".to_string(),
                    },
                    required: false,
                    constraints: vec![],
                    meta: Meta::default(),
                },
            )]),
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn recursive_payload_attribute() -> semantic_data::schema::AttributeType {
        semantic_data::schema::AttributeType {
            id: "suite.tree.payload".to_string(),
            name: "payload".to_string(),
            ty: Type {
                kind: TypeKind::Ref(TypeRef {
                    name: "suite.tree.payload_type".to_string(),
                    args: vec![],
                }),
                constraints: vec![],
                annotations: vec![],
            },
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn recursive_payload_type_def(module: Option<String>) -> TypeDef {
        TypeDef {
            name: "suite.tree.payload_type".to_string(),
            module,
            params: Vec::new(),
            ty: Type {
                kind: TypeKind::Record(RecordType {
                    fields: BTreeMap::from([(
                        "children".to_string(),
                        Field {
                            ty: Type {
                                kind: TypeKind::List(ListType {
                                    items: Box::new(Type {
                                        kind: TypeKind::Ref(TypeRef {
                                            name: "suite.tree.payload_type".to_string(),
                                            args: vec![],
                                        }),
                                        constraints: vec![],
                                        annotations: vec![],
                                    }),
                                }),
                                constraints: vec![],
                                annotations: vec![],
                            },
                            required: false,
                            readonly: false,
                            writeonly: false,
                            default: None,
                            meta: Meta::default(),
                        },
                    )]),
                    open: false,
                    additional: None,
                    required_order: None,
                }),
                constraints: vec![],
                annotations: vec![],
            },
            visibility: Visibility::Public,
            meta: Meta {
                title: Some("TreePayload".to_string()),
                ..Meta::default()
            },
        }
    }
}
