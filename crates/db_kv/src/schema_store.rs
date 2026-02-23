use semantic_data::value::{Object, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{
    Catalog, CatalogStorageSnapshot, StoredAttribute, StoredClass, StoredCollection, StoredIndex,
    StoredRecordType,
};
use semantic_db_core::{
    CORE_CATALOG_ATTRIBUTES_COLLECTION, CORE_CATALOG_CLASSES_COLLECTION,
    CORE_CATALOG_COLLECTIONS_COLLECTION, CORE_CATALOG_INDEXES_COLLECTION,
    CORE_CATALOG_META_COLLECTION, CORE_CATALOG_RECORD_TYPES_COLLECTION, catalog::OBJECT_TYPE_FIELD,
};

use crate::storage::{
    EntityStore, KvEngine, KvWriteOp, StoredEntity, StoredEntityKind, encode_entity, entity_key,
    index_key,
};

const META_ROW_ID: &str = "__catalog_meta__";
const PAYLOAD_FIELD: &str = "payload";
const LID_FIELD: &str = "lid";
const ID_FIELD: &str = "id";

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
struct CatalogMetaRow {
    next_field_id: usize,
}

struct CatalogCollections {
    attributes: semantic_db_core::catalog::LocalCollectionId,
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
        record_types: collection_lid(CORE_CATALOG_RECORD_TYPES_COLLECTION)?,
        classes: collection_lid(CORE_CATALOG_CLASSES_COLLECTION)?,
        collections: collection_lid(CORE_CATALOG_COLLECTIONS_COLLECTION)?,
        indexes: collection_lid(CORE_CATALOG_INDEXES_COLLECTION)?,
        meta: collection_lid(CORE_CATALOG_META_COLLECTION)?,
    })
}

fn payload_from_object(object: &Object) -> std::result::Result<&str, DbError> {
    object
        .get(PAYLOAD_FIELD)
        .and_then(Value::as_str)
        .ok_or_else(|| DbError::Deserialization("missing catalog payload".to_string()))
}

fn encode_entity_payload(
    collection: semantic_db_core::catalog::LocalCollectionId,
    id: String,
    lid: usize,
    payload: String,
) -> StoredEntity {
    let mut object = Object::new();
    object.insert(ID_FIELD.to_string(), Value::String(id.clone()));
    object.insert(LID_FIELD.to_string(), Value::U64(lid as u64));
    object.insert(
        OBJECT_TYPE_FIELD.to_string(),
        Value::String("class".to_string()),
    );
    object.insert(PAYLOAD_FIELD.to_string(), Value::String(payload));
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
        attributes.push(
            facet_json::from_str::<StoredAttribute>(payload_from_object(&row.object)?)
                .map_err(|err| DbError::Deserialization(err.to_string()))?,
        );
    }
    let mut record_types = Vec::<StoredRecordType>::new();
    for row in &record_types_rows {
        record_types.push(
            facet_json::from_str::<StoredRecordType>(payload_from_object(&row.object)?)
                .map_err(|err| DbError::Deserialization(err.to_string()))?,
        );
    }
    let mut classes = Vec::<StoredClass>::new();
    for row in &classes_rows {
        classes.push(
            facet_json::from_str::<StoredClass>(payload_from_object(&row.object)?)
                .map_err(|err| DbError::Deserialization(err.to_string()))?,
        );
    }
    let mut collections = Vec::<StoredCollection>::new();
    for row in &collections_rows {
        collections.push(
            facet_json::from_str::<StoredCollection>(payload_from_object(&row.object)?)
                .map_err(|err| DbError::Deserialization(err.to_string()))?,
        );
    }
    let mut indexes = Vec::<StoredIndex>::new();
    for row in &indexes_rows {
        indexes.push(
            facet_json::from_str::<StoredIndex>(payload_from_object(&row.object)?)
                .map_err(|err| DbError::Deserialization(err.to_string()))?,
        );
    }

    let mut next_field_id = 0usize;
    for row in &meta_rows {
        if row.id == META_ROW_ID {
            next_field_id =
                facet_json::from_str::<CatalogMetaRow>(payload_from_object(&row.object)?)
                    .map_err(|err| DbError::Deserialization(err.to_string()))?
                    .next_field_id;
            break;
        }
    }

    let snapshot = CatalogStorageSnapshot {
        attributes,
        record_types,
        classes,
        collections,
        indexes,
        next_field_id,
    };
    let catalog = Catalog::from_storage_snapshot(snapshot).map_err(DbError::from)?;
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

    let snapshot = catalog.to_storage_snapshot();
    for item in &snapshot.attributes {
        let payload =
            facet_json::to_string(item).map_err(|err| DbError::Serialization(err.to_string()))?;
        let entity = encode_entity_payload(
            core.attributes,
            item.attribute.id.clone(),
            item.lid.0,
            payload,
        );
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }
    for item in &snapshot.record_types {
        let payload =
            facet_json::to_string(item).map_err(|err| DbError::Serialization(err.to_string()))?;
        let entity = encode_entity_payload(core.record_types, item.id.clone(), item.lid.0, payload);
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }
    for item in &snapshot.classes {
        let payload =
            facet_json::to_string(item).map_err(|err| DbError::Serialization(err.to_string()))?;
        let entity =
            encode_entity_payload(core.classes, item.class.id.clone(), item.lid.0, payload);
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }
    for item in &snapshot.collections {
        let payload =
            facet_json::to_string(item).map_err(|err| DbError::Serialization(err.to_string()))?;
        let entity =
            encode_entity_payload(core.collections, item.name.clone(), item.lid.0, payload);
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }
    for item in &snapshot.indexes {
        let payload =
            facet_json::to_string(item).map_err(|err| DbError::Serialization(err.to_string()))?;
        let entity = encode_entity_payload(
            core.indexes,
            format!("{}::{}", item.collection.0, item.name),
            item.lid.0,
            payload,
        );
        push_entity_with_indexes(catalog, &entity, &mut ops)?;
    }

    let meta_entity = encode_entity_payload(
        core.meta,
        META_ROW_ID.to_string(),
        0,
        facet_json::to_string(&CatalogMetaRow {
            next_field_id: snapshot.next_field_id,
        })
        .map_err(|err| DbError::Serialization(err.to_string()))?,
    );
    push_entity_with_indexes(catalog, &meta_entity, &mut ops)?;

    Ok(ops)
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
    let ids = store.scan_index_value(index.lid, &type_value)?;
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
        if let Some(value) = entity.object.get(&index.canonical_field) {
            ops.push(KvWriteOp::Put {
                key: index_key(index.lid, value, &entity.id)?,
                value: Vec::new(),
            });
        }
    }
    Ok(())
}
