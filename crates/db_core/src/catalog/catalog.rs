use std::collections::{BTreeMap, BTreeSet};

use fnv::FnvHashMap;
use semantic_data::schema::{
    IndexKind, Package, VariantPayload,
    attribute::attribute_type::AttributeType,
    class::class_type::ClassType,
    collections::key_path::KeyPath,
    core::{
        meta::Meta, type_def::TypeDef, type_kind::TypeKind, type_node::Type, type_param::TypeParam,
        type_ref::TypeRef, visibility::Visibility,
    },
    primitives::string_type::StringType,
    record::record_type::RecordType,
    relation::relation_mode::RelationMode,
    relation::relation_type::RelationType,
};

use crate::AppliedMigration;
use crate::catalog::{
    AttributeSchema, CatalogError, CatalogStorageSnapshot, ClassSchema, CollectionKind,
    CollectionSchema, IMPLICIT_ROOT_PACKAGE, IdMap, IndexSchema, IntegrityMode, LocalAttrId,
    LocalClassId, LocalCollectionId, LocalFieldId, LocalIndexId, LocalPackageId, LocalRecordTypeId,
    LocalRelationId, LocalTypeDefId, NameSet, RecordTypeSchema, RelationshipSchema,
    StoredAppliedMigration, StoredAttribute, StoredClass, StoredCollection, StoredFieldId,
    StoredIndex, StoredPackage, StoredRecordType, StoredRelationship, StoredTypeDef, TypeDefSchema,
    is_special_builtin_field, nameset_for_identifier, nameset_for_qualified,
};

#[derive(Debug, Clone)]
pub struct Catalog {
    attributes: IdMap<LocalAttrId, AttributeSchema>,
    type_defs: IdMap<LocalTypeDefId, TypeDefSchema>,
    record_types: IdMap<LocalRecordTypeId, RecordTypeSchema>,
    classes: IdMap<LocalClassId, ClassSchema>,
    collections: IdMap<LocalCollectionId, CollectionSchema>,
    indexes: IdMap<LocalIndexId, IndexSchema>,
    relationships: IdMap<LocalRelationId, RelationshipSchema>,
    packages: IdMap<LocalPackageId, Package>,
    applied_migrations: BTreeMap<String, AppliedMigration>,
    collection_indexes: FnvHashMap<LocalCollectionId, Vec<LocalIndexId>>,
    next_field_id: usize,
    auto_index_enabled: bool,
}

pub const PRIMARY_ID_FIELD: &str = semantic_data::builtin::ATTR_ID;
pub const OBJECT_TYPE_FIELD: &str = semantic_data::builtin::ATTR_TYPE;
pub const PARENT_RELATION_FIELD: &str = "parent";
pub const ATTR_PARENT_RELATION: &str = "semantic:parent";
pub const PRIMARY_ID_INDEX_NAME: &str = "__builtin_pk_id";
pub const OBJECT_TYPE_INDEX_NAME: &str = "__builtin_type";
pub const BUILTIN_PARENT_RELATION_ID: &str = "__builtin.parent";
pub const RELATION_CLASS_ID: &str = "semantic:relation";
pub const ATTR_RELATION_RELATION: &str = "semantic:relation:relation";
pub const ATTR_RELATION_FROM: &str = "semantic:relation:from";
pub const ATTR_RELATION_TO: &str = "semantic:relation:to";
pub const AUTO_PATH_INDEX_NAME: &str = "__auto_index_all_paths";
pub const AUTO_PATH_INDEX_FIELD: &str = "__path__";

#[derive(Debug, Clone, PartialEq)]
pub enum CatalogBatchOperation {
    UpsertAttribute {
        attribute: AttributeType,
        module: Option<String>,
    },
    DeleteAttribute {
        id: String,
    },
    UpsertTypeDef {
        type_def: TypeDef,
    },
    DeleteTypeDef {
        name: String,
    },
    UpsertRecordType {
        id: String,
        name: String,
        record: RecordType,
        module: Option<String>,
    },
    DeleteRecordType {
        id: String,
    },
    UpsertClass {
        class: ClassType,
        module: Option<String>,
    },
    DeleteClass {
        id: String,
    },
    UpsertCollection {
        name: String,
        kind: CollectionKind,
        integrity_mode: IntegrityMode,
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
        relationship: RelationType,
    },
    DeleteRelationship {
        id: String,
    },
    SetAutoIndex {
        enabled: bool,
    },
}

impl Catalog {
    pub fn new() -> Self {
        Self {
            attributes: IdMap::new(),
            type_defs: IdMap::new(),
            record_types: IdMap::new(),
            classes: IdMap::new(),
            collections: IdMap::new(),
            indexes: IdMap::new(),
            relationships: IdMap::new(),
            packages: IdMap::new(),
            applied_migrations: BTreeMap::new(),
            collection_indexes: FnvHashMap::default(),
            next_field_id: 0,
            auto_index_enabled: true,
        }
    }

    fn allocate_field_id(&mut self) -> LocalFieldId {
        let out = LocalFieldId(self.next_field_id);
        self.next_field_id += 1;
        out
    }

    fn ensure_next_field_id(&mut self, field_id: LocalFieldId) {
        self.next_field_id = self.next_field_id.max(field_id.0.saturating_add(1));
    }

    pub fn attributes(&self) -> impl Iterator<Item = (LocalAttrId, &AttributeSchema)> {
        self.attributes.iter()
    }

    pub fn record_types(&self) -> impl Iterator<Item = (LocalRecordTypeId, &RecordTypeSchema)> {
        self.record_types.iter()
    }

    pub fn type_defs(&self) -> impl Iterator<Item = (LocalTypeDefId, &TypeDefSchema)> {
        self.type_defs.iter()
    }

    pub fn classes(&self) -> impl Iterator<Item = (LocalClassId, &ClassSchema)> {
        self.classes.iter()
    }

    pub fn indexes(&self) -> impl Iterator<Item = (LocalIndexId, &IndexSchema)> {
        self.indexes.iter()
    }

    pub fn relationships(&self) -> impl Iterator<Item = (LocalRelationId, &RelationshipSchema)> {
        self.relationships.iter()
    }

    pub fn packages(&self) -> impl Iterator<Item = (LocalPackageId, &Package)> {
        self.packages.iter()
    }

    pub fn package_by_name(&self, name: &str) -> Option<&Package> {
        self.packages.get_key(name)
    }

    pub fn upsert_package(&mut self, package: Package) {
        let names = package_nameset(&package.name);
        let _ = self.packages.insert(names, |_| package);
    }

    pub fn applied_migrations(&self) -> impl Iterator<Item = (&String, &AppliedMigration)> {
        self.applied_migrations.iter()
    }

    pub fn applied_migration(
        &self,
        package: &str,
        module: &str,
        name: &str,
    ) -> Option<&AppliedMigration> {
        self.applied_migrations
            .get(&crate::applied_migration_key(package, module, name))
    }

    pub fn record_applied_migration(&mut self, applied: AppliedMigration) {
        let _ = self.applied_migrations.insert(applied.key(), applied);
    }

    pub fn apply_batch(
        &mut self,
        operations: &[CatalogBatchOperation],
    ) -> Result<(), CatalogError> {
        let mut pending_type_ops = Vec::<&CatalogBatchOperation>::new();
        for operation in operations {
            if Self::is_type_operation(operation) {
                pending_type_ops.push(operation);
                continue;
            }

            self.flush_pending_type_operations(&mut pending_type_ops)?;

            match operation {
                CatalogBatchOperation::UpsertCollection {
                    name,
                    kind,
                    integrity_mode,
                } => {
                    let _ = self.upsert_collection(name.clone(), kind.clone(), *integrity_mode)?;
                }
                CatalogBatchOperation::DeleteCollection { name } => {
                    let _ = self.delete_collection(name);
                }
                CatalogBatchOperation::UpsertIndex {
                    name,
                    collection,
                    field,
                    unique,
                } => {
                    let collection_schema =
                        self.collection_by_name(collection).ok_or_else(|| {
                            CatalogError::InvalidSchema(format!(
                                "collection '{collection}' not found"
                            ))
                        })?;
                    let _ = self.upsert_index(
                        name.clone(),
                        collection_schema.lid,
                        field.clone(),
                        *unique,
                    )?;
                }
                CatalogBatchOperation::DeleteIndex { name, collection } => {
                    let collection_schema =
                        self.collection_by_name(collection).ok_or_else(|| {
                            CatalogError::InvalidSchema(format!(
                                "collection '{collection}' not found"
                            ))
                        })?;
                    let _ = self.delete_index(collection_schema.lid, name);
                }
                CatalogBatchOperation::UpsertRelationship { relationship } => {
                    let _ = self.upsert_relationship(relationship.clone())?;
                }
                CatalogBatchOperation::DeleteRelationship { id } => {
                    let _ = self.delete_relationship(id);
                }
                CatalogBatchOperation::SetAutoIndex { enabled } => {
                    self.set_auto_index_enabled(*enabled);
                }
                CatalogBatchOperation::UpsertAttribute { .. }
                | CatalogBatchOperation::DeleteAttribute { .. }
                | CatalogBatchOperation::UpsertTypeDef { .. }
                | CatalogBatchOperation::DeleteTypeDef { .. }
                | CatalogBatchOperation::UpsertRecordType { .. }
                | CatalogBatchOperation::DeleteRecordType { .. }
                | CatalogBatchOperation::UpsertClass { .. }
                | CatalogBatchOperation::DeleteClass { .. } => {
                    unreachable!("type operations are flushed before immediate catalog ops");
                }
            }
        }

        self.flush_pending_type_operations(&mut pending_type_ops)
    }

    pub fn register_attribute(&mut self, attr: AttributeType) -> LocalAttrId {
        self.upsert_attribute(attr)
    }

    pub fn upsert_attribute(&mut self, attr: AttributeType) -> LocalAttrId {
        self.upsert_attribute_with_module(attr, None)
    }

    pub fn upsert_attribute_with_module(
        &mut self,
        attr: AttributeType,
        module: Option<String>,
    ) -> LocalAttrId {
        let key = attr.id.clone();
        self.upsert_type_def(type_def_from_attribute(&attr, module));
        self.attribute_id(&key)
            .expect("attribute projection must exist for attribute type-def")
    }

    pub fn delete_attribute(&mut self, id: &str) -> bool {
        self.delete_type_def(id)
    }

    pub fn attribute_id(&self, id: &str) -> Option<LocalAttrId> {
        self.attributes.get_key_id(id)
    }

    pub fn attribute_ids(&self, id: &str) -> Vec<LocalAttrId> {
        self.attributes.get_key_ids(id)
    }

    pub fn attribute_by_id(&self, id: &str) -> Option<&AttributeSchema> {
        self.attributes.get_key(id)
    }

    pub fn attribute_by_lid(&self, lid: LocalAttrId) -> Option<&AttributeSchema> {
        self.attributes.get(lid)
    }

    pub fn register_type_def(&mut self, type_def: TypeDef) -> LocalTypeDefId {
        self.upsert_type_def(type_def)
    }

    pub fn upsert_type_def(&mut self, type_def: TypeDef) -> LocalTypeDefId {
        let type_def = normalize_type_def(type_def);
        let key = type_def.name.clone();
        let key_names = nameset_for_qualified(&key);
        match &type_def.ty.kind {
            TypeKind::Attribute(attribute) => {
                let attr_names = nameset_for_qualified(&key);
                self.attributes.insert(attr_names, |lid| AttributeSchema {
                    lid,
                    names: nameset_for_qualified(&key),
                    attribute: (*attribute.clone()),
                });
                let _ = self.record_types.remove_key(&key);
                let _ = self.classes.remove_key(&key);
            }
            TypeKind::Record(record) => {
                let record_names = nameset_for_qualified(&key);
                self.record_types
                    .insert(record_names, |lid| RecordTypeSchema {
                        lid,
                        names: nameset_for_qualified(&key),
                        id: key.clone(),
                        name: type_def.meta.title.clone().unwrap_or_else(|| key.clone()),
                        record: record.clone(),
                    });
                let _ = self.attributes.remove_key(&key);
                let _ = self.classes.remove_key(&key);
            }
            TypeKind::Class(class) => {
                let mut class_attributes = BTreeMap::new();
                let mut unresolved = false;
                for (alias, class_attr) in &class.attributes {
                    let Some(attr) = self.attribute_by_id(&class_attr.attribute.id) else {
                        unresolved = true;
                        break;
                    };
                    class_attributes.insert(alias.clone(), attr.lid);
                    class_attributes.insert(class_attr.attribute.id.clone(), attr.lid);
                    class_attributes.insert(attr.names.plain_name.clone(), attr.lid);
                    class_attributes.insert(attr.names.underscore_name.clone(), attr.lid);
                }
                if unresolved {
                    let _ = self.classes.remove_key(&key);
                } else {
                    let class_names = nameset_for_qualified(&key);
                    self.classes.insert(class_names, |lid| ClassSchema {
                        lid,
                        names: nameset_for_qualified(&key),
                        class: class.clone(),
                        attributes: class_attributes,
                    });
                }
                let _ = self.attributes.remove_key(&key);
                let _ = self.record_types.remove_key(&key);
            }
            _ => {
                let _ = self.attributes.remove_key(&key);
                let _ = self.record_types.remove_key(&key);
                let _ = self.classes.remove_key(&key);
            }
        }
        self.type_defs
            .insert(key_names.clone(), |lid| TypeDefSchema {
                lid,
                names: key_names.clone(),
                type_def,
            })
    }

    pub fn delete_type_def(&mut self, name: &str) -> bool {
        let removed = self.type_defs.remove_key(name).is_some();
        let _ = self.attributes.remove_key(name);
        let _ = self.record_types.remove_key(name);
        let _ = self.classes.remove_key(name);
        removed
    }

    pub fn type_def_id(&self, name: &str) -> Option<LocalTypeDefId> {
        self.type_defs.get_key_id(name)
    }

    pub fn type_def_by_name(&self, name: &str) -> Option<&TypeDefSchema> {
        self.type_defs.get_key(name)
    }

    pub fn type_def_by_lid(&self, lid: LocalTypeDefId) -> Option<&TypeDefSchema> {
        self.type_defs.get(lid)
    }

    pub fn register_record_type(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        record: RecordType,
    ) -> LocalRecordTypeId {
        self.upsert_record_type(id, name, record)
    }

    pub fn upsert_record_type(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        record: RecordType,
    ) -> LocalRecordTypeId {
        self.upsert_record_type_with_module(id, name, record, None)
    }

    pub fn upsert_record_type_with_module(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        record: RecordType,
        module: Option<String>,
    ) -> LocalRecordTypeId {
        let id = id.into();
        let key = id.clone();
        let name = name.into();
        self.upsert_type_def(type_def_from_record_type(id, name, record, module));
        self.record_type_id(&key)
            .expect("record projection must exist for record type-def")
    }

    pub fn delete_record_type(&mut self, id: &str) -> bool {
        self.delete_type_def(id)
    }

    pub fn record_type_id(&self, id: &str) -> Option<LocalRecordTypeId> {
        self.record_types.get_key_id(id)
    }

    pub fn record_type_ids(&self, id: &str) -> Vec<LocalRecordTypeId> {
        self.record_types.get_key_ids(id)
    }

    pub fn record_type_by_lid(&self, lid: LocalRecordTypeId) -> Option<&RecordTypeSchema> {
        self.record_types.get(lid)
    }

    pub fn register_class(&mut self, class: ClassType) -> Result<LocalClassId, CatalogError> {
        self.upsert_class(class)
    }

    pub fn upsert_class(&mut self, class: ClassType) -> Result<LocalClassId, CatalogError> {
        self.upsert_class_with_module(class, None)
    }

    pub fn upsert_class_with_module(
        &mut self,
        class: ClassType,
        module: Option<String>,
    ) -> Result<LocalClassId, CatalogError> {
        for class_attr in class.attributes.values() {
            if self.attribute_by_id(&class_attr.attribute.id).is_none() {
                return Err(CatalogError::UnknownAttribute {
                    id: class_attr.attribute.id.clone(),
                });
            }
        }
        let key = class.id.clone();
        self.upsert_type_def(type_def_from_class(class, module));
        self.class_id(&key).ok_or(CatalogError::InvalidSchema(
            "class projection must have local id".to_string(),
        ))
    }

    pub fn delete_class(&mut self, id: &str) -> bool {
        self.delete_type_def(id)
    }

    pub fn class_id(&self, id: &str) -> Option<LocalClassId> {
        self.classes.get_key_id(id)
    }

    pub fn class_ids(&self, id: &str) -> Vec<LocalClassId> {
        self.classes.get_key_ids(id)
    }

    pub fn class_by_lid(&self, lid: LocalClassId) -> Option<&ClassSchema> {
        self.classes.get(lid)
    }

    pub fn class_field_for_alias(&self, class_lid: LocalClassId, alias: &str) -> Option<String> {
        fn visit(
            catalog: &Catalog,
            class_lid: LocalClassId,
            alias: &str,
            visited: &mut BTreeSet<LocalClassId>,
        ) -> Option<String> {
            if !visited.insert(class_lid) {
                return None;
            }

            let class = catalog.class_by_lid(class_lid)?;
            let mut out = None;

            if let Some(inherits) = &class.class.inherits
                && let Some(base_lid) = catalog.class_id(&inherits.id)
            {
                out = visit(catalog, base_lid, alias, visited);
            }

            for ext in &class.class.extends {
                if let Some(ext_lid) = catalog.class_id(&ext.id)
                    && let Some(field) = visit(catalog, ext_lid, alias, visited)
                {
                    out = Some(field);
                }
            }

            for (field_alias, class_attr) in &class.class.attributes {
                let Some(attr) = catalog.attribute_by_id(&class_attr.attribute.id) else {
                    continue;
                };
                if field_alias == alias
                    || attr.attribute.id == alias
                    || attr.names.plain_name == alias
                    || attr.names.underscore_name == alias
                {
                    out = Some(attr.attribute.id.clone());
                }
            }

            out
        }

        visit(self, class_lid, alias, &mut BTreeSet::new())
    }

    pub fn class_named_collection_field_for_alias(
        &self,
        collection: &CollectionSchema,
        alias: &str,
    ) -> Option<String> {
        let class_ids = self.class_ids(&collection.name);
        if class_ids.len() != 1 {
            return None;
        }
        self.class_field_for_alias(class_ids[0], alias)
    }

    pub fn register_collection(
        &mut self,
        name: impl Into<String>,
        kind: CollectionKind,
    ) -> Result<LocalCollectionId, CatalogError> {
        let name = name.into();
        if self.collections.contains_key(&name) {
            return Err(CatalogError::CollectionAlreadyExists { name });
        }
        self.upsert_collection(name, kind, IntegrityMode::Permissive)
    }

    pub fn upsert_collection(
        &mut self,
        name: impl Into<String>,
        kind: CollectionKind,
        integrity_mode: IntegrityMode,
    ) -> Result<LocalCollectionId, CatalogError> {
        let name = name.into();
        let existing_lid = self.collections.get_key_id(&name);
        let lid = existing_lid.unwrap_or(self.collections.next_id());
        let internal = existing_lid
            .and_then(|id| self.collections.get(id))
            .is_some_and(|schema| schema.internal);
        let existing_field_ids =
            existing_lid
                .and_then(|id| self.collections.get(id))
                .map(|schema| {
                    schema
                        .fields()
                        .map(|(field_id, field_name)| (field_name.to_string(), field_id))
                        .collect::<FnvHashMap<_, _>>()
                });
        let schema = self.build_collection_schema_for_lid(
            lid,
            name.clone(),
            kind,
            integrity_mode,
            internal,
            existing_field_ids.as_ref(),
        )?;

        self.collections
            .insert_fixed(lid, verbatim_nameset(&name), schema);
        // Ensure builtin indexes are always present and flow through normal index machinery.
        let _ = self.upsert_index(PRIMARY_ID_INDEX_NAME, lid, PRIMARY_ID_FIELD, true)?;
        let _ = self.upsert_index(OBJECT_TYPE_INDEX_NAME, lid, OBJECT_TYPE_FIELD, false)?;
        if self.auto_index_enabled {
            let _ = self.upsert_path_index(lid)?;
        }
        let _ = self.upsert_builtin_parent_relationship(lid)?;
        Ok(lid)
    }

    pub fn set_collection_internal(
        &mut self,
        name: &str,
        internal: bool,
    ) -> Result<(), CatalogError> {
        let lid = self
            .collections
            .get_key_id(name)
            .ok_or_else(|| CatalogError::InvalidSchema(format!("unknown collection '{name}'")))?;
        let Some(collection) = self.collections.get_mut(lid) else {
            return Err(CatalogError::InvalidSchema(format!(
                "unknown collection '{name}'"
            )));
        };
        collection.internal = internal;
        Ok(())
    }

    pub fn delete_collection(&mut self, name: &str) -> bool {
        let Some(collection_lid) = self.collections.get_key_id(name) else {
            return false;
        };

        if let Some(index_ids) = self.collection_indexes.remove(&collection_lid) {
            for index_id in index_ids {
                if let Some(index) = self.indexes.get(index_id) {
                    let key = Self::index_key(&index.schema.collection, &index.schema.name);
                    let _ = self.indexes.remove_key(&key);
                }
            }
        }
        self.collections.remove_key(name).is_some()
    }

    pub fn upsert_relationship(
        &mut self,
        relationship: RelationType,
    ) -> Result<LocalRelationId, CatalogError> {
        self.validate_relationship(&relationship)?;
        let key = relationship.id.clone();
        Ok(self
            .relationships
            .insert(verbatim_nameset(&key), |lid| RelationshipSchema {
                lid,
                relationship,
            }))
    }

    fn validate_relationship(&self, relationship: &RelationType) -> Result<(), CatalogError> {
        let source_collection = self
            .collection_by_name(&relationship.source_collection)
            .ok_or_else(|| {
                CatalogError::InvalidSchema(format!(
                    "relationship '{}' references unknown source collection '{}'",
                    relationship.id, relationship.source_collection
                ))
            })?
            .clone();
        match &relationship.mode {
            RelationMode::Embedded { attribute } => {
                let attr = self.attribute_by_id(attribute).ok_or_else(|| {
                    CatalogError::InvalidSchema(format!(
                        "relationship '{}' references unknown attribute '{}'",
                        relationship.id, attribute
                    ))
                })?;
                if !matches!(attr.attribute.ty.kind, TypeKind::Ref(_)) {
                    return Err(CatalogError::InvalidSchema(format!(
                        "relationship '{}' requires embedded attribute '{}' to have type 'ref'",
                        relationship.id, attribute
                    )));
                }
                let canonical = source_collection.canonical_field_name(attribute);
                if source_collection.is_closed_field_set()
                    && !source_collection.knows_field(canonical)
                {
                    return Err(CatalogError::InvalidSchema(format!(
                        "relationship '{}' references unknown embedded field '{}' in source collection '{}'",
                        relationship.id, canonical, source_collection.name
                    )));
                }
            }
            RelationMode::External => {
                for field in [ATTR_RELATION_FROM, ATTR_RELATION_TO] {
                    let canonical = source_collection.canonical_field_name(field);
                    if source_collection.is_closed_field_set()
                        && !source_collection.knows_field(canonical)
                    {
                        return Err(CatalogError::InvalidSchema(format!(
                            "external relationship '{}' requires field '{}' in source collection '{}'",
                            relationship.id, canonical, source_collection.name
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn delete_relationship(&mut self, id: &str) -> bool {
        self.relationships.remove_key(id).is_some()
    }

    pub fn relationship_by_id(&self, id: &str) -> Option<&RelationshipSchema> {
        self.relationships.get_key(id)
    }

    pub fn collection_by_name(&self, name: &str) -> Option<&CollectionSchema> {
        self.collections.get_key(name)
    }

    pub fn collection_by_lid(&self, lid: LocalCollectionId) -> Option<&CollectionSchema> {
        self.collections.get(lid)
    }

    pub fn collections(&self) -> impl Iterator<Item = (LocalCollectionId, &CollectionSchema)> {
        self.collections.iter()
    }

    pub fn register_index(
        &mut self,
        name: impl Into<String>,
        collection: LocalCollectionId,
        field: impl Into<String>,
        unique: bool,
    ) -> Result<LocalIndexId, CatalogError> {
        self.upsert_index(name, collection, field, unique)
    }

    pub fn upsert_index(
        &mut self,
        name: impl Into<String>,
        collection: LocalCollectionId,
        field: impl Into<String>,
        unique: bool,
    ) -> Result<LocalIndexId, CatalogError> {
        self.upsert_index_with_kind(name, collection, field, unique, IndexKind::Equality)
    }

    pub fn upsert_index_with_kind(
        &mut self,
        name: impl Into<String>,
        collection: LocalCollectionId,
        field: impl Into<String>,
        unique: bool,
        kind: IndexKind,
    ) -> Result<LocalIndexId, CatalogError> {
        let name = name.into();
        let collection_schema = self
            .collections
            .get(collection)
            .ok_or(CatalogError::UnknownCollection(collection))?;
        let key = Self::index_key(&collection_schema.name, &name);
        let lid = self
            .indexes
            .get_key_id(&key)
            .unwrap_or_else(|| self.indexes.next_id());
        let index =
            self.build_index_schema_for_lid(lid, name, collection, field.into(), unique, kind)?;

        self.indexes
            .insert_fixed(lid, verbatim_nameset(&key), index);
        let ids = self.collection_indexes.entry(collection).or_default();
        if !ids.contains(&lid) {
            ids.push(lid);
        }
        Ok(lid)
    }

    fn build_index_schema_for_lid(
        &self,
        lid: LocalIndexId,
        name: String,
        collection: LocalCollectionId,
        field: String,
        unique: bool,
        kind: IndexKind,
    ) -> Result<IndexSchema, CatalogError> {
        let collection_schema = self
            .collections
            .get(collection)
            .ok_or(CatalogError::UnknownCollection(collection))?;
        let canonical_field = collection_schema.canonical_field_name(&field).to_string();
        if kind == IndexKind::Equality
            && collection_schema.is_closed_field_set()
            && !collection_schema.knows_field(&canonical_field)
        {
            return Err(CatalogError::InvalidSchema(format!(
                "cannot create index on unknown field '{canonical_field}' in collection '{}'",
                collection_schema.name
            )));
        }

        Ok(IndexSchema {
            lid,
            schema: semantic_data::schema::IndexSchema {
                id: format!("{}.{}", collection_schema.name, name),
                name: name.clone(),
                kind,
                collection: collection_schema.name.clone(),
                key_path: KeyPath {
                    segments: if kind == IndexKind::PathEquality {
                        vec![AUTO_PATH_INDEX_FIELD.to_string()]
                    } else {
                        vec![canonical_field.clone()]
                    },
                },
                unique,
            },
            collection,
            canonical_field,
            field_id: if kind == IndexKind::PathEquality {
                None
            } else {
                collection_schema.field_id(&field)
            },
            attr_id: if kind == IndexKind::PathEquality {
                None
            } else {
                collection_schema
                    .field_id(&field)
                    .and_then(|field_id| collection_schema.attr_for_field_id(field_id))
            },
        })
    }

    pub fn delete_index(&mut self, collection: LocalCollectionId, name: &str) -> bool {
        let Some(collection_schema) = self.collection_by_lid(collection) else {
            return false;
        };
        let key = Self::index_key(&collection_schema.name, name);
        let Some(index) = self.indexes.remove_key(&key) else {
            return false;
        };
        if let Some(ids) = self.collection_indexes.get_mut(&collection) {
            ids.retain(|id| *id != index.lid);
        }
        true
    }

    pub fn index_by_lid(&self, lid: LocalIndexId) -> Option<&IndexSchema> {
        self.indexes.get(lid)
    }

    pub fn indexes_for_collection(
        &self,
        collection: LocalCollectionId,
    ) -> impl Iterator<Item = &IndexSchema> {
        self.collection_indexes
            .get(&collection)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| self.indexes.get(*id))
    }

    pub fn find_equality_index(
        &self,
        collection: LocalCollectionId,
        canonical_field: &str,
    ) -> Option<&IndexSchema> {
        self.indexes_for_collection(collection).find(|index| {
            index.schema.kind == IndexKind::Equality && index.canonical_field == canonical_field
        })
    }

    pub fn find_path_equality_index(&self, collection: LocalCollectionId) -> Option<&IndexSchema> {
        self.indexes_for_collection(collection)
            .find(|index| index.schema.kind == IndexKind::PathEquality)
    }

    pub fn next_field_id(&self) -> usize {
        self.next_field_id
    }

    pub fn auto_index_enabled(&self) -> bool {
        self.auto_index_enabled
    }

    pub fn set_auto_index_enabled(&mut self, enabled: bool) {
        self.auto_index_enabled = enabled;
        let _ = self.sync_auto_path_indexes();
    }

    pub fn to_storage_snapshot(&self) -> CatalogStorageSnapshot {
        CatalogStorageSnapshot {
            attributes: self
                .attributes()
                .map(|(lid, attr)| StoredAttribute {
                    lid,
                    attribute: attr.attribute.clone(),
                })
                .collect(),
            type_defs: self
                .type_defs()
                .map(|(lid, type_def)| StoredTypeDef {
                    lid,
                    type_def: type_def.type_def.clone(),
                })
                .collect(),
            record_types: self
                .record_types()
                .map(|(lid, ty)| StoredRecordType {
                    lid,
                    id: ty.id.clone(),
                    name: ty.name.clone(),
                    record: ty.record.clone(),
                })
                .collect(),
            classes: self
                .classes()
                .map(|(lid, class)| StoredClass {
                    lid,
                    class: class.class.clone(),
                })
                .collect(),
            collections: self
                .collections()
                .map(|(lid, col)| StoredCollection {
                    lid,
                    name: col.name.clone(),
                    kind: Some(col.kind.clone()),
                    integrity_mode: col.integrity_mode,
                    internal: col.internal,
                    field_ids: col
                        .fields()
                        .map(|(field_id, name)| StoredFieldId {
                            field_id,
                            canonical_field: name.to_string(),
                        })
                        .collect(),
                })
                .collect(),
            indexes: self
                .indexes()
                .map(|(lid, index)| StoredIndex {
                    lid,
                    name: index.schema.name.clone(),
                    collection: index.collection,
                    field: index.canonical_field.clone(),
                    unique: index.schema.unique,
                    kind: index.schema.kind,
                })
                .collect(),
            relationships: self
                .relationships()
                .map(|(lid, relationship)| StoredRelationship {
                    lid,
                    relationship: relationship.relationship.clone(),
                })
                .collect(),
            packages: self
                .packages()
                .map(|(_, package)| StoredPackage {
                    package: package.clone(),
                })
                .collect(),
            applied_migrations: self
                .applied_migrations()
                .map(|(_, applied)| StoredAppliedMigration {
                    applied: applied.clone(),
                })
                .collect(),
            next_field_id: self.next_field_id,
            auto_index_enabled: self.auto_index_enabled,
        }
    }

    pub fn from_storage_snapshot(snapshot: CatalogStorageSnapshot) -> Result<Self, CatalogError> {
        Self::from_stored_rows(
            snapshot.attributes,
            snapshot.type_defs,
            snapshot.record_types,
            snapshot.classes,
            snapshot.collections,
            snapshot.indexes,
            snapshot.relationships,
            snapshot.packages,
            snapshot.applied_migrations,
            snapshot.next_field_id,
            snapshot.auto_index_enabled,
        )
    }

    pub fn from_stored_rows(
        attributes: Vec<StoredAttribute>,
        type_defs: Vec<StoredTypeDef>,
        record_types: Vec<StoredRecordType>,
        classes: Vec<StoredClass>,
        collections: Vec<StoredCollection>,
        indexes: Vec<StoredIndex>,
        relationships: Vec<StoredRelationship>,
        packages: Vec<StoredPackage>,
        applied_migrations: Vec<StoredAppliedMigration>,
        next_field_id: usize,
        auto_index_enabled: bool,
    ) -> Result<Self, CatalogError> {
        let mut catalog = Self::new();

        for item in type_defs {
            let key = item.type_def.name.clone();
            let names = nameset_for_qualified(&key);
            catalog.type_defs.insert_fixed(
                item.lid,
                names.clone(),
                TypeDefSchema {
                    lid: item.lid,
                    names,
                    type_def: item.type_def,
                },
            );
        }

        let mut legacy_attr_ids = FnvHashMap::default();
        for item in &attributes {
            legacy_attr_ids.insert(item.attribute.id.clone(), item.lid);
        }
        let mut legacy_record_ids = FnvHashMap::default();
        for item in &record_types {
            legacy_record_ids.insert(item.id.clone(), item.lid);
        }
        let mut legacy_class_ids = FnvHashMap::default();
        for item in &classes {
            legacy_class_ids.insert(item.class.id.clone(), item.lid);
        }

        if catalog.type_defs().next().is_some() {
            // Older catalog persistence used the same entity identity for a
            // typedef and its class/record projection. The projection was
            // written last, so recover missing typedefs from those saved rows.
            // Existing typedefs remain authoritative (including their metadata).
            for item in &classes {
                if catalog.type_def_by_name(&item.class.id).is_none() {
                    catalog.upsert_type_def_raw(type_def_from_class(item.class.clone(), None));
                }
            }
            for item in &record_types {
                if catalog.type_def_by_name(&item.id).is_none() {
                    catalog.upsert_type_def_raw(type_def_from_record_type(
                        item.id.clone(),
                        item.name.clone(),
                        item.record.clone(),
                        None,
                    ));
                }
            }
            let projected_attrs = catalog
                .type_defs()
                .filter_map(|(_, type_def)| match &type_def.type_def.ty.kind {
                    TypeKind::Attribute(attr) => {
                        Some((type_def.type_def.name.clone(), *attr.clone()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            for (id, attribute) in projected_attrs {
                let lid = legacy_attr_ids
                    .get(&id)
                    .copied()
                    .unwrap_or(catalog.attributes.next_id());
                catalog.attributes.insert_fixed(
                    lid,
                    nameset_for_qualified(&id),
                    AttributeSchema {
                        lid,
                        names: nameset_for_qualified(&id),
                        attribute,
                    },
                );
            }

            let projected_records = catalog
                .type_defs()
                .filter_map(|(_, type_def)| match &type_def.type_def.ty.kind {
                    TypeKind::Record(record) => Some((
                        type_def.type_def.name.clone(),
                        type_def
                            .type_def
                            .meta
                            .title
                            .clone()
                            .unwrap_or_else(|| type_def.type_def.name.clone()),
                        record.clone(),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>();
            for (id, name, record) in projected_records {
                let lid = legacy_record_ids
                    .get(&id)
                    .copied()
                    .unwrap_or(catalog.record_types.next_id());
                catalog.record_types.insert_fixed(
                    lid,
                    nameset_for_qualified(&id),
                    RecordTypeSchema {
                        lid,
                        names: nameset_for_qualified(&id),
                        id,
                        name,
                        record,
                    },
                );
            }

            let projected_classes = catalog
                .type_defs()
                .filter_map(|(_, type_def)| match &type_def.type_def.ty.kind {
                    TypeKind::Class(class) => Some(class.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            for class in projected_classes {
                let lid = legacy_class_ids
                    .get(&class.id)
                    .copied()
                    .unwrap_or(catalog.classes.next_id());
                let mut class_attributes = BTreeMap::new();
                for (alias, class_attr) in &class.attributes {
                    let Some(attr) = catalog.attribute_by_id(&class_attr.attribute.id) else {
                        return Err(CatalogError::UnknownAttribute {
                            id: class_attr.attribute.id.clone(),
                        });
                    };
                    class_attributes.insert(alias.clone(), attr.lid);
                    class_attributes.insert(class_attr.attribute.id.clone(), attr.lid);
                    class_attributes.insert(attr.names.plain_name.clone(), attr.lid);
                    class_attributes.insert(attr.names.underscore_name.clone(), attr.lid);
                }
                catalog.classes.insert_fixed(
                    lid,
                    nameset_for_qualified(&class.id),
                    ClassSchema {
                        lid,
                        names: nameset_for_qualified(&class.id),
                        class,
                        attributes: class_attributes,
                    },
                );
            }
        } else {
            for item in attributes {
                let key = item.attribute.id.clone();
                catalog.attributes.insert_fixed(
                    item.lid,
                    nameset_for_qualified(&key),
                    AttributeSchema {
                        lid: item.lid,
                        names: nameset_for_qualified(&key),
                        attribute: item.attribute,
                    },
                );
            }

            for item in record_types {
                let key = item.id.clone();
                catalog.record_types.insert_fixed(
                    item.lid,
                    nameset_for_qualified(&key),
                    RecordTypeSchema {
                        lid: item.lid,
                        names: nameset_for_qualified(&key),
                        id: item.id,
                        name: item.name,
                        record: item.record,
                    },
                );
            }

            for item in classes {
                let lid = item.lid;
                let class = item.class;
                let mut class_attributes = BTreeMap::new();
                for (alias, class_attr) in &class.attributes {
                    let Some(attr) = catalog.attribute_by_id(&class_attr.attribute.id) else {
                        return Err(CatalogError::UnknownAttribute {
                            id: class_attr.attribute.id.clone(),
                        });
                    };
                    class_attributes.insert(alias.clone(), attr.lid);
                    class_attributes.insert(class_attr.attribute.id.clone(), attr.lid);
                    class_attributes.insert(attr.names.plain_name.clone(), attr.lid);
                    class_attributes.insert(attr.names.underscore_name.clone(), attr.lid);
                }
                let key = class.id.clone();
                catalog.classes.insert_fixed(
                    lid,
                    nameset_for_qualified(&key),
                    ClassSchema {
                        lid,
                        names: nameset_for_qualified(&key),
                        class,
                        attributes: class_attributes,
                    },
                );
            }

            let generated_defs = catalog
                .attributes()
                .map(|(_, attr)| type_def_from_attribute(&attr.attribute, None))
                .chain(catalog.record_types().map(|(_, rec)| {
                    type_def_from_record_type(
                        rec.id.clone(),
                        rec.name.clone(),
                        rec.record.clone(),
                        None,
                    )
                }))
                .chain(
                    catalog
                        .classes()
                        .map(|(_, class)| type_def_from_class(class.class.clone(), None)),
                )
                .collect::<Vec<_>>();
            for type_def in generated_defs {
                let key = type_def.name.clone();
                let _ = catalog
                    .type_defs
                    .insert(nameset_for_qualified(&key), |lid| TypeDefSchema {
                        lid,
                        names: nameset_for_qualified(&key),
                        type_def,
                    });
            }
        }

        for item in collections {
            let field_ids = item
                .field_ids
                .into_iter()
                .map(|f| (f.canonical_field, f.field_id))
                .collect::<FnvHashMap<_, _>>();
            let schema = catalog.build_collection_schema_for_lid(
                item.lid,
                item.name.clone(),
                item.kind.unwrap_or(CollectionKind::Schema),
                item.integrity_mode,
                item.internal,
                Some(&field_ids),
            )?;
            catalog
                .collections
                .insert_fixed(item.lid, verbatim_nameset(&item.name), schema);
            for field_id in field_ids.values() {
                catalog.ensure_next_field_id(*field_id);
            }
        }

        for item in indexes {
            let key = {
                let collection = catalog
                    .collection_by_lid(item.collection)
                    .ok_or(CatalogError::UnknownCollection(item.collection))?;
                Self::index_key(&collection.name, &item.name)
            };
            // Index storage is keyed by the saved ID; restoring must not allocate another one.
            let index = catalog.build_index_schema_for_lid(
                item.lid,
                item.name,
                item.collection,
                item.field,
                item.unique,
                item.kind,
            )?;
            catalog
                .indexes
                .insert_fixed(item.lid, verbatim_nameset(&key), index);
            catalog
                .collection_indexes
                .entry(item.collection)
                .or_default()
                .push(item.lid);
        }
        for item in relationships {
            catalog.validate_relationship(&item.relationship)?;
            let key = item.relationship.id.clone();
            // Restore both the map slot and embedded ID without allocating a temporary entry.
            catalog.relationships.insert_fixed(
                item.lid,
                verbatim_nameset(&key),
                RelationshipSchema {
                    lid: item.lid,
                    relationship: item.relationship,
                },
            );
        }
        for item in packages {
            catalog.upsert_package(item.package);
        }
        for item in applied_migrations {
            catalog.record_applied_migration(item.applied);
        }
        let collection_ids = catalog
            .collections()
            .map(|(collection_id, _)| collection_id)
            .collect::<Vec<_>>();
        for collection_id in collection_ids {
            let _ = catalog.upsert_index(
                PRIMARY_ID_INDEX_NAME,
                collection_id,
                PRIMARY_ID_FIELD,
                true,
            )?;
            let _ = catalog.upsert_index(
                OBJECT_TYPE_INDEX_NAME,
                collection_id,
                OBJECT_TYPE_FIELD,
                false,
            )?;
            let _ = catalog.upsert_builtin_parent_relationship(collection_id)?;
        }

        catalog.next_field_id = catalog.next_field_id.max(next_field_id);
        catalog.auto_index_enabled = auto_index_enabled;
        let _ = catalog.sync_auto_path_indexes()?;
        Ok(catalog)
    }

    fn upsert_path_index(
        &mut self,
        collection: LocalCollectionId,
    ) -> Result<LocalIndexId, CatalogError> {
        self.upsert_index_with_kind(
            AUTO_PATH_INDEX_NAME,
            collection,
            AUTO_PATH_INDEX_FIELD,
            false,
            IndexKind::PathEquality,
        )
    }

    fn upsert_builtin_parent_relationship(
        &mut self,
        collection: LocalCollectionId,
    ) -> Result<(), CatalogError> {
        if self.attribute_by_id(ATTR_PARENT_RELATION).is_none() {
            return Ok(());
        }
        let collection_name = self
            .collection_by_lid(collection)
            .ok_or(CatalogError::UnknownCollection(collection))?
            .name
            .clone();
        let _ = self.upsert_relationship(RelationType {
            id: format!("{BUILTIN_PARENT_RELATION_ID}.{collection_name}"),
            name: "parent".to_string(),
            source_collection: collection_name,
            mode: RelationMode::Embedded {
                attribute: ATTR_PARENT_RELATION.to_string(),
            },
            indexing_mode: semantic_data::schema::RelationIndexingMode::Enabled,
            meta: Meta::default(),
        })?;
        Ok(())
    }

    fn sync_auto_path_indexes(&mut self) -> Result<(), CatalogError> {
        let collection_ids = self
            .collections()
            .map(|(collection_id, _)| collection_id)
            .collect::<Vec<_>>();
        for collection_id in collection_ids {
            if self.auto_index_enabled {
                let _ = self.upsert_path_index(collection_id)?;
            } else {
                let _ = self.delete_index(collection_id, AUTO_PATH_INDEX_NAME);
            }
        }
        Ok(())
    }

    fn build_collection_schema_for_lid(
        &mut self,
        lid: LocalCollectionId,
        name: String,
        kind: CollectionKind,
        integrity_mode: IntegrityMode,
        internal: bool,
        fixed_field_ids: Option<&FnvHashMap<String, LocalFieldId>>,
    ) -> Result<CollectionSchema, CatalogError> {
        let mut field_aliases = FnvHashMap::default();
        let mut field_types = FnvHashMap::default();
        let mut field_attrs = FnvHashMap::<String, LocalAttrId>::default();
        let schema_driven = matches!(kind, CollectionKind::Schema | CollectionKind::Polymorphic);
        let closed_fields =
            schema_driven && integrity_mode == IntegrityMode::StrictRegisteredSchema;

        let mut field_ids = FnvHashMap::default();
        let mut field_names_by_id = FnvHashMap::default();
        let mut attr_by_field_id = FnvHashMap::default();
        let system_string = Type {
            kind: TypeKind::String(StringType {
                format: None,
                normalization: None,
            }),
            constraints: vec![],
            annotations: vec![],
        };
        field_types
            .entry(PRIMARY_ID_FIELD.to_string())
            .or_insert_with(|| system_string.clone());
        field_types
            .entry(OBJECT_TYPE_FIELD.to_string())
            .or_insert(system_string);
        field_types
            .entry(PARENT_RELATION_FIELD.to_string())
            .or_insert_with(|| Type {
                kind: TypeKind::Ref(TypeRef {
                    name: PRIMARY_ID_FIELD.to_string(),
                    args: vec![],
                }),
                constraints: vec![],
                annotations: vec![],
            });

        if schema_driven {
            for (_, attr) in self.attributes() {
                field_types
                    .entry(attr.attribute.id.clone())
                    .or_insert_with(|| attr.attribute.ty.clone());
                field_attrs.insert(attr.attribute.id.clone(), attr.lid);
                insert_field_alias_if_not_builtin_shadow(
                    &mut field_aliases,
                    &attr.names.plain_name,
                    &attr.attribute.id,
                );
                insert_field_alias_if_not_builtin_shadow(
                    &mut field_aliases,
                    &attr.names.underscore_name,
                    &attr.attribute.id,
                );
            }
        }

        // Collect computed fields from all registered classes.
        let mut computed_fields = BTreeSet::<String>::new();
        for (_, class) in self.classes() {
            for (_, class_attr) in &class.class.attributes {
                if class_attr.computed.is_some() {
                    computed_fields.insert(class_attr.attribute.id.clone());
                }
            }
        }

        for canonical in field_types.keys() {
            let names = nameset_for_qualified(canonical);
            field_aliases
                .entry(names.plain_name)
                .or_insert_with(|| canonical.clone());
            field_aliases
                .entry(names.underscore_name)
                .or_insert_with(|| canonical.clone());
        }

        let mut field_names = field_types.keys().cloned().collect::<Vec<_>>();
        field_names.sort();
        for field_name in field_names {
            let field_id = if let Some(fixed) = fixed_field_ids.and_then(|m| m.get(&field_name)) {
                *fixed
            } else {
                self.allocate_field_id()
            };
            self.ensure_next_field_id(field_id);
            field_ids.insert(field_name.clone(), field_id);
            field_names_by_id.insert(field_id, field_name.clone());
            if let Some(attr_id) = field_attrs.get(&field_name) {
                attr_by_field_id.insert(field_id, *attr_id);
            }
        }

        Ok(CollectionSchema::new(
            lid,
            name,
            kind,
            integrity_mode,
            internal,
            field_aliases,
            field_types,
            field_ids,
            field_names_by_id,
            attr_by_field_id,
            computed_fields,
            closed_fields,
        ))
    }

    fn index_key(collection_name: &str, index_name: &str) -> String {
        format!("{collection_name}::{index_name}")
    }

    fn is_type_operation(operation: &CatalogBatchOperation) -> bool {
        matches!(
            operation,
            CatalogBatchOperation::UpsertAttribute { .. }
                | CatalogBatchOperation::DeleteAttribute { .. }
                | CatalogBatchOperation::UpsertTypeDef { .. }
                | CatalogBatchOperation::DeleteTypeDef { .. }
                | CatalogBatchOperation::UpsertRecordType { .. }
                | CatalogBatchOperation::DeleteRecordType { .. }
                | CatalogBatchOperation::UpsertClass { .. }
                | CatalogBatchOperation::DeleteClass { .. }
        )
    }

    fn flush_pending_type_operations(
        &mut self,
        pending_type_ops: &mut Vec<&CatalogBatchOperation>,
    ) -> Result<(), CatalogError> {
        if pending_type_ops.is_empty() {
            return Ok(());
        }

        for operation in pending_type_ops.iter().copied() {
            match operation {
                CatalogBatchOperation::UpsertAttribute { attribute, module } => {
                    let _ = self
                        .upsert_type_def_raw(type_def_from_attribute(attribute, module.clone()));
                }
                CatalogBatchOperation::DeleteAttribute { id } => {
                    let _ = self.delete_type_def_raw(id);
                }
                CatalogBatchOperation::UpsertTypeDef { type_def } => {
                    let _ = self.upsert_type_def_raw(type_def.clone());
                }
                CatalogBatchOperation::DeleteTypeDef { name } => {
                    let _ = self.delete_type_def_raw(name);
                }
                CatalogBatchOperation::UpsertRecordType {
                    id,
                    name,
                    record,
                    module,
                } => {
                    let _ = self.upsert_type_def_raw(type_def_from_record_type(
                        id.clone(),
                        name.clone(),
                        record.clone(),
                        module.clone(),
                    ));
                }
                CatalogBatchOperation::DeleteRecordType { id } => {
                    let _ = self.delete_type_def_raw(id);
                }
                CatalogBatchOperation::UpsertClass { class, module } => {
                    let _ = self
                        .upsert_type_def_raw(type_def_from_class(class.clone(), module.clone()));
                }
                CatalogBatchOperation::DeleteClass { id } => {
                    let _ = self.delete_type_def_raw(id);
                }
                CatalogBatchOperation::UpsertCollection { .. }
                | CatalogBatchOperation::DeleteCollection { .. }
                | CatalogBatchOperation::UpsertIndex { .. }
                | CatalogBatchOperation::DeleteIndex { .. }
                | CatalogBatchOperation::UpsertRelationship { .. }
                | CatalogBatchOperation::DeleteRelationship { .. }
                | CatalogBatchOperation::SetAutoIndex { .. } => {
                    unreachable!("non-type operations must not be flushed as type operations");
                }
            }
        }

        self.rebuild_type_projections()?;
        self.rebuild_collection_projections()?;
        pending_type_ops.clear();
        Ok(())
    }

    fn upsert_type_def_raw(&mut self, type_def: TypeDef) -> LocalTypeDefId {
        let type_def = normalize_type_def(type_def);
        let key = type_def.name.clone();
        self.type_defs
            .insert(nameset_for_qualified(&key), |lid| TypeDefSchema {
                lid,
                names: nameset_for_qualified(&key),
                type_def,
            })
    }

    fn delete_type_def_raw(&mut self, name: &str) -> bool {
        self.type_defs.remove_key(name).is_some()
    }

    fn rebuild_type_projections(&mut self) -> Result<(), CatalogError> {
        let legacy_attr_ids = self
            .attributes()
            .map(|(lid, attr)| (attr.attribute.id.clone(), lid))
            .collect::<FnvHashMap<_, _>>();
        let legacy_record_ids = self
            .record_types()
            .map(|(lid, record)| (record.id.clone(), lid))
            .collect::<FnvHashMap<_, _>>();
        let legacy_class_ids = self
            .classes()
            .map(|(lid, class)| (class.class.id.clone(), lid))
            .collect::<FnvHashMap<_, _>>();

        let projected_attrs = self
            .type_defs()
            .filter_map(|(_, type_def)| match &type_def.type_def.ty.kind {
                TypeKind::Attribute(attribute) => {
                    Some((type_def.type_def.name.clone(), *attribute.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let projected_records = self
            .type_defs()
            .filter_map(|(_, type_def)| match &type_def.type_def.ty.kind {
                TypeKind::Record(record) => Some((
                    type_def.type_def.name.clone(),
                    type_def
                        .type_def
                        .meta
                        .title
                        .clone()
                        .unwrap_or_else(|| type_def.type_def.name.clone()),
                    record.clone(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        let projected_classes = self
            .type_defs()
            .filter_map(|(_, type_def)| match &type_def.type_def.ty.kind {
                TypeKind::Class(class) => Some(class.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();

        for (_, type_def) in self.type_defs() {
            validate_type_def_invariants(&type_def.type_def)?;
        }

        self.attributes = IdMap::new();
        self.record_types = IdMap::new();
        self.classes = IdMap::new();

        for (id, attribute) in projected_attrs {
            let lid = legacy_attr_ids
                .get(&id)
                .copied()
                .unwrap_or(self.attributes.next_id());
            self.attributes.insert_fixed(
                lid,
                nameset_for_qualified(&id),
                AttributeSchema {
                    lid,
                    names: nameset_for_qualified(&id),
                    attribute,
                },
            );
        }

        for (id, name, record) in projected_records {
            let lid = legacy_record_ids
                .get(&id)
                .copied()
                .unwrap_or(self.record_types.next_id());
            self.record_types.insert_fixed(
                lid,
                nameset_for_qualified(&id),
                RecordTypeSchema {
                    lid,
                    names: nameset_for_qualified(&id),
                    id,
                    name,
                    record,
                },
            );
        }

        for class in &projected_classes {
            validate_class_attribute_resolution(&class, self)?;
        }
        validate_class_graph_invariants(projected_classes.iter())?;

        for class in projected_classes {
            let lid = legacy_class_ids
                .get(&class.id)
                .copied()
                .unwrap_or(self.classes.next_id());
            let mut class_attributes = BTreeMap::new();
            for (alias, class_attr) in &class.attributes {
                let Some(attr) = self.attribute_by_id(&class_attr.attribute.id) else {
                    return Err(CatalogError::UnknownAttribute {
                        id: class_attr.attribute.id.clone(),
                    });
                };
                class_attributes.insert(alias.clone(), attr.lid);
                class_attributes.insert(class_attr.attribute.id.clone(), attr.lid);
                class_attributes.insert(attr.names.plain_name.clone(), attr.lid);
                class_attributes.insert(attr.names.underscore_name.clone(), attr.lid);
            }
            self.classes.insert_fixed(
                lid,
                nameset_for_qualified(&class.id),
                ClassSchema {
                    lid,
                    names: nameset_for_qualified(&class.id),
                    class,
                    attributes: class_attributes,
                },
            );
        }

        for (_, type_def) in self.type_defs() {
            validate_foreign_keys(self, &type_def.type_def.ty, 0)?;
        }
        for (lid, class) in self.classes() {
            for constraint in &class.class.constraints {
                if let semantic_data::schema::ClassConstraint::Field { attribute, .. } = constraint
                {
                    if self.class_field_for_alias(lid, &attribute.id).is_none() {
                        return invalid_schema(format!(
                            "class '{}' field constraint references unknown attribute '{}'",
                            class.class.id, attribute.id
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn rebuild_collection_projections(&mut self) -> Result<(), CatalogError> {
        let collections = self
            .collections()
            .map(|(lid, schema)| {
                (
                    lid,
                    schema.name.clone(),
                    schema.kind.clone(),
                    schema.integrity_mode,
                    schema.internal,
                    schema
                        .fields()
                        .map(|(field_id, name)| (name.to_string(), field_id))
                        .collect::<FnvHashMap<_, _>>(),
                )
            })
            .collect::<Vec<_>>();

        for (lid, name, kind, integrity_mode, internal, field_ids) in collections {
            let schema = self.build_collection_schema_for_lid(
                lid,
                name.clone(),
                kind,
                integrity_mode,
                internal,
                Some(&field_ids),
            )?;
            self.collections
                .insert_fixed(lid, verbatim_nameset(&name), schema);
        }
        Ok(())
    }
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}

fn type_def_from_attribute(attribute: &AttributeType, module: Option<String>) -> TypeDef {
    let mut attribute = attribute.clone();
    attribute.id = nameset_for_identifier(&attribute.id, module.as_deref()).qualified_name;
    TypeDef {
        name: attribute.id.clone(),
        module: module.clone(),
        params: Vec::<TypeParam>::new(),
        ty: Type {
            kind: TypeKind::Attribute(Box::new(attribute)),
            constraints: vec![],
            annotations: vec![],
        },
        visibility: Visibility::Public,
        meta: Meta::default(),
    }
}

fn type_def_from_record_type(
    id: String,
    name: String,
    record: RecordType,
    module: Option<String>,
) -> TypeDef {
    let id = nameset_for_identifier(&id, module.as_deref()).qualified_name;
    TypeDef {
        name: id,
        module: module.clone(),
        params: Vec::<TypeParam>::new(),
        ty: Type {
            kind: TypeKind::Record(record),
            constraints: vec![],
            annotations: vec![],
        },
        visibility: Visibility::Public,
        meta: Meta {
            title: Some(name),
            ..Meta::default()
        },
    }
}

fn type_def_from_class(class: ClassType, module: Option<String>) -> TypeDef {
    let class = normalize_class_type(class, module.as_deref());
    TypeDef {
        name: class.id.clone(),
        module: module.clone(),
        params: Vec::<TypeParam>::new(),
        ty: Type {
            kind: TypeKind::Class(class.clone()),
            constraints: vec![],
            annotations: vec![],
        },
        visibility: Visibility::Public,
        meta: Meta {
            title: Some(class.name),
            ..Meta::default()
        },
    }
}

fn normalize_type_def(mut type_def: TypeDef) -> TypeDef {
    let module = type_def.module.clone();
    let names = nameset_for_identifier(&type_def.name, module.as_deref());
    type_def.name = names.qualified_name.clone();
    type_def.params = type_def
        .params
        .into_iter()
        .map(|param| normalize_type_param(param, module.as_deref()))
        .collect();

    type_def.ty = normalize_type(type_def.ty, module.as_deref());
    match &mut type_def.ty.kind {
        TypeKind::Attribute(attribute) => {
            attribute.id = names.qualified_name;
            attribute.ty = normalize_type(attribute.ty.clone(), module.as_deref());
        }
        TypeKind::Class(class) => {
            *class = normalize_class_type(class.clone(), module.as_deref());
        }
        _ => {}
    }
    type_def
}

fn normalize_class_type(mut class: ClassType, module: Option<&str>) -> ClassType {
    class.id = nameset_for_identifier(&class.id, module).qualified_name;
    if let Some(inherits) = class.inherits.as_mut() {
        inherits.id = nameset_for_identifier(&inherits.id, module).qualified_name;
    }
    for ext in &mut class.extends {
        ext.id = nameset_for_identifier(&ext.id, module).qualified_name;
    }
    for class_attr in class.attributes.values_mut() {
        class_attr.attribute.id =
            nameset_for_identifier(&class_attr.attribute.id, module).qualified_name;
        normalize_constraints(&mut class_attr.constraints, module);
    }
    for constraint in &mut class.constraints {
        if let semantic_data::schema::ClassConstraint::Field {
            attribute,
            constraint,
        } = constraint
        {
            if let Some(field) = class.attributes.get(&attribute.id) {
                attribute.id = field.attribute.id.clone();
            }
            normalize_constraints(std::slice::from_mut(constraint), module);
        }
    }
    class
}

fn normalize_type(mut ty: Type, module: Option<&str>) -> Type {
    normalize_constraints(&mut ty.constraints, module);
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
        TypeKind::Set(mut set) => {
            set.items = Box::new(normalize_type(*set.items, module));
            TypeKind::Set(set)
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
        TypeKind::Record(mut record) => {
            for field in record.fields.values_mut() {
                field.ty = normalize_type(field.ty.clone(), module);
            }
            record.additional = record
                .additional
                .map(|additional| Box::new(normalize_type(*additional, module)));
            TypeKind::Record(record)
        }
        TypeKind::Attribute(mut attribute) => {
            normalize_constraints(&mut attribute.constraints, module);
            attribute.ty = normalize_type(attribute.ty.clone(), module);
            TypeKind::Attribute(attribute)
        }
        TypeKind::Class(class) => TypeKind::Class(normalize_class_type(class, module)),
        TypeKind::Union(mut union) => {
            union.variants = union
                .variants
                .into_iter()
                .map(|variant| normalize_type(variant, module))
                .collect();
            TypeKind::Union(union)
        }
        TypeKind::Intersection(mut intersection) => {
            intersection.variants = intersection
                .variants
                .into_iter()
                .map(|variant| normalize_type(variant, module))
                .collect();
            TypeKind::Intersection(intersection)
        }
        TypeKind::Variant(mut variant) => {
            for case in &mut variant.variants {
                case.payload = normalize_variant_payload(case.payload.clone(), module);
            }
            TypeKind::Variant(variant)
        }
        TypeKind::Result(mut result) => {
            result.ok = Box::new(normalize_type(*result.ok, module));
            result.err = Box::new(normalize_type(*result.err, module));
            TypeKind::Result(result)
        }
        TypeKind::Function(function) => {
            TypeKind::Function(normalize_function_type(function, module))
        }
        TypeKind::Interface(mut interface) => {
            for method in &mut interface.methods {
                method.signature = normalize_function_type(method.signature.clone(), module);
            }
            TypeKind::Interface(interface)
        }
        TypeKind::Handle(mut handle) => {
            handle.interface = normalize_type_ref(handle.interface, module);
            TypeKind::Handle(handle)
        }
        TypeKind::Stream(mut stream) => {
            stream.element = Box::new(normalize_type(*stream.element, module));
            stream.end = stream.end.map(|end| Box::new(normalize_type(*end, module)));
            TypeKind::Stream(stream)
        }
        TypeKind::Ref(type_ref) => TypeKind::Ref(normalize_type_ref(type_ref, module)),
        kind => kind,
    };
    ty
}

fn normalize_constraints(
    constraints: &mut [semantic_data::schema::Constraint],
    module: Option<&str>,
) {
    for constraint in constraints {
        if let semantic_data::schema::Constraint::ForeignKey(fk) = constraint {
            fk.to = normalize_type_ref(fk.to.clone(), module);
        }
    }
}

fn validate_foreign_keys(catalog: &Catalog, ty: &Type, depth: usize) -> Result<(), CatalogError> {
    use semantic_data::schema::{ClassConstraint, Constraint};
    if depth > 128 {
        return invalid_schema("foreign key type recursion exceeds 128 levels".into());
    }
    let check = |constraints: &[Constraint]| -> Result<(), CatalogError> {
        for constraint in constraints {
            if let Constraint::ForeignKey(fk) = constraint {
                if fk.fields.len() != 1
                    || !matches!(fk.fields[0].as_str(), "id" | "semantic:id")
                    || !fk.to.args.is_empty()
                {
                    return invalid_schema("scalar ForeignKey requires the target primary ID field [id] and no type arguments".into());
                }
                if !foreign_key_resolves_class(catalog, &fk.to.name, &mut BTreeSet::new()) {
                    return invalid_schema(format!(
                        "ForeignKey target '{}' does not resolve to a class",
                        fk.to.name
                    ));
                }
            }
        }
        Ok(())
    };
    check(&ty.constraints)?;
    let mut nested = Vec::new();
    match &ty.kind {
        TypeKind::Optional(t) => nested.push(t.inner.as_ref()),
        TypeKind::Array(t) => nested.push(t.items.as_ref()),
        TypeKind::List(t) => nested.push(t.items.as_ref()),
        TypeKind::Set(t) => nested.push(t.items.as_ref()),
        TypeKind::Tuple(t) => {
            nested.extend(t.items.iter());
            nested.extend(t.rest.as_deref());
        }
        TypeKind::Map(t) => {
            nested.push(t.keys.as_ref());
            nested.push(t.values.as_ref());
        }
        TypeKind::Record(t) => {
            nested.extend(t.fields.values().map(|f| &f.ty));
            nested.extend(t.additional.as_deref());
        }
        TypeKind::Attribute(t) => {
            check(&t.constraints)?;
            nested.push(&t.ty);
        }
        TypeKind::Class(t) => {
            for attr in t.attributes.values() {
                check(&attr.constraints)?;
            }
            for constraint in &t.constraints {
                if let ClassConstraint::Field { constraint, .. } = constraint {
                    check(std::slice::from_ref(constraint))?;
                }
            }
        }
        TypeKind::Union(t) => nested.extend(t.variants.iter()),
        TypeKind::Intersection(t) => nested.extend(t.variants.iter()),
        _ => {}
    }
    for ty in nested {
        validate_foreign_keys(catalog, ty, depth + 1)?;
    }
    Ok(())
}

fn foreign_key_resolves_class(catalog: &Catalog, name: &str, seen: &mut BTreeSet<String>) -> bool {
    if !seen.insert(name.to_string()) {
        return false;
    }
    if catalog.class_ids(name).len() == 1 {
        return true;
    }
    match catalog
        .type_def_by_name(name)
        .map(|def| &def.type_def.ty.kind)
    {
        Some(TypeKind::Ref(reference)) if reference.args.is_empty() => {
            foreign_key_resolves_class(catalog, &reference.name, seen)
        }
        _ => false,
    }
}

fn normalize_type_param(mut param: TypeParam, module: Option<&str>) -> TypeParam {
    param.bounds = param
        .bounds
        .into_iter()
        .map(|bound| normalize_type_ref(bound, module))
        .collect();
    param.default = param.default.map(|default| normalize_type(default, module));
    param
}

fn normalize_type_ref(mut type_ref: TypeRef, module: Option<&str>) -> TypeRef {
    type_ref.name = nameset_for_identifier(&type_ref.name, module).qualified_name;
    type_ref.args = type_ref
        .args
        .into_iter()
        .map(|arg| normalize_type(arg, module))
        .collect();
    type_ref
}

fn normalize_variant_payload(payload: VariantPayload, module: Option<&str>) -> VariantPayload {
    match payload {
        VariantPayload::Unit => VariantPayload::Unit,
        VariantPayload::Tuple(items) => VariantPayload::Tuple(
            items
                .into_iter()
                .map(|item| normalize_type(item, module))
                .collect(),
        ),
        VariantPayload::Record(mut record) => {
            for field in record.fields.values_mut() {
                field.ty = normalize_type(field.ty.clone(), module);
            }
            record.additional = record
                .additional
                .map(|additional| Box::new(normalize_type(*additional, module)));
            VariantPayload::Record(record)
        }
        VariantPayload::Newtype(inner) => {
            VariantPayload::Newtype(Box::new(normalize_type(*inner, module)))
        }
    }
}

fn normalize_function_type(
    mut function: semantic_data::schema::FunctionType,
    module: Option<&str>,
) -> semantic_data::schema::FunctionType {
    for param in &mut function.params {
        param.ty = normalize_type(param.ty.clone(), module);
    }
    function.results = function
        .results
        .into_iter()
        .map(|result| normalize_type(result, module))
        .collect();
    function.throws = function
        .throws
        .map(|throws| Box::new(normalize_type(*throws, module)));
    function
}

fn validate_type_def_invariants(type_def: &TypeDef) -> Result<(), CatalogError> {
    validate_type_invariants(&type_def.ty, &format!("type '{}'", type_def.name))
}

fn validate_type_invariants(ty: &Type, context: &str) -> Result<(), CatalogError> {
    validate_constraints(&ty.constraints, &ConstraintTarget::Type(ty), context)?;
    match &ty.kind {
        TypeKind::Number(number) => validate_number_type(number, context)?,
        TypeKind::Array(array) => {
            if let Some(length) = &array.length {
                validate_length_spec(length, context)?;
            }
            validate_type_invariants(&array.items, &format!("{context} array item"))?;
        }
        TypeKind::Optional(optional) => {
            validate_type_invariants(&optional.inner, &format!("{context} optional inner"))?;
        }
        TypeKind::List(list) => {
            validate_type_invariants(&list.items, &format!("{context} list item"))?;
        }
        TypeKind::Set(set) => {
            validate_type_invariants(&set.items, &format!("{context} set item"))?;
        }
        TypeKind::Tuple(tuple) => {
            for (index, item) in tuple.items.iter().enumerate() {
                validate_type_invariants(item, &format!("{context} tuple item {index}"))?;
            }
            if let Some(rest) = &tuple.rest {
                validate_type_invariants(rest, &format!("{context} tuple rest"))?;
            }
        }
        TypeKind::Map(map) => {
            validate_type_invariants(&map.keys, &format!("{context} map key"))?;
            validate_type_invariants(&map.values, &format!("{context} map value"))?;
        }
        TypeKind::Record(record) => validate_record_invariants(record, context)?,
        TypeKind::Attribute(attribute) => {
            if attribute.id.trim().is_empty() {
                return invalid_schema(format!("{context} has an empty attribute id"));
            }
            validate_constraints(
                &attribute.constraints,
                &ConstraintTarget::Attribute(&attribute.ty),
                context,
            )?;
            validate_type_invariants(&attribute.ty, &format!("{context} attribute type"))?;
        }
        TypeKind::Class(class) => validate_class_shape_invariants(class, context)?,
        TypeKind::Union(union) => {
            if union.variants.is_empty() {
                return invalid_schema(format!("{context} union has no variants"));
            }
            for (index, variant) in union.variants.iter().enumerate() {
                validate_type_invariants(variant, &format!("{context} union variant {index}"))?;
            }
        }
        TypeKind::Intersection(intersection) => {
            if intersection.variants.is_empty() {
                return invalid_schema(format!("{context} intersection has no variants"));
            }
            for (index, variant) in intersection.variants.iter().enumerate() {
                validate_type_invariants(
                    variant,
                    &format!("{context} intersection variant {index}"),
                )?;
            }
        }
        TypeKind::Variant(variant) => validate_variant_invariants(variant, context)?,
        TypeKind::Enum(enum_type) => validate_enum_invariants(enum_type, context)?,
        TypeKind::Result(result) => {
            validate_type_invariants(&result.ok, &format!("{context} result ok"))?;
            validate_type_invariants(&result.err, &format!("{context} result err"))?;
        }
        TypeKind::Function(function) => validate_function_invariants(function, context)?,
        TypeKind::Interface(interface) => {
            for method in &interface.methods {
                validate_function_invariants(&method.signature, context)?;
            }
        }
        TypeKind::Stream(stream) => {
            validate_type_invariants(&stream.element, &format!("{context} stream element"))?;
            if let Some(end) = &stream.end {
                validate_type_invariants(end, &format!("{context} stream end"))?;
            }
        }
        TypeKind::Ref(type_ref) => {
            for arg in &type_ref.args {
                validate_type_invariants(arg, &format!("{context} ref arg"))?;
            }
        }
        TypeKind::Any(_)
        | TypeKind::Never(_)
        | TypeKind::Unknown(_)
        | TypeKind::Null(_)
        | TypeKind::Bool(_)
        | TypeKind::Char(_)
        | TypeKind::String(_)
        | TypeKind::Bytes(_)
        | TypeKind::Temporal(_)
        | TypeKind::Uuid
        | TypeKind::IpAddr(_)
        | TypeKind::Json
        | TypeKind::Handle(_)
        | TypeKind::Opaque(_)
        | TypeKind::Extension(_) => {}
    }
    Ok(())
}

fn validate_record_invariants(
    record: &semantic_data::schema::RecordType,
    context: &str,
) -> Result<(), CatalogError> {
    if !record.open && record.additional.is_some() {
        return invalid_schema(format!(
            "{context} is closed but declares additional fields"
        ));
    }
    if let Some(required_order) = &record.required_order {
        let mut seen = BTreeSet::new();
        for field_name in required_order {
            if !record.fields.contains_key(field_name) {
                return invalid_schema(format!(
                    "{context} required_order references unknown field '{field_name}'"
                ));
            }
            if !seen.insert(field_name) {
                return invalid_schema(format!(
                    "{context} required_order contains duplicate field '{field_name}'"
                ));
            }
        }
    }
    for (field_name, field) in &record.fields {
        if field.readonly && field.writeonly {
            return invalid_schema(format!(
                "{context} field '{field_name}' is both readonly and writeonly"
            ));
        }
        if let Some(default) = &field.default {
            if !literal_matches_type(default, &field.ty) {
                return invalid_schema(format!(
                    "{context} field '{field_name}' default does not match its type"
                ));
            }
        }
        validate_type_invariants(&field.ty, &format!("{context} field '{field_name}'"))?;
    }
    if let Some(additional) = &record.additional {
        validate_type_invariants(additional, &format!("{context} additional field"))?;
    }
    Ok(())
}

fn validate_class_shape_invariants(class: &ClassType, context: &str) -> Result<(), CatalogError> {
    if class.id.trim().is_empty() {
        return invalid_schema(format!("{context} has an empty class id"));
    }
    if class.name.trim().is_empty() {
        return invalid_schema(format!("class '{}' has an empty name", class.id));
    }
    let fields = class
        .attributes
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    validate_class_constraints(class, &fields, context)?;
    for (alias, class_attr) in &class.attributes {
        if alias.trim().is_empty() {
            return invalid_schema(format!("class '{}' has an empty attribute alias", class.id));
        }
        if class_attr.attribute.id.trim().is_empty() {
            return invalid_schema(format!(
                "class '{}' attribute alias '{alias}' has an empty attribute id",
                class.id
            ));
        }
        validate_constraints(
            &class_attr.constraints,
            &ConstraintTarget::UnresolvedAttribute,
            &format!("class '{}' attribute '{alias}'", class.id),
        )?;
    }
    Ok(())
}

fn validate_class_constraints(
    class: &ClassType,
    fields: &BTreeSet<&str>,
    context: &str,
) -> Result<(), CatalogError> {
    for constraint in &class.constraints {
        match constraint {
            semantic_data::schema::ClassConstraint::Field {
                attribute,
                constraint,
            } => {
                if attribute.id.trim().is_empty() {
                    return invalid_schema(format!(
                        "{context} class field constraint references an empty attribute"
                    ));
                }
                let attribute_exists = fields.contains(attribute.id.as_str())
                    || class
                        .attributes
                        .values()
                        .any(|class_attr| class_attr.attribute.id == attribute.id);
                if !attribute_exists && class.inherits.is_none() && class.extends.is_empty() {
                    return invalid_schema(format!(
                        "{context} class field constraint references unknown attribute '{}'",
                        attribute.id
                    ));
                }
                validate_constraints(
                    std::slice::from_ref(constraint),
                    &ConstraintTarget::UnresolvedAttribute,
                    context,
                )?;
            }
            semantic_data::schema::ClassConstraint::MultiFieldExpr { .. } => {}
        }
    }
    Ok(())
}

fn validate_class_attribute_resolution(
    class: &ClassType,
    catalog: &Catalog,
) -> Result<(), CatalogError> {
    validate_class_shape_invariants(class, &format!("class '{}'", class.id))?;
    let mut seen_attribute_ids = BTreeSet::new();
    let mut seen_aliases = BTreeMap::<String, String>::new();
    for (alias, class_attr) in &class.attributes {
        let attr = catalog
            .attribute_by_id(&class_attr.attribute.id)
            .ok_or_else(|| CatalogError::UnknownAttribute {
                id: class_attr.attribute.id.clone(),
            })?;
        if !seen_attribute_ids.insert(attr.attribute.id.clone()) {
            return invalid_schema(format!(
                "class '{}' declares attribute '{}' more than once",
                class.id, attr.attribute.id
            ));
        }
        validate_constraints(
            &class_attr.constraints,
            &ConstraintTarget::Attribute(&attr.attribute.ty),
            &format!("class '{}' attribute '{alias}'", class.id),
        )?;
        for candidate in [
            alias.as_str(),
            class_attr.attribute.id.as_str(),
            attr.names.plain_name.as_str(),
            attr.names.underscore_name.as_str(),
        ] {
            if let Some(existing) = seen_aliases.get(candidate) {
                if existing != &attr.attribute.id {
                    return invalid_schema(format!(
                        "class '{}' attribute alias '{}' collides between '{}' and '{}'",
                        class.id, candidate, existing, attr.attribute.id
                    ));
                }
            } else {
                seen_aliases.insert(candidate.to_string(), attr.attribute.id.clone());
            }
        }
    }
    Ok(())
}

fn validate_class_graph_invariants<'a>(
    classes: impl IntoIterator<Item = &'a ClassType>,
) -> Result<(), CatalogError> {
    let classes = classes
        .into_iter()
        .map(|class| (class.id.as_str(), class))
        .collect::<BTreeMap<_, _>>();
    let mut visited = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    for class_id in classes.keys() {
        validate_class_graph_visit(class_id, &classes, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn validate_class_graph_visit<'a>(
    class_id: &'a str,
    classes: &BTreeMap<&'a str, &'a ClassType>,
    visiting: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
) -> Result<(), CatalogError> {
    if visited.contains(class_id) {
        return Ok(());
    }
    if !visiting.insert(class_id) {
        return invalid_schema(format!("class inheritance cycle includes '{class_id}'"));
    }
    let Some(class) = classes.get(class_id) else {
        return Ok(());
    };
    if let Some(inherits) = &class.inherits {
        if classes.contains_key(inherits.id.as_str()) {
            validate_class_graph_visit(inherits.id.as_str(), classes, visiting, visited)?;
        }
    }
    for extends in &class.extends {
        if classes.contains_key(extends.id.as_str()) {
            validate_class_graph_visit(extends.id.as_str(), classes, visiting, visited)?;
        }
    }
    visiting.remove(class_id);
    visited.insert(class_id);
    Ok(())
}

fn validate_enum_invariants(
    enum_type: &semantic_data::schema::EnumType,
    context: &str,
) -> Result<(), CatalogError> {
    if enum_type.variants.is_empty() {
        return invalid_schema(format!("{context} enum has no variants"));
    }
    let mut names = BTreeSet::new();
    let mut values = BTreeSet::new();
    let mut symbols = BTreeSet::new();
    for variant in &enum_type.variants {
        if variant.name.trim().is_empty() {
            return invalid_schema(format!("{context} enum has an empty variant name"));
        }
        if !names.insert(variant.name.as_str()) {
            return invalid_schema(format!(
                "{context} enum has duplicate variant '{}'",
                variant.name
            ));
        }
        if let Some(value) = variant.value {
            if !values.insert(value) {
                return invalid_schema(format!("{context} enum has duplicate value {value}"));
            }
        }
        if let Some(symbol) = &variant.symbol {
            if !symbols.insert(symbol.as_str()) {
                return invalid_schema(format!("{context} enum has duplicate symbol '{symbol}'"));
            }
        }
    }
    Ok(())
}

fn validate_variant_invariants(
    variant_type: &semantic_data::schema::VariantType,
    context: &str,
) -> Result<(), CatalogError> {
    if variant_type.variants.is_empty() {
        return invalid_schema(format!("{context} variant has no cases"));
    }
    let mut names = BTreeSet::new();
    let mut discriminants = BTreeSet::new();
    for case in &variant_type.variants {
        if case.name.trim().is_empty() {
            return invalid_schema(format!("{context} variant has an empty case name"));
        }
        if !names.insert(case.name.as_str()) {
            return invalid_schema(format!(
                "{context} variant has duplicate case '{}'",
                case.name
            ));
        }
        if let Some(discriminant) = &case.discriminant {
            if !discriminants.insert(discriminant) {
                return invalid_schema(format!(
                    "{context} variant has duplicate discriminant for case '{}'",
                    case.name
                ));
            }
        }
        match &case.payload {
            VariantPayload::Unit => {}
            VariantPayload::Tuple(items) => {
                for (index, item) in items.iter().enumerate() {
                    validate_type_invariants(
                        item,
                        &format!("{context} variant case '{}' tuple item {index}", case.name),
                    )?;
                }
            }
            VariantPayload::Record(record) => validate_record_invariants(
                record,
                &format!("{context} variant case '{}' record", case.name),
            )?,
            VariantPayload::Newtype(inner) => validate_type_invariants(
                inner,
                &format!("{context} variant case '{}' payload", case.name),
            )?,
        }
    }
    Ok(())
}

fn validate_function_invariants(
    function: &semantic_data::schema::FunctionType,
    context: &str,
) -> Result<(), CatalogError> {
    for param in &function.params {
        let name = param.name.as_deref().unwrap_or("<anonymous>");
        validate_type_invariants(&param.ty, &format!("{context} function param '{name}'"))?;
    }
    for (index, result) in function.results.iter().enumerate() {
        validate_type_invariants(result, &format!("{context} function result {index}"))?;
    }
    if let Some(throws) = &function.throws {
        validate_type_invariants(throws, &format!("{context} function throws"))?;
    }
    Ok(())
}

fn validate_number_type(
    number: &semantic_data::schema::NumberType,
    context: &str,
) -> Result<(), CatalogError> {
    match number {
        semantic_data::schema::NumberType::BigInt(big_int) => {
            validate_bit_range(big_int.min_bits, big_int.max_bits, context)?;
        }
        semantic_data::schema::NumberType::BigUInt(big_uint) => {
            validate_bit_range(big_uint.min_bits, big_uint.max_bits, context)?;
        }
        semantic_data::schema::NumberType::Decimal(decimal) => {
            if decimal.precision == Some(0) {
                return invalid_schema(format!(
                    "{context} decimal precision must be greater than 0"
                ));
            }
            if let (Some(precision), Some(scale)) = (decimal.precision, decimal.scale) {
                if scale >= 0 && scale as u32 > precision {
                    return invalid_schema(format!(
                        "{context} decimal scale must not exceed precision"
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_bit_range(
    min_bits: Option<u32>,
    max_bits: Option<u32>,
    context: &str,
) -> Result<(), CatalogError> {
    if min_bits == Some(0) || max_bits == Some(0) {
        return invalid_schema(format!("{context} bit widths must be greater than 0"));
    }
    if let (Some(min), Some(max)) = (min_bits, max_bits) {
        if min > max {
            return invalid_schema(format!("{context} min_bits must not exceed max_bits"));
        }
    }
    Ok(())
}

enum ConstraintTarget<'a> {
    Type(&'a Type),
    Attribute(&'a Type),
    UnresolvedAttribute,
}

fn validate_constraints(
    constraints: &[semantic_data::schema::Constraint],
    target: &ConstraintTarget<'_>,
    context: &str,
) -> Result<(), CatalogError> {
    use semantic_data::schema::Constraint;

    let mut min_items = None;
    let mut max_items = None;
    let mut min_properties = None;
    let mut max_properties = None;
    let mut min_number = None;
    let mut max_number = None;
    for constraint in constraints {
        match constraint {
            Constraint::Min(bound) => {
                ensure_numeric_constraint_target(target, context, "Min")?;
                min_number = Some(parse_number_bound(bound, context, "Min")?);
            }
            Constraint::Max(bound) => {
                ensure_numeric_constraint_target(target, context, "Max")?;
                max_number = Some(parse_number_bound(bound, context, "Max")?);
            }
            Constraint::MultipleOf(value) => {
                ensure_numeric_constraint_target(target, context, "MultipleOf")?;
                let value = parse_constraint_number(value, context, "MultipleOf")?;
                if value <= 0.0 {
                    return invalid_schema(format!("{context} MultipleOf must be greater than 0"));
                }
            }
            Constraint::Length(length) => {
                ensure_string_constraint_target(target, context, "Length")?;
                validate_length_spec(length, context)?;
            }
            Constraint::Pattern(pattern) => {
                ensure_text_pattern_constraint_target(target, context, "Pattern")?;
                validate_regex_pattern(pattern, context, "Pattern")?;
            }
            Constraint::Prefix(value) => {
                ensure_text_pattern_constraint_target(target, context, "Prefix")?;
                if value.is_empty() {
                    return invalid_schema(format!("{context} Prefix must not be empty"));
                }
            }
            Constraint::Suffix(value) => {
                ensure_text_pattern_constraint_target(target, context, "Suffix")?;
                if value.is_empty() {
                    return invalid_schema(format!("{context} Suffix must not be empty"));
                }
            }
            Constraint::Charset(_) => {
                ensure_text_pattern_constraint_target(target, context, "Charset")?;
            }
            Constraint::Collation(value) => {
                ensure_text_pattern_constraint_target(target, context, "Collation")?;
                if value.trim().is_empty() {
                    return invalid_schema(format!("{context} Collation must not be empty"));
                }
            }
            Constraint::Precision { precision, scale } => {
                ensure_numeric_constraint_target(target, context, "Precision")?;
                if *precision == 0 {
                    return invalid_schema(format!("{context} precision must be greater than 0"));
                }
                if scale > precision {
                    return invalid_schema(format!("{context} scale must not exceed precision"));
                }
            }
            Constraint::MinItems(value) => {
                ensure_collection_constraint_target(target, context, "MinItems")?;
                min_items = Some(*value);
            }
            Constraint::MaxItems(value) => {
                ensure_collection_constraint_target(target, context, "MaxItems")?;
                max_items = Some(*value);
            }
            Constraint::MinProperties(value) => {
                ensure_property_constraint_target(target, context, "MinProperties")?;
                min_properties = Some(*value)
            }
            Constraint::MaxProperties(value) => {
                ensure_property_constraint_target(target, context, "MaxProperties")?;
                max_properties = Some(*value)
            }
            Constraint::RequiredFields(fields) => {
                ensure_property_constraint_target(target, context, "RequiredFields")?;
                validate_field_refs(fields, target, context, "RequiredFields")?;
            }
            Constraint::KeyPattern(pattern) => {
                ensure_property_constraint_target(target, context, "KeyPattern")?;
                validate_regex_pattern(pattern, context, "KeyPattern")?;
            }
            Constraint::PrimaryKey => {
                ensure_db_constraint_target(target, context, "PrimaryKey")?;
            }
            Constraint::ForeignKey(foreign_key) => {
                if let Some(ty) = target_value_type(target) {
                    if !matches!(
                        ty.kind,
                        TypeKind::String(_)
                            | TypeKind::Ref(_)
                            | TypeKind::Optional(_)
                            | TypeKind::Union(_)
                    ) {
                        return invalid_schema(format!(
                            "{context} ForeignKey requires a scalar string ID attribute"
                        ));
                    }
                }
                if foreign_key.fields.is_empty() {
                    return invalid_schema(format!(
                        "{context} foreign key constraint has no fields"
                    ));
                }
            }
            Constraint::Index { fields, .. } => {
                ensure_db_constraint_target(target, context, "Index")?;
                if fields.is_empty() {
                    return invalid_schema(format!("{context} index constraint has no fields"));
                }
                validate_field_refs(fields, target, context, "Index")?;
            }
            Constraint::DefaultValue { value } => {
                if let Some(ty) = target_value_type(target) {
                    if !literal_matches_type(value, ty) {
                        return invalid_schema(format!(
                            "{context} DefaultValue does not match its target type"
                        ));
                    }
                }
            }
            Constraint::DefaultExpr { expr } => {
                let expr_type = crate::validate_default_expression(expr)
                    .map_err(|err| CatalogError::InvalidSchema(format!("{context} {err}")))?;
                if let Some(ty) = target_value_type(target)
                    && !default_expression_matches_type(expr_type, ty)
                {
                    return invalid_schema(format!(
                        "{context} DefaultExpr does not match its target type"
                    ));
                }
            }
            _ => {}
        }
    }
    if let (Some(min), Some(max)) = (min_number, max_number) {
        if min > max {
            return invalid_schema(format!("{context} Min must not exceed Max"));
        }
    }
    if let (Some(min), Some(max)) = (min_items, max_items) {
        if min > max {
            return invalid_schema(format!("{context} MinItems must not exceed MaxItems"));
        }
    }
    if let (Some(min), Some(max)) = (min_properties, max_properties) {
        if min > max {
            return invalid_schema(format!(
                "{context} MinProperties must not exceed MaxProperties"
            ));
        }
    }
    Ok(())
}

fn target_value_type<'a>(target: &'a ConstraintTarget<'a>) -> Option<&'a Type> {
    match target {
        ConstraintTarget::Type(ty) | ConstraintTarget::Attribute(ty) => Some(ty),
        ConstraintTarget::UnresolvedAttribute => None,
    }
}

fn ensure_numeric_constraint_target(
    target: &ConstraintTarget<'_>,
    context: &str,
    constraint: &str,
) -> Result<(), CatalogError> {
    if matches!(target, ConstraintTarget::UnresolvedAttribute) {
        return Ok(());
    }
    if target_value_type(target).is_some_and(is_numeric_type) {
        return Ok(());
    }
    invalid_schema(format!(
        "{context} {constraint} constraint requires a numeric target"
    ))
}

fn ensure_string_constraint_target(
    target: &ConstraintTarget<'_>,
    context: &str,
    constraint: &str,
) -> Result<(), CatalogError> {
    if matches!(target, ConstraintTarget::UnresolvedAttribute) {
        return Ok(());
    }
    if target_value_type(target).is_some_and(is_string_like_type) {
        return Ok(());
    }
    invalid_schema(format!(
        "{context} {constraint} constraint requires a string, bytes, or char target"
    ))
}

fn ensure_text_pattern_constraint_target(
    target: &ConstraintTarget<'_>,
    context: &str,
    constraint: &str,
) -> Result<(), CatalogError> {
    if matches!(target, ConstraintTarget::UnresolvedAttribute) {
        return Ok(());
    }
    if target_value_type(target).is_some_and(is_text_type) {
        return Ok(());
    }
    invalid_schema(format!(
        "{context} {constraint} constraint requires a string or char target"
    ))
}

fn ensure_collection_constraint_target(
    target: &ConstraintTarget<'_>,
    context: &str,
    constraint: &str,
) -> Result<(), CatalogError> {
    if matches!(target, ConstraintTarget::UnresolvedAttribute) {
        return Ok(());
    }
    if target_value_type(target).is_some_and(is_collection_type) {
        return Ok(());
    }
    invalid_schema(format!(
        "{context} {constraint} constraint requires a collection target"
    ))
}

fn ensure_property_constraint_target(
    target: &ConstraintTarget<'_>,
    context: &str,
    constraint: &str,
) -> Result<(), CatalogError> {
    if matches!(target, ConstraintTarget::UnresolvedAttribute) {
        return Ok(());
    }
    if target_value_type(target).is_some_and(is_property_type) {
        return Ok(());
    }
    invalid_schema(format!(
        "{context} {constraint} constraint requires a record, map, or class target"
    ))
}

fn ensure_db_constraint_target(
    target: &ConstraintTarget<'_>,
    context: &str,
    constraint: &str,
) -> Result<(), CatalogError> {
    if matches!(
        target,
        ConstraintTarget::Attribute(_) | ConstraintTarget::UnresolvedAttribute
    ) || target_value_type(target).is_some_and(is_record_type)
        || target_value_type(target).is_some_and(is_class_type)
    {
        return Ok(());
    }
    invalid_schema(format!(
        "{context} {constraint} constraint requires a record, class, or attribute target"
    ))
}

fn validate_field_refs(
    fields: &[String],
    target: &ConstraintTarget<'_>,
    context: &str,
    constraint: &str,
) -> Result<(), CatalogError> {
    let mut seen = BTreeSet::new();
    for field in fields {
        if field.trim().is_empty() {
            return invalid_schema(format!("{context} {constraint} references an empty field"));
        }
        if !seen.insert(field.as_str()) {
            return invalid_schema(format!(
                "{context} {constraint} references duplicate field '{field}'"
            ));
        }
        if !target_has_field(target, field) {
            return invalid_schema(format!(
                "{context} {constraint} references unknown field '{field}'"
            ));
        }
    }
    Ok(())
}

fn target_has_field(target: &ConstraintTarget<'_>, field: &str) -> bool {
    match target {
        ConstraintTarget::Type(ty) => match &ty.kind {
            TypeKind::Record(record) => record.fields.contains_key(field),
            TypeKind::Class(class) => class.attributes.contains_key(field),
            _ => true,
        },
        ConstraintTarget::Attribute(_) | ConstraintTarget::UnresolvedAttribute => true,
    }
}

fn parse_number_bound(
    bound: &semantic_data::schema::NumberBound,
    context: &str,
    constraint: &str,
) -> Result<f64, CatalogError> {
    match bound {
        semantic_data::schema::NumberBound::Inclusive(value)
        | semantic_data::schema::NumberBound::Exclusive(value) => {
            parse_constraint_number(value, context, constraint)
        }
    }
}

fn parse_constraint_number(
    value: &str,
    context: &str,
    constraint: &str,
) -> Result<f64, CatalogError> {
    let parsed = value.parse::<f64>().map_err(|_| {
        CatalogError::InvalidSchema(format!("{context} {constraint} must be a valid number"))
    })?;
    if !parsed.is_finite() {
        return invalid_schema(format!("{context} {constraint} must be finite"));
    }
    Ok(parsed)
}

fn validate_regex_pattern(
    pattern: &str,
    context: &str,
    constraint: &str,
) -> Result<(), CatalogError> {
    if pattern.is_empty() {
        return invalid_schema(format!("{context} {constraint} must not be empty"));
    }
    regex::Regex::new(pattern).map_err(|err| {
        CatalogError::InvalidSchema(format!(
            "{context} {constraint} is not a valid regex: {err}"
        ))
    })?;
    Ok(())
}

fn is_numeric_type(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::Number(_))
}

fn is_string_like_type(ty: &Type) -> bool {
    matches!(
        ty.kind,
        TypeKind::String(_) | TypeKind::Bytes(_) | TypeKind::Char(_)
    )
}

fn is_text_type(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::String(_) | TypeKind::Char(_))
}

fn is_collection_type(ty: &Type) -> bool {
    matches!(
        ty.kind,
        TypeKind::Array(_)
            | TypeKind::List(_)
            | TypeKind::Tuple(_)
            | TypeKind::Set(_)
            | TypeKind::Map(_)
    )
}

fn is_property_type(ty: &Type) -> bool {
    matches!(
        ty.kind,
        TypeKind::Record(_) | TypeKind::Map(_) | TypeKind::Class(_)
    )
}

fn is_record_type(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::Record(_))
}

fn is_class_type(ty: &Type) -> bool {
    matches!(ty.kind, TypeKind::Class(_))
}

fn validate_length_spec(
    length: &semantic_data::schema::LengthSpec,
    context: &str,
) -> Result<(), CatalogError> {
    if let semantic_data::schema::LengthSpec::Range {
        min: Some(min),
        max: Some(max),
    } = length
    {
        if min > max {
            return invalid_schema(format!("{context} length min must not exceed max"));
        }
    }
    Ok(())
}

fn literal_matches_type(value: &semantic_data::value::Value, ty: &Type) -> bool {
    use semantic_data::value::Value;

    match (&ty.kind, value) {
        (_, Value::Null) => matches!(
            ty.kind,
            TypeKind::Null(_) | TypeKind::Optional(_) | TypeKind::Any(_) | TypeKind::Unknown(_)
        ),
        (TypeKind::Any(_) | TypeKind::Unknown(_) | TypeKind::Json, _) => true,
        (TypeKind::Bool(_), Value::Bool(_)) => true,
        (TypeKind::Char(_), Value::String(value)) => value.chars().count() == 1,
        (TypeKind::String(_), Value::String(_)) => true,
        (TypeKind::Bytes(_), Value::Bytes(_)) => true,
        (
            TypeKind::Number(number),
            Value::I8(_) | Value::I16(_) | Value::I32(_) | Value::I64(_) | Value::I128(_),
        ) => !matches!(
            number,
            semantic_data::schema::NumberType::UInt(_)
                | semantic_data::schema::NumberType::BigUInt(_)
        ),
        (
            TypeKind::Number(_),
            Value::U8(_) | Value::U16(_) | Value::U32(_) | Value::U64(_) | Value::U128(_),
        ) => true,
        (TypeKind::Number(_), Value::F32(_) | Value::F64(_)) => true,
        (TypeKind::Optional(optional), value) => literal_matches_type(value, &optional.inner),
        (TypeKind::Array(array), Value::List(_)) => {
            matches!(
                array.length,
                None | Some(semantic_data::schema::LengthSpec::Range { .. })
            )
        }
        (TypeKind::List(_), Value::List(_)) | (TypeKind::Set(_), Value::List(_)) => true,
        (TypeKind::Tuple(tuple), Value::List(values)) => {
            values.len() == tuple.items.len() || tuple.rest.is_some()
        }
        (TypeKind::Map(_), Value::Object(_)) | (TypeKind::Record(_), Value::Object(_)) => true,
        (TypeKind::Enum(enum_type), Value::String(value)) => enum_type
            .variants
            .iter()
            .any(|variant| variant.symbol.as_ref() == Some(value) || variant.name == *value),
        (TypeKind::Enum(enum_type), Value::I128(value)) => {
            i64::try_from(*value).ok().is_some_and(|value| {
                enum_type
                    .variants
                    .iter()
                    .any(|variant| variant.value == Some(value))
            })
        }
        _ => false,
    }
}

fn default_expression_matches_type(expr_type: crate::DefaultExpressionType, ty: &Type) -> bool {
    match &ty.kind {
        TypeKind::Any(_) | TypeKind::Unknown(_) => true,
        TypeKind::Optional(optional) => default_expression_matches_type(expr_type, &optional.inner),
        TypeKind::Temporal(temporal) => matches!(
            (expr_type, temporal),
            (
                crate::DefaultExpressionType::DateTime,
                semantic_data::schema::TemporalType::DateTime
            )
        ),
        _ => false,
    }
}

fn invalid_schema<T>(message: String) -> Result<T, CatalogError> {
    Err(CatalogError::InvalidSchema(message))
}

fn insert_field_alias_if_not_builtin_shadow(
    field_aliases: &mut FnvHashMap<String, String>,
    alias: &str,
    canonical: &str,
) {
    if is_special_builtin_field(alias) && alias != canonical {
        return;
    }
    field_aliases
        .entry(alias.to_string())
        .or_insert_with(|| canonical.to_string());
}

fn package_nameset(name: &str) -> NameSet {
    verbatim_nameset(name)
}

fn verbatim_nameset(name: &str) -> NameSet {
    if name.is_empty() {
        return NameSet {
            qualified_name: IMPLICIT_ROOT_PACKAGE.to_string(),
            plain_name: IMPLICIT_ROOT_PACKAGE.to_string(),
            underscore_name: IMPLICIT_ROOT_PACKAGE.to_string(),
        };
    }

    NameSet {
        qualified_name: name.to_string(),
        plain_name: name.to_string(),
        underscore_name: name.replace(':', "_"),
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::schema::{
        AttributeType, ClassAttribute, ClassConstraint, ClassType, Constraint, EnumType,
        EnumVariant, IntWidth, ListType, NumberBound, NumberType, VariantCase, VariantPayload,
        VariantTag, VariantType, attribute::attribute_ref::AttributeRef, record::field::Field,
        union::union_type::UnionType,
    };

    use super::*;

    #[test]
    fn restoring_indexes_preserves_sparse_ids_in_any_row_order() {
        let mut catalog = Catalog::new();
        for name in ["directories", "other"] {
            let collection = catalog
                .register_collection(name, CollectionKind::Polymorphic)
                .unwrap();
            catalog
                .upsert_index("by_name", collection, "name", true)
                .unwrap();
        }
        let mut snapshot = catalog.to_storage_snapshot();
        for index in &mut snapshot.indexes {
            index.lid = LocalIndexId(index.lid.0 * 2 + 3);
        }
        snapshot.indexes.reverse();
        let expected = snapshot.indexes.clone();

        let mut restored = Catalog::from_storage_snapshot(snapshot).unwrap();

        assert_eq!(restored.indexes().count(), expected.len());
        for saved in &expected {
            let index = restored.index_by_lid(saved.lid).unwrap();
            assert_eq!(index.lid, saved.lid);
            assert_eq!(index.collection, saved.collection);
            assert_eq!(index.schema.name, saved.name);
            assert_eq!(index.canonical_field, saved.field);
            assert_eq!(index.schema.unique, saved.unique);
            assert_eq!(index.schema.kind, saved.kind);
            assert_eq!(
                restored
                    .indexes_for_collection(saved.collection)
                    .filter(|candidate| candidate.schema.name == saved.name)
                    .map(|candidate| candidate.lid)
                    .collect::<Vec<_>>(),
                vec![saved.lid]
            );
        }

        let collection = restored.collection_by_name("directories").unwrap().lid;
        let saved = expected
            .iter()
            .find(|index| index.collection == collection && index.name == "by_name")
            .unwrap();
        let updated = restored
            .upsert_index("by_name", collection, "title", false)
            .unwrap();
        assert_eq!(updated, saved.lid);
        assert_eq!(restored.indexes().count(), expected.len());
        let index = restored.index_by_lid(updated).unwrap();
        assert_eq!(index.canonical_field, "title");
        assert!(!index.schema.unique);
        assert!(restored.delete_index(collection, "by_name"));
        assert!(!restored.delete_index(collection, "by_name"));
        assert!(restored.index_by_lid(updated).is_none());
        assert!(
            restored
                .indexes_for_collection(collection)
                .all(|index| index.schema.name != "by_name")
        );
        let recreated = restored
            .upsert_index("by_name", collection, "name", true)
            .unwrap();
        assert!(recreated > expected.iter().map(|index| index.lid).max().unwrap());
        let new_id = restored
            .upsert_index("by_title", collection, "title", false)
            .unwrap();
        assert!(new_id > recreated);
        assert_eq!(restored.indexes().count(), expected.len() + 1);
        let snapshot = restored.to_storage_snapshot();
        assert_eq!(
            Catalog::from_storage_snapshot(snapshot.clone())
                .unwrap()
                .to_storage_snapshot(),
            snapshot
        );
    }

    #[test]
    fn restoring_relationships_preserves_sparse_ids_and_lifecycle() {
        let mut catalog = Catalog::new();
        for name in ["directories", "other"] {
            catalog
                .register_collection(name, CollectionKind::Polymorphic)
                .unwrap();
        }
        for id in ["test:first", "test:second", "test:third"] {
            catalog
                .upsert_relationship(RelationType {
                    id: id.to_string(),
                    name: id.to_string(),
                    source_collection: "directories".to_string(),
                    mode: RelationMode::External,
                    indexing_mode: semantic_data::schema::RelationIndexingMode::Enabled,
                    meta: Meta::default(),
                })
                .unwrap();
        }
        let mut snapshot = catalog.to_storage_snapshot();
        for relationship in &mut snapshot.relationships {
            relationship.lid = LocalRelationId(relationship.lid.0 * 3 + 4);
        }
        snapshot.relationships.reverse();
        let expected = snapshot.relationships.clone();
        let mut restored = Catalog::from_storage_snapshot(snapshot).unwrap();
        assert_eq!(restored.relationships().count(), expected.len());
        for saved in &expected {
            let relationship = restored.relationship_by_id(&saved.relationship.id).unwrap();
            assert_eq!(relationship.lid, saved.lid);
            assert_eq!(relationship.relationship, saved.relationship);
            assert_eq!(
                restored
                    .relationships()
                    .filter(
                        |(_, relationship)| relationship.relationship.id == saved.relationship.id
                    )
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
                vec![saved.lid]
            );
        }

        let saved = &expected[0];
        let mut changed = saved.relationship.clone();
        changed.name = "updated".to_string();
        changed.source_collection = "other".to_string();
        assert_eq!(
            restored.upsert_relationship(changed.clone()).unwrap(),
            saved.lid
        );
        assert_eq!(
            restored
                .relationship_by_id(&changed.id)
                .unwrap()
                .relationship,
            changed
        );
        assert_eq!(restored.relationships().count(), expected.len());
        assert!(restored.delete_relationship(&changed.id));
        assert!(!restored.delete_relationship(&changed.id));
        assert!(restored.relationship_by_id(&changed.id).is_none());
        assert!(restored.relationships().all(
            |(id, relationship)| id != saved.lid && relationship.relationship.id != changed.id
        ));
        let recreated = restored.upsert_relationship(changed.clone()).unwrap();
        assert!(
            recreated
                > expected
                    .iter()
                    .map(|relationship| relationship.lid)
                    .max()
                    .unwrap()
        );
        changed.id = "test:new".to_string();
        assert!(restored.upsert_relationship(changed).unwrap() > recreated);
        assert_eq!(restored.relationships().count(), expected.len() + 1);
        let snapshot = restored.to_storage_snapshot();
        assert_eq!(
            Catalog::from_storage_snapshot(snapshot.clone())
                .unwrap()
                .to_storage_snapshot(),
            snapshot
        );
    }

    #[test]
    fn restoring_relationships_validates_definitions() {
        let mut catalog = Catalog::new();
        catalog
            .register_collection("directories", CollectionKind::Polymorphic)
            .unwrap();
        let relationship = RelationType {
            id: "test:invalid".to_string(),
            name: "invalid".to_string(),
            source_collection: "missing".to_string(),
            mode: RelationMode::External,
            indexing_mode: semantic_data::schema::RelationIndexingMode::Enabled,
            meta: Meta::default(),
        };
        let mut snapshot = catalog.to_storage_snapshot();
        snapshot.relationships.push(StoredRelationship {
            lid: LocalRelationId(9),
            relationship,
        });
        let error = Catalog::from_storage_snapshot(snapshot.clone()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unknown source collection 'missing'")
        );

        snapshot.relationships[0].relationship.source_collection = "directories".to_string();
        snapshot.relationships[0].relationship.mode = RelationMode::Embedded {
            attribute: "missing".to_string(),
        };
        let error = Catalog::from_storage_snapshot(snapshot).unwrap_err();
        assert!(error.to_string().contains("unknown attribute 'missing'"));
    }

    #[test]
    fn restores_class_and_record_typedefs_overwritten_by_legacy_projection_rows() {
        let mut catalog = Catalog::new();
        catalog.upsert_attribute(attribute("label"));
        let class_id = catalog
            .upsert_class(ClassType {
                id: "test:Person".into(),
                name: "Person".into(),
                inherits: None,
                extends: vec![],
                strict_schema: false,
                creatable_in_ui: None,
                attributes: BTreeMap::new(),
                constraints: vec![],
                meta: Meta::default(),
            })
            .unwrap();
        let record_id = catalog.upsert_record_type(
            "test:Record",
            "Record",
            RecordType {
                fields: BTreeMap::new(),
                open: true,
                additional: None,
                required_order: None,
            },
        );
        let mut snapshot = catalog.to_storage_snapshot();
        snapshot.type_defs.retain(|row| {
            !matches!(
                row.type_def.ty.kind,
                TypeKind::Class(_) | TypeKind::Record(_)
            )
        });
        let restored = Catalog::from_storage_snapshot(snapshot).unwrap();
        assert_eq!(
            restored.class_by_lid(class_id).unwrap().class.id,
            "test:Person"
        );
        assert_eq!(
            restored.record_type_by_lid(record_id).unwrap().id,
            "test:Record"
        );
        assert!(matches!(
            restored
                .type_def_by_name("test:Person")
                .unwrap()
                .type_def
                .ty
                .kind,
            TypeKind::Class(_)
        ));
        assert!(matches!(
            restored
                .type_def_by_name("test:Record")
                .unwrap()
                .type_def
                .ty
                .kind,
            TypeKind::Record(_)
        ));
        let snapshot = restored.to_storage_snapshot();
        assert_eq!(
            Catalog::from_storage_snapshot(snapshot.clone())
                .unwrap()
                .to_storage_snapshot(),
            snapshot
        );
    }

    #[test]
    fn normalize_type_def_qualifies_nested_record_and_union_refs() {
        let normalized = normalize_type_def(TypeDef {
            name: "Container".to_string(),
            module: Some("inventory".to_string()),
            params: Vec::new(),
            ty: Type {
                kind: TypeKind::Record(RecordType {
                    fields: BTreeMap::from([(
                        "item".to_string(),
                        Field {
                            ty: ref_type("Item"),
                            required: true,
                            readonly: false,
                            writeonly: false,
                            default: None,
                            meta: Meta::default(),
                        },
                    )]),
                    open: true,
                    additional: Some(Box::new(Type {
                        kind: TypeKind::Union(UnionType {
                            variants: vec![ref_type("Fallback")],
                        }),
                        constraints: vec![],
                        annotations: vec![],
                    })),
                    required_order: None,
                }),
                constraints: vec![],
                annotations: vec![],
            },
            visibility: Visibility::Public,
            meta: Meta::default(),
        });

        let TypeKind::Record(record) = &normalized.ty.kind else {
            panic!("expected record type");
        };
        let TypeKind::Ref(type_ref) = &record.fields["item"].ty.kind else {
            panic!("expected record field ref");
        };
        assert_eq!(type_ref.name, "local:inventory:Item");

        let TypeKind::Union(union) = &record.additional.as_ref().unwrap().kind else {
            panic!("expected additional union");
        };
        let TypeKind::Ref(type_ref) = &union.variants[0].kind else {
            panic!("expected union variant ref");
        };
        assert_eq!(type_ref.name, "local:inventory:Fallback");
    }

    #[test]
    fn apply_batch_rejects_record_required_order_unknown_field() {
        let mut catalog = Catalog::new();
        let err = catalog
            .apply_batch(&[CatalogBatchOperation::UpsertRecordType {
                id: "Broken".to_string(),
                name: "Broken".to_string(),
                record: RecordType {
                    fields: BTreeMap::new(),
                    open: true,
                    additional: None,
                    required_order: Some(vec!["missing".to_string()]),
                },
                module: Some("test".to_string()),
            }])
            .unwrap_err();

        assert!(
            matches!(err, CatalogError::InvalidSchema(message) if message.contains("required_order"))
        );
    }

    #[test]
    fn apply_batch_rejects_empty_union_and_enum() {
        let mut catalog = Catalog::new();
        let union_err = catalog
            .apply_batch(&[CatalogBatchOperation::UpsertTypeDef {
                type_def: test_type_def(
                    "EmptyUnion",
                    TypeKind::Union(UnionType { variants: vec![] }),
                ),
            }])
            .unwrap_err();
        assert!(
            matches!(union_err, CatalogError::InvalidSchema(message) if message.contains("union has no variants"))
        );

        let mut catalog = Catalog::new();
        let enum_err = catalog
            .apply_batch(&[CatalogBatchOperation::UpsertTypeDef {
                type_def: test_type_def(
                    "EmptyEnum",
                    TypeKind::Enum(EnumType {
                        repr: semantic_data::schema::EnumRepr::String,
                        variants: vec![],
                    }),
                ),
            }])
            .unwrap_err();
        assert!(
            matches!(enum_err, CatalogError::InvalidSchema(message) if message.contains("enum has no variants"))
        );
    }

    #[test]
    fn apply_batch_rejects_duplicate_enum_and_variant_names() {
        let mut catalog = Catalog::new();
        let enum_err = catalog
            .apply_batch(&[CatalogBatchOperation::UpsertTypeDef {
                type_def: test_type_def(
                    "DuplicateEnum",
                    TypeKind::Enum(EnumType {
                        repr: semantic_data::schema::EnumRepr::String,
                        variants: vec![enum_variant("open"), enum_variant("open")],
                    }),
                ),
            }])
            .unwrap_err();
        assert!(
            matches!(enum_err, CatalogError::InvalidSchema(message) if message.contains("duplicate variant"))
        );

        let mut catalog = Catalog::new();
        let variant_err = catalog
            .apply_batch(&[CatalogBatchOperation::UpsertTypeDef {
                type_def: test_type_def(
                    "DuplicateVariant",
                    TypeKind::Variant(VariantType {
                        tag: VariantTag::ExternallyTagged,
                        variants: vec![variant_case("ok"), variant_case("ok")],
                    }),
                ),
            }])
            .unwrap_err();
        assert!(
            matches!(variant_err, CatalogError::InvalidSchema(message) if message.contains("duplicate case"))
        );
    }

    #[test]
    fn apply_batch_rejects_class_attribute_alias_collision() {
        let mut catalog = Catalog::new();
        let err = catalog
            .apply_batch(&[
                CatalogBatchOperation::UpsertAttribute {
                    attribute: attribute("Title"),
                    module: Some("test".to_string()),
                },
                CatalogBatchOperation::UpsertAttribute {
                    attribute: attribute("Summary"),
                    module: Some("test".to_string()),
                },
                CatalogBatchOperation::UpsertClass {
                    class: ClassType {
                        id: "Article".to_string(),
                        name: "Article".to_string(),
                        inherits: None,
                        extends: vec![],
                        strict_schema: false,
                        creatable_in_ui: None,
                        attributes: BTreeMap::from([
                            ("local:test:Summary".to_string(), class_attribute("Title")),
                            ("title_alias".to_string(), class_attribute("Summary")),
                        ]),
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                    module: Some("test".to_string()),
                },
            ])
            .unwrap_err();

        assert!(
            matches!(err, CatalogError::InvalidSchema(message) if message.contains("collides"))
        );
    }

    #[test]
    fn apply_batch_rejects_numeric_constraint_on_string() {
        let mut catalog = Catalog::new();
        let err = catalog
            .apply_batch(&[CatalogBatchOperation::UpsertTypeDef {
                type_def: test_type_def_with_constraints(
                    "StringWithMin",
                    TypeKind::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    vec![Constraint::Min(NumberBound::Inclusive("1".to_string()))],
                ),
            }])
            .unwrap_err();

        assert!(
            matches!(err, CatalogError::InvalidSchema(message) if message.contains("numeric target"))
        );
    }

    #[test]
    fn apply_batch_rejects_invalid_min_max_item_bounds() {
        let mut catalog = Catalog::new();
        let err = catalog
            .apply_batch(&[CatalogBatchOperation::UpsertTypeDef {
                type_def: test_type_def_with_constraints(
                    "ListWithBadBounds",
                    TypeKind::List(ListType {
                        items: Box::new(Type::new(TypeKind::Number(NumberType::Int(
                            IntWidth::I64,
                        )))),
                    }),
                    vec![Constraint::MinItems(3), Constraint::MaxItems(2)],
                ),
            }])
            .unwrap_err();

        assert!(
            matches!(err, CatalogError::InvalidSchema(message) if message.contains("MinItems"))
        );
    }

    #[test]
    fn apply_batch_rejects_unknown_required_and_index_fields() {
        let record = || {
            TypeKind::Record(RecordType {
                fields: BTreeMap::from([(
                    "id".to_string(),
                    Field {
                        ty: Type::new(TypeKind::String(StringType {
                            format: None,
                            normalization: None,
                        })),
                        required: true,
                        readonly: false,
                        writeonly: false,
                        default: None,
                        meta: Meta::default(),
                    },
                )]),
                open: true,
                additional: None,
                required_order: None,
            })
        };

        let mut catalog = Catalog::new();
        let required_err = catalog
            .apply_batch(&[CatalogBatchOperation::UpsertTypeDef {
                type_def: test_type_def_with_constraints(
                    "RecordWithMissingRequired",
                    record(),
                    vec![Constraint::RequiredFields(vec!["missing".to_string()])],
                ),
            }])
            .unwrap_err();
        assert!(
            matches!(required_err, CatalogError::InvalidSchema(message) if message.contains("unknown field 'missing'"))
        );

        let mut catalog = Catalog::new();
        let index_err = catalog
            .apply_batch(&[CatalogBatchOperation::UpsertTypeDef {
                type_def: test_type_def_with_constraints(
                    "RecordWithMissingIndex",
                    record(),
                    vec![Constraint::Index {
                        name: None,
                        fields: vec!["missing".to_string()],
                        unique: false,
                    }],
                ),
            }])
            .unwrap_err();
        assert!(
            matches!(index_err, CatalogError::InvalidSchema(message) if message.contains("unknown field 'missing'"))
        );
    }

    #[test]
    fn apply_batch_allows_open_ended_class_expression_constraint() {
        let mut catalog = Catalog::new();
        catalog
            .apply_batch(&[CatalogBatchOperation::UpsertClass {
                class: ClassType {
                    id: "Article".to_string(),
                    name: "Article".to_string(),
                    inherits: None,
                    extends: vec![],
                    strict_schema: false,
                    creatable_in_ui: None,
                    attributes: BTreeMap::new(),
                    constraints: vec![ClassConstraint::MultiFieldExpr {
                        expr: semantic_data::expr::Expr::Literal(
                            semantic_data::expr::LiteralExpr {
                                value: semantic_data::value::Value::Bool(true),
                            },
                        ),
                        description: Some("dynamic rule owned by an extension".to_string()),
                    }],
                    meta: Meta::default(),
                },
                module: Some("test".to_string()),
            }])
            .unwrap();
    }

    fn test_type_def(name: &str, kind: TypeKind) -> TypeDef {
        test_type_def_with_constraints(name, kind, vec![])
    }

    fn test_type_def_with_constraints(
        name: &str,
        kind: TypeKind,
        constraints: Vec<Constraint>,
    ) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            module: Some("test".to_string()),
            params: vec![],
            ty: Type {
                kind,
                constraints,
                annotations: vec![],
            },
            visibility: Visibility::Public,
            meta: Meta::default(),
        }
    }

    fn enum_variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            value: None,
            symbol: None,
            meta: Meta::default(),
        }
    }

    fn variant_case(name: &str) -> VariantCase {
        VariantCase {
            name: name.to_string(),
            payload: VariantPayload::Unit,
            discriminant: None,
            meta: Meta::default(),
        }
    }

    fn attribute(id: &str) -> AttributeType {
        AttributeType {
            id: id.to_string(),
            name: id.to_string(),
            ty: Type::new(TypeKind::String(StringType {
                format: None,
                normalization: None,
            })),
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn class_attribute(id: &str) -> ClassAttribute {
        ClassAttribute {
            attribute: AttributeRef { id: id.to_string() },
            required: false,
            ui_order: None,
            computed: None,
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn ref_type(name: &str) -> Type {
        Type {
            kind: TypeKind::Ref(TypeRef {
                name: name.to_string(),
                args: vec![],
            }),
            constraints: vec![],
            annotations: vec![],
        }
    }
}
