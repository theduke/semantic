use std::collections::{BTreeSet, HashSet};

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
    #[error("field '{field}' is computed and cannot be written in collection '{collection}'")]
    ComputedFieldNotWritable { collection: String, field: String },
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
    let mut registered_field_required = FnvHashMap::default();
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
                    &mut registered_field_required,
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
        None => {
            best_effort_normalize_registered_attributes(catalog, collection, object)?;
            reject_unknown_fields |=
                collection.integrity_mode == IntegrityMode::StrictRegisteredSchema;
        }
    }

    // Reject writes to computed fields.
    if !collection.computed_fields.is_empty() {
        for key in object.keys() {
            let canonical = collection.canonical_field_name(key);
            if collection.computed_fields.contains(canonical) {
                return Err(ObjectNormalizationError::ComputedFieldNotWritable {
                    collection: collection.name.clone(),
                    field: canonical.to_string(),
                });
            }
        }
    }

    prune_nullish_optional_registered_fields(
        object,
        &registered_field_types,
        &registered_field_required,
    );

    validate_object_fields(
        collection,
        object,
        &registered_field_types,
        reject_unknown_fields,
    )
}

pub fn resolved_field_types_for_object(
    catalog: &Catalog,
    collection: &CollectionSchema,
    object: &Object,
) -> FnvHashMap<String, Type> {
    let mut field_types = FnvHashMap::default();

    for (_, field_name) in collection.fields() {
        if let Some(ty) = collection.field_type(field_name) {
            field_types.insert(field_name.to_string(), ty.clone());
        }
    }

    if collection.kind == CollectionKind::Untyped {
        return field_types;
    }

    let Some(object_type) = object.get(OBJECT_TYPE_FIELD).and_then(Value::as_str) else {
        return field_types;
    };

    let class_ids = catalog.class_ids(object_type);
    let record_ids = catalog.record_type_ids(object_type);
    if class_ids.len() + record_ids.len() != 1 {
        return field_types;
    }

    if let Some(class_lid) = class_ids.first().copied() {
        let mut class_aliases = FnvHashMap::default();
        let mut field_required = FnvHashMap::default();
        collect_class_fields(
            catalog,
            class_lid,
            &mut class_aliases,
            &mut field_types,
            &mut field_required,
        );
    } else if let Some(record_lid) = record_ids.first().copied()
        && let Some(record_type) = catalog.record_type_by_lid(record_lid)
    {
        for (field_name, field) in &record_type.record.fields {
            field_types.insert(field_name.clone(), field.ty.clone());
        }
    }

    field_types
}

pub fn ref_target_class_ids(catalog: &Catalog, ty: &Type) -> Vec<String> {
    let mut out = BTreeSet::new();
    collect_ref_target_class_ids(catalog, ty, &mut out);
    out.into_iter().collect()
}

fn collect_ref_target_class_ids(catalog: &Catalog, ty: &Type, out: &mut BTreeSet<String>) {
    match &ty.kind {
        TypeKind::Ref(type_ref) => {
            let class_ids = catalog.class_ids(&type_ref.name);
            if class_ids.len() != 1 {
                return;
            }
            let target_lid = class_ids[0];
            let Some(target_class) = catalog.class_by_lid(target_lid) else {
                return;
            };
            let target_id = target_class.class.id.clone();
            out.insert(target_id.clone());
            for (class_lid, class) in catalog.classes() {
                if class_lid != target_lid && class_reaches(catalog, class_lid, &target_id) {
                    out.insert(class.class.id.clone());
                }
            }
        }
        TypeKind::Union(union) => {
            for variant in &union.variants {
                collect_ref_target_class_ids(catalog, variant, out);
            }
        }
        TypeKind::Optional(optional) => {
            collect_ref_target_class_ids(catalog, &optional.inner, out);
        }
        _ => {}
    }
}

fn class_reaches(catalog: &Catalog, class_lid: LocalClassId, target_class_id: &str) -> bool {
    fn visit(
        catalog: &Catalog,
        class_lid: LocalClassId,
        target_class_id: &str,
        seen: &mut HashSet<LocalClassId>,
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

    let mut seen = HashSet::new();
    visit(catalog, class_lid, target_class_id, &mut seen)
}

fn best_effort_normalize_registered_attributes(
    catalog: &Catalog,
    collection: &CollectionSchema,
    object: &mut Object,
) -> ObjectNormalizationResult<()> {
    let mut moved = Vec::<(String, String)>::new();
    for (key, value) in object.iter() {
        if is_special_builtin_field(key) {
            // Keep builtins (for example "id"/"type") stable so fallback
            // path traversal and collection-local semantics remain predictable.
            continue;
        }
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
    field_required: &mut FnvHashMap<String, bool>,
) {
    fn visit(
        catalog: &Catalog,
        class_lid: LocalClassId,
        visited: &mut BTreeSet<LocalClassId>,
        field_aliases: &mut FnvHashMap<String, String>,
        field_types: &mut FnvHashMap<String, Type>,
        field_required: &mut FnvHashMap<String, bool>,
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
            visit(
                catalog,
                base_lid,
                visited,
                field_aliases,
                field_types,
                field_required,
            );
        }
        for ext in &class.class.extends {
            if let Some(ext_lid) = catalog.class_id(&ext.id) {
                visit(
                    catalog,
                    ext_lid,
                    visited,
                    field_aliases,
                    field_types,
                    field_required,
                );
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
            field_required.insert(attr.attribute.id.clone(), class_attr.required);
        }
    }

    let mut visited = BTreeSet::new();
    visit(
        catalog,
        class_lid,
        &mut visited,
        field_aliases,
        field_types,
        field_required,
    );
}

fn prune_nullish_optional_registered_fields(
    object: &mut Object,
    registered_field_types: &FnvHashMap<String, Type>,
    registered_field_required: &FnvHashMap<String, bool>,
) {
    object.retain(|key, value| {
        if !value.is_nullish() {
            return true;
        }
        if registered_field_required.get(key).copied().unwrap_or(true) {
            return true;
        }
        let Some(ty) = registered_field_types.get(key) else {
            return true;
        };
        matches!(&ty.kind, TypeKind::Optional(_))
    });
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

/// Error type for computed attribute validation during DDL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputedAttrValidationError {
    pub attribute: String,
    pub message: String,
}

impl std::fmt::Display for ComputedAttrValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "computed attribute '{}': {}",
            self.attribute, self.message
        )
    }
}

/// Validate all computed attributes in a class definition.
/// Returns a list of errors (may be empty).
pub fn validate_class_computed_attributes(
    catalog: &Catalog,
    class: &semantic_data::schema::class::class_type::ClassType,
) -> Vec<ComputedAttrValidationError> {
    // Collect all computed attributes with their expressions.
    let computed_attrs: Vec<(
        &String,
        &semantic_data::schema::class::class_attribute::ClassAttribute,
    )> = class
        .attributes
        .iter()
        .filter(|(_, attr)| attr.computed.is_some())
        .collect();

    if computed_attrs.is_empty() {
        return Vec::new();
    }

    let mut errors = Vec::new();
    let builtins: std::collections::BTreeSet<&str> = ["stringify"].into_iter().collect();

    for (attr_name, attr) in &computed_attrs {
        let expr = attr.computed.as_ref().expect("filtered for Some");

        // Validate the expression tree.
        collect_validation_errors(catalog, class, attr_name, expr, &builtins, &mut errors);
    }

    // Cycle detection among computed attributes.
    detect_computed_cycles(class, &computed_attrs, &mut errors);

    errors
}

fn collect_validation_errors(
    catalog: &Catalog,
    class: &semantic_data::schema::class::class_type::ClassType,
    attr_name: &str,
    expr: &semantic_data::expr::Expr,
    builtins: &std::collections::BTreeSet<&str>,
    errors: &mut Vec<ComputedAttrValidationError>,
) {
    use semantic_data::expr::{self, BinaryOperator};

    match expr {
        expr::Expr::Literal(_) => {}
        expr::Expr::Ref(ref_expr) => {
            match ref_expr {
                expr::RefExpr::Identifier(name) if name == "self" => {
                    // Bare self is allowed as a null check. Not erroring here.
                }
                expr::RefExpr::Identifier(name) => {
                    errors.push(ComputedAttrValidationError {
                        attribute: attr_name.to_string(),
                        message: format!(
                            "unexpected reference '{}' outside 'self' in computed expression",
                            name
                        ),
                    });
                }
                _ => {
                    errors.push(ComputedAttrValidationError {
                        attribute: attr_name.to_string(),
                        message: format!(
                            "unsupported ref variant {:?} in computed expression",
                            ref_expr
                        ),
                    });
                }
            }
        }
        expr::Expr::FieldAccess(fa) => {
            // Resolve self.field
            if let expr::Expr::Ref(expr::RefExpr::Identifier(name)) = &fa.target {
                if name != "self" {
                    errors.push(ComputedAttrValidationError {
                        attribute: attr_name.to_string(),
                        message: format!("field access target must be 'self', got '{}'", name),
                    });
                }
            } else {
                errors.push(ComputedAttrValidationError {
                    attribute: attr_name.to_string(),
                    message: "field access target must be a 'self' reference".to_string(),
                });
            }

            // Resolve the field name against class attributes (including inherited/extended).
            let field_exists = resolve_class_field(catalog, class, &fa.field).is_some();
            if !field_exists {
                errors.push(ComputedAttrValidationError {
                    attribute: attr_name.to_string(),
                    message: format!("field '{}' not found in class '{}'", fa.field, class.id),
                });
            }
        }
        expr::Expr::Binary(bin) => {
            // Both operands must be valid expressions.
            collect_validation_errors(catalog, class, attr_name, &bin.left, builtins, errors);
            collect_validation_errors(catalog, class, attr_name, &bin.right, builtins, errors);

            // Type inference check for Concat.
            if bin.op == BinaryOperator::Concat {
                let left_ty = infer_expr_type(catalog, class, &bin.left);
                let right_ty = infer_expr_type(catalog, class, &bin.right);
                if !is_string_type(&left_ty) {
                    errors.push(ComputedAttrValidationError {
                        attribute: attr_name.to_string(),
                        message: format!(
                            "left operand of concat must be string, got {:?}",
                            left_ty.map(|t| t.kind)
                        ),
                    });
                }
                if !is_string_type(&right_ty) {
                    errors.push(ComputedAttrValidationError {
                        attribute: attr_name.to_string(),
                        message: format!(
                            "right operand of concat must be string, got {:?}",
                            right_ty.map(|t| t.kind)
                        ),
                    });
                }
            }
        }
        expr::Expr::Unary(unary) => {
            collect_validation_errors(catalog, class, attr_name, &unary.operand, builtins, errors);
        }
        expr::Expr::Call(call) => {
            match &call.callee {
                expr::Callee::Name(name) if name.as_slice() == ["stringify"] => {
                    // Validate args are position-only, single arg.
                    if call.args.len() != 1 {
                        errors.push(ComputedAttrValidationError {
                            attribute: attr_name.to_string(),
                            message: format!(
                                "stringify expects exactly 1 argument, got {}",
                                call.args.len()
                            ),
                        });
                    }
                    for arg in &call.args {
                        if let expr::CallArg::Positional(e) = arg {
                            collect_validation_errors(
                                catalog, class, attr_name, e, builtins, errors,
                            );
                        } else {
                            errors.push(ComputedAttrValidationError {
                                attribute: attr_name.to_string(),
                                message: "named arguments not supported in computed expressions"
                                    .to_string(),
                            });
                        }
                    }
                }
                expr::Callee::Name(name) => {
                    let fn_name = name.join(".");
                    if !builtins.contains(fn_name.as_str()) {
                        errors.push(ComputedAttrValidationError {
                            attribute: attr_name.to_string(),
                            message: format!(
                                "unknown function '{}' in computed expression",
                                fn_name
                            ),
                        });
                    }
                }
                expr::Callee::Expr(_) => {
                    errors.push(ComputedAttrValidationError {
                        attribute: attr_name.to_string(),
                        message: "dynamic callee expressions not supported in computed expressions"
                            .to_string(),
                    });
                }
            }
        }
        expr::Expr::Cast(cast) => {
            collect_validation_errors(catalog, class, attr_name, &cast.expr, builtins, errors);
        }
        expr::Expr::If(if_expr) => {
            collect_validation_errors(
                catalog,
                class,
                attr_name,
                &if_expr.condition,
                builtins,
                errors,
            );
            collect_validation_errors(
                catalog,
                class,
                attr_name,
                &if_expr.then_expr,
                builtins,
                errors,
            );
            collect_validation_errors(
                catalog,
                class,
                attr_name,
                &if_expr.else_expr,
                builtins,
                errors,
            );
        }
        expr::Expr::Case(case) => {
            if let Some(ref operand) = case.operand {
                collect_validation_errors(catalog, class, attr_name, operand, builtins, errors);
            }
            for branch in &case.branches {
                collect_validation_errors(
                    catalog,
                    class,
                    attr_name,
                    &branch.when,
                    builtins,
                    errors,
                );
                collect_validation_errors(
                    catalog,
                    class,
                    attr_name,
                    &branch.then_expr,
                    builtins,
                    errors,
                );
            }
            if let Some(ref else_expr) = case.else_expr {
                collect_validation_errors(catalog, class, attr_name, else_expr, builtins, errors);
            }
        }
        expr::Expr::Let(let_expr) => {
            for binding in &let_expr.bindings {
                collect_validation_errors(
                    catalog,
                    class,
                    attr_name,
                    &binding.value,
                    builtins,
                    errors,
                );
            }
            collect_validation_errors(catalog, class, attr_name, &let_expr.body, builtins, errors);
        }
        // Reject prohibited expressions.
        expr::Expr::Query(_)
        | expr::Expr::Subquery(_)
        | expr::Expr::Exists(_)
        | expr::Expr::In(_)
        | expr::Expr::Lambda(_)
        | expr::Expr::Tuple(_)
        | expr::Expr::List(_)
        | expr::Expr::Map(_)
        | expr::Expr::IndexAccess(_)
        | expr::Expr::Between(_)
        | expr::Expr::Like(_)
        | expr::Expr::Regex(_)
        | expr::Expr::IsNull(_) => {
            errors.push(ComputedAttrValidationError {
                attribute: attr_name.to_string(),
                message: format!(
                    "{:?} is not allowed in computed expressions",
                    std::mem::discriminant(expr)
                ),
            });
        }
    }
}

/// Resolve a field name against a class's own attributes and inherited/extended classes.
fn resolve_class_field(
    catalog: &Catalog,
    class: &semantic_data::schema::class::class_type::ClassType,
    field_name: &str,
) -> Option<()> {
    use semantic_data::schema::class::class_type::ClassType;

    fn visit(
        catalog: &Catalog,
        class: &ClassType,
        field_name: &str,
        visited: &mut std::collections::BTreeSet<String>,
    ) -> Option<()> {
        if !visited.insert(class.id.clone()) {
            return None;
        }
        // Check own attributes.
        for (alias, attr) in &class.attributes {
            if alias == field_name || attr.attribute.id == field_name {
                return Some(());
            }
            // Also check the attribute's resolved canonical id.
            if let Some(attr_schema) = catalog.attribute_by_id(&attr.attribute.id) {
                if attr_schema.names.plain_name == field_name
                    || attr_schema.names.underscore_name == field_name
                {
                    return Some(());
                }
            }
        }
        // Check inherited class.
        if let Some(inherits) = &class.inherits {
            if let Some(base_lid) = catalog.class_id(&inherits.id) {
                if let Some(base_class) = catalog.class_by_lid(base_lid) {
                    if visit(catalog, &base_class.class, field_name, visited).is_some() {
                        return Some(());
                    }
                }
            }
        }
        // Check extended classes.
        for ext in &class.extends {
            if let Some(ext_lid) = catalog.class_id(&ext.id) {
                if let Some(ext_class) = catalog.class_by_lid(ext_lid) {
                    if visit(catalog, &ext_class.class, field_name, visited).is_some() {
                        return Some(());
                    }
                }
            }
        }
        None
    }

    let mut visited = std::collections::BTreeSet::new();
    visit(catalog, class, field_name, &mut visited)
}

/// Minimal type inference for expressions in computed attributes.
fn infer_expr_type(
    catalog: &Catalog,
    class: &semantic_data::schema::class::class_type::ClassType,
    expr: &semantic_data::expr::Expr,
) -> Option<semantic_data::schema::core::type_node::Type> {
    use semantic_data::{
        expr::{self, BinaryOperator},
        schema::{
            core::{type_kind::TypeKind as TK, type_node::Type},
            primitives::string_type::StringType,
        },
    };

    match expr {
        expr::Expr::Literal(lit) => match &lit.value {
            semantic_data::schema::core::literal_value::LiteralValue::String(_) => {
                return Some(Type {
                    kind: TK::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    constraints: vec![],
                    annotations: vec![],
                });
            }
            _ => return None,
        },
        expr::Expr::Ref(expr::RefExpr::Identifier(name)) if name == "self" => {
            return None;
        }
        expr::Expr::FieldAccess(fa) => {
            if let expr::Expr::Ref(expr::RefExpr::Identifier(n)) = &fa.target {
                if n == "self" {
                    let mut visited = std::collections::BTreeSet::new();
                    return resolve_field_type(catalog, class, &fa.field, &mut visited);
                }
            }
            return None;
        }
        expr::Expr::Binary(bin) => {
            if bin.op == BinaryOperator::Concat {
                let left = infer_expr_type(catalog, class, &bin.left);
                let right = infer_expr_type(catalog, class, &bin.right);
                if left.is_some() || right.is_some() {
                    return Some(Type {
                        kind: TK::String(StringType {
                            format: None,
                            normalization: None,
                        }),
                        constraints: vec![],
                        annotations: vec![],
                    });
                }
            }
        }
        expr::Expr::Call(call) => {
            if matches!(&call.callee, expr::Callee::Name(name) if name.as_slice() == ["stringify"])
            {
                return Some(Type {
                    kind: TK::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    constraints: vec![],
                    annotations: vec![],
                });
            }
        }
        _ => {}
    }
    None
}

fn is_string_type(ty: &Option<Type>) -> bool {
    ty.as_ref()
        .is_some_and(|t| matches!(t.kind, TypeKind::String(_)))
}

fn resolve_field_type(
    catalog: &Catalog,
    class: &semantic_data::schema::class::class_type::ClassType,
    field_name: &str,
    visited: &mut std::collections::BTreeSet<String>,
) -> Option<Type> {
    if !visited.insert(class.id.clone()) {
        return None;
    }
    // Check own attributes.
    for (alias, attr) in &class.attributes {
        if alias == field_name || attr.attribute.id == field_name {
            if let Some(attr_schema) = catalog.attribute_by_id(&attr.attribute.id) {
                return Some(attr_schema.attribute.ty.clone());
            }
        }
        // Also check aliases.
        if let Some(attr_schema) = catalog.attribute_by_id(&attr.attribute.id) {
            if attr_schema.names.plain_name == field_name
                || attr_schema.names.underscore_name == field_name
            {
                return Some(attr_schema.attribute.ty.clone());
            }
        }
    }
    // Check inherited class.
    if let Some(inherits) = &class.inherits {
        if let Some(base_lid) = catalog.class_id(&inherits.id) {
            if let Some(base_class) = catalog.class_by_lid(base_lid) {
                if let Some(ty) =
                    resolve_field_type(catalog, &base_class.class, field_name, visited)
                {
                    return Some(ty);
                }
            }
        }
    }
    // Check extended classes.
    for ext in &class.extends {
        if let Some(ext_lid) = catalog.class_id(&ext.id) {
            if let Some(ext_class) = catalog.class_by_lid(ext_lid) {
                if let Some(ty) = resolve_field_type(catalog, &ext_class.class, field_name, visited)
                {
                    return Some(ty);
                }
            }
        }
    }
    None
}

/// Detect cycles among computed attributes.
fn detect_computed_cycles(
    _class: &semantic_data::schema::class::class_type::ClassType,
    computed_attrs: &[(
        &String,
        &semantic_data::schema::class::class_attribute::ClassAttribute,
    )],
    errors: &mut Vec<ComputedAttrValidationError>,
) {
    use std::collections::{BTreeMap, BTreeSet};

    // Build a dependency graph: computed_attr_name -> fields it references.
    let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (attr_name, attr) in computed_attrs {
        let expr = attr.computed.as_ref().expect("filtered for Some");
        let mut refs = Vec::new();
        collect_field_refs_owned(expr, &mut refs);
        deps.insert(attr_name.to_string(), refs);
    }

    // Check for cycles using DFS.
    for attr_name in deps.keys().cloned().collect::<Vec<_>>() {
        if has_cycle_owned(&attr_name, &attr_name, &deps, &mut BTreeSet::new()) {
            errors.push(ComputedAttrValidationError {
                attribute: attr_name,
                message: "computed attribute contains a cycle (directly or transitively references itself)".to_string(),
            });
        }
    }
}

fn collect_field_refs_owned(expr: &semantic_data::expr::Expr, refs: &mut Vec<String>) {
    use semantic_data::expr::{self, Expr as E};
    match expr {
        E::FieldAccess(fa) => {
            if let E::Ref(expr::RefExpr::Identifier(name)) = &fa.target {
                if name == "self" {
                    refs.push(fa.field.clone());
                }
            }
            collect_field_refs_owned(&fa.target, refs);
        }
        E::Literal(_) | E::Ref(_) => {}
        E::Unary(unary) => collect_field_refs_owned(&unary.operand, refs),
        E::Binary(bin) => {
            collect_field_refs_owned(&bin.left, refs);
            collect_field_refs_owned(&bin.right, refs);
        }
        E::Call(call) => {
            for arg in &call.args {
                if let expr::CallArg::Positional(e) = arg {
                    collect_field_refs_owned(e, refs);
                }
            }
        }
        E::Cast(cast) => collect_field_refs_owned(&cast.expr, refs),
        E::If(if_expr) => {
            collect_field_refs_owned(&if_expr.condition, refs);
            collect_field_refs_owned(&if_expr.then_expr, refs);
            collect_field_refs_owned(&if_expr.else_expr, refs);
        }
        E::Case(case) => {
            if let Some(ref operand) = case.operand {
                collect_field_refs_owned(operand, refs);
            }
            for branch in &case.branches {
                collect_field_refs_owned(&branch.when, refs);
                collect_field_refs_owned(&branch.then_expr, refs);
            }
            if let Some(ref else_expr) = case.else_expr {
                collect_field_refs_owned(else_expr, refs);
            }
        }
        E::Let(let_expr) => {
            for binding in &let_expr.bindings {
                collect_field_refs_owned(&binding.value, refs);
            }
            collect_field_refs_owned(&let_expr.body, refs);
        }
        E::Tuple(tuple) => {
            for item in &tuple.items {
                collect_field_refs_owned(item, refs);
            }
        }
        E::List(list) => {
            for item in &list.items {
                collect_field_refs_owned(item, refs);
            }
        }
        E::Map(map) => {
            for entry in &map.entries {
                collect_field_refs_owned(&entry.key, refs);
                collect_field_refs_owned(&entry.value, refs);
            }
        }
        E::IndexAccess(ia) => {
            collect_field_refs_owned(&ia.target, refs);
            collect_field_refs_owned(&ia.index, refs);
        }
        E::Between(between) => {
            collect_field_refs_owned(&between.value, refs);
            collect_field_refs_owned(&between.lower, refs);
            collect_field_refs_owned(&between.upper, refs);
        }
        E::In(in_expr) => {
            collect_field_refs_owned(&in_expr.value, refs);
            if let expr::InSet::Exprs(items) = &in_expr.set {
                for item in items {
                    collect_field_refs_owned(item, refs);
                }
            }
        }
        E::Like(like) => {
            collect_field_refs_owned(&like.value, refs);
            collect_field_refs_owned(&like.pattern, refs);
        }
        E::Regex(regex) => {
            collect_field_refs_owned(&regex.value, refs);
            collect_field_refs_owned(&regex.pattern, refs);
        }
        E::IsNull(is_null) => collect_field_refs_owned(&is_null.value, refs),
        E::Exists(_) => {}
        E::Query(_) | E::Subquery(_) | E::Lambda(_) => {}
    }
}

fn has_cycle_owned(
    start: &str,
    current: &str,
    deps: &std::collections::BTreeMap<String, Vec<String>>,
    visited: &mut std::collections::BTreeSet<String>,
) -> bool {
    if !visited.insert(current.to_string()) {
        return current == start;
    }

    if let Some(refs) = deps.get(current) {
        for dep in refs {
            if deps.contains_key(dep.as_str()) {
                if dep == start || has_cycle_owned(start, dep, deps, visited) {
                    return true;
                }
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semantic_data::schema::{
        attribute::attribute_ref::AttributeRef,
        class::{class_attribute::ClassAttribute, class_type::ClassType},
        collections::optional_type::OptionalType,
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
        }
    }

    fn optional_string_type() -> Type {
        Type {
            kind: TypeKind::Optional(OptionalType {
                inner: Box::new(string_type()),
            }),
            constraints: vec![],
            annotations: vec![],
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
                        ui_order: None,
                        computed: None,
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
    fn prunes_nullish_optional_class_attributes_unless_type_is_optional() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(semantic_data::schema::AttributeType {
            id: "semantic:title".to_string(),
            name: "title".to_string(),
            ty: string_type(),
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog.upsert_attribute(semantic_data::schema::AttributeType {
            id: "semantic:subtitle".to_string(),
            name: "subtitle".to_string(),
            ty: string_type(),
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog.upsert_attribute(semantic_data::schema::AttributeType {
            id: "semantic:maybe_title".to_string(),
            name: "maybe_title".to_string(),
            ty: optional_string_type(),
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_class(ClassType {
                id: "semantic:article".to_string(),
                name: "Article".to_string(),
                inherits: None,
                extends: vec![],
                attributes: BTreeMap::from([
                    (
                        "title".to_string(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: "semantic:title".to_string(),
                            },
                            required: false,
                            ui_order: None,
                            computed: None,
                            constraints: vec![],
                            meta: Meta::default(),
                        },
                    ),
                    (
                        "subtitle".to_string(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: "semantic:subtitle".to_string(),
                            },
                            required: false,
                            ui_order: None,
                            computed: None,
                            constraints: vec![],
                            meta: Meta::default(),
                        },
                    ),
                    (
                        "maybe_title".to_string(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: "semantic:maybe_title".to_string(),
                            },
                            required: false,
                            ui_order: None,
                            computed: None,
                            constraints: vec![],
                            meta: Meta::default(),
                        },
                    ),
                ]),
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
            semantic_data::value::Value::String("semantic:article".to_string()),
        );
        object.insert("title".to_string(), semantic_data::value::Value::Null);
        object.insert("subtitle".to_string(), semantic_data::value::Value::Void);
        object.insert("maybe_title".to_string(), semantic_data::value::Value::Null);

        normalize_object_for_collection(&catalog, collection, &mut object).unwrap();

        assert!(!object.contains_key("semantic:title"));
        assert!(!object.contains_key("semantic:subtitle"));
        assert!(matches!(
            object.get("semantic:maybe_title"),
            Some(semantic_data::value::Value::Null)
        ));
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

    #[test]
    fn strict_registered_schema_allows_typeless_known_fields_and_rejects_unknown() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(semantic_data::schema::AttributeType {
            id: "semantic:title".to_string(),
            name: "title".to_string(),
            ty: string_type(),
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_collection(
                "items",
                CollectionKind::Schema,
                IntegrityMode::StrictRegisteredSchema,
            )
            .unwrap();
        let collection = catalog.collection_by_name("items").unwrap();

        let mut typeless_known = semantic_data::value::Object::new();
        typeless_known.insert(
            "semantic_title".to_string(),
            semantic_data::value::Value::String("hello".to_string()),
        );
        normalize_object_for_collection(&catalog, collection, &mut typeless_known).unwrap();
        assert!(typeless_known.contains_key("semantic:title"));

        let mut typeless_unknown = semantic_data::value::Object::new();
        typeless_unknown.insert(
            "rogue".to_string(),
            semantic_data::value::Value::String("x".to_string()),
        );
        let err = normalize_object_for_collection(&catalog, collection, &mut typeless_unknown)
            .unwrap_err();
        assert!(matches!(
            err,
            ObjectNormalizationError::UnknownField { field, .. } if field == "rogue"
        ));
    }
}
