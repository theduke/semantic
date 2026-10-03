//! Schema conformance shared by native implementations and remote proxies.
use super::*;
use futures::StreamExt;
use semantic_data::schema::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Validate configuration against the authoritative, non-stream schema.
pub fn validate_configuration(
    value: &Value,
    ty: &Type,
    definitions: &BTreeMap<String, TypeDef>,
) -> Result<(), InvocationError> {
    profile(ty, false, definitions, &mut BTreeSet::new())?;
    validate(value, ty, definitions, "invalid_configuration")
}

pub struct ConformingImplementation {
    inner: Arc<dyn InterfaceImplementation>,
    declarations: BTreeMap<String, InterfaceType>,
    definitions: Arc<BTreeMap<String, TypeDef>>,
}

impl ConformingImplementation {
    pub fn new(
        inner: Arc<dyn InterfaceImplementation>,
        declarations: BTreeMap<String, InterfaceType>,
        definitions: BTreeMap<String, TypeDef>,
    ) -> Result<Self, InvocationError> {
        let mut exports = BTreeSet::new();
        for descriptor in inner.descriptors() {
            if !exports.insert(&descriptor.export) {
                return Err(incompatible("duplicate implementation export"));
            }
            let declaration = declarations
                .get(&descriptor.export)
                .ok_or_else(|| incompatible("missing authoritative interface declaration"))?;
            let mut methods = BTreeSet::new();
            for method in &declaration.methods {
                if !methods.insert(&method.name) {
                    return Err(incompatible("duplicate interface method"));
                }
                let signature = &method.signature;
                let mut input_streams = 0;
                for param in &signature.params {
                    input_streams +=
                        profile(&param.ty, true, &definitions, &mut BTreeSet::new())? as usize;
                }
                let mut output_streams = 0;
                for result in &signature.results {
                    output_streams +=
                        profile(result, true, &definitions, &mut BTreeSet::new())? as usize;
                }
                if input_streams > 1
                    || output_streams > 1
                    || (output_streams == 1 && signature.results.len() != 1)
                {
                    return Err(incompatible(
                        "profile permits one input stream and either values or one output stream",
                    ));
                }
                if let Some(error) = &signature.throws {
                    profile(error, false, &definitions, &mut BTreeSet::new())?;
                }
            }
        }
        if exports.len() != declarations.len() {
            return Err(incompatible(
                "implementation export set does not match declarations",
            ));
        }
        Ok(Self {
            inner,
            declarations,
            definitions: Arc::new(definitions),
        })
    }
}

impl InterfaceImplementation for ConformingImplementation {
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        self.inner.descriptors()
    }
    fn invoke<'a>(
        &'a self,
        mut call: ValidatedInvocation,
        context: InvocationContext,
    ) -> InvocationFuture<'a> {
        Box::pin(async move {
            let method = self
                .declarations
                .get(&call.export)
                .and_then(|interface| {
                    interface
                        .methods
                        .iter()
                        .find(|method| method.name == call.method)
                })
                .ok_or_else(|| invalid("invalid_argument", "unknown export or method"))?;
            let signature = &method.signature;
            if call.arguments.len() != signature.params.len() {
                return Err(invalid("invalid_argument", "incorrect argument count"));
            }
            let mut arguments = Vec::new();
            for (argument, parameter) in call.arguments.into_iter().zip(&signature.params) {
                let ty = resolve(&parameter.ty, &self.definitions)?;
                arguments.push(match (argument, &ty.kind) {
                    (InvocationArgument::Stream(stream), TypeKind::Stream(schema)) => {
                        InvocationArgument::Stream(validated_stream(
                            stream,
                            schema.clone(),
                            signature.throws.clone(),
                            self.definitions.clone(),
                            "invalid_argument",
                        ))
                    }
                    (InvocationArgument::Value(value), kind)
                        if !matches!(kind, TypeKind::Stream(_)) =>
                    {
                        validate(&value, &parameter.ty, &self.definitions, "invalid_argument")?;
                        InvocationArgument::Value(value)
                    }
                    _ => {
                        return Err(invalid(
                            "invalid_argument",
                            "argument value/stream kind mismatch",
                        ));
                    }
                });
            }
            call.arguments = arguments;
            let output = match self.inner.invoke(call, context).await {
                Ok(output) => output,
                Err(error) => {
                    return Err(validate_error(
                        error,
                        signature.throws.as_deref(),
                        &self.definitions,
                    ));
                }
            };
            match output {
                InvocationOutput::Values(values) => {
                    if values.len() != signature.results.len() {
                        return Err(invalid("invalid_output", "incorrect result count"));
                    }
                    for (value, ty) in values.iter().zip(&signature.results) {
                        validate(value, ty, &self.definitions, "invalid_output")?;
                    }
                    Ok(InvocationOutput::Values(values))
                }
                InvocationOutput::Stream(stream) => {
                    let Some(ty) = signature
                        .results
                        .first()
                        .filter(|_| signature.results.len() == 1)
                    else {
                        return Err(invalid("invalid_output", "unexpected output stream"));
                    };
                    let TypeKind::Stream(schema) = &resolve(ty, &self.definitions)?.kind else {
                        return Err(invalid("invalid_output", "unexpected output stream"));
                    };
                    Ok(InvocationOutput::Stream(validated_stream(
                        stream,
                        schema.clone(),
                        signature.throws.clone(),
                        self.definitions.clone(),
                        "invalid_output",
                    )))
                }
            }
        })
    }
}

fn validated_stream(
    stream: OwnedValueStream,
    schema: StreamType,
    throws: Option<Box<Type>>,
    definitions: Arc<BTreeMap<String, TypeDef>>,
    code: &'static str,
) -> OwnedValueStream {
    let metadata = stream.metadata.clone();
    let origin = stream.origin.clone();
    let mut stream = OwnedValueStream::new(stream.map(move |event| match event {
        Ok(StreamEvent::Item(value)) => {
            validate(&value, &schema.element, &definitions, code)?;
            Ok(StreamEvent::Item(value))
        }
        Ok(StreamEvent::End(value)) => {
            match (&value, &schema.end) {
                (Some(value), Some(ty)) => validate(value, ty, &definitions, code)?,
                (None, None) => {}
                _ => {
                    return Err(invalid(
                        code,
                        "stream terminal value does not match declaration",
                    ));
                }
            }
            Ok(StreamEvent::End(value))
        }
        Err(error) => Err(validate_error(error, throws.as_deref(), &definitions)),
    }));
    stream.metadata = metadata;
    stream.origin = origin;
    stream
}

fn validate_error(
    error: InvocationError,
    throws: Option<&Type>,
    definitions: &BTreeMap<String, TypeDef>,
) -> InvocationError {
    if let Some(value) = &error.data {
        let Some(ty) = throws else {
            return invalid("invalid_output", "undeclared application error");
        };
        if let Err(error) = validate(value, ty, definitions, "invalid_output") {
            return error;
        }
    }
    error
}

fn invalid(code: &str, message: &str) -> InvocationError {
    InvocationError::new(code, message)
}
fn incompatible(message: &str) -> InvocationError {
    invalid("interface_incompatible", message)
}

fn resolve<'a>(
    mut ty: &'a Type,
    definitions: &'a BTreeMap<String, TypeDef>,
) -> Result<&'a Type, InvocationError> {
    let mut visited = BTreeSet::new();
    while let TypeKind::Named(reference) = &ty.kind {
        if !reference.args.is_empty() {
            return Err(incompatible("generic interface references are unsupported"));
        }
        if !visited.insert(&reference.name) {
            return Err(incompatible("unproductive recursive type alias"));
        }
        ty = &definitions
            .get(&reference.name)
            .ok_or_else(|| incompatible("unresolved interface type"))?
            .ty;
    }
    Ok(ty)
}

fn profile(
    ty: &Type,
    top: bool,
    definitions: &BTreeMap<String, TypeDef>,
    visited: &mut BTreeSet<String>,
) -> Result<bool, InvocationError> {
    for constraint in &ty.constraints {
        match constraint {
            Constraint::Length(_)
            | Constraint::Prefix(_)
            | Constraint::Suffix(_)
            | Constraint::MinItems(_)
            | Constraint::MaxItems(_)
            | Constraint::MinProperties(_)
            | Constraint::MaxProperties(_)
            | Constraint::RequiredFields(_)
            | Constraint::Contains(_)
            | Constraint::Distinct
            | Constraint::Unique => {}
            Constraint::Pattern(pattern) | Constraint::KeyPattern(pattern) => {
                regex::Regex::new(pattern)
                    .map_err(|_| incompatible("invalid interface regex constraint"))?;
            }
            Constraint::DefaultValue { .. }
            | Constraint::DefaultExpr { .. }
            | Constraint::Transport { .. } => {}
            _ => {
                return Err(incompatible(
                    "interface constraint is not supported by this runtime profile",
                ));
            }
        }
    }
    let mut child = |ty: &Type| profile(ty, false, definitions, visited).map(|_| ());
    match &ty.kind {
        TypeKind::Named(reference) => {
            if !reference.args.is_empty() {
                return Err(incompatible("generic interface references are unsupported"));
            }
            if !top && matches!(resolve(ty, definitions)?.kind, TypeKind::Stream(_)) {
                return Err(incompatible("nested stream reference is unsupported"));
            }
            if !visited.insert(reference.name.clone()) {
                return Ok(false);
            }
            let definition = definitions
                .get(&reference.name)
                .ok_or_else(|| incompatible("unresolved interface type"))?;
            let result = profile(&definition.ty, top, definitions, visited);
            visited.remove(&reference.name);
            result
        }
        TypeKind::Stream(stream) if top => {
            child(&stream.element)?;
            if let Some(end) = &stream.end {
                child(end)?;
            }
            Ok(true)
        }
        TypeKind::Optional(optional) => {
            child(&optional.inner)?;
            Ok(false)
        }
        TypeKind::Array(array) => {
            child(&array.items)?;
            Ok(false)
        }
        TypeKind::List(list) => {
            child(&list.items)?;
            Ok(false)
        }
        TypeKind::Set(set) => {
            child(&set.items)?;
            Ok(false)
        }
        TypeKind::Tuple(tuple) => {
            for ty in &tuple.items {
                child(ty)?;
            }
            if let Some(rest) = &tuple.rest {
                child(rest)?;
            }
            Ok(false)
        }
        TypeKind::Map(map) => {
            child(&map.keys)?;
            child(&map.values)?;
            Ok(false)
        }
        TypeKind::Record(record) => {
            for field in record.fields.values() {
                child(&field.ty)?;
            }
            if let Some(additional) = &record.additional {
                child(additional)?;
            }
            Ok(false)
        }
        TypeKind::Variant(variant) => {
            let internal = match &variant.tag {
                VariantTag::ExternallyTagged => None,
                VariantTag::InternallyTagged { field } => Some(field),
                _ => return Err(incompatible("unsupported interface variant tagging")),
            };
            let mut names = BTreeSet::new();
            for case in &variant.variants {
                if !names.insert(&case.name) || case.discriminant.is_some() {
                    return Err(incompatible("unsupported interface variant discriminant"));
                }
                if let Some(field) = internal {
                    match &case.payload {
                        VariantPayload::Unit => {}
                        VariantPayload::Record(record) if !record.fields.contains_key(field) => {}
                        _ => {
                            return Err(incompatible(
                                "internal variant requires a record or unit payload without the tag field",
                            ));
                        }
                    }
                }
                child(&variant_payload_type(&case.payload))?;
            }
            Ok(false)
        }
        TypeKind::Union(union) => {
            for ty in &union.variants {
                child(ty)?;
            }
            Ok(false)
        }
        TypeKind::Intersection(intersection) => {
            for ty in &intersection.variants {
                child(ty)?;
            }
            Ok(false)
        }
        TypeKind::String(string) if string.format.is_some() || string.normalization.is_some() => {
            Err(incompatible(
                "formatted/normalized strings are not yet supported by the interface validator",
            ))
        }
        TypeKind::Number(number) => {
            let supported = match number {
                NumberType::Int(width) => matches!(
                    width,
                    IntWidth::I8 | IntWidth::I16 | IntWidth::I32 | IntWidth::I64 | IntWidth::I128
                ),
                NumberType::UInt(width) => matches!(
                    width,
                    UIntWidth::U8
                        | UIntWidth::U16
                        | UIntWidth::U32
                        | UIntWidth::U64
                        | UIntWidth::U128
                ),
                NumberType::Float(width) => matches!(width, FloatWidth::F32 | FloatWidth::F64),
                NumberType::Unspecified => true,
                _ => false,
            };
            if supported {
                Ok(false)
            } else {
                Err(incompatible(
                    "number type has no supported Value representation",
                ))
            }
        }
        TypeKind::Temporal(temporal)
            if !matches!(
                temporal,
                TemporalType::Date
                    | TemporalType::DateTime
                    | TemporalType::Time
                    | TemporalType::Duration
            ) =>
        {
            Err(incompatible(
                "temporal type has no supported interface Value representation",
            ))
        }
        TypeKind::Any(_)
        | TypeKind::Unknown(_)
        | TypeKind::Never(_)
        | TypeKind::Null(_)
        | TypeKind::Bool(_)
        | TypeKind::Char(_)
        | TypeKind::String(_)
        | TypeKind::Bytes(_)
        | TypeKind::Uuid
        | TypeKind::IpAddr(_)
        | TypeKind::Temporal(_)
        | TypeKind::Json
        | TypeKind::Ref(_)
        | TypeKind::Enum(_) => Ok(false),
        _ => Err(incompatible(
            "unsupported nested stream, handle, callable, or non-value interface type",
        )),
    }
}

fn variant_payload_type(payload: &VariantPayload) -> Type {
    Type::new(match payload {
        VariantPayload::Unit => TypeKind::Null(NullType {}),
        VariantPayload::Newtype(ty) => return (**ty).clone(),
        VariantPayload::Record(record) => TypeKind::Record(record.clone()),
        VariantPayload::Tuple(items) => TypeKind::Tuple(TupleType {
            items: items.clone(),
            rest: None,
        }),
    })
}

fn variant_matches(
    value: &Value,
    variant: &VariantType,
    definitions: &BTreeMap<String, TypeDef>,
    code: &str,
) -> bool {
    match (&variant.tag, value) {
        (VariantTag::ExternallyTagged, Value::String(name)) => variant
            .variants
            .iter()
            .any(|case| case.name == *name && matches!(case.payload, VariantPayload::Unit)),
        (VariantTag::ExternallyTagged, Value::Object(object)) if object.len() == 1 => {
            let (name, payload) = object.iter().next().expect("single variant field");
            variant.variants.iter().any(|case| {
                case.name == *name
                    && !matches!(case.payload, VariantPayload::Unit)
                    && validate(
                        payload,
                        &variant_payload_type(&case.payload),
                        definitions,
                        code,
                    )
                    .is_ok()
            })
        }
        (VariantTag::InternallyTagged { field }, Value::Object(object)) => {
            let Some(Value::String(name)) = object.get(field) else {
                return false;
            };
            let Some(case) = variant.variants.iter().find(|case| case.name == *name) else {
                return false;
            };
            let mut payload = object.clone();
            payload.remove(field);
            match &case.payload {
                VariantPayload::Unit => payload.is_empty(),
                VariantPayload::Record(_) => validate(
                    &Value::Object(payload),
                    &variant_payload_type(&case.payload),
                    definitions,
                    code,
                )
                .is_ok(),
                _ => false,
            }
        }
        _ => false,
    }
}

fn validate(
    value: &Value,
    ty: &Type,
    definitions: &BTreeMap<String, TypeDef>,
    code: &str,
) -> Result<(), InvocationError> {
    // Validate every alias layer; resolving directly would discard constraints
    // attached to intermediate references.
    let mut ty = ty;
    let mut visited = BTreeSet::new();
    loop {
        for constraint in &ty.constraints {
            if !constraint_matches(value, constraint) {
                return Err(invalid(code, "value violates interface constraint"));
            }
        }
        let TypeKind::Named(reference) = &ty.kind else {
            break;
        };
        if !reference.args.is_empty() || !visited.insert(&reference.name) {
            return Err(incompatible("unsupported or recursive interface reference"));
        }
        ty = &definitions
            .get(&reference.name)
            .ok_or_else(|| incompatible("unresolved interface type"))?
            .ty;
    }
    let matches = match (&ty.kind, value) {
        (TypeKind::Any(_) | TypeKind::Unknown(_), _) => true,
        (TypeKind::Null(_), Value::Null)
        | (TypeKind::Bool(_), Value::Bool(_))
        | (TypeKind::Bytes(_), Value::Bytes(_))
        | (TypeKind::String(_), Value::String(_))
        | (TypeKind::Ref(_), Value::String(_))
        | (TypeKind::Uuid, Value::Uuid(_))
        | (TypeKind::IpAddr(_), Value::IpAddr(_)) => true,
        (TypeKind::Char(_), Value::String(text)) => text.chars().count() == 1,
        (TypeKind::Number(number), value) => number_matches(number, value),
        (TypeKind::Optional(_), Value::Null) => true,
        (TypeKind::Optional(optional), value) => {
            validate(value, &optional.inner, definitions, code).is_ok()
        }
        (TypeKind::List(list), Value::List(values)) => values
            .iter()
            .all(|value| validate(value, &list.items, definitions, code).is_ok()),
        (TypeKind::Array(array), Value::List(values)) => {
            array
                .length
                .as_ref()
                .is_none_or(|length| length_matches(length, values.len() as u64))
                && values
                    .iter()
                    .all(|value| validate(value, &array.items, definitions, code).is_ok())
        }
        (TypeKind::Set(set), Value::List(values)) => {
            values.iter().collect::<BTreeSet<_>>().len() == values.len()
                && values
                    .iter()
                    .all(|value| validate(value, &set.items, definitions, code).is_ok())
        }
        (TypeKind::Tuple(tuple), Value::List(values)) => {
            values.len() >= tuple.items.len()
                && (tuple.rest.is_some() || values.len() == tuple.items.len())
                && values.iter().enumerate().all(|(i, value)| {
                    tuple
                        .items
                        .get(i)
                        .or(tuple.rest.as_deref())
                        .is_some_and(|ty| validate(value, ty, definitions, code).is_ok())
                })
        }
        (TypeKind::Record(record), Value::Object(values)) => {
            record
                .fields
                .iter()
                .all(|(name, field)| match values.get(name) {
                    Some(value) => validate(value, &field.ty, definitions, code).is_ok(),
                    None => !field.required,
                })
                && values.iter().all(|(name, value)| {
                    record.fields.contains_key(name)
                        || record.additional.as_deref().map_or(record.open, |ty| {
                            validate(value, ty, definitions, code).is_ok()
                        })
                })
        }
        (TypeKind::Map(map), Value::Map(values)) => values.iter().all(|(key, value)| {
            validate(key, &map.keys, definitions, code).is_ok()
                && validate(value, &map.values, definitions, code).is_ok()
        }),
        (TypeKind::Union(union), value) => union
            .variants
            .iter()
            .any(|ty| validate(value, ty, definitions, code).is_ok()),
        (TypeKind::Intersection(intersection), value) => intersection
            .variants
            .iter()
            .all(|ty| validate(value, ty, definitions, code).is_ok()),
        (TypeKind::Variant(variant), value) => variant_matches(value, variant, definitions, code),
        (TypeKind::Enum(enumeration), Value::String(name))
            if enumeration.repr == EnumRepr::String =>
        {
            enumeration
                .variants
                .iter()
                .any(|variant| variant.symbol.as_ref().unwrap_or(&variant.name) == name)
        }
        (TypeKind::Enum(enumeration), Value::I64(number)) if enumeration.repr == EnumRepr::Int => {
            enumeration
                .variants
                .iter()
                .any(|variant| variant.value == Some(*number))
        }
        (TypeKind::Json, _) => json_value(value),
        (TypeKind::Temporal(temporal), value) => matches!(
            (temporal, value),
            (TemporalType::Date, Value::Date(_))
                | (TemporalType::DateTime, Value::DateTime(_))
                | (TemporalType::Time, Value::Time(_))
                | (TemporalType::Duration, Value::Duration(_))
        ),
        _ => false,
    };
    if !matches {
        return Err(invalid(
            code,
            "value does not match declared interface type",
        ));
    }
    Ok(())
}

fn number_matches(number: &NumberType, value: &Value) -> bool {
    match number {
        NumberType::Int(width) => matches!(
            (width, value),
            (IntWidth::I8, Value::I8(_))
                | (IntWidth::I16, Value::I16(_))
                | (IntWidth::I32, Value::I32(_))
                | (IntWidth::I64, Value::I64(_))
                | (IntWidth::I128, Value::I128(_))
        ),
        NumberType::UInt(width) => matches!(
            (width, value),
            (UIntWidth::U8, Value::U8(_))
                | (UIntWidth::U16, Value::U16(_))
                | (UIntWidth::U32, Value::U32(_))
                | (UIntWidth::U64, Value::U64(_))
                | (UIntWidth::U128, Value::U128(_))
        ),
        NumberType::Float(width) => matches!(
            (width, value),
            (FloatWidth::F32, Value::F32(_)) | (FloatWidth::F64, Value::F64(_))
        ),
        NumberType::Unspecified => matches!(
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
        _ => false,
    }
}

fn json_value(value: &Value) -> bool {
    match value {
        Value::Null
        | Value::Bool(_)
        | Value::String(_)
        | Value::I8(_)
        | Value::I16(_)
        | Value::I32(_)
        | Value::I64(_)
        | Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::U64(_) => true,
        Value::F32(value) => value.0.is_finite(),
        Value::F64(value) => value.0.is_finite(),
        Value::List(values) => values.iter().all(json_value),
        Value::Object(values) => values.values().all(json_value),
        _ => false,
    }
}
fn length_matches(length: &LengthSpec, actual: u64) -> bool {
    match length {
        LengthSpec::Exactly(expected) => actual == *expected,
        LengthSpec::Range { min, max } => {
            min.is_none_or(|min| actual >= min) && max.is_none_or(|max| actual <= max)
        }
    }
}
fn constraint_matches(value: &Value, constraint: &Constraint) -> bool {
    let length = match value {
        Value::String(value) => Some(value.chars().count()),
        Value::Bytes(value) => Some(value.len()),
        Value::List(value) => Some(value.len()),
        Value::Object(value) => Some(value.len()),
        Value::Map(value) => Some(value.len()),
        _ => None,
    }
    .map(|value| value as u64);
    match constraint {
        Constraint::Length(spec) => length.is_some_and(|length| length_matches(spec, length)),
        Constraint::MinItems(min) | Constraint::MinProperties(min) => {
            length.is_some_and(|length| length >= *min)
        }
        Constraint::MaxItems(max) | Constraint::MaxProperties(max) => {
            length.is_some_and(|length| length <= *max)
        }
        Constraint::Prefix(prefix) => {
            matches!(value, Value::String(value) if value.starts_with(prefix))
        }
        Constraint::Suffix(suffix) => {
            matches!(value, Value::String(value) if value.ends_with(suffix))
        }
        Constraint::Pattern(pattern) => {
            matches!(value, Value::String(value) if regex::Regex::new(pattern).is_ok_and(|pattern| pattern.is_match(value)))
        }
        Constraint::KeyPattern(pattern) => {
            matches!(value, Value::Object(value) if regex::Regex::new(pattern).is_ok_and(|pattern| value.keys().all(|key| pattern.is_match(key))))
        }
        Constraint::RequiredFields(fields) => {
            matches!(value, Value::Object(value) if fields.iter().all(|field| value.contains_key(field)))
        }
        Constraint::Contains(expected) => {
            matches!(value, Value::List(value) if value.contains(expected))
        }
        Constraint::Distinct | Constraint::Unique => {
            matches!(value, Value::List(value) if value.iter().collect::<BTreeSet<_>>().len() == value.len())
        }
        Constraint::DefaultValue { .. }
        | Constraint::DefaultExpr { .. }
        | Constraint::Transport { .. } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{executor::block_on, stream};
    use semantic_data::value::{FromValue, IntoValue, SemanticType};

    fn variant_type(tag: VariantTag, cases: Vec<(&str, VariantPayload)>) -> Type {
        Type::new(TypeKind::Variant(VariantType {
            tag,
            variants: cases
                .into_iter()
                .map(|(name, payload)| VariantCase {
                    name: name.into(),
                    payload,
                    discriminant: None,
                    meta: Meta::default(),
                })
                .collect(),
        }))
    }

    fn object(fields: Vec<(&str, Value)>) -> Value {
        Value::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    }

    #[test]
    fn tagged_variants_validate_tags_and_payloads_strictly() {
        let summary = semantic_data::vdb::ScanSummary::semantic_type();
        let TypeKind::Record(record) = summary.kind else {
            panic!("record")
        };
        let external = variant_type(
            VariantTag::ExternallyTagged,
            vec![
                ("unit", VariantPayload::Unit),
                (
                    "scalar",
                    VariantPayload::Newtype(Box::new(Type::new_bool())),
                ),
                ("record", VariantPayload::Record(record.clone())),
                ("tuple", VariantPayload::Tuple(vec![Type::new_bool()])),
            ],
        );
        for value in [
            Value::String("unit".into()),
            object(vec![("scalar", Value::Bool(true))]),
            object(vec![("record", object(vec![("rows", Value::U64(1))]))]),
            object(vec![("tuple", Value::List(vec![Value::Bool(true)]))]),
        ] {
            validate_configuration(&value, &external, &BTreeMap::new()).unwrap();
        }
        for value in [
            Value::Null,
            Value::String("unknown".into()),
            Value::String("scalar".into()),
            object(vec![]),
            object(vec![("unknown", Value::Null)]),
            object(vec![("unit", Value::Null)]),
            object(vec![("scalar", Value::Null)]),
            object(vec![("record", object(vec![]))]),
            object(vec![("record", object(vec![("rows", Value::Bool(true))]))]),
            object(vec![(
                "tuple",
                Value::List(vec![Value::Bool(true), Value::Bool(false)]),
            )]),
            object(vec![("scalar", Value::Bool(true)), ("other", Value::Null)]),
        ] {
            assert_eq!(
                validate_configuration(&value, &external, &BTreeMap::new())
                    .unwrap_err()
                    .code,
                "invalid_configuration"
            );
        }
        let internal = variant_type(
            VariantTag::InternallyTagged {
                field: "status".into(),
            },
            vec![
                ("unit", VariantPayload::Unit),
                ("record", VariantPayload::Record(record)),
            ],
        );
        for value in [
            object(vec![("status", Value::String("unit".into()))]),
            object(vec![
                ("status", Value::String("record".into())),
                ("rows", Value::U64(1)),
            ]),
        ] {
            validate_configuration(&value, &internal, &BTreeMap::new()).unwrap();
        }
        for value in [
            Value::String("unit".into()),
            object(vec![]),
            object(vec![("status", Value::Bool(true))]),
            object(vec![("status", Value::String("unknown".into()))]),
            object(vec![("status", Value::String("record".into()))]),
            object(vec![
                ("status", Value::String("record".into())),
                ("rows", Value::Bool(true)),
            ]),
            object(vec![
                ("status", Value::String("unit".into())),
                ("rows", Value::U64(1)),
            ]),
        ] {
            assert_eq!(
                validate_configuration(&value, &internal, &BTreeMap::new())
                    .unwrap_err()
                    .code,
                "invalid_configuration"
            );
        }
    }

    #[test]
    fn variants_preserve_forbidden_nested_types_and_unsupported_tags() {
        for nested in [
            TypeKind::Stream(StreamType {
                element: Box::new(Type::new_bool()),
                end: None,
            }),
            TypeKind::Handle(HandleType {
                interface: TypeRef::new("test"),
                mode: HandleMode::Own,
            }),
            TypeKind::Function(FunctionType {
                params: vec![],
                results: vec![],
                throws: None,
                async_fn: false,
            }),
        ] {
            let ty = variant_type(
                VariantTag::ExternallyTagged,
                vec![("bad", VariantPayload::Newtype(Box::new(Type::new(nested))))],
            );
            assert_eq!(
                profile(&ty, true, &BTreeMap::new(), &mut BTreeSet::new())
                    .unwrap_err()
                    .code,
                "interface_incompatible"
            );
        }
        for tag in [
            VariantTag::Untagged,
            VariantTag::AdjacentlyTagged {
                tag_field: "tag".into(),
                data_field: "data".into(),
            },
        ] {
            let ty = variant_type(tag, vec![("unit", VariantPayload::Unit)]);
            assert!(profile(&ty, true, &BTreeMap::new(), &mut BTreeSet::new()).is_err());
        }
        let scalar_internal = variant_type(
            VariantTag::InternallyTagged {
                field: "tag".into(),
            },
            vec![("bad", VariantPayload::Newtype(Box::new(Type::new_bool())))],
        );
        assert!(
            profile(
                &scalar_internal,
                true,
                &BTreeMap::new(),
                &mut BTreeSet::new()
            )
            .is_err()
        );
    }

    #[test]
    fn actual_vdb_interface_activates_and_validates_recursive_dtos() {
        use semantic_data::{
            query::{BinaryOp, Expr, Operand},
            vdb::*,
        };
        struct Vdb {
            descriptors: Vec<ImplementationDescriptor>,
        }
        impl InterfaceImplementation for Vdb {
            fn descriptors(&self) -> &[ImplementationDescriptor] {
                &self.descriptors
            }
            fn invoke<'a>(
                &'a self,
                call: ValidatedInvocation,
                _: InvocationContext,
            ) -> InvocationFuture<'a> {
                Box::pin(async move {
                    match call.method.as_str() {
                        "describe" => Ok(InvocationOutput::Values(vec![
                            DatabaseDescriptor {
                                title: "Test".into(),
                                description: None,
                                schema_revision: "1".into(),
                                allow_untyped: true,
                                schema: DatabaseSchema {
                                    attributes: vec![AttributeType {
                                        id: "test:flag".into(),
                                        name: "Flag".into(),
                                        ty: Type::new_bool(),
                                        constraints: vec![],
                                        meta: Meta::default(),
                                    }],
                                    ..Default::default()
                                },
                            }
                            .into_value(),
                        ])),
                        "negotiate" => {
                            let InvocationArgument::Value(request) =
                                call.arguments.into_iter().next().unwrap()
                            else {
                                panic!("request")
                            };
                            let request = ScanRequest::from_value(request).unwrap();
                            Ok(InvocationOutput::Values(vec![
                                ScanPlan::Accepted {
                                    plan: AcceptedScan {
                                        filters: vec![
                                            FilterSupport::Unsupported;
                                            request.filters.len()
                                        ],
                                        ordered_prefix: 0,
                                        limit_applied: false,
                                        offset_applied: false,
                                        estimated_rows: None,
                                        token: None,
                                        schema_revision: "1".into(),
                                    },
                                }
                                .into_value(),
                            ]))
                        }
                        "scan" => Ok(InvocationOutput::Stream(OwnedValueStream::new(
                            stream::iter([
                                Ok(StreamEvent::Item(object(vec![(
                                    "id",
                                    Value::String("one".into()),
                                )]))),
                                Ok(StreamEvent::End(Some(ScanSummary { rows: 1 }.into_value()))),
                            ]),
                        ))),
                        _ => panic!("method"),
                    }
                })
            }
        }
        let declarations = package().root.interfaces;
        let declaration = declarations[INTERFACE_NAME].clone();
        let definitions = semantic_data::query::semantic::definitions();
        let fingerprint = interface_fingerprint(&declaration, &definitions).unwrap();
        let implementation = ConformingImplementation::new(
            Arc::new(Vdb {
                descriptors: vec![ImplementationDescriptor {
                    export: "database".into(),
                    interface: InterfaceRef {
                        package: PACKAGE_NAME.into(),
                        module: MODULE_NAME.into(),
                        contract: None,
                        name: INTERFACE_NAME.into(),
                    },
                    package_version: "1.0.0".into(),
                    fingerprint: fingerprint.clone(),
                }],
            }),
            BTreeMap::from([("database".into(), declaration.clone())]),
            definitions.clone(),
        )
        .unwrap();
        assert_eq!(implementation.descriptors()[0].fingerprint, fingerprint);
        block_on(async {
            let invoke = |method: &str, args: Vec<Value>| {
                implementation.invoke(
                    ValidatedInvocation {
                        export: "database".into(),
                        method: method.into(),
                        arguments: args.into_iter().map(InvocationArgument::Value).collect(),
                    },
                    InvocationContext::default(),
                )
            };
            let InvocationOutput::Values(values) = invoke("describe", vec![]).await.unwrap() else {
                panic!("values")
            };
            DatabaseDescriptor::from_value(values[0].clone()).unwrap();
            let Err(error) = invoke("negotiate", vec![Value::Null]).await else {
                panic!("invalid input accepted")
            };
            assert_eq!(error.code, "invalid_argument");
            let request = ScanRequest {
                filters: vec![Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(
                        semantic_data::value::FieldPath::from_fields(["id"]),
                    ))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String("one".into())))),
                }],
                ..Default::default()
            };
            let InvocationOutput::Values(values) =
                invoke("negotiate", vec![request.clone().into_value()])
                    .await
                    .unwrap()
            else {
                panic!("values")
            };
            let ScanPlan::Accepted { plan } = ScanPlan::from_value(values[0].clone()).unwrap()
            else {
                panic!("accepted")
            };
            let InvocationOutput::Stream(mut rows) = invoke(
                "scan",
                vec![request.into_value(), plan.into_value(), object(vec![])],
            )
            .await
            .unwrap() else {
                panic!("stream")
            };
            assert!(matches!(
                rows.next().await.unwrap().unwrap(),
                StreamEvent::Item(_)
            ));
            assert!(matches!(
                rows.next().await.unwrap().unwrap(),
                StreamEvent::End(Some(_))
            ));
        });
    }

    #[test]
    fn reference_constraints_survive_argument_result_and_stream_validation() {
        struct Echo;
        impl InterfaceImplementation for Echo {
            fn descriptors(&self) -> &[ImplementationDescriptor] {
                &[]
            }
            fn invoke<'a>(
                &'a self,
                mut call: ValidatedInvocation,
                _: InvocationContext,
            ) -> InvocationFuture<'a> {
                Box::pin(async move {
                    let InvocationArgument::Value(value) = call.arguments.remove(0) else {
                        panic!("value")
                    };
                    Ok(InvocationOutput::Values(vec![value]))
                })
            }
        }
        let reference = |name: &str| Type::new(TypeKind::Named(TypeRef::new(name)));
        let mut outer = reference("middle");
        outer.constraints.push(Constraint::Prefix("a".into()));
        let mut middle = reference("base");
        middle.constraints.push(Constraint::Suffix("z".into()));
        let string = || {
            Type::new(TypeKind::String(StringType {
                format: None,
                normalization: None,
            }))
        };
        let mut base = string();
        base.constraints
            .push(Constraint::Length(LengthSpec::Exactly(3)));
        let definitions: BTreeMap<_, _> = [("middle", middle), ("base", base)]
            .into_iter()
            .map(|(name, ty)| {
                (
                    name.to_owned(),
                    TypeDef {
                        name: name.into(),
                        module: None,
                        params: vec![],
                        ty,
                        visibility: Visibility::Public,
                        meta: Default::default(),
                    },
                )
            })
            .collect();
        block_on(async {
            for text in ["noz", "abc", "az"] {
                let value = Value::String(text.into());
                for result in [false, true] {
                    let mut implementation = ConformingImplementation {
                        inner: Arc::new(Echo),
                        declarations: BTreeMap::new(),
                        definitions: Arc::new(definitions.clone()),
                    };
                    implementation.declarations.insert(
                        "test".into(),
                        InterfaceType {
                            methods: vec![InterfaceMethod {
                                name: "echo".into(),
                                signature: FunctionType {
                                    params: vec![FunctionParam {
                                        name: None,
                                        ty: if result { string() } else { outer.clone() },
                                    }],
                                    results: vec![if result { outer.clone() } else { string() }],
                                    throws: None,
                                    async_fn: true,
                                },
                            }],
                        },
                    );
                    let error = implementation
                        .invoke(
                            ValidatedInvocation {
                                export: "test".into(),
                                method: "echo".into(),
                                arguments: vec![InvocationArgument::Value(value.clone())],
                            },
                            InvocationContext::default(),
                        )
                        .await
                        .err()
                        .unwrap();
                    assert_eq!(
                        error.code,
                        if result {
                            "invalid_output"
                        } else {
                            "invalid_argument"
                        }
                    );
                }
                for event in [
                    StreamEvent::Item(value.clone()),
                    StreamEvent::End(Some(value.clone())),
                ] {
                    let mut stream = validated_stream(
                        OwnedValueStream::new(stream::iter([Ok(event)])),
                        StreamType {
                            element: Box::new(outer.clone()),
                            end: Some(Box::new(outer.clone())),
                        },
                        None,
                        Arc::new(definitions.clone()),
                        "invalid_output",
                    );
                    assert_eq!(
                        stream.next().await.unwrap().unwrap_err().code,
                        "invalid_output"
                    );
                }
                assert!(validate_configuration(&value, &outer, &definitions).is_err());
            }
            validate_configuration(&Value::String("abz".into()), &outer, &definitions).unwrap();
        });
    }

    #[test]
    fn canonical_import_profile_and_wrong_item_terminal_error() {
        let package = semantic_data::import::package();
        for interface in package.root.interfaces.values() {
            for method in &interface.methods {
                for ty in method
                    .signature
                    .params
                    .iter()
                    .map(|p| &p.ty)
                    .chain(method.signature.results.iter())
                {
                    profile(ty, true, &BTreeMap::new(), &mut BTreeSet::new()).unwrap();
                }
            }
        }
        let schema = StreamType {
            element: Box::new(Type::new_bool()),
            end: Some(Box::new(Type::new_bool())),
        };
        block_on(async {
            for event in [
                StreamEvent::Item(Value::Null),
                StreamEvent::End(None),
                StreamEvent::End(Some(Value::String("wrong".into()))),
            ] {
                let mut stream = validated_stream(
                    OwnedValueStream::new(stream::iter([Ok(event)])),
                    schema.clone(),
                    None,
                    Arc::new(BTreeMap::new()),
                    "invalid_output",
                );
                assert_eq!(
                    stream.next().await.unwrap().unwrap_err().code,
                    "invalid_output"
                );
                assert!(stream.next().await.is_none());
            }
        });
        let error = InvocationError {
            code: "declared".into(),
            message: "bad".into(),
            data: Some(Value::Null),
        };
        assert_eq!(
            validate_error(error, None, &BTreeMap::new()).code,
            "invalid_output"
        );
        assert_eq!(
            validate_error(
                InvocationError::new("connection_lost", "gone"),
                None,
                &BTreeMap::new()
            )
            .code,
            "connection_lost"
        );
    }

    #[test]
    fn nested_streams_and_value_constraints_are_rejected() {
        let stream = Type::new(TypeKind::Stream(StreamType {
            element: Box::new(Type::new_bool()),
            end: None,
        }));
        let nested = Type::new(TypeKind::List(ListType {
            items: Box::new(stream),
        }));
        assert!(profile(&nested, true, &BTreeMap::new(), &mut BTreeSet::new()).is_err());
        let mut ty = Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        }));
        ty.constraints.push(Constraint::Prefix("ok".into()));
        assert!(
            validate(
                &Value::String("wrong".into()),
                &ty,
                &BTreeMap::new(),
                "invalid_argument"
            )
            .is_err()
        );
        assert!(
            validate(
                &Value::String("okay".into()),
                &ty,
                &BTreeMap::new(),
                "invalid_argument"
            )
            .is_ok()
        );
    }
}
