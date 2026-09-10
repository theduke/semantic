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
    while let TypeKind::Ref(reference) = &ty.kind {
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
        TypeKind::Ref(reference) => {
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
        | TypeKind::Enum(_) => Ok(false),
        _ => Err(incompatible(
            "unsupported nested stream, handle, callable, or non-value interface type",
        )),
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
        let TypeKind::Ref(reference) = &ty.kind else {
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
        let reference = |name: &str| Type::new(TypeKind::Ref(TypeRef::new(name)));
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
