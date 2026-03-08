use semantic_data::value::{Object, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{
    Catalog, StoredAttribute, StoredClass, StoredCollection, StoredCollectionKind, StoredFieldId,
    StoredIndex, StoredRecordType, StoredRelationship, StoredTypeDef,
};
use semantic_db_core::{
    CORE_CATALOG_ATTRIBUTES_COLLECTION, CORE_CATALOG_CLASSES_COLLECTION,
    CORE_CATALOG_COLLECTIONS_COLLECTION, CORE_CATALOG_INDEXES_COLLECTION,
    CORE_CATALOG_META_COLLECTION, CORE_CATALOG_RECORD_TYPES_COLLECTION,
    CORE_CATALOG_TYPE_DEFS_COLLECTION, catalog::OBJECT_TYPE_FIELD,
};

use crate::storage::{
    EntityStore, KvEngine, KvWriteOp, StoredEntity, StoredEntityKind, collect_index_entries,
    encode_entity, entity_key, index_key,
};

const META_ROW_ID: &str = "__catalog_meta__";
const LID_FIELD: &str = "lid";
const ID_FIELD: &str = "id";
const ATTRIBUTE_FIELD: &str = "attribute";
const TYPE_DEF_FIELD: &str = "type_def";
const RECORD_FIELD: &str = "record";
const CLASS_FIELD: &str = "class";
const COLLECTION_KIND_FIELD: &str = "collection_kind";
const FIELD_IDS_FIELD: &str = "field_ids";
const COLLECTION_FIELD: &str = "collection";
const FIELD_FIELD: &str = "field";
const INDEX_KIND_FIELD: &str = "index_kind";
const UNIQUE_FIELD: &str = "unique";
const NEXT_FIELD_ID_FIELD: &str = "next_field_id";
const AUTO_INDEX_ENABLED_FIELD: &str = "auto_index_enabled";
const RELATIONSHIPS_FIELD: &str = "relationships";

struct CatalogCollections {
    attributes: semantic_db_core::catalog::LocalCollectionId,
    type_defs: semantic_db_core::catalog::LocalCollectionId,
    record_types: semantic_db_core::catalog::LocalCollectionId,
    classes: semantic_db_core::catalog::LocalCollectionId,
    collections: semantic_db_core::catalog::LocalCollectionId,
    indexes: semantic_db_core::catalog::LocalCollectionId,
    meta: semantic_db_core::catalog::LocalCollectionId,
}

fn core_collection_ids(catalog: &Catalog) -> std::result::Result<CatalogCollections, DbError> {
    let collection_lid = |name: &str| -> std::result::Result<_, DbError> {
        catalog
            .collection_by_name(name)
            .map(|schema| schema.lid)
            .ok_or_else(|| {
                DbError::InvalidQuery(format!("missing core catalog collection '{name}'"))
            })
    };
    Ok(CatalogCollections {
        attributes: collection_lid(CORE_CATALOG_ATTRIBUTES_COLLECTION)?,
        type_defs: collection_lid(CORE_CATALOG_TYPE_DEFS_COLLECTION)?,
        record_types: collection_lid(CORE_CATALOG_RECORD_TYPES_COLLECTION)?,
        classes: collection_lid(CORE_CATALOG_CLASSES_COLLECTION)?,
        collections: collection_lid(CORE_CATALOG_COLLECTIONS_COLLECTION)?,
        indexes: collection_lid(CORE_CATALOG_INDEXES_COLLECTION)?,
        meta: collection_lid(CORE_CATALOG_META_COLLECTION)?,
    })
}

fn encode_entity_base(
    collection: semantic_db_core::catalog::LocalCollectionId,
    id: String,
    lid: usize,
) -> StoredEntity {
    let mut object = Object::new();
    object.insert(ID_FIELD.to_string(), Value::String(id.clone()));
    object.insert(LID_FIELD.to_string(), Value::U64(lid as u64));
    object.insert(
        OBJECT_TYPE_FIELD.to_string(),
        Value::String("class".to_string()),
    );
    StoredEntity {
        id,
        collection: collection.0,
        kind: StoredEntityKind::Class,
        object,
    }
}

pub fn load_catalog<E: KvEngine>(
    store: &EntityStore<E>,
    bootstrap_catalog: &Catalog,
) -> std::result::Result<Option<Catalog>, DbError> {
    let core = core_collection_ids(bootstrap_catalog)?;
    let attributes_rows = load_rows_by_type(
        store,
        bootstrap_catalog,
        core.attributes,
        StoredEntityKind::Class,
    )?;
    let type_defs_rows = load_rows_by_type(
        store,
        bootstrap_catalog,
        core.type_defs,
        StoredEntityKind::Class,
    )?;
    let record_types_rows = load_rows_by_type(
        store,
        bootstrap_catalog,
        core.record_types,
        StoredEntityKind::Class,
    )?;
    let classes_rows = load_rows_by_type(
        store,
        bootstrap_catalog,
        core.classes,
        StoredEntityKind::Class,
    )?;
    let collections_rows = load_rows_by_type(
        store,
        bootstrap_catalog,
        core.collections,
        StoredEntityKind::Class,
    )?;
    let indexes_rows = load_rows_by_type(
        store,
        bootstrap_catalog,
        core.indexes,
        StoredEntityKind::Class,
    )?;
    let meta_rows =
        load_rows_by_type(store, bootstrap_catalog, core.meta, StoredEntityKind::Class)?;

    if attributes_rows.is_empty()
        && type_defs_rows.is_empty()
        && record_types_rows.is_empty()
        && classes_rows.is_empty()
        && collections_rows.is_empty()
        && indexes_rows.is_empty()
        && meta_rows.is_empty()
    {
        return Ok(None);
    }

    let mut attributes = Vec::<StoredAttribute>::new();
    for row in &attributes_rows {
        let lid = object_lid(&row.object)?;
        let attribute: semantic_data::schema::AttributeType =
            object_json_field(&row.object, ATTRIBUTE_FIELD)?;
        attributes.push(StoredAttribute { lid, attribute });
    }
    let mut type_defs = Vec::<StoredTypeDef>::new();
    for row in &type_defs_rows {
        let lid = object_lid(&row.object)?;
        let type_def: semantic_data::schema::TypeDef =
            object_json_field(&row.object, TYPE_DEF_FIELD)?;
        type_defs.push(StoredTypeDef { lid, type_def });
    }
    let mut record_types = Vec::<StoredRecordType>::new();
    for row in &record_types_rows {
        let lid = object_lid(&row.object)?;
        let id = object_string_field(&row.object, ID_FIELD)?;
        let name = object_string_field(&row.object, "name")?;
        let record: semantic_data::schema::RecordType =
            object_json_field(&row.object, RECORD_FIELD)?;
        record_types.push(StoredRecordType {
            lid,
            id,
            name,
            record,
        });
    }
    let mut classes = Vec::<StoredClass>::new();
    for row in &classes_rows {
        let lid = object_lid(&row.object)?;
        let class: semantic_data::schema::ClassType = object_json_field(&row.object, CLASS_FIELD)?;
        classes.push(StoredClass { lid, class });
    }
    let mut collections = Vec::<StoredCollection>::new();
    for row in &collections_rows {
        let lid = object_lid(&row.object)?;
        let name = object_string_field(&row.object, "name")?;
        let kind: StoredCollectionKind = object_json_field(&row.object, COLLECTION_KIND_FIELD)?;
        let field_ids: Vec<StoredFieldId> = object_json_field(&row.object, FIELD_IDS_FIELD)?;
        collections.push(StoredCollection {
            lid,
            name,
            kind,
            field_ids,
        });
    }
    let mut indexes = Vec::<StoredIndex>::new();
    for row in &indexes_rows {
        let lid = object_lid(&row.object)?;
        let name = object_string_field(&row.object, "name")?;
        let collection = semantic_db_core::catalog::LocalCollectionId(object_usize_field(
            &row.object,
            COLLECTION_FIELD,
        )?);
        let field = object_string_field(&row.object, FIELD_FIELD)?;
        let kind: semantic_data::schema::IndexKind = object_json_field_default(
            &row.object,
            INDEX_KIND_FIELD,
            semantic_data::schema::IndexKind::Equality,
        )?;
        let unique = object_bool_field(&row.object, UNIQUE_FIELD)?;
        indexes.push(StoredIndex {
            lid,
            name,
            collection,
            field,
            unique,
            kind,
        });
    }

    let mut next_field_id = 0usize;
    let mut auto_index_enabled = false;
    let mut relationships = Vec::<StoredRelationship>::new();
    for row in &meta_rows {
        if row.id == META_ROW_ID {
            next_field_id = object_usize_field(&row.object, NEXT_FIELD_ID_FIELD)?;
            auto_index_enabled =
                object_bool_field_default(&row.object, AUTO_INDEX_ENABLED_FIELD, false);
            relationships =
                object_json_field_default(&row.object, RELATIONSHIPS_FIELD, Vec::new())?;
            break;
        }
    }

    let catalog = Catalog::from_stored_rows(
        attributes,
        type_defs,
        record_types,
        classes,
        collections,
        indexes,
        relationships,
        next_field_id,
        auto_index_enabled,
    )
    .map_err(DbError::from)?;
    Ok(Some(catalog))
}

pub fn catalog_write_ops<E: KvEngine>(
    store: &EntityStore<E>,
    catalog: &Catalog,
) -> std::result::Result<Vec<KvWriteOp>, DbError> {
    let core = core_collection_ids(catalog)?;
    let mut ops = Vec::<KvWriteOp>::new();

    for key in store.collection_keys(core.attributes)? {
        ops.push(KvWriteOp::Delete { key });
    }
    for key in store.collection_keys(core.type_defs)? {
        ops.push(KvWriteOp::Delete { key });
    }
    for key in store.collection_keys(core.record_types)? {
        ops.push(KvWriteOp::Delete { key });
    }
    for key in store.collection_keys(core.classes)? {
        ops.push(KvWriteOp::Delete { key });
    }
    for key in store.collection_keys(core.collections)? {
        ops.push(KvWriteOp::Delete { key });
    }
    for key in store.collection_keys(core.indexes)? {
        ops.push(KvWriteOp::Delete { key });
    }
    for key in store.collection_keys(core.meta)? {
        ops.push(KvWriteOp::Delete { key });
    }
    for index in catalog.indexes_for_collection(core.attributes) {
        for key in store.index_keys(index.lid)? {
            ops.push(KvWriteOp::Delete { key });
        }
    }
    for index in catalog.indexes_for_collection(core.type_defs) {
        for key in store.index_keys(index.lid)? {
            ops.push(KvWriteOp::Delete { key });
        }
    }
    for index in catalog.indexes_for_collection(core.record_types) {
        for key in store.index_keys(index.lid)? {
            ops.push(KvWriteOp::Delete { key });
        }
    }
    for index in catalog.indexes_for_collection(core.classes) {
        for key in store.index_keys(index.lid)? {
            ops.push(KvWriteOp::Delete { key });
        }
    }
    for index in catalog.indexes_for_collection(core.collections) {
        for key in store.index_keys(index.lid)? {
            ops.push(KvWriteOp::Delete { key });
        }
    }
    for index in catalog.indexes_for_collection(core.indexes) {
        for key in store.index_keys(index.lid)? {
            ops.push(KvWriteOp::Delete { key });
        }
    }
    for index in catalog.indexes_for_collection(core.meta) {
        for key in store.index_keys(index.lid)? {
            ops.push(KvWriteOp::Delete { key });
        }
    }

    for (lid, item) in catalog.attributes() {
        let mut entity = encode_entity_base(core.attributes, item.attribute.id.clone(), lid.0);
        entity.object.insert(
            ATTRIBUTE_FIELD.to_string(),
            Value::String(
                facet_json::to_string(&item.attribute)
                    .map_err(|err| DbError::Serialization(err.to_string()))?,
            ),
        );
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }
    for (lid, item) in catalog.type_defs() {
        let mut entity = encode_entity_base(core.type_defs, item.type_def.name.clone(), lid.0);
        entity.object.insert(
            TYPE_DEF_FIELD.to_string(),
            Value::String(
                facet_json::to_string(&item.type_def)
                    .map_err(|err| DbError::Serialization(err.to_string()))?,
            ),
        );
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }
    for (lid, item) in catalog.record_types() {
        let mut entity = encode_entity_base(core.record_types, item.id.clone(), lid.0);
        entity
            .object
            .insert("name".to_string(), Value::String(item.name.clone()));
        entity.object.insert(
            RECORD_FIELD.to_string(),
            Value::String(
                facet_json::to_string(&item.record)
                    .map_err(|err| DbError::Serialization(err.to_string()))?,
            ),
        );
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }
    for (lid, item) in catalog.classes() {
        let mut entity = encode_entity_base(core.classes, item.class.id.clone(), lid.0);
        entity.object.insert(
            CLASS_FIELD.to_string(),
            Value::String(
                facet_json::to_string(&item.class)
                    .map_err(|err| DbError::Serialization(err.to_string()))?,
            ),
        );
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }
    for (lid, item) in catalog.collections() {
        let mut entity = encode_entity_base(core.collections, item.name.clone(), lid.0);
        entity
            .object
            .insert("name".to_string(), Value::String(item.name.clone()));
        let stored_kind = match item.kind {
            semantic_db_core::catalog::CollectionKind::Untyped => StoredCollectionKind::Untyped,
            semantic_db_core::catalog::CollectionKind::Record { record_type } => {
                StoredCollectionKind::Record { record_type }
            }
            semantic_db_core::catalog::CollectionKind::Class { class } => {
                StoredCollectionKind::Class { class }
            }
        };
        let field_ids = item
            .fields()
            .map(|(field_id, canonical_field)| StoredFieldId {
                field_id,
                canonical_field: canonical_field.to_string(),
            })
            .collect::<Vec<_>>();
        entity.object.insert(
            COLLECTION_KIND_FIELD.to_string(),
            Value::String(
                facet_json::to_string(&stored_kind)
                    .map_err(|err| DbError::Serialization(err.to_string()))?,
            ),
        );
        entity.object.insert(
            FIELD_IDS_FIELD.to_string(),
            Value::String(
                facet_json::to_string(&field_ids)
                    .map_err(|err| DbError::Serialization(err.to_string()))?,
            ),
        );
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }
    for (lid, item) in catalog.indexes() {
        let mut entity = encode_entity_base(
            core.indexes,
            format!("{}::{}", item.collection.0, item.schema.name),
            lid.0,
        );
        entity
            .object
            .insert("name".to_string(), Value::String(item.schema.name.clone()));
        entity.object.insert(
            COLLECTION_FIELD.to_string(),
            Value::U64(item.collection.0 as u64),
        );
        entity.object.insert(
            FIELD_FIELD.to_string(),
            Value::String(item.canonical_field.clone()),
        );
        entity.object.insert(
            INDEX_KIND_FIELD.to_string(),
            Value::String(
                facet_json::to_string(&item.schema.kind)
                    .map_err(|err| DbError::Serialization(err.to_string()))?,
            ),
        );
        entity
            .object
            .insert(UNIQUE_FIELD.to_string(), Value::Bool(item.schema.unique));
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }

    let mut meta_entity = encode_entity_base(core.meta, META_ROW_ID.to_string(), 0);
    meta_entity.object.insert(
        NEXT_FIELD_ID_FIELD.to_string(),
        Value::U64(catalog.next_field_id() as u64),
    );
    meta_entity.object.insert(
        AUTO_INDEX_ENABLED_FIELD.to_string(),
        Value::Bool(catalog.auto_index_enabled()),
    );
    let relationships = catalog
        .relationships()
        .map(|(lid, relationship)| StoredRelationship {
            lid,
            relationship: relationship.relationship.clone(),
        })
        .collect::<Vec<_>>();
    meta_entity.object.insert(
        RELATIONSHIPS_FIELD.to_string(),
        Value::String(
            facet_json::to_string(&relationships)
                .map_err(|err| DbError::Serialization(err.to_string()))?,
        ),
    );
    push_entity_with_indexes(catalog, &meta_entity, &mut ops)?;

    Ok(ops)
}

fn object_string_field(object: &Object, field: &str) -> std::result::Result<String, DbError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| DbError::Deserialization(format!("missing string field '{field}'")))
}

fn object_u64_field(object: &Object, field: &str) -> std::result::Result<u64, DbError> {
    let Some(value) = object.get(field) else {
        return Err(DbError::Deserialization(format!(
            "missing u64 field '{field}'"
        )));
    };
    match value {
        Value::U8(v) => Ok(u64::from(*v)),
        Value::U16(v) => Ok(u64::from(*v)),
        Value::U32(v) => Ok(u64::from(*v)),
        Value::U64(v) => Ok(*v),
        Value::I8(v) => u64::try_from(*v).map_err(|_| {
            DbError::Deserialization(format!("field '{field}' must be a non-negative integer"))
        }),
        Value::I16(v) => u64::try_from(*v).map_err(|_| {
            DbError::Deserialization(format!("field '{field}' must be a non-negative integer"))
        }),
        Value::I32(v) => u64::try_from(*v).map_err(|_| {
            DbError::Deserialization(format!("field '{field}' must be a non-negative integer"))
        }),
        Value::I64(v) => u64::try_from(*v).map_err(|_| {
            DbError::Deserialization(format!("field '{field}' must be a non-negative integer"))
        }),
        _ => Err(DbError::Deserialization(format!(
            "field '{field}' must be an integer"
        ))),
    }
}

fn object_usize_field(object: &Object, field: &str) -> std::result::Result<usize, DbError> {
    usize::try_from(object_u64_field(object, field)?)
        .map_err(|_| DbError::Deserialization(format!("field '{field}' does not fit usize")))
}

fn object_lid<T>(object: &Object) -> std::result::Result<T, DbError>
where
    T: From<usize>,
{
    Ok(T::from(object_usize_field(object, LID_FIELD)?))
}

fn object_bool_field(object: &Object, field: &str) -> std::result::Result<bool, DbError> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| DbError::Deserialization(format!("missing bool field '{field}'")))
}

fn object_bool_field_default(object: &Object, field: &str, default: bool) -> bool {
    object
        .get(field)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn object_json_field<T>(object: &Object, field: &str) -> std::result::Result<T, DbError>
where
    T: facet::Facet<'static>,
{
    let json = object_string_field(object, field)?;
    facet_json::from_str::<T>(&json).map_err(|err| DbError::Deserialization(err.to_string()))
}

fn object_json_field_default<T>(
    object: &Object,
    field: &str,
    default: T,
) -> std::result::Result<T, DbError>
where
    T: facet::Facet<'static>,
{
    let Some(value) = object.get(field).and_then(Value::as_str) else {
        return Ok(default);
    };
    facet_json::from_str::<T>(value).map_err(|err| DbError::Deserialization(err.to_string()))
}

fn load_rows_by_type<E: KvEngine>(
    store: &EntityStore<E>,
    catalog: &Catalog,
    collection: semantic_db_core::catalog::LocalCollectionId,
    kind: StoredEntityKind,
) -> std::result::Result<Vec<StoredEntity>, DbError> {
    let type_value = Value::String(
        match kind {
            StoredEntityKind::Untyped => "untyped",
            StoredEntityKind::Record => "record",
            StoredEntityKind::Class => "class",
        }
        .to_string(),
    );
    let Some(index) = catalog.find_equality_index(collection, OBJECT_TYPE_FIELD) else {
        return Ok(store.scan_collection(collection)?);
    };
    let ids = store.scan_index_value(index.lid, None, &type_value)?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(entity) = store.get_entity(collection, &id)? {
            out.push(entity);
        }
    }
    Ok(out)
}

fn push_entity_with_indexes(
    catalog: &Catalog,
    entity: &StoredEntity,
    ops: &mut Vec<KvWriteOp>,
) -> std::result::Result<(), DbError> {
    let collection = semantic_db_core::catalog::LocalCollectionId(entity.collection);
    ops.push(KvWriteOp::Put {
        key: entity_key(collection, &entity.id),
        value: encode_entity(entity)?,
    });
    for index in catalog.indexes_for_collection(collection) {
        ops.push(KvWriteOp::Put {
            key: crate::storage::index_format_key(index.lid),
            value: crate::storage::index_format_value(),
        });
        match index.schema.kind {
            semantic_data::schema::IndexKind::Equality => {
                if let Some(value) = entity.object.get(&index.canonical_field) {
                    ops.push(KvWriteOp::Put {
                        key: index_key(index.lid, None, value, &entity.id)?,
                        value: Vec::new(),
                    });
                }
            }
            semantic_data::schema::IndexKind::PathEquality => {
                for (path, value) in collect_index_entries(&entity.object) {
                    ops.push(KvWriteOp::Put {
                        key: index_key(index.lid, Some(&path), &value, &entity.id)?,
                        value: Vec::new(),
                    });
                }
            }
            semantic_data::schema::IndexKind::Range
            | semantic_data::schema::IndexKind::FullText => {}
        }
    }
    Ok(())
}
