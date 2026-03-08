use std::collections::{BTreeMap, BTreeSet};

use fnv::FnvHashMap;
use semantic_data::schema::{
    IndexKind, Package,
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
    CollectionSchema, IdMap, IndexSchema, IntegrityMode, LocalAttrId, LocalClassId,
    LocalCollectionId, LocalFieldId, LocalIndexId, LocalRecordTypeId, LocalRelationId,
    LocalTypeDefId, RecordTypeSchema, RelationshipSchema, StoredAppliedMigration, StoredAttribute,
    StoredClass, StoredCollection, StoredFieldId, StoredIndex, StoredPackage, StoredRecordType,
    StoredRelationship, StoredTypeDef, TypeDefSchema,
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
    packages: BTreeMap<String, Package>,
    applied_migrations: BTreeMap<String, AppliedMigration>,
    collection_indexes: FnvHashMap<LocalCollectionId, Vec<LocalIndexId>>,
    next_field_id: usize,
    auto_index_enabled: bool,
}

pub const PRIMARY_ID_FIELD: &str = "id";
pub const OBJECT_TYPE_FIELD: &str = "type";
pub const PARENT_RELATION_FIELD: &str = "parent";
pub const PARENT_RELATION_ATTRIBUTE: &str = "parent";
pub const PRIMARY_ID_INDEX_NAME: &str = "__builtin_pk_id";
pub const OBJECT_TYPE_INDEX_NAME: &str = "__builtin_type";
pub const BUILTIN_PARENT_RELATION_ID: &str = "__builtin.parent";
pub const RELATION_CLASS_ID: &str = "semantic.relation";
pub const RELATION_FROM_ATTRIBUTE: &str = "semantic.relation.from";
pub const RELATION_TO_ATTRIBUTE: &str = "semantic.relation.to";
pub const AUTO_PATH_INDEX_NAME: &str = "__auto_index_all_paths";
pub const AUTO_PATH_INDEX_FIELD: &str = "__path__";

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
            packages: BTreeMap::new(),
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

    pub fn packages(&self) -> impl Iterator<Item = (&String, &Package)> {
        self.packages.iter()
    }

    pub fn package_by_name(&self, name: &str) -> Option<&Package> {
        self.packages.get(name)
    }

    pub fn upsert_package(&mut self, package: Package) {
        let _ = self.packages.insert(package.name.clone(), package);
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
        let key = type_def.name.clone();
        match &type_def.ty.kind {
            TypeKind::Attribute(attribute) => {
                self.attributes.insert(key.clone(), |lid| AttributeSchema {
                    lid,
                    attribute: (*attribute.clone()),
                });
                let _ = self.record_types.remove_key(&key);
                let _ = self.classes.remove_key(&key);
            }
            TypeKind::Record(record) => {
                self.record_types
                    .insert(key.clone(), |lid| RecordTypeSchema {
                        lid,
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
                }
                if unresolved {
                    let _ = self.classes.remove_key(&key);
                } else {
                    self.classes.insert(key.clone(), |lid| ClassSchema {
                        lid,
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
            .insert(key, |lid| TypeDefSchema { lid, type_def })
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

    pub fn class_by_lid(&self, lid: LocalClassId) -> Option<&ClassSchema> {
        self.classes.get(lid)
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
            existing_field_ids.as_ref(),
        )?;

        self.collections.insert_fixed(lid, name, schema);
        // Ensure builtin indexes are always present and flow through normal index machinery.
        let _ = self.upsert_index(PRIMARY_ID_INDEX_NAME, lid, PRIMARY_ID_FIELD, true)?;
        let _ = self.upsert_index(OBJECT_TYPE_INDEX_NAME, lid, OBJECT_TYPE_FIELD, false)?;
        if self.auto_index_enabled {
            let _ = self.upsert_path_index(lid)?;
        }
        let _ = self.upsert_builtin_parent_relationship(lid)?;
        Ok(lid)
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
                for field in [RELATION_FROM_ATTRIBUTE, RELATION_TO_ATTRIBUTE] {
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
        let key = relationship.id.clone();
        Ok(self
            .relationships
            .insert(key, |lid| RelationshipSchema { lid, relationship }))
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
        let collection_schema = self
            .collections
            .get(collection)
            .ok_or(CatalogError::UnknownCollection(collection))?;
        let field = field.into();
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

        let name = name.into();
        let key = Self::index_key(&collection_schema.name, &name);
        let existing_lid = self.indexes.get_key_id(&key);
        let lid = existing_lid.unwrap_or(self.indexes.next_id());
        let index = IndexSchema {
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
        };

        self.indexes.insert_fixed(lid, key, index);
        let ids = self.collection_indexes.entry(collection).or_default();
        if !ids.contains(&lid) {
            ids.push(lid);
        }
        Ok(lid)
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
                    integrity_mode: col.integrity_mode,
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
            catalog.type_defs.insert_fixed(
                item.lid,
                key,
                TypeDefSchema {
                    lid: item.lid,
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
                catalog
                    .attributes
                    .insert_fixed(lid, id, AttributeSchema { lid, attribute });
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
                    id.clone(),
                    RecordTypeSchema {
                        lid,
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
                }
                catalog.classes.insert_fixed(
                    lid,
                    class.id.clone(),
                    ClassSchema {
                        lid,
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
                    key,
                    AttributeSchema {
                        lid: item.lid,
                        attribute: item.attribute,
                    },
                );
            }

            for item in record_types {
                let key = item.id.clone();
                catalog.record_types.insert_fixed(
                    item.lid,
                    key,
                    RecordTypeSchema {
                        lid: item.lid,
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
                }
                let key = class.id.clone();
                catalog.classes.insert_fixed(
                    lid,
                    key,
                    ClassSchema {
                        lid,
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
                    .insert(key, |lid| TypeDefSchema { lid, type_def });
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
                CollectionKind::Polymorphic,
                item.integrity_mode,
                Some(&field_ids),
            )?;
            catalog
                .collections
                .insert_fixed(item.lid, item.name.clone(), schema);
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
            let _ = catalog.upsert_index_with_kind(
                item.name,
                item.collection,
                item.field,
                item.unique,
                item.kind,
            )?;
            if let Some(index) = catalog.indexes.get_key(&key) {
                let index_value = index.clone();
                catalog.indexes.insert_fixed(item.lid, key, index_value);
                let ids = catalog
                    .collection_indexes
                    .entry(item.collection)
                    .or_default();
                if !ids.contains(&item.lid) {
                    ids.push(item.lid);
                }
            }
        }
        for item in relationships {
            let _ = catalog.upsert_relationship(item.relationship.clone())?;
            if let Some(existing) = catalog.relationship_by_id(&item.relationship.id) {
                let value = existing.clone();
                catalog
                    .relationships
                    .insert_fixed(item.lid, item.relationship.id, value);
            }
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
    ) -> Result<LocalRelationId, CatalogError> {
        let collection_name = self
            .collection_by_lid(collection)
            .ok_or(CatalogError::UnknownCollection(collection))?
            .name
            .clone();
        self.upsert_relationship(RelationType {
            id: format!("{BUILTIN_PARENT_RELATION_ID}.{collection_name}"),
            name: "parent".to_string(),
            source_collection: collection_name,
            mode: RelationMode::Embedded {
                attribute: PARENT_RELATION_ATTRIBUTE.to_string(),
            },
            indexing_mode: semantic_data::schema::RelationIndexingMode::Enabled,
            meta: Meta::default(),
        })
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
        fixed_field_ids: Option<&FnvHashMap<String, LocalFieldId>>,
    ) -> Result<CollectionSchema, CatalogError> {
        let field_aliases = FnvHashMap::default();
        let mut field_types = FnvHashMap::default();
        let field_attrs = FnvHashMap::<String, LocalAttrId>::default();
        let closed_fields = false;

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
            meta: Meta::default(),
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
                meta: Meta::default(),
            });

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
            field_aliases,
            field_types,
            field_ids,
            field_names_by_id,
            attr_by_field_id,
            closed_fields,
        ))
    }

    fn index_key(collection_name: &str, index_name: &str) -> String {
        format!("{collection_name}::{index_name}")
    }

    fn collect_class_fields(
        &self,
        class_lid: LocalClassId,
        field_aliases: &mut FnvHashMap<String, String>,
        field_types: &mut FnvHashMap<String, Type>,
        field_attrs: &mut FnvHashMap<String, LocalAttrId>,
    ) -> Result<(), CatalogError> {
        fn visit(
            catalog: &Catalog,
            class_lid: LocalClassId,
            visited: &mut BTreeSet<LocalClassId>,
            field_aliases: &mut FnvHashMap<String, String>,
            field_types: &mut FnvHashMap<String, Type>,
            field_attrs: &mut FnvHashMap<String, LocalAttrId>,
        ) -> Result<(), CatalogError> {
            if !visited.insert(class_lid) {
                return Ok(());
            }
            let class = catalog
                .classes
                .get(class_lid)
                .ok_or(CatalogError::UnknownClass(class_lid))?;
            if let Some(inherits) = &class.class.inherits {
                let Some(base_lid) = catalog.class_id(&inherits.id) else {
                    return Err(CatalogError::InvalidSchema(format!(
                        "class '{}' inherits unknown class '{}'",
                        class.class.id, inherits.id
                    )));
                };
                visit(
                    catalog,
                    base_lid,
                    visited,
                    field_aliases,
                    field_types,
                    field_attrs,
                )?;
            }
            for ext in &class.class.extends {
                let Some(ext_lid) = catalog.class_id(&ext.id) else {
                    return Err(CatalogError::InvalidSchema(format!(
                        "class '{}' extends unknown class '{}'",
                        class.class.id, ext.id
                    )));
                };
                visit(
                    catalog,
                    ext_lid,
                    visited,
                    field_aliases,
                    field_types,
                    field_attrs,
                )?;
            }
            for (alias, class_attr) in &class.class.attributes {
                let Some(attr) = catalog.attribute_by_id(&class_attr.attribute.id) else {
                    return Err(CatalogError::UnknownAttribute {
                        id: class_attr.attribute.id.clone(),
                    });
                };
                field_aliases.insert(alias.clone(), attr.attribute.id.clone());
                field_types.insert(attr.attribute.id.clone(), attr.attribute.ty.clone());
                field_attrs.insert(attr.attribute.id.clone(), attr.lid);
            }
            Ok(())
        }

        let mut visited = BTreeSet::new();
        visit(
            self,
            class_lid,
            &mut visited,
            field_aliases,
            field_types,
            field_attrs,
        )
    }

    fn class_inherits(&self, class_lid: LocalClassId, target_class_id: &str) -> bool {
        fn visit(
            catalog: &Catalog,
            class_lid: LocalClassId,
            target_class_id: &str,
            seen: &mut BTreeSet<LocalClassId>,
        ) -> bool {
            if !seen.insert(class_lid) {
                return false;
            }
            let Some(class) = catalog.class_by_lid(class_lid) else {
                return false;
            };
            if class.class.id == target_class_id {
                return true;
            }
            if let Some(inherits) = &class.class.inherits
                && let Some(base_lid) = catalog.class_id(&inherits.id)
                && visit(catalog, base_lid, target_class_id, seen)
            {
                return true;
            }
            for ext in &class.class.extends {
                if let Some(ext_lid) = catalog.class_id(&ext.id)
                    && visit(catalog, ext_lid, target_class_id, seen)
                {
                    return true;
                }
            }
            false
        }

        let mut seen = BTreeSet::new();
        visit(self, class_lid, target_class_id, &mut seen)
    }
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}

fn type_def_from_attribute(attribute: &AttributeType, module: Option<String>) -> TypeDef {
    TypeDef {
        name: attribute.id.clone(),
        module,
        params: Vec::<TypeParam>::new(),
        ty: Type {
            kind: TypeKind::Attribute(Box::new(attribute.clone())),
            constraints: vec![],
            annotations: vec![],
            meta: Meta::default(),
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
    TypeDef {
        name: id,
        module,
        params: Vec::<TypeParam>::new(),
        ty: Type {
            kind: TypeKind::Record(record),
            constraints: vec![],
            annotations: vec![],
            meta: Meta::default(),
        },
        visibility: Visibility::Public,
        meta: Meta {
            title: Some(name),
            ..Meta::default()
        },
    }
}

fn type_def_from_class(class: ClassType, module: Option<String>) -> TypeDef {
    TypeDef {
        name: class.id.clone(),
        module,
        params: Vec::<TypeParam>::new(),
        ty: Type {
            kind: TypeKind::Class(class.clone()),
            constraints: vec![],
            annotations: vec![],
            meta: Meta::default(),
        },
        visibility: Visibility::Public,
        meta: Meta {
            title: Some(class.name),
            ..Meta::default()
        },
    }
}
