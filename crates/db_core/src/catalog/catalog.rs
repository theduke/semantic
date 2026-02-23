use std::collections::BTreeMap;

use fnv::FnvHashMap;
use semantic_data::schema::{
    IndexKind, attribute::attribute_type::AttributeType, class::class_type::ClassType,
    collections::key_path::KeyPath, record::record_type::RecordType,
};

use crate::catalog::{
    AttributeSchema, CatalogError, CatalogStorageSnapshot, ClassSchema, CollectionKind,
    CollectionSchema, IdMap, IndexSchema, LocalAttrId, LocalClassId, LocalCollectionId,
    LocalFieldId, LocalIndexId, LocalRecordTypeId, RecordTypeSchema, StoredAttribute, StoredClass,
    StoredCollection, StoredCollectionKind, StoredFieldId, StoredIndex, StoredRecordType,
};

#[derive(Debug, Clone)]
pub struct Catalog {
    attributes: IdMap<LocalAttrId, AttributeSchema>,
    record_types: IdMap<LocalRecordTypeId, RecordTypeSchema>,
    classes: IdMap<LocalClassId, ClassSchema>,
    collections: IdMap<LocalCollectionId, CollectionSchema>,
    indexes: IdMap<LocalIndexId, IndexSchema>,
    collection_indexes: FnvHashMap<LocalCollectionId, Vec<LocalIndexId>>,
    next_field_id: usize,
}

impl Catalog {
    pub fn new() -> Self {
        Self {
            attributes: IdMap::new(),
            record_types: IdMap::new(),
            classes: IdMap::new(),
            collections: IdMap::new(),
            indexes: IdMap::new(),
            collection_indexes: FnvHashMap::default(),
            next_field_id: 0,
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

    pub fn classes(&self) -> impl Iterator<Item = (LocalClassId, &ClassSchema)> {
        self.classes.iter()
    }

    pub fn indexes(&self) -> impl Iterator<Item = (LocalIndexId, &IndexSchema)> {
        self.indexes.iter()
    }

    pub fn register_attribute(&mut self, attr: AttributeType) -> LocalAttrId {
        self.upsert_attribute(attr)
    }

    pub fn upsert_attribute(&mut self, attr: AttributeType) -> LocalAttrId {
        let key = attr.id.clone();
        self.attributes.insert(key, |lid| AttributeSchema {
            lid,
            attribute: attr,
        })
    }

    pub fn delete_attribute(&mut self, id: &str) -> bool {
        self.attributes.remove_key(id).is_some()
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
        let id = id.into();
        let name = name.into();
        self.record_types
            .insert(id.clone(), |lid| RecordTypeSchema {
                lid,
                id,
                name,
                record,
            })
    }

    pub fn delete_record_type(&mut self, id: &str) -> bool {
        self.record_types.remove_key(id).is_some()
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
        let mut attributes = BTreeMap::new();

        for (alias, class_attr) in &class.attributes {
            let Some(attr) = self.attribute_by_id(&class_attr.attribute.id) else {
                return Err(CatalogError::UnknownAttribute {
                    id: class_attr.attribute.id.clone(),
                });
            };
            attributes.insert(alias.clone(), attr.lid);
            attributes.insert(class_attr.attribute.id.clone(), attr.lid);
        }

        let key = class.id.clone();
        Ok(self.classes.insert(key, |lid| ClassSchema {
            lid,
            class,
            attributes,
        }))
    }

    pub fn delete_class(&mut self, id: &str) -> bool {
        self.classes.remove_key(id).is_some()
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
        self.upsert_collection(name, kind)
    }

    pub fn upsert_collection(
        &mut self,
        name: impl Into<String>,
        kind: CollectionKind,
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
            existing_field_ids.as_ref(),
        )?;

        self.collections.insert_fixed(lid, name, schema);
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
        let collection_schema = self
            .collections
            .get(collection)
            .ok_or(CatalogError::UnknownCollection(collection))?;
        let field = field.into();
        let canonical_field = collection_schema.canonical_field_name(&field).to_string();
        if collection_schema.is_closed_field_set()
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
                kind: IndexKind::Equality,
                collection: collection_schema.name.clone(),
                key_path: KeyPath {
                    segments: vec![canonical_field.clone()],
                },
                unique,
            },
            collection,
            canonical_field,
            field_id: collection_schema.field_id(&field),
            attr_id: collection_schema
                .field_id(&field)
                .and_then(|field_id| collection_schema.attr_for_field_id(field_id)),
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

    pub fn to_storage_snapshot(&self) -> CatalogStorageSnapshot {
        CatalogStorageSnapshot {
            attributes: self
                .attributes()
                .map(|(lid, attr)| StoredAttribute {
                    lid,
                    attribute: attr.attribute.clone(),
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
                    kind: match col.kind {
                        CollectionKind::Untyped => StoredCollectionKind::Untyped,
                        CollectionKind::Record { record_type } => {
                            StoredCollectionKind::Record { record_type }
                        }
                        CollectionKind::Class { class } => StoredCollectionKind::Class { class },
                    },
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
                })
                .collect(),
            next_field_id: self.next_field_id,
        }
    }

    pub fn from_storage_snapshot(snapshot: CatalogStorageSnapshot) -> Result<Self, CatalogError> {
        let mut catalog = Self::new();

        for item in snapshot.attributes {
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

        for item in snapshot.record_types {
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

        for item in snapshot.classes {
            let lid = item.lid;
            let class = item.class;
            let mut attributes = BTreeMap::new();
            for (alias, class_attr) in &class.attributes {
                let Some(attr) = catalog.attribute_by_id(&class_attr.attribute.id) else {
                    return Err(CatalogError::UnknownAttribute {
                        id: class_attr.attribute.id.clone(),
                    });
                };
                attributes.insert(alias.clone(), attr.lid);
                attributes.insert(class_attr.attribute.id.clone(), attr.lid);
            }
            let key = class.id.clone();
            catalog.classes.insert_fixed(
                lid,
                key,
                ClassSchema {
                    lid,
                    class,
                    attributes,
                },
            );
        }

        for item in snapshot.collections {
            let field_ids = item
                .field_ids
                .into_iter()
                .map(|f| (f.canonical_field, f.field_id))
                .collect::<FnvHashMap<_, _>>();
            let schema = catalog.build_collection_schema_for_lid(
                item.lid,
                item.name.clone(),
                match item.kind {
                    StoredCollectionKind::Untyped => CollectionKind::Untyped,
                    StoredCollectionKind::Record { record_type } => {
                        CollectionKind::Record { record_type }
                    }
                    StoredCollectionKind::Class { class } => CollectionKind::Class { class },
                },
                Some(&field_ids),
            )?;
            catalog
                .collections
                .insert_fixed(item.lid, item.name.clone(), schema);
            for field_id in field_ids.values() {
                catalog.ensure_next_field_id(*field_id);
            }
        }

        for item in snapshot.indexes {
            let key = {
                let collection = catalog
                    .collection_by_lid(item.collection)
                    .ok_or(CatalogError::UnknownCollection(item.collection))?;
                Self::index_key(&collection.name, &item.name)
            };
            let _ = catalog.upsert_index(item.name, item.collection, item.field, item.unique)?;
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

        catalog.next_field_id = catalog.next_field_id.max(snapshot.next_field_id);
        Ok(catalog)
    }

    fn build_collection_schema_for_lid(
        &mut self,
        lid: LocalCollectionId,
        name: String,
        kind: CollectionKind,
        fixed_field_ids: Option<&FnvHashMap<String, LocalFieldId>>,
    ) -> Result<CollectionSchema, CatalogError> {
        let (field_aliases, field_types, field_attrs, closed_fields) = match &kind {
            CollectionKind::Untyped => (
                FnvHashMap::default(),
                FnvHashMap::default(),
                FnvHashMap::default(),
                false,
            ),
            CollectionKind::Record { record_type } => {
                let record = self
                    .record_types
                    .get(*record_type)
                    .ok_or(CatalogError::UnknownRecordType(*record_type))?;
                let mut field_types = FnvHashMap::default();
                for (field_name, field) in &record.record.fields {
                    field_types.insert(field_name.clone(), field.ty.clone());
                }
                (
                    FnvHashMap::default(),
                    field_types,
                    FnvHashMap::default(),
                    !record.record.open,
                )
            }
            CollectionKind::Class { class } => {
                let class = self
                    .classes
                    .get(*class)
                    .ok_or(CatalogError::UnknownClass(*class))?;
                let mut field_aliases = FnvHashMap::default();
                let mut field_types = FnvHashMap::default();
                let mut field_attrs = FnvHashMap::default();

                for (alias, attr_lid) in &class.attributes {
                    let attr = self
                        .attributes
                        .get(*attr_lid)
                        .expect("attribute must exist");
                    field_aliases.insert(alias.clone(), attr.attribute.id.clone());
                    field_types.insert(attr.attribute.id.clone(), attr.attribute.ty.clone());
                    field_attrs.insert(attr.attribute.id.clone(), *attr_lid);
                }
                (field_aliases, field_types, field_attrs, true)
            }
        };

        let mut field_ids = FnvHashMap::default();
        let mut field_names_by_id = FnvHashMap::default();
        let mut attr_by_field_id = FnvHashMap::default();

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
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}
