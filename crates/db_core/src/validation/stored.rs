//! Catalog-aware validation shared by dataset and transaction-overlay execution.
use super::*;
use crate::{DbError, batch_return::EntityKey};
use semantic_data::{
    schema::{ClassConstraint, Constraint, EnumRepr, LengthSpec, NumberBound},
    value::{FieldPath, PathSegment},
};

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet, Error)]
#[error("validation failed at {path:?}: {rule} (expected {expected}, got {actual})")]
pub struct ValidationError {
    pub class: String,
    pub attribute: String,
    pub path: FieldPath,
    pub rule: String,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct ValidationViolation {
    pub collection: String,
    pub id: String,
    pub error: ValidationError,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedReference {
    pub owner: EntityKey,
    pub path: FieldPath,
    pub target: EntityKey,
}

#[derive(Clone)]
struct StoredField {
    ty: Type,
    required: bool,
    computed: bool,
}

fn class_fields(
    catalog: &Catalog,
    lid: LocalClassId,
    out: &mut std::collections::BTreeMap<String, StoredField>,
    seen: &mut BTreeSet<LocalClassId>,
) {
    if !seen.insert(lid) {
        return;
    }
    let class = &catalog.class_by_lid(lid).unwrap().class;
    definition_fields(catalog, class, out, seen);
}

fn definition_fields(
    catalog: &Catalog,
    class: &semantic_data::schema::ClassType,
    out: &mut std::collections::BTreeMap<String, StoredField>,
    seen: &mut BTreeSet<LocalClassId>,
) {
    for base in class.inherits.iter().chain(&class.extends) {
        if let Some(lid) = catalog.class_id(&base.id) {
            class_fields(catalog, lid, out, seen);
        }
    }
    for attr in class.attributes.values() {
        if let Some(global) = catalog.attribute_by_id(&attr.attribute.id) {
            let mut ty = global.attribute.ty.clone();
            if let Some(inherited) = out.get(&global.attribute.id) {
                ty.constraints.extend(inherited.ty.constraints.clone());
            }
            ty.constraints.extend(global.attribute.constraints.clone());
            ty.constraints.extend(attr.constraints.clone());
            out.insert(
                global.attribute.id.clone(),
                StoredField {
                    ty,
                    required: attr.required,
                    computed: attr.computed.is_some(),
                },
            );
        }
    }
    for constraint in &class.constraints {
        if let ClassConstraint::Field {
            attribute,
            constraint,
        } = constraint
        {
            let canonical = catalog
                .attribute_by_id(&attribute.id)
                .map(|a| a.attribute.id.clone())
                .or_else(|| {
                    class
                        .attributes
                        .get(&attribute.id)
                        .map(|attr| attr.attribute.id.clone())
                })
                .or_else(|| {
                    catalog
                        .class_id(&class.id)
                        .and_then(|lid| catalog.class_field_for_alias(lid, &attribute.id))
                });
            if let Some(field) = canonical.and_then(|id| out.get_mut(&id)) {
                field.ty.constraints.push(constraint.clone());
            }
        }
    }
}

fn object_fields(
    catalog: &Catalog,
    collection: &CollectionSchema,
    object: &Object,
) -> std::collections::BTreeMap<String, StoredField> {
    let mut fields = std::collections::BTreeMap::new();
    for (name, mut ty) in resolved_field_types_for_object(catalog, collection, object) {
        if let Some(attr) = catalog.attribute_by_id(&name) {
            ty.constraints.extend(attr.attribute.constraints.clone());
        }
        fields.insert(
            name,
            StoredField {
                ty,
                required: false,
                computed: false,
            },
        );
    }
    if collection.kind != CollectionKind::Untyped {
        if let Some(name) = object.get(OBJECT_TYPE_FIELD).and_then(Value::as_str) {
            if let Some(lid) = catalog.class_id(name) {
                class_fields(catalog, lid, &mut fields, &mut BTreeSet::new());
            } else if let Some(record) = catalog
                .record_type_id(name)
                .and_then(|lid| catalog.record_type_by_lid(lid))
            {
                for (name, field) in &record.record.fields {
                    fields.insert(
                        name.clone(),
                        StoredField {
                            ty: field.ty.clone(),
                            required: field.required,
                            computed: false,
                        },
                    );
                }
            }
        }
    }
    fields
}

fn child(path: &FieldPath, segment: PathSegment) -> FieldPath {
    let mut path = path.clone();
    path.0.push(segment);
    path
}

struct Validator<'a, F> {
    catalog: &'a Catalog,
    owner: &'a EntityKey,
    class: String,
    lookup: F,
    references: Vec<ResolvedReference>,
}

impl<F: FnMut(&EntityKey) -> Result<Option<Object>, DbError>> Validator<'_, F> {
    fn fail<T>(
        &self,
        path: &FieldPath,
        rule: &str,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Result<T, DbError> {
        Err(ValidationError {
            class: self.class.clone(),
            attribute: path
                .0
                .first()
                .and_then(|s| match s {
                    PathSegment::Field(f) => Some(f.clone()),
                    _ => None,
                })
                .unwrap_or_default(),
            path: path.clone(),
            rule: rule.into(),
            expected: expected.into(),
            actual: actual.into(),
        }
        .into())
    }

    fn reference(&mut self, value: &Value, ty: &Type, path: &FieldPath) -> Result<(), DbError> {
        let Some(id) = value.as_str().filter(|id| !id.is_empty()) else {
            return self.fail(
                path,
                "reference",
                "non-empty string ID",
                describe_value(value),
            );
        };
        let target_key = (self.owner.0.clone(), id.to_string());
        let Some(target) = (self.lookup)(&target_key)? else {
            return self.fail(path, "reference", "existing target", id);
        };
        let allowed = ref_target_class_ids(self.catalog, ty);
        let actual = target
            .get(OBJECT_TYPE_FIELD)
            .and_then(Value::as_str)
            .unwrap_or("");
        let canonical = self
            .catalog
            .class_id(actual)
            .and_then(|lid| self.catalog.class_by_lid(lid))
            .map(|c| c.class.id.as_str())
            .unwrap_or(actual);
        if !allowed.is_empty() && !allowed.iter().any(|name| name == canonical) {
            return self.fail(path, "reference_type", allowed.join(" | "), actual);
        }
        self.references.push(ResolvedReference {
            owner: self.owner.clone(),
            path: path.clone(),
            target: target_key,
        });
        Ok(())
    }

    fn value(
        &mut self,
        value: &Value,
        ty: &Type,
        path: &FieldPath,
        depth: usize,
        skip_ref: bool,
    ) -> Result<(), DbError> {
        if depth > 128 {
            return self.fail(
                path,
                "depth",
                "at most 128 nested types",
                "recursive type limit",
            );
        }
        match &ty.kind {
            TypeKind::Optional(optional) if !value.is_nullish() => {
                self.value(value, &optional.inner, path, depth + 1, skip_ref)?
            }
            TypeKind::Optional(_) => return Ok(()),
            TypeKind::Union(union) => {
                let mut errors = Vec::new();
                let mut accepted = false;
                for variant in &union.variants {
                    let checkpoint = self.references.len();
                    match self.value(value, variant, path, depth + 1, skip_ref) {
                        Ok(()) => {
                            accepted = true;
                            break;
                        }
                        Err(DbError::Validation(error)) => {
                            self.references.truncate(checkpoint);
                            errors.push(error);
                        }
                        Err(error) => return Err(error),
                    }
                }
                if !accepted {
                    return self.fail(
                        path,
                        "union",
                        errors
                            .iter()
                            .map(|e| e.expected.as_str())
                            .collect::<Vec<_>>()
                            .join(" | "),
                        describe_value(value),
                    );
                }
                if value.is_nullish() {
                    return Ok(());
                }
            }
            TypeKind::Ref(reference) => {
                if self.catalog.class_id(&reference.name).is_some() {
                    if !skip_ref {
                        self.reference(value, ty, path)?;
                    }
                } else if let Some(def) = self.catalog.type_def_by_name(&reference.name) {
                    let resolved = def.type_def.ty.clone();
                    self.value(value, &resolved, path, depth + 1, skip_ref)?;
                } else if !skip_ref {
                    self.reference(value, ty, path)?;
                }
            }
            TypeKind::Attribute(attr) => {
                self.value(value, &attr.ty, path, depth + 1, skip_ref)?;
                self.constraints(value, &attr.constraints, path)?;
            }
            TypeKind::Array(array) => {
                self.items(value, &array.items, path, depth, skip_ref)?;
                if let Some(length) = &array.length {
                    self.constraints(value, &[Constraint::Length(length.clone())], path)?;
                }
            }
            TypeKind::List(list) => self.items(value, &list.items, path, depth, skip_ref)?,
            TypeKind::Tuple(tuple) => {
                let Value::List(items) = value else {
                    return self.fail(path, "type", "tuple", describe_value(value));
                };
                if items.len() < tuple.items.len()
                    || (tuple.rest.is_none() && items.len() != tuple.items.len())
                {
                    return self.fail(
                        path,
                        "tuple_length",
                        tuple.items.len().to_string(),
                        items.len().to_string(),
                    );
                }
                for (index, item) in items.iter().enumerate() {
                    let ty = tuple.items.get(index).or(tuple.rest.as_deref()).unwrap();
                    self.value(
                        item,
                        ty,
                        &child(path, PathSegment::Index(index)),
                        depth + 1,
                        skip_ref,
                    )?;
                }
            }
            TypeKind::Map(map) => {
                let Value::Map(entries) = value else {
                    return self.fail(path, "type", "map", describe_value(value));
                };
                for (index, (key, value)) in entries.iter().enumerate() {
                    let path = child(path, PathSegment::Index(index));
                    self.value(
                        key,
                        &map.keys,
                        &child(&path, PathSegment::Field("key".into())),
                        depth + 1,
                        skip_ref,
                    )?;
                    self.value(
                        value,
                        &map.values,
                        &child(&path, PathSegment::Field("value".into())),
                        depth + 1,
                        skip_ref,
                    )?;
                }
            }
            TypeKind::Record(record) => {
                let Value::Object(object) = value else {
                    return self.fail(path, "type", "record", describe_value(value));
                };
                for (name, field) in &record.fields {
                    let path = child(path, PathSegment::Field(name.clone()));
                    if let Some(value) = object.get(name) {
                        self.value(value, &field.ty, &path, depth + 1, skip_ref)?;
                    } else if field.required {
                        return self.fail(&path, "required", "stored value", "missing");
                    }
                }
                for (name, value) in object
                    .iter()
                    .filter(|(name, _)| !record.fields.contains_key(*name))
                {
                    let path = child(path, PathSegment::Field(name.clone()));
                    if let Some(ty) = &record.additional {
                        self.value(value, ty, &path, depth + 1, skip_ref)?;
                    } else if !record.open {
                        return self.fail(&path, "unknown_field", "declared field", name);
                    }
                }
            }
            TypeKind::Class(class) => {
                let Value::Object(object) = value else {
                    return self.fail(path, "type", "class object", describe_value(value));
                };
                let mut fields = std::collections::BTreeMap::new();
                definition_fields(self.catalog, class, &mut fields, &mut BTreeSet::new());
                if class.strict_schema {
                    for name in object.keys() {
                        if !fields.contains_key(name) && !is_special_builtin_field(name) {
                            return self.fail(
                                &child(path, PathSegment::Field(name.clone())),
                                "unknown_field",
                                "declared class attribute",
                                name,
                            );
                        }
                    }
                }
                self.fields(object, fields, path, depth + 1)?;
            }
            TypeKind::Enum(enumeration) => {
                let matches = match enumeration.repr {
                    EnumRepr::String => value.as_str().is_some_and(|value| {
                        enumeration.variants.iter().any(|variant| {
                            value == variant.symbol.as_deref().unwrap_or(&variant.name)
                        })
                    }),
                    EnumRepr::Int => value.as_i64().is_some_and(|value| {
                        enumeration
                            .variants
                            .iter()
                            .enumerate()
                            .any(|(index, variant)| value == variant.value.unwrap_or(index as i64))
                    }),
                };
                if !matches {
                    return self.fail(path, "enum", "declared enum value", describe_value(value));
                }
            }
            _ if !type_matches_value(ty, value) => {
                return self.fail(path, "type", describe_type(ty), describe_value(value));
            }
            _ => {}
        }
        self.constraints(value, &ty.constraints, path)
    }

    fn items(
        &mut self,
        value: &Value,
        ty: &Type,
        path: &FieldPath,
        depth: usize,
        skip_ref: bool,
    ) -> Result<(), DbError> {
        let Value::List(items) = value else {
            return self.fail(path, "type", "list", describe_value(value));
        };
        for (index, item) in items.iter().enumerate() {
            self.value(
                item,
                ty,
                &child(path, PathSegment::Index(index)),
                depth + 1,
                skip_ref,
            )?;
        }
        Ok(())
    }

    fn fields(
        &mut self,
        object: &Object,
        fields: std::collections::BTreeMap<String, StoredField>,
        path: &FieldPath,
        depth: usize,
    ) -> Result<(), DbError> {
        for (name, field) in fields {
            if field.computed {
                if object.contains_key(&name) {
                    return self.fail(
                        &child(path, PathSegment::Field(name)),
                        "computed",
                        "no stored value",
                        "stored value",
                    );
                }
                continue;
            }
            let path = child(path, PathSegment::Field(name.clone()));
            if let Some(value) = object.get(&name) {
                let endpoint = name == crate::catalog::ATTR_RELATION_FROM
                    || name == crate::catalog::ATTR_RELATION_TO;
                self.value(value, &field.ty, &path, depth, endpoint)?;
            } else if field.required {
                return self.fail(&path, "required", "stored value", "missing");
            }
        }
        Ok(())
    }

    fn constraints(
        &mut self,
        value: &Value,
        constraints: &[Constraint],
        path: &FieldPath,
    ) -> Result<(), DbError> {
        for constraint in constraints {
            let length = || match value {
                Value::String(s) => Some(s.chars().count() as u64),
                Value::Bytes(b) => Some(b.len() as u64),
                Value::List(v) => Some(v.len() as u64),
                Value::Map(v) => Some(v.len() as u64),
                Value::Object(v) => Some(v.len() as u64),
                _ => None,
            };
            let (rule, valid) = match constraint {
                Constraint::Min(bound) | Constraint::Max(bound) => {
                    let (NumberBound::Inclusive(text) | NumberBound::Exclusive(text)) = bound;
                    let valid = compare_number_bound(value, text).is_some_and(|order| {
                        match (constraint, bound) {
                            (Constraint::Min(_), NumberBound::Inclusive(_)) => !order.is_lt(),
                            (Constraint::Min(_), _) => order.is_gt(),
                            (_, NumberBound::Inclusive(_)) => !order.is_gt(),
                            _ => order.is_lt(),
                        }
                    });
                    (
                        if matches!(constraint, Constraint::Min(_)) {
                            "min"
                        } else {
                            "max"
                        },
                        valid,
                    )
                }
                Constraint::Length(spec) => (
                    "length",
                    length().is_some_and(|n| match spec {
                        LengthSpec::Exactly(want) => n == *want,
                        LengthSpec::Range { min, max } => {
                            min.is_none_or(|min| n >= min) && max.is_none_or(|max| n <= max)
                        }
                    }),
                ),
                Constraint::Pattern(pattern) => (
                    "pattern",
                    value
                        .as_str()
                        .is_some_and(|v| regex::Regex::new(pattern).is_ok_and(|r| r.is_match(v))),
                ),
                Constraint::Prefix(prefix) => (
                    "prefix",
                    value.as_str().is_some_and(|v| v.starts_with(prefix)),
                ),
                Constraint::Suffix(suffix) => (
                    "suffix",
                    value.as_str().is_some_and(|v| v.ends_with(suffix)),
                ),
                Constraint::MinItems(min) => ("min_items", length().is_some_and(|n| n >= *min)),
                Constraint::MaxItems(max) => ("max_items", length().is_some_and(|n| n <= *max)),
                Constraint::MinProperties(min) => {
                    ("min_properties", length().is_some_and(|n| n >= *min))
                }
                Constraint::MaxProperties(max) => {
                    ("max_properties", length().is_some_and(|n| n <= *max))
                }
                Constraint::RequiredFields(fields) => (
                    "required_fields",
                    matches!(value, Value::Object(object) if fields.iter().all(|name| object.contains_key(name))),
                ),
                Constraint::ForeignKey(fk) => {
                    self.reference(value, &Type::new(TypeKind::Ref(fk.to.clone())), path)?;
                    continue;
                }
                Constraint::DefaultValue { .. }
                | Constraint::DefaultExpr { .. }
                | Constraint::Transport { .. } => continue,
                other => {
                    return Err(DbError::UnsupportedConstraint {
                        kind: constraint_kind(other).into(),
                    });
                }
            };
            if !valid {
                return self.fail(path, rule, format!("{constraint:?}"), describe_value(value));
            }
        }
        Ok(())
    }
}

pub fn validate_stored_object<F: FnMut(&EntityKey) -> Result<Option<Object>, DbError>>(
    catalog: &Catalog,
    owner: &EntityKey,
    object: &Object,
    lookup: F,
) -> Result<Vec<ResolvedReference>, DbError> {
    let collection =
        catalog
            .collection_by_name(&owner.0)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: owner.0.clone(),
            })?;
    let mut validator = Validator {
        catalog,
        owner,
        class: object
            .get(OBJECT_TYPE_FIELD)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        lookup,
        references: Vec::new(),
    };
    validator.fields(
        object,
        object_fields(catalog, collection, object),
        &FieldPath::new(),
        0,
    )?;
    validator.references.sort();
    validator.references.dedup();
    Ok(validator.references)
}

fn constraint_kind(constraint: &Constraint) -> &'static str {
    match constraint {
        Constraint::MultipleOf(_) => "multiple_of",
        Constraint::Contains(_) => "contains",
        Constraint::Precision { .. } => "precision",
        Constraint::Charset(_) => "charset",
        Constraint::Collation(_) => "collation",
        Constraint::TimeZone(_) => "time_zone",
        Constraint::KeyPattern(_) => "key_pattern",
        Constraint::Unique => "unique",
        Constraint::Distinct => "distinct",
        Constraint::PrimaryKey => "primary_key",
        Constraint::Index { .. } => "index",
        _ => "unknown",
    }
}

// Compare decimal magnitudes instead of converting integer IDs/counts to f64,
// which loses distinctions above 2^53 (including i128/u128 boundaries).
fn compare_number_bound(value: &Value, bound: &str) -> Option<std::cmp::Ordering> {
    fn parts(text: &str) -> Option<(bool, String, i64)> {
        let text = text.trim();
        let negative = text.starts_with('-');
        let text = text.strip_prefix(['-', '+']).unwrap_or(text);
        let (mantissa, exponent) = match text.split_once(['e', 'E']) {
            Some((mantissa, exponent)) => (mantissa, exponent.parse::<i64>().ok()?),
            None => (text, 0),
        };
        let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        if whole.is_empty() && fraction.is_empty() {
            return None;
        }
        if !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let digits = format!("{whole}{fraction}");
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            return Some((false, "0".into(), 0));
        }
        let magnitude = exponent
            .checked_sub(fraction.len() as i64)?
            .checked_add(digits.len() as i64)?;
        Some((negative, digits.trim_end_matches('0').into(), magnitude))
    }
    let value = match value {
        Value::I8(v) => v.to_string(),
        Value::I16(v) => v.to_string(),
        Value::I32(v) => v.to_string(),
        Value::I64(v) => v.to_string(),
        Value::I128(v) => v.to_string(),
        Value::U8(v) => v.to_string(),
        Value::U16(v) => v.to_string(),
        Value::U32(v) => v.to_string(),
        Value::U64(v) => v.to_string(),
        Value::U128(v) => v.to_string(),
        Value::F32(v) => v.into_inner().to_string(),
        Value::F64(v) => v.into_inner().to_string(),
        _ => return None,
    };
    let (negative, digits, magnitude) = parts(&value)?;
    let (bound_negative, bound_digits, bound_magnitude) = parts(bound)?;
    if negative != bound_negative {
        return Some(if negative {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        });
    }
    let order = if digits == "0" || bound_digits == "0" {
        (digits != "0").cmp(&(bound_digits != "0"))
    } else {
        magnitude.cmp(&bound_magnitude).then_with(|| {
            let length = digits.len().max(bound_digits.len());
            digits
                .bytes()
                .chain(std::iter::repeat(b'0'))
                .take(length)
                .cmp(
                    bound_digits
                        .bytes()
                        .chain(std::iter::repeat(b'0'))
                        .take(length),
                )
        })
    };
    Some(if negative { order.reverse() } else { order })
}

pub(crate) fn validate_enforcement_support(catalog: &Catalog) -> Result<(), DbError> {
    fn constraints(values: &[Constraint]) -> Result<(), DbError> {
        for constraint in values {
            match constraint {
                Constraint::Min(_)
                | Constraint::Max(_)
                | Constraint::Length(_)
                | Constraint::Pattern(_)
                | Constraint::Prefix(_)
                | Constraint::Suffix(_)
                | Constraint::MinItems(_)
                | Constraint::MaxItems(_)
                | Constraint::MinProperties(_)
                | Constraint::MaxProperties(_)
                | Constraint::RequiredFields(_)
                | Constraint::ForeignKey(_)
                | Constraint::DefaultValue { .. }
                | Constraint::DefaultExpr { .. }
                | Constraint::Transport { .. } => {}
                other => {
                    return Err(DbError::UnsupportedConstraint {
                        kind: constraint_kind(other).into(),
                    });
                }
            }
        }
        Ok(())
    }
    fn visit(ty: &Type) -> Result<(), DbError> {
        constraints(&ty.constraints)?;
        let mut nested = Vec::new();
        match &ty.kind {
            TypeKind::Optional(t) => nested.push(t.inner.as_ref()),
            TypeKind::Array(t) => nested.push(t.items.as_ref()),
            TypeKind::List(t) => nested.push(t.items.as_ref()),
            TypeKind::Set(t) => nested.push(t.items.as_ref()),
            TypeKind::Tuple(t) => {
                nested.extend(t.items.iter());
                nested.extend(t.rest.as_deref());
            }
            TypeKind::Map(t) => {
                nested.push(t.keys.as_ref());
                nested.push(t.values.as_ref());
            }
            TypeKind::Record(t) => {
                nested.extend(t.fields.values().map(|f| &f.ty));
                nested.extend(t.additional.as_deref());
            }
            TypeKind::Attribute(t) => {
                constraints(&t.constraints)?;
                nested.push(&t.ty);
            }
            TypeKind::Class(t) => {
                for attr in t.attributes.values().filter(|attr| attr.computed.is_none()) {
                    constraints(&attr.constraints)?;
                }
                for constraint in &t.constraints {
                    match constraint {
                        ClassConstraint::Field { constraint, .. } => {
                            constraints(std::slice::from_ref(constraint))?
                        }
                        ClassConstraint::MultiFieldExpr { .. } => {
                            return Err(DbError::UnsupportedConstraint {
                                kind: "multi_field_expr".into(),
                            });
                        }
                    }
                }
            }
            TypeKind::Union(t) => nested.extend(t.variants.iter()),
            TypeKind::Intersection(t) => nested.extend(t.variants.iter()),
            _ => {}
        }
        for ty in nested {
            visit(ty)?;
        }
        Ok(())
    }
    for (_, def) in catalog.type_defs() {
        visit(&def.type_def.ty)?;
    }
    Ok(())
}

pub(super) fn normalize_nested_values(
    catalog: &Catalog,
    collection: &CollectionSchema,
    object: &mut Object,
    defaults: bool,
) {
    fn shape_matches(catalog: &Catalog, ty: &Type, value: &Value, depth: usize) -> bool {
        if depth > 128 {
            return false;
        }
        match &ty.kind {
            TypeKind::Ref(r) if catalog.class_id(&r.name).is_none() => catalog
                .type_def_by_name(&r.name)
                .map(|def| shape_matches(catalog, &def.type_def.ty, value, depth + 1))
                .unwrap_or(matches!(value, Value::String(_))),
            TypeKind::Ref(_) => matches!(value, Value::String(_)),
            TypeKind::Union(u) => u
                .variants
                .iter()
                .any(|ty| shape_matches(catalog, ty, value, depth + 1)),
            TypeKind::Optional(o) => {
                value.is_nullish() || shape_matches(catalog, &o.inner, value, depth + 1)
            }
            TypeKind::Enum(e) => match e.repr {
                EnumRepr::String => value.as_str().is_some(),
                EnumRepr::Int => value.as_i64().is_some(),
            },
            TypeKind::Record(r) => {
                matches!(value, Value::Object(object) if r.fields.iter().all(|(name, field)| match object.get(name) { Some(value) => shape_matches(catalog, &field.ty, value, depth + 1), None => !field.required || field.default.is_some() || field.ty.constraints.iter().any(|c| matches!(c, Constraint::DefaultValue { .. })) }))
            }
            _ => type_matches_value(ty, value),
        }
    }
    fn normalize(catalog: &Catalog, value: &mut Value, ty: &Type, defaults: bool, depth: usize) {
        if depth > 128 {
            return;
        }
        match &ty.kind {
            TypeKind::Ref(r) if catalog.class_id(&r.name).is_none() => {
                if let Some(def) = catalog.type_def_by_name(&r.name) {
                    normalize(catalog, value, &def.type_def.ty, defaults, depth + 1);
                }
            }
            TypeKind::Optional(o) if !value.is_nullish() => {
                normalize(catalog, value, &o.inner, defaults, depth + 1)
            }
            TypeKind::Union(u) => {
                if let Some(ty) = u
                    .variants
                    .iter()
                    .find(|ty| shape_matches(catalog, ty, value, depth + 1))
                {
                    normalize(catalog, value, ty, defaults, depth + 1);
                }
            }
            TypeKind::Attribute(a) => normalize(catalog, value, &a.ty, defaults, depth + 1),
            TypeKind::Array(_) | TypeKind::List(_) | TypeKind::Tuple(_) => {
                if let Value::List(items) = value {
                    for (index, item) in items.iter_mut().enumerate() {
                        let ty = match &ty.kind {
                            TypeKind::Array(a) => Some(a.items.as_ref()),
                            TypeKind::List(l) => Some(l.items.as_ref()),
                            TypeKind::Tuple(t) => t.items.get(index).or(t.rest.as_deref()),
                            _ => None,
                        };
                        if let Some(ty) = ty {
                            normalize(catalog, item, ty, defaults, depth + 1);
                        }
                    }
                }
            }
            TypeKind::Record(record) => {
                if let Value::Object(object) = value {
                    for (name, field) in &record.fields {
                        if defaults && !object.contains_key(name) {
                            if let Some(default) = field.default.as_ref().or_else(|| {
                                field.ty.constraints.iter().find_map(|c| match c {
                                    Constraint::DefaultValue { value } => Some(value),
                                    _ => None,
                                })
                            }) {
                                object.insert(name.clone(), default.clone());
                            }
                        }
                        if !field.required
                            && !matches!(field.ty.kind, TypeKind::Optional(_))
                            && object.get(name).is_some_and(Value::is_nullish)
                        {
                            object.remove(name);
                        }
                        if let Some(value) = object.get_mut(name) {
                            normalize(catalog, value, &field.ty, defaults, depth + 1);
                        }
                    }
                    if let Some(additional) = &record.additional {
                        for (name, value) in object.iter_mut() {
                            if !record.fields.contains_key(name) {
                                normalize(catalog, value, additional, defaults, depth + 1);
                            }
                        }
                    }
                }
            }
            TypeKind::Class(class) => {
                if let Value::Object(object) = value {
                    {
                        let mut fields = std::collections::BTreeMap::new();
                        definition_fields(catalog, class, &mut fields, &mut BTreeSet::new());
                        let aliases = object
                            .keys()
                            .filter_map(|name| {
                                class
                                    .attributes
                                    .get(name)
                                    .map(|attr| attr.attribute.id.clone())
                                    .or_else(|| {
                                        catalog.class_id(&class.id).and_then(|lid| {
                                            catalog.class_field_for_alias(lid, name)
                                        })
                                    })
                                    .filter(|canonical| canonical != name)
                                    .map(|canonical| (name.clone(), canonical))
                            })
                            .collect::<Vec<_>>();
                        for (alias, canonical) in aliases {
                            if !object.contains_key(&canonical) {
                                if let Some(value) = object.remove(&alias) {
                                    object.insert(canonical, value);
                                }
                            }
                        }
                        normalize_fields(catalog, object, fields, defaults, depth + 1);
                    }
                }
            }
            TypeKind::Map(map) => {
                if let Value::Map(entries) = value {
                    for value in entries.values_mut() {
                        normalize(catalog, value, &map.values, defaults, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }
    fn normalize_fields(
        catalog: &Catalog,
        object: &mut Object,
        fields: std::collections::BTreeMap<String, StoredField>,
        defaults: bool,
        depth: usize,
    ) {
        for (name, field) in fields {
            if !field.computed {
                if defaults && !object.contains_key(&name) {
                    if let Some(value) = field.ty.constraints.iter().find_map(|c| match c {
                        Constraint::DefaultValue { value } => Some(value),
                        _ => None,
                    }) {
                        object.insert(name.clone(), value.clone());
                    }
                }
                if !field.required
                    && !matches!(field.ty.kind, TypeKind::Optional(_))
                    && object.get(&name).is_some_and(Value::is_nullish)
                {
                    object.remove(&name);
                }
                if let Some(value) = object.get_mut(&name) {
                    normalize(catalog, value, &field.ty, defaults, depth + 1);
                }
            }
        }
    }
    normalize_fields(
        catalog,
        object,
        object_fields(catalog, collection, object),
        defaults,
        0,
    );
}

/// Conservative candidate references: unions can contribute more than one
/// candidate, and validation decides which alternative actually accepts a row.
pub(crate) fn stored_references(
    catalog: &Catalog,
    owner: &EntityKey,
    object: &Object,
) -> Vec<ResolvedReference> {
    fn visit(
        catalog: &Catalog,
        owner: &EntityKey,
        value: &Value,
        ty: &Type,
        path: FieldPath,
        out: &mut Vec<ResolvedReference>,
        depth: usize,
        skip_ref: bool,
    ) {
        if depth > 128 {
            return;
        }
        let mut reference = || {
            if let Some(id) = value.as_str().filter(|id| !id.is_empty()) {
                out.push(ResolvedReference {
                    owner: owner.clone(),
                    path: path.clone(),
                    target: (owner.0.clone(), id.into()),
                });
            }
        };
        if ty
            .constraints
            .iter()
            .any(|c| matches!(c, Constraint::ForeignKey(_)))
        {
            reference();
        }
        match &ty.kind {
            TypeKind::Ref(r) => {
                if catalog.class_id(&r.name).is_some() {
                    if !skip_ref {
                        reference();
                    }
                } else if let Some(def) = catalog.type_def_by_name(&r.name) {
                    visit(
                        catalog,
                        owner,
                        value,
                        &def.type_def.ty,
                        path,
                        out,
                        depth + 1,
                        skip_ref,
                    );
                } else if !skip_ref {
                    reference();
                }
            }
            TypeKind::Optional(o) if !value.is_nullish() => visit(
                catalog,
                owner,
                value,
                &o.inner,
                path,
                out,
                depth + 1,
                skip_ref,
            ),
            TypeKind::Union(u) => {
                for ty in &u.variants {
                    visit(
                        catalog,
                        owner,
                        value,
                        ty,
                        path.clone(),
                        out,
                        depth + 1,
                        skip_ref,
                    );
                }
            }
            TypeKind::Array(_) | TypeKind::List(_) | TypeKind::Tuple(_) => {
                if let Value::List(items) = value {
                    for (index, value) in items.iter().enumerate() {
                        let ty = match &ty.kind {
                            TypeKind::Array(a) => Some(a.items.as_ref()),
                            TypeKind::List(l) => Some(l.items.as_ref()),
                            TypeKind::Tuple(t) => t.items.get(index).or(t.rest.as_deref()),
                            _ => None,
                        };
                        if let Some(ty) = ty {
                            visit(
                                catalog,
                                owner,
                                value,
                                ty,
                                child(&path, PathSegment::Index(index)),
                                out,
                                depth + 1,
                                skip_ref,
                            );
                        }
                    }
                }
            }
            TypeKind::Record(record) => {
                if let Value::Object(object) = value {
                    for (name, value) in object {
                        if let Some(ty) = record
                            .fields
                            .get(name)
                            .map(|f| &f.ty)
                            .or(record.additional.as_deref())
                        {
                            visit(
                                catalog,
                                owner,
                                value,
                                ty,
                                child(&path, PathSegment::Field(name.clone())),
                                out,
                                depth + 1,
                                skip_ref,
                            );
                        }
                    }
                }
            }
            TypeKind::Class(class) => {
                if let Value::Object(object) = value {
                    {
                        let mut fields = std::collections::BTreeMap::new();
                        definition_fields(catalog, class, &mut fields, &mut BTreeSet::new());
                        for (name, field) in fields {
                            if let Some(value) = object.get(&name) {
                                visit(
                                    catalog,
                                    owner,
                                    value,
                                    &field.ty,
                                    child(&path, PathSegment::Field(name)),
                                    out,
                                    depth + 1,
                                    skip_ref,
                                );
                            }
                        }
                    }
                }
            }
            TypeKind::Map(map) => {
                if let Value::Map(entries) = value {
                    for (index, (key, value)) in entries.iter().enumerate() {
                        let path = child(&path, PathSegment::Index(index));
                        visit(
                            catalog,
                            owner,
                            key,
                            &map.keys,
                            child(&path, PathSegment::Field("key".into())),
                            out,
                            depth + 1,
                            skip_ref,
                        );
                        visit(
                            catalog,
                            owner,
                            value,
                            &map.values,
                            child(&path, PathSegment::Field("value".into())),
                            out,
                            depth + 1,
                            skip_ref,
                        );
                    }
                }
            }
            TypeKind::Attribute(attr) => {
                let mut ty = attr.ty.clone();
                ty.constraints.extend(attr.constraints.clone());
                visit(catalog, owner, value, &ty, path, out, depth + 1, skip_ref);
            }
            _ => {}
        }
    }
    let Some(collection) = catalog.collection_by_name(&owner.0) else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    for (name, field) in object_fields(catalog, collection, object) {
        if !field.computed {
            if let Some(value) = object.get(&name) {
                let skip_ref = name == crate::catalog::ATTR_RELATION_FROM
                    || name == crate::catalog::ATTR_RELATION_TO;
                visit(
                    catalog,
                    owner,
                    value,
                    &field.ty,
                    FieldPath::from_fields([name]),
                    &mut refs,
                    0,
                    skip_ref,
                );
            }
        }
    }
    refs.sort();
    refs.dedup();
    refs
}
