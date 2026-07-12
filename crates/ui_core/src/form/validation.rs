use dxform::{FieldPath, FieldValidator, FormError, FormErrorSource};
use semantic_data::{
    schema::{AttributeType, ClassAttribute, Constraint, LengthSpec, NumberBound, Type, TypeKind},
    value::Value,
};

pub fn validators_for_attribute(
    attribute: &AttributeType,
    class_attribute: &ClassAttribute,
) -> Vec<FieldValidator<Value, Value>> {
    let attribute = attribute.clone();
    let class_attribute = class_attribute.clone();
    vec![FieldValidator::sync(move |ctx| {
        let mut errors = Vec::new();
        if class_attribute.required && is_empty_value(&ctx.value) {
            errors.push(coded(ctx.path.clone(), "required", "required field"));
        }
        errors.extend(validate_value_against_type(
            ctx.path.clone(),
            &ctx.value,
            &attribute.ty,
        ));
        errors.extend(validate_value_constraints(
            ctx.path.clone(),
            &ctx.value,
            &attribute.constraints,
        ));
        errors.extend(validate_value_constraints(
            ctx.path,
            &ctx.value,
            &class_attribute.constraints,
        ));
        errors
    })]
}

pub fn validate_value_against_type(path: FieldPath, value: &Value, ty: &Type) -> Vec<FormError> {
    if value.is_nullish() || matches!(ty.kind, TypeKind::Any(_) | TypeKind::Unknown(_)) {
        return Vec::new();
    }
    let ok = match &ty.kind {
        TypeKind::Null(_) => matches!(value, Value::Null),
        TypeKind::Bool(_) => matches!(value, Value::Bool(_)),
        TypeKind::Char(_) => matches!(value, Value::String(v) if v.chars().count() <= 1),
        TypeKind::Number(_) => is_number(value),
        TypeKind::String(_) => matches!(value, Value::String(_)),
        TypeKind::Bytes(_) => matches!(value, Value::Bytes(_)),
        TypeKind::Temporal(_) => matches!(
            value,
            Value::Date(_) | Value::Time(_) | Value::DateTime(_) | Value::Duration(_)
        ),
        TypeKind::Uuid => matches!(value, Value::Uuid(_)),
        TypeKind::IpAddr(_) => matches!(value, Value::IpAddr(_)),
        TypeKind::Optional(optional) => {
            value.is_nullish()
                || validate_value_against_type(path.clone(), value, &optional.inner).is_empty()
        }
        TypeKind::Array(_) | TypeKind::List(_) | TypeKind::Set(_) | TypeKind::Tuple(_) => {
            matches!(value, Value::List(_))
        }
        TypeKind::Map(_) => matches!(value, Value::Map(_)),
        TypeKind::Record(_) | TypeKind::Class(_) => matches!(value, Value::Object(_)),
        TypeKind::Attribute(attribute) => {
            validate_value_against_type(path.clone(), value, &attribute.ty).is_empty()
        }
        _ => true,
    };
    if ok {
        Vec::new()
    } else {
        vec![coded(path, "type", "value does not match expected type")]
    }
}

pub fn validate_value_constraints(
    path: FieldPath,
    value: &Value,
    constraints: &[Constraint],
) -> Vec<FormError> {
    let mut errors = Vec::new();
    for constraint in constraints {
        match constraint {
            Constraint::Min(bound) => validate_min(&mut errors, &path, value, bound),
            Constraint::Max(bound) => validate_max(&mut errors, &path, value, bound),
            Constraint::Length(length) => validate_length(&mut errors, &path, value, length),
            Constraint::MinItems(min) => {
                if let Value::List(items) = value {
                    if items.len() < *min as usize {
                        errors.push(coded(path.clone(), "min_items", "too few items"));
                    }
                }
            }
            Constraint::MaxItems(max) => {
                if let Value::List(items) = value {
                    if items.len() > *max as usize {
                        errors.push(coded(path.clone(), "max_items", "too many items"));
                    }
                }
            }
            Constraint::MinProperties(min) => {
                if let Value::Object(object) = value {
                    if object.len() < *min as usize {
                        errors.push(coded(path.clone(), "min_properties", "too few fields"));
                    }
                }
            }
            Constraint::MaxProperties(max) => {
                if let Value::Object(object) = value {
                    if object.len() > *max as usize {
                        errors.push(coded(path.clone(), "max_properties", "too many fields"));
                    }
                }
            }
            Constraint::Pattern(pattern) => {
                if let Value::String(value) = value {
                    match regex::Regex::new(pattern) {
                        Ok(regex) if !regex.is_match(value) => {
                            errors.push(coded(path.clone(), "pattern", "invalid format"));
                        }
                        Err(_) => errors.push(coded(path.clone(), "pattern", "invalid pattern")),
                        _ => {}
                    }
                }
            }
            Constraint::Prefix(prefix) => {
                if let Value::String(value) = value {
                    if !value.starts_with(prefix) {
                        errors.push(coded(path.clone(), "prefix", "invalid prefix"));
                    }
                }
            }
            Constraint::Suffix(suffix) => {
                if let Value::String(value) = value {
                    if !value.ends_with(suffix) {
                        errors.push(coded(path.clone(), "suffix", "invalid suffix"));
                    }
                }
            }
            Constraint::Contains(expected) => {
                validate_contains(&mut errors, &path, value, expected)
            }
            // Database, transport, precision and expression constraints are intentionally ignored
            // in UI-only validation until the core schema validator is exposed for direct reuse.
            _ => {}
        }
    }
    errors
}

pub fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Void | Value::Null => true,
        Value::String(value) => value.trim().is_empty(),
        Value::Bytes(value) => value.is_empty(),
        Value::List(value) => value.is_empty(),
        Value::Map(value) => value.is_empty(),
        Value::Object(value) => value.is_empty(),
        _ => false,
    }
}

fn validate_min(errors: &mut Vec<FormError>, path: &FieldPath, value: &Value, bound: &NumberBound) {
    let Some(number) = value.as_f64() else {
        return;
    };
    let Some(bound) = parse_bound(bound) else {
        return;
    };
    if number < bound {
        errors.push(coded(path.clone(), "min", "value is too small"));
    }
}

fn validate_max(errors: &mut Vec<FormError>, path: &FieldPath, value: &Value, bound: &NumberBound) {
    let Some(number) = value.as_f64() else {
        return;
    };
    let Some(bound) = parse_bound(bound) else {
        return;
    };
    if number > bound {
        errors.push(coded(path.clone(), "max", "value is too large"));
    }
}

fn validate_length(
    errors: &mut Vec<FormError>,
    path: &FieldPath,
    value: &Value,
    length: &LengthSpec,
) {
    let Some(len) = value_length(value) else {
        return;
    };
    match length {
        LengthSpec::Exactly(exact) if len != *exact as usize => {
            errors.push(coded(path.clone(), "length", "invalid length"));
        }
        LengthSpec::Range { min, max } => {
            if min.is_some_and(|min| len < min as usize)
                || max.is_some_and(|max| len > max as usize)
            {
                errors.push(coded(path.clone(), "length", "invalid length"));
            }
        }
        _ => {}
    }
}

fn validate_contains(
    errors: &mut Vec<FormError>,
    path: &FieldPath,
    value: &Value,
    expected: &Value,
) {
    let contains = match (value, expected) {
        (Value::String(value), Value::String(expected)) => value.contains(expected),
        (Value::List(items), expected) => items.contains(expected),
        _ => true,
    };
    if !contains {
        errors.push(coded(path.clone(), "contains", "required value missing"));
    }
}

fn value_length(value: &Value) -> Option<usize> {
    match value {
        Value::String(value) => Some(value.chars().count()),
        Value::Bytes(value) => Some(value.len()),
        Value::List(value) => Some(value.len()),
        _ => None,
    }
}

fn parse_bound(bound: &NumberBound) -> Option<f64> {
    match bound {
        NumberBound::Inclusive(value) | NumberBound::Exclusive(value) => value.parse().ok(),
    }
}

fn is_number(value: &Value) -> bool {
    matches!(
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
    )
}

fn coded(path: FieldPath, code: impl Into<String>, message: impl Into<String>) -> FormError {
    FormError::coded(path, code, message).with_source(FormErrorSource::Validation)
}
