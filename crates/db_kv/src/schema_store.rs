use semantic_data::value::{Object, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{
    Catalog, IntegrityMode, StoredAppliedMigration, StoredAttribute, StoredClass, StoredCollection,
    StoredFieldId, StoredIndex, StoredPackage, StoredRecordType, StoredRelationship, StoredTypeDef,
};
use semantic_db_core::{
    CORE_CATALOG_ATTRIBUTE_ENTRY_CLASS_ID, CORE_CATALOG_CLASS_ENTRY_CLASS_ID,
    CORE_CATALOG_COLLECTION_ENTRY_CLASS_ID, CORE_CATALOG_INDEX_ENTRY_CLASS_ID,
    CORE_CATALOG_META_ENTRY_CLASS_ID, CORE_CATALOG_RECORD_TYPE_ENTRY_CLASS_ID,
    CORE_CATALOG_SCHEMA_COLLECTION, CORE_CATALOG_TYPE_DEF_ENTRY_CLASS_ID,
    catalog::OBJECT_TYPE_FIELD,
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
const INTEGRITY_MODE_FIELD: &str = "integrity_mode";
const COLLECTION_KIND_FIELD: &str = "collection_kind";
const INTERNAL_FIELD: &str = "internal";
const FIELD_IDS_FIELD: &str = "field_ids";
const COLLECTION_FIELD: &str = "collection";
const FIELD_FIELD: &str = "field";
const INDEX_KIND_FIELD: &str = "index_kind";
const UNIQUE_FIELD: &str = "unique";
const NEXT_FIELD_ID_FIELD: &str = "next_field_id";
const AUTO_INDEX_ENABLED_FIELD: &str = "auto_index_enabled";
const RELATIONSHIPS_FIELD: &str = "relationships";
const PACKAGES_FIELD: &str = "packages";
const APPLIED_MIGRATIONS_FIELD: &str = "applied_migrations";

struct CatalogCollections {
    schema: semantic_db_core::catalog::LocalCollectionId,
}

fn core_collection_ids(catalog: &Catalog) -> std::result::Result<CatalogCollections, DbError> {
    let collection_lid = catalog
        .collection_by_name(CORE_CATALOG_SCHEMA_COLLECTION)
        .map(|schema| schema.lid)
        .ok_or_else(|| {
            DbError::InvalidQuery(format!(
                "missing core catalog collection '{CORE_CATALOG_SCHEMA_COLLECTION}'"
            ))
        })?;
    Ok(CatalogCollections {
        schema: collection_lid,
    })
}

const ENTRY_TYPE_ATTRIBUTE: &str = CORE_CATALOG_ATTRIBUTE_ENTRY_CLASS_ID;
const ENTRY_TYPE_TYPE_DEF: &str = CORE_CATALOG_TYPE_DEF_ENTRY_CLASS_ID;
const ENTRY_TYPE_RECORD_TYPE: &str = CORE_CATALOG_RECORD_TYPE_ENTRY_CLASS_ID;
const ENTRY_TYPE_CLASS: &str = CORE_CATALOG_CLASS_ENTRY_CLASS_ID;
const ENTRY_TYPE_COLLECTION: &str = CORE_CATALOG_COLLECTION_ENTRY_CLASS_ID;
const ENTRY_TYPE_INDEX: &str = CORE_CATALOG_INDEX_ENTRY_CLASS_ID;
const ENTRY_TYPE_META: &str = CORE_CATALOG_META_ENTRY_CLASS_ID;

fn encode_entity_base(
    collection: semantic_db_core::catalog::LocalCollectionId,
    entry_type: &str,
    id: String,
    lid: usize,
) -> StoredEntity {
    let mut object = Object::new();
    object.insert(ID_FIELD.to_string(), Value::String(id.clone()));
    object.insert(LID_FIELD.to_string(), Value::U64(lid as u64));
    object.insert(
        OBJECT_TYPE_FIELD.to_string(),
        Value::String(entry_type.to_string()),
    );
    StoredEntity {
        id,
        collection: collection.0,
        // Catalog rows always carry a registered catalog entry class in `type`.
        kind: StoredEntityKind::Class,
        object,
    }
}

pub fn load_catalog<E: KvEngine>(
    store: &EntityStore<E>,
    bootstrap_catalog: &Catalog,
) -> std::result::Result<Option<Catalog>, DbError> {
    let core = core_collection_ids(bootstrap_catalog)?;
    let attributes_rows =
        load_rows_by_entry_type(store, bootstrap_catalog, core.schema, ENTRY_TYPE_ATTRIBUTE)?;
    let type_defs_rows =
        load_rows_by_entry_type(store, bootstrap_catalog, core.schema, ENTRY_TYPE_TYPE_DEF)?;
    let record_types_rows = load_rows_by_entry_type(
        store,
        bootstrap_catalog,
        core.schema,
        ENTRY_TYPE_RECORD_TYPE,
    )?;
    let classes_rows =
        load_rows_by_entry_type(store, bootstrap_catalog, core.schema, ENTRY_TYPE_CLASS)?;
    let collections_rows =
        load_rows_by_entry_type(store, bootstrap_catalog, core.schema, ENTRY_TYPE_COLLECTION)?;
    let indexes_rows =
        load_rows_by_entry_type(store, bootstrap_catalog, core.schema, ENTRY_TYPE_INDEX)?;
    let meta_rows =
        load_rows_by_entry_type(store, bootstrap_catalog, core.schema, ENTRY_TYPE_META)?;

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
        let kind = object_json_field_default(
            &row.object,
            COLLECTION_KIND_FIELD,
            semantic_db_core::catalog::CollectionKind::Schema,
        )?;
        let integrity_mode: IntegrityMode = object_json_field(&row.object, INTEGRITY_MODE_FIELD)?;
        let internal = object_bool_field_default(&row.object, INTERNAL_FIELD, false);
        let field_ids: Vec<StoredFieldId> = object_json_field(&row.object, FIELD_IDS_FIELD)?;
        collections.push(StoredCollection {
            lid,
            name,
            kind: Some(kind),
            integrity_mode,
            internal,
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
    let mut packages = Vec::<StoredPackage>::new();
    let mut applied_migrations = Vec::<StoredAppliedMigration>::new();
    for row in &meta_rows {
        if row.id == META_ROW_ID {
            next_field_id = object_usize_field(&row.object, NEXT_FIELD_ID_FIELD)?;
            auto_index_enabled =
                object_bool_field_default(&row.object, AUTO_INDEX_ENABLED_FIELD, false);
            relationships =
                object_json_field_default(&row.object, RELATIONSHIPS_FIELD, Vec::new())?;
            packages = object_json_field_default(&row.object, PACKAGES_FIELD, Vec::new())?;
            applied_migrations =
                object_json_field_default(&row.object, APPLIED_MIGRATIONS_FIELD, Vec::new())?;
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
        packages,
        applied_migrations,
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

    for key in store.collection_keys(core.schema)? {
        ops.push(KvWriteOp::Delete { key });
    }
    for index in catalog.indexes_for_collection(core.schema) {
        for key in store.index_keys(index.lid)? {
            ops.push(KvWriteOp::Delete { key });
        }
    }

    for (lid, item) in catalog.attributes() {
        let mut entity = encode_entity_base(
            core.schema,
            ENTRY_TYPE_ATTRIBUTE,
            item.attribute.id.clone(),
            lid.0,
        );
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
        let mut entity = encode_entity_base(
            core.schema,
            ENTRY_TYPE_TYPE_DEF,
            item.type_def.name.clone(),
            lid.0,
        );
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
        let mut entity =
            encode_entity_base(core.schema, ENTRY_TYPE_RECORD_TYPE, item.id.clone(), lid.0);
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
        let mut entity =
            encode_entity_base(core.schema, ENTRY_TYPE_CLASS, item.class.id.clone(), lid.0);
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
        let mut entity =
            encode_entity_base(core.schema, ENTRY_TYPE_COLLECTION, item.name.clone(), lid.0);
        entity
            .object
            .insert("name".to_string(), Value::String(item.name.clone()));
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
                facet_json::to_string(&item.kind)
                    .map_err(|err| DbError::Serialization(err.to_string()))?,
            ),
        );
        entity.object.insert(
            INTEGRITY_MODE_FIELD.to_string(),
            Value::String(
                facet_json::to_string(&item.integrity_mode)
                    .map_err(|err| DbError::Serialization(err.to_string()))?,
            ),
        );
        entity
            .object
            .insert(INTERNAL_FIELD.to_string(), Value::Bool(item.internal));
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
            core.schema,
            ENTRY_TYPE_INDEX,
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

    let mut meta_entity =
        encode_entity_base(core.schema, ENTRY_TYPE_META, META_ROW_ID.to_string(), 0);
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
    let packages = catalog
        .packages()
        .map(|(_, package)| StoredPackage {
            package: package.clone(),
        })
        .collect::<Vec<_>>();
    meta_entity.object.insert(
        PACKAGES_FIELD.to_string(),
        Value::String(
            facet_json::to_string(&packages)
                .map_err(|err| DbError::Serialization(err.to_string()))?,
        ),
    );
    let applied_migrations = catalog
        .applied_migrations()
        .map(|(_, applied)| StoredAppliedMigration {
            applied: applied.clone(),
        })
        .collect::<Vec<_>>();
    meta_entity.object.insert(
        APPLIED_MIGRATIONS_FIELD.to_string(),
        Value::String(
            facet_json::to_string(&applied_migrations)
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

fn load_rows_by_entry_type<E: KvEngine>(
    store: &EntityStore<E>,
    _catalog: &Catalog,
    collection: semantic_db_core::catalog::LocalCollectionId,
    entry_type: &str,
) -> std::result::Result<Vec<StoredEntity>, DbError> {
    Ok(store
        .scan_collection(collection)?
        .into_iter()
        .filter(|entity| {
            entity.object.get(OBJECT_TYPE_FIELD).and_then(Value::as_str) == Some(entry_type)
        })
        .collect())
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
