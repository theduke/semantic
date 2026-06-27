use semantic_data::{
    schema::{
        ClassType, Constraint, Field, LiteralValue, NumberType, RecordType, Type, TypeKind,
        UIntWidth,
    },
    value::{Map, Object, Value},
};
use semantic_db_core::catalog::OBJECT_TYPE_FIELD;

use crate::UiCatalog;

pub fn default_value_for_type(ty: &Type) -> Value {
    if let Some(value) = ty.constraints.iter().find_map(default_constraint_value) {
        return value;
    }
    default_value_for_type_kind(&ty.kind)
}

pub fn default_value_for_type_kind(kind: &TypeKind) -> Value {
    match kind {
        TypeKind::Bool(_) => Value::Bool(false),
        TypeKind::Char(_) | TypeKind::String(_) => Value::String(String::new()),
        TypeKind::Number(number) => default_number_value(number),
        TypeKind::Bytes(_) => Value::Bytes(bytes::Bytes::new()),
        TypeKind::Optional(_) | TypeKind::Temporal(_) | TypeKind::Uuid | TypeKind::IpAddr(_) => {
            Value::Null
        }
        TypeKind::Array(_) | TypeKind::List(_) | TypeKind::Set(_) | TypeKind::Tuple(_) => {
            Value::List(Vec::new())
        }
        TypeKind::Map(_) => Value::Map(Map::new()),
        TypeKind::Record(record) => default_value_for_record(record),
        TypeKind::Class(class) => default_value_for_class_without_catalog(class),
        TypeKind::Attribute(attribute) => default_value_for_type(&attribute.ty),
        TypeKind::Opaque(opaque) if opaque.repr.as_deref() == Some("string") => {
            Value::String(String::new())
        }
        _ => Value::Null,
    }
}

pub fn default_value_for_class(class: &ClassType, catalog: &UiCatalog) -> Value {
    let mut object = match default_value_for_class_without_catalog(class) {
        Value::Object(object) => object,
        _ => Object::new(),
    };
    for field in catalog.class_form_fields(class) {
        if field.class_attribute.computed.is_some() {
            continue;
        }
        if field.field_name != field.storage_field_name {
            object.remove(&field.field_name);
        }
        if let Some(default) = field
            .attribute
            .constraints
            .iter()
            .chain(field.class_attribute.constraints.iter())
            .find_map(default_constraint_value)
        {
            object.insert(field.storage_field_name, default);
        } else if field.class_attribute.required && !object.contains_key(&field.storage_field_name)
        {
            object.insert(field.storage_field_name, Value::Null);
        }
    }
    Value::Object(object)
}

pub fn literal_to_value(value: &LiteralValue) -> Value {
    match value {
        LiteralValue::Null => Value::Null,
        LiteralValue::Bool(value) => Value::Bool(*value),
        LiteralValue::Int(value) => i64::try_from(*value).map(Value::I64).unwrap_or(Value::Null),
        LiteralValue::UInt(value) => u64::try_from(*value).map(Value::U64).unwrap_or(Value::Null),
        LiteralValue::Float(value) => value.parse::<f64>().map(Value::from).unwrap_or(Value::Null),
        LiteralValue::String(value) => Value::String(value.clone()),
        LiteralValue::Bytes(value) => Value::Bytes(bytes::Bytes::from(value.clone())),
        LiteralValue::List(values) => Value::List(values.iter().map(literal_to_value).collect()),
        LiteralValue::Map(values) => {
            let mut object = Object::new();
            for (key, value) in values {
                object.insert(key.clone(), literal_to_value(value));
            }
            Value::Object(object)
        }
    }
}

pub fn object_field_value(parent: &Value, field_name: &str, fallback: Value) -> Value {
    match parent {
        Value::Object(object) => object.get(field_name).cloned().unwrap_or(fallback),
        _ => fallback,
    }
}

pub fn set_object_field_value(parent: &mut Value, field_name: &str, value: Value) {
    let object = ensure_object(parent);
    object.insert(field_name.to_string(), value);
}

pub fn set_optional_object_field_value(parent: &mut Value, field_name: &str, value: Value) {
    let object = ensure_object(parent);
    if matches!(value, Value::Null | Value::Void) {
        object.remove(field_name);
    } else {
        object.insert(field_name.to_string(), value);
    }
}

pub fn value_as_list(value: &Value) -> Vec<Value> {
    match value {
        Value::List(values) => values.clone(),
        _ => Vec::new(),
    }
}

pub fn set_value_list(value: &mut Value, items: Vec<Value>) {
    *value = Value::List(items);
}

fn ensure_object(value: &mut Value) -> &mut Object {
    if !matches!(value, Value::Object(_)) {
        *value = Value::Object(Object::new());
    }
    match value {
        Value::Object(object) => object,
        _ => unreachable!("value was normalized to object"),
    }
}

fn default_value_for_class_without_catalog(class: &ClassType) -> Value {
    let mut object = Object::new();
    object.insert(OBJECT_TYPE_FIELD, Value::String(class.id.clone()));
    for (field_name, class_attribute) in &class.attributes {
        if class_attribute.computed.is_some() {
            continue;
        }
        if let Some(default) = class_attribute
            .constraints
            .iter()
            .find_map(default_constraint_value)
        {
            object.insert(field_name.clone(), default);
        } else if class_attribute.required {
            object.insert(field_name.clone(), Value::Null);
        }
    }
    Value::Object(object)
}

fn default_value_for_record(record: &RecordType) -> Value {
    let mut object = Object::new();
    for (name, field) in &record.fields {
        if field.required {
            object.insert(name.as_str().to_string(), default_value_for_field(field));
        } else if let Some(default) = field.default.as_ref() {
            object.insert(name.as_str().to_string(), literal_to_value(default));
        }
    }
    Value::Object(object)
}

fn default_value_for_field(field: &Field) -> Value {
    field
        .default
        .as_ref()
        .map(literal_to_value)
        .unwrap_or_else(|| default_value_for_type(&field.ty))
}

fn default_number_value(number: &NumberType) -> Value {
    match number {
        NumberType::UInt(width)
            if !matches!(
                width,
                UIntWidth::U128
                    | UIntWidth::U256
                    | UIntWidth::U24
                    | UIntWidth::U40
                    | UIntWidth::U48
                    | UIntWidth::U56
            ) =>
        {
            Value::U64(0)
        }
        NumberType::UInt(_) => Value::U64(0),
        NumberType::Int(_) | NumberType::BigInt(_) => Value::I64(0),
        NumberType::BigUInt(_) => Value::U64(0),
        NumberType::Float(_)
        | NumberType::Decimal(_)
        | NumberType::Rational(_)
        | NumberType::Complex(_)
        | NumberType::Unspecified => Value::from(0.0f64),
    }
}

fn default_constraint_value(constraint: &Constraint) -> Option<Value> {
    match constraint {
        Constraint::DefaultValue { value } => Some(literal_to_value(value)),
        _ => None,
    }
}
