use std::collections::BTreeSet;

use fnv::FnvHashMap;
use semantic_data::{
    schema::{core::type_kind::TypeKind, core::type_node::Type},
    value::{Object, Value},
};
use thiserror::Error;

use crate::catalog::{
    Catalog, CollectionKind, CollectionSchema, IntegrityMode, LocalClassId, OBJECT_TYPE_FIELD,
    is_special_builtin_field,
};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ObjectNormalizationError {
    #[error(
        "field alias conflict in collection '{collection}': both '{alias}' and '{canonical}' are present"
    )]
    AliasConflict {
        collection: String,
        alias: String,
        canonical: String,
    },
    #[error("field '{field}' is not allowed in closed collection '{collection}'")]
    UnknownField { collection: String, field: String },
    #[error(
        "type mismatch in collection '{collection}' for field '{field}': expected {expected}, got {actual}"
    )]
    TypeMismatch {
        collection: String,
        field: String,
        expected: String,
        actual: String,
    },
    #[error("collection '{collection}' requires a registered object type")]
    MissingObjectType { collection: String },
    #[error("collection '{collection}' references unknown object type '{object_type}'")]
    UnknownObjectType {
        collection: String,
        object_type: String,
    },
    #[error("collection '{collection}' has ambiguous object type alias '{object_type}'")]
    AmbiguousObjectType {
        collection: String,
        object_type: String,
    },
    #[error("field alias '{alias}' is ambiguous in collection '{collection}'")]
    AmbiguousFieldAlias { collection: String, alias: String },
}

pub type ObjectNormalizationResult<T> = std::result::Result<T, ObjectNormalizationError>;

pub fn normalize_object_for_collection(
    catalog: &Catalog,
    collection: &CollectionSchema,
    object: &mut Object,
) -> ObjectNormalizationResult<()> {
    for key in object.keys() {
        let attr_ids = catalog.attribute_ids(key);
        if attr_ids.len() > 1 && !is_special_builtin_field(key) {
            return Err(ObjectNormalizationError::AmbiguousFieldAlias {
                collection: collection.name.clone(),
                alias: key.clone(),
            });
        }
    }

    normalize_aliases(collection, object, |key| {
        Some(collection.canonical_field_name(key).to_string())
    })?;

    let mut registered_field_types = FnvHashMap::default();
    let mut reject_unknown_fields = collection.is_closed_field_set();

    if collection.kind == CollectionKind::Untyped {
        return validate_object_fields(
            collection,
            object,
            &registered_field_types,
            reject_unknown_fields,
        );
    }

    match object.get(OBJECT_TYPE_FIELD).and_then(Value::as_str) {
        Some(object_type) => {
            let class_ids = catalog.class_ids(object_type);
            let record_ids = catalog.record_type_ids(object_type);
            if class_ids.len() + record_ids.len() > 1 {
                return Err(ObjectNormalizationError::AmbiguousObjectType {
                    collection: collection.name.clone(),
                    object_type: object_type.to_string(),
                });
            }
            if let Some(class_lid) = class_ids.first().copied() {
                if let Some(class) = catalog.class_by_lid(class_lid) {
                    if should_canonicalize_object_type(object_type) {
                        object.insert(
                            OBJECT_TYPE_FIELD.to_string(),
                            Value::String(class.class.id.clone()),
                        );
                    }
                }
                let mut class_aliases = FnvHashMap::default();
                collect_class_fields(
                    catalog,
                    class_lid,
                    &mut class_aliases,
                    &mut registered_field_types,
                );
                normalize_aliases(collection, object, |key| class_aliases.get(key).cloned())?;
                reject_unknown_fields |=
                    collection.integrity_mode == IntegrityMode::StrictRegisteredSchema;
            } else if let Some(record_lid) = record_ids.first().copied() {
                let record_type = catalog
                    .record_type_by_lid(record_lid)
                    .expect("record type id must resolve");
                if should_canonicalize_object_type(object_type) {
                    object.insert(
                        OBJECT_TYPE_FIELD.to_string(),
                        Value::String(record_type.id.clone()),
                    );
                }
                for (field_name, field) in &record_type.record.fields {
                    registered_field_types.insert(field_name.clone(), field.ty.clone());
                }
                reject_unknown_fields |= collection.integrity_mode
                    == IntegrityMode::StrictRegisteredSchema
                    || !record_type.record.open;
            } else if collection.integrity_mode == IntegrityMode::StrictRegisteredSchema {
                return Err(ObjectNormalizationError::UnknownObjectType {
                    collection: collection.name.clone(),
                    object_type: object_type.to_string(),
                });
            }
        }
        None if collection.integrity_mode == IntegrityMode::StrictRegisteredSchema => {
            return Err(ObjectNormalizationError::MissingObjectType {
                collection: collection.name.clone(),
            });
        }
        None => {
            best_effort_normalize_registered_attributes(catalog, collection, object)?;
        }
    }

    validate_object_fields(
        collection,
        object,
        &registered_field_types,
        reject_unknown_fields,
    )
}

fn best_effort_normalize_registered_attributes(
    catalog: &Catalog,
    collection: &CollectionSchema,
    object: &mut Object,
) -> ObjectNormalizationResult<()> {
    let mut moved = Vec::<(String, String)>::new();
    for (key, value) in object.iter() {
        let attr_ids = catalog.attribute_ids(key);
        if attr_ids.len() > 1 && !is_special_builtin_field(key) {
            return Err(ObjectNormalizationError::AmbiguousFieldAlias {
                collection: collection.name.clone(),
                alias: key.clone(),
            });
        }
        let Some(attr_id) = attr_ids.first().copied() else {
            continue;
        };
        let Some(attribute) = catalog.attribute_by_lid(attr_id) else {
            continue;
        };
        let canonical = attribute.attribute.id.clone();
        if canonical == *key {
            continue;
        }
        if !type_matches_value(&attribute.attribute.ty, value) {
            continue;
        }
        moved.push((key.clone(), canonical));
    }

    for (from, to) in moved {
        if object.contains_key(&to) {
            return Err(ObjectNormalizationError::AliasConflict {
                collection: collection.name.clone(),
                alias: from,
                canonical: to,
            });
        }
        let value = object.remove(&from).expect("key must exist");
        object.insert(to, value);
    }
    Ok(())
}

fn should_canonicalize_object_type(value: &str) -> bool {
    value.contains(':') || value.contains('.') || value.contains('_')
}

fn normalize_aliases<F>(
    collection: &CollectionSchema,
    object: &mut Object,
    canonicalize: F,
) -> ObjectNormalizationResult<()>
where
    F: Fn(&str) -> Option<String>,
{
    let mut moved = Vec::<(String, String)>::new();

    for key in object.keys() {
        let Some(canonical) = canonicalize(key) else {
            continue;
        };
        if canonical.as_str() != key {
            moved.push((key.clone(), canonical));
        }
    }

    for (from, to) in moved {
        if object.contains_key(&to) {
            return Err(ObjectNormalizationError::AliasConflict {
                collection: collection.name.clone(),
                alias: from,
                canonical: to,
            });
        }
        let value = object.remove(&from).expect("field must exist");
        object.insert(to, value);
    }

    Ok(())
}

fn collect_class_fields(
    catalog: &Catalog,
    class_lid: LocalClassId,
    field_aliases: &mut FnvHashMap<String, String>,
    field_types: &mut FnvHashMap<String, Type>,
) {
    fn visit(
        catalog: &Catalog,
        class_lid: LocalClassId,
        visited: &mut BTreeSet<LocalClassId>,
        field_aliases: &mut FnvHashMap<String, String>,
        field_types: &mut FnvHashMap<String, Type>,
    ) {
        if !visited.insert(class_lid) {
            return;
        }
        let class = catalog
            .class_by_lid(class_lid)
            .expect("class id must resolve in catalog");
        if let Some(inherits) = &class.class.inherits
            && let Some(base_lid) = catalog.class_id(&inherits.id)
        {
            visit(catalog, base_lid, visited, field_aliases, field_types);
        }
        for ext in &class.class.extends {
            if let Some(ext_lid) = catalog.class_id(&ext.id) {
                visit(catalog, ext_lid, visited, field_aliases, field_types);
            }
        }
        for (alias, class_attr) in &class.class.attributes {
            let attr = catalog
                .attribute_by_id(&class_attr.attribute.id)
                .expect("class attribute must resolve in catalog");
            field_aliases.insert(alias.clone(), attr.attribute.id.clone());
            field_aliases.insert(attr.names.plain_name.clone(), attr.attribute.id.clone());
            field_aliases.insert(
                attr.names.underscore_name.clone(),
                attr.attribute.id.clone(),
            );
            field_types.insert(attr.attribute.id.clone(), attr.attribute.ty.clone());
        }
    }

    let mut visited = BTreeSet::new();
    visit(catalog, class_lid, &mut visited, field_aliases, field_types);
}

fn validate_object_fields(
    collection: &CollectionSchema,
    object: &Object,
    registered_field_types: &FnvHashMap<String, Type>,
    reject_unknown_fields: bool,
) -> ObjectNormalizationResult<()> {
    if reject_unknown_fields {
        for key in object.keys() {
            if !collection.knows_field(key) && !registered_field_types.contains_key(key) {
                return Err(ObjectNormalizationError::UnknownField {
                    collection: collection.name.clone(),
                    field: key.clone(),
                });
            }
        }
    }

    for (key, value) in object.iter() {
        let ty = collection
            .field_type(key)
            .or_else(|| registered_field_types.get(key));
        if let Some(ty) = ty
            && !type_matches_value(ty, value)
        {
            return Err(ObjectNormalizationError::TypeMismatch {
                collection: collection.name.clone(),
                field: key.clone(),
                expected: describe_type(ty),
                actual: describe_value(value),
            });
        }
    }

    Ok(())
}

fn describe_type(ty: &Type) -> String {
    format!("{:?}", ty.kind)
}

fn describe_value(value: &Value) -> String {
    match value {
        Value::Void => "void",
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::I8(_)
        | Value::I16(_)
        | Value::I32(_)
        | Value::I64(_)
        | Value::I128(_)
        | Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::U64(_)
        | Value::U128(_)
        | Value::F32(_)
        | Value::F64(_) => "number",
        Value::String(_) => "string",
        Value::Bytes(_) => "bytes",
        Value::Date(_) => "date",
        Value::Time(_) => "time",
        Value::DateTime(_) => "datetime",
        Value::Duration(_) => "duration",
        Value::Uuid(_) => "uuid",
        Value::IpAddr(_) => "ip_addr",
        Value::List(_) => "list",
        Value::Map(_) => "map",
        Value::Object(_) => "object",
        Value::Variant(_) => "variant",
    }
    .to_string()
}

fn type_matches_value(ty: &Type, value: &Value) -> bool {
    match &ty.kind {
        TypeKind::Any(_) | TypeKind::Unknown(_) => true,
        TypeKind::Never(_) => false,

        TypeKind::Null(_) => matches!(value, Value::Null),
        TypeKind::Bool(_) => matches!(value, Value::Bool(_)),
        TypeKind::Char(_) => matches!(value, Value::String(v) if v.chars().count() == 1),

        TypeKind::Number(_) => matches!(
            value,
            Value::I8(_)
                | Value::I16(_)
                | Value::I32(_)
                | Value::I64(_)
                | Value::I128(_)
                | Value::U8(_)
                | Value::U16(_)
                | Value::U32(_)
                | Value::U64(_)
                | Value::U128(_)
                | Value::F32(_)
                | Value::F64(_)
        ),
        TypeKind::String(_) => matches!(value, Value::String(_)),
        TypeKind::Bytes(_) => matches!(value, Value::Bytes(_)),
        TypeKind::Temporal(_) => matches!(
            value,
            Value::Date(_) | Value::Time(_) | Value::DateTime(_) | Value::Duration(_)
        ),
        TypeKind::Uuid => matches!(value, Value::Uuid(_)),
        TypeKind::IpAddr(_) => matches!(value, Value::IpAddr(_)),

        TypeKind::Optional(optional) => {
            value.is_nullish() || type_matches_value(&optional.inner, value)
        }
        TypeKind::Array(array) => match value {
            Value::List(items) => items
                .iter()
                .all(|item| type_matches_value(&array.items, item)),
            _ => false,
        },
        TypeKind::List(list) => match value {
            Value::List(items) => items
                .iter()
                .all(|item| type_matches_value(&list.items, item)),
            _ => false,
        },
        TypeKind::Tuple(tuple) => match value {
            Value::List(items) => {
                if items.len() < tuple.items.len() {
                    return false;
                }

                for (item, ty) in items.iter().zip(tuple.items.iter()) {
                    if !type_matches_value(ty, item) {
                        return false;
                    }
                }

                if items.len() > tuple.items.len() {
                    if let Some(rest) = &tuple.rest {
                        for item in items.iter().skip(tuple.items.len()) {
                            if !type_matches_value(rest, item) {
                                return false;
                            }
                        }
                    } else {
                        return false;
                    }
                }

                true
            }
            _ => false,
        },
        TypeKind::Map(map) => match value {
            Value::Map(entries) => entries.iter().all(|(key, value)| {
                type_matches_value(&map.keys, key) && type_matches_value(&map.values, value)
            }),
            _ => false,
        },

        TypeKind::Record(_) | TypeKind::Class(_) | TypeKind::Json => {
            matches!(value, Value::Object(_))
        }

        TypeKind::Union(_)
        | TypeKind::Intersection(_)
        | TypeKind::Variant(_)
        | TypeKind::Enum(_)
        | TypeKind::Result(_)
        | TypeKind::Function(_)
        | TypeKind::Interface(_)
        | TypeKind::Handle(_)
        | TypeKind::Stream(_)
        | TypeKind::Set(_)
        | TypeKind::Opaque(_)
        | TypeKind::Extension(_)
        | TypeKind::Attribute(_)
        | TypeKind::Ref(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semantic_data::schema::{
        attribute::attribute_ref::AttributeRef,
        class::{class_attribute::ClassAttribute, class_type::ClassType},
        core::{meta::Meta, type_kind::TypeKind, type_node::Type},
        primitives::string_type::StringType,
    };

    use crate::catalog::{Catalog, CollectionKind, IntegrityMode, OBJECT_TYPE_FIELD};

    use super::{ObjectNormalizationError, normalize_object_for_collection};

    fn string_type() -> Type {
        Type {
            kind: TypeKind::String(StringType {
                format: None,
                normalization: None,
            }),
            constraints: vec![],
            annotations: vec![],
            meta: Meta::default(),
        }
    }

    #[test]
    fn normalizes_schema_object_by_class_and_aliases() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(semantic_data::schema::AttributeType {
            id: "semantic:title".to_string(),
            name: "title".to_string(),
            ty: string_type(),
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_class(ClassType {
                id: "semantic:article".to_string(),
                name: "Article".to_string(),
                inherits: None,
                extends: vec![],
                attributes: BTreeMap::from([(
                    "title".to_string(),
                    ClassAttribute {
                        attribute: AttributeRef {
                            id: "semantic:title".to_string(),
                        },
                        required: false,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )]),
                constraints: vec![],
                meta: Meta::default(),
            })
            .unwrap();
        let _ = catalog
            .upsert_collection(
                "items",
                CollectionKind::Schema,
                IntegrityMode::StrictRegisteredSchema,
            )
            .unwrap();
        let collection = catalog.collection_by_name("items").unwrap();

        let mut object = semantic_data::value::Object::new();
        object.insert(
            OBJECT_TYPE_FIELD.to_string(),
            semantic_data::value::Value::String("semantic_article".to_string()),
        );
        object.insert(
            "title".to_string(),
            semantic_data::value::Value::String("hello".to_string()),
        );

        normalize_object_for_collection(&catalog, collection, &mut object).unwrap();

        assert_eq!(
            object
                .get(OBJECT_TYPE_FIELD)
                .and_then(semantic_data::value::Value::as_str),
            Some("semantic:article")
        );
        assert_eq!(
            object
                .get("semantic:title")
                .and_then(semantic_data::value::Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn permissive_best_effort_normalizes_known_attribute_without_class() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(semantic_data::schema::AttributeType {
            id: "semantic:title".to_string(),
            name: "title".to_string(),
            ty: string_type(),
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_collection("items", CollectionKind::Schema, IntegrityMode::Permissive)
            .unwrap();
        let collection = catalog.collection_by_name("items").unwrap();

        let mut object = semantic_data::value::Object::new();
        object.insert(
            "semantic_title".to_string(),
            semantic_data::value::Value::String("hello".to_string()),
        );
        normalize_object_for_collection(&catalog, collection, &mut object).unwrap();
        assert!(object.contains_key("semantic:title"));
    }

    #[test]
    fn permissive_insert_rejects_ambiguous_plain_attribute_alias() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(semantic_data::schema::AttributeType {
            id: "semantic:title".to_string(),
            name: "title".to_string(),
            ty: string_type(),
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog.upsert_attribute(semantic_data::schema::AttributeType {
            id: "shared:blog:title".to_string(),
            name: "title".to_string(),
            ty: string_type(),
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_collection("items", CollectionKind::Schema, IntegrityMode::Permissive)
            .unwrap();
        let collection = catalog.collection_by_name("items").unwrap();

        let mut object = semantic_data::value::Object::new();
        object.insert(
            "title".to_string(),
            semantic_data::value::Value::String("hello".to_string()),
        );
        let err = normalize_object_for_collection(&catalog, collection, &mut object).unwrap_err();
        assert!(matches!(
            err,
            ObjectNormalizationError::AmbiguousFieldAlias { alias, .. } if alias == "title"
        ));
    }
}
