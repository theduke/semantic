use semantic_data::{
    schema::{core::type_kind::TypeKind, core::type_node::Type},
    value::{Object, Value},
};
use thiserror::Error;

use crate::catalog::CollectionSchema;

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
}

pub type ObjectNormalizationResult<T> = std::result::Result<T, ObjectNormalizationError>;

pub fn normalize_object_for_collection(
    collection: &CollectionSchema,
    object: &mut Object,
) -> ObjectNormalizationResult<()> {
    let mut moved = Vec::<(String, String)>::new();

    for key in object.keys() {
        let canonical = collection.canonical_field_name(key);
        if canonical != key {
            moved.push((key.clone(), canonical.to_string()));
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

    if collection.is_closed_field_set() {
        for key in object.keys() {
            if !collection.knows_field(key) {
                return Err(ObjectNormalizationError::UnknownField {
                    collection: collection.name.clone(),
                    field: key.clone(),
                });
            }
        }
    }

    for (key, value) in object.iter() {
        if let Some(ty) = collection.field_type(key) {
            if !type_matches_value(ty, value) {
                return Err(ObjectNormalizationError::TypeMismatch {
                    collection: collection.name.clone(),
                    field: key.clone(),
                    expected: describe_type(ty),
                    actual: describe_value(value),
                });
            }
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
