use crate::schema::{self, Type, TypeDef, TypeKind, TypeRef};
use crate::value::{FromValue, FromValueError, Object, SemanticType, Value};

pub(super) trait Definition: SemanticType {
    const NAME: &'static str;
    fn definition() -> Type;
}

pub(super) fn named(name: &str) -> Type {
    Type::new(TypeKind::Named(TypeRef::new(name)))
}

pub(super) fn definition<T: Definition>() -> TypeDef {
    TypeDef {
        name: T::NAME.into(),
        module: Some(super::MODULE_NAME.into()),
        params: Vec::new(),
        ty: T::definition(),
        visibility: schema::Visibility::Public,
        meta: schema::Meta::default(),
    }
}

pub(super) fn record(fields: Vec<(&str, Type, bool, Option<Value>)>) -> schema::RecordType {
    schema::RecordType {
        fields: fields
            .into_iter()
            .map(|(name, mut ty, required, mut default)| {
                let mut meta = schema::Meta::default();
                normalize_dictionary_type(&mut ty);
                // Catalog default checks currently inspect inline shapes. Inline
                // only these finite default-bearing leaves; keep recursion named.
                if default.is_some() {
                    if let TypeKind::Named(reference) = &ty.kind {
                        ty = match reference.name.as_str() {
                            "semantic:query:FieldFormat" => <crate::query::FieldFormat as Definition>::definition(),
                            "semantic:query:TextAnalyzer" => <crate::query::TextAnalyzer as Definition>::definition(),
                            "semantic:query:TextMatchMode" => <crate::query::TextMatchMode as Definition>::definition(),
                            "semantic:schema:IndexKind" => <schema::IndexKind as Definition>::definition(),
                            "semantic:schema:OnDelete" => <schema::OnDelete as Definition>::definition(),
                            "semantic:query:Expr" => {
                                default = None;
                                meta.description = Some("Defaults to Expr::Operand(Operand::Literal(Value::U64(0))) when omitted.".into());
                                ty
                            }
                            _ => ty,
                        };
                    }
                }
                (
                    name.into(),
                    schema::Field {
                        ty,
                        required,
                        readonly: false,
                        writeonly: false,
                        default,
                        meta,
                    },
                )
            })
            .collect(),
        open: false,
        additional: None,
        required_order: None,
    }
}

// Generic BTreeMap schemas currently carry `additional` with `open: false`.
// Catalogs require dictionaries to be open with a typed additional value.
// Correct the inline dictionary shapes in this contract without altering
// unrelated SemanticType implementations or historical migrations.
pub(super) fn normalize_dictionary_type(ty: &mut Type) {
    match &mut ty.kind {
        TypeKind::Record(record) => {
            if let Some(additional) = &mut record.additional {
                record.open = true;
                normalize_dictionary_type(additional);
            }
            for field in record.fields.values_mut() {
                normalize_dictionary_type(&mut field.ty);
            }
        }
        TypeKind::Optional(optional) => normalize_dictionary_type(&mut optional.inner),
        TypeKind::List(list) => normalize_dictionary_type(&mut list.items),
        TypeKind::Variant(variant) => {
            for case in &mut variant.variants {
                match &mut case.payload {
                    schema::VariantPayload::Unit => {}
                    schema::VariantPayload::Newtype(inner) => normalize_dictionary_type(inner),
                    schema::VariantPayload::Tuple(items) => {
                        for item in items {
                            normalize_dictionary_type(item);
                        }
                    }
                    schema::VariantPayload::Record(record) => {
                        for field in record.fields.values_mut() {
                            normalize_dictionary_type(&mut field.ty);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

pub(super) fn object(value: Value) -> Result<Object, FromValueError> {
    Object::from_value(value)
}

pub(super) fn finish(object: Object) -> Result<(), FromValueError> {
    if let Some((name, _)) = object.into_btree().into_iter().next() {
        Err(FromValueError::new("unknown field").in_field(&name))
    } else {
        Ok(())
    }
}

pub(super) fn take<T: FromValue>(object: &mut Object, name: &str) -> Result<T, FromValueError> {
    T::from_value(
        object
            .remove(name)
            .ok_or_else(|| FromValueError::missing_field(name))?,
    )
    .map_err(|error| error.in_field(name))
}

pub(super) fn tagged(name: &str, payload: Option<Value>) -> Value {
    payload.map_or_else(
        || Value::String(name.into()),
        |payload| Value::Object([(name.to_owned(), payload)].into_iter().collect()),
    )
}

pub(super) fn variant(value: Value) -> Result<(String, Option<Value>), FromValueError> {
    match value {
        Value::String(name) => Ok((name, None)),
        Value::Object(object) if object.len() == 1 => {
            let (name, payload) = object.into_btree().into_iter().next().unwrap();
            Ok((name, Some(payload)))
        }
        other => Err(FromValueError::expected(
            "variant name or single-key object",
            &other,
        )),
    }
}

pub(super) fn payload(value: Option<Value>) -> Result<Value, FromValueError> {
    value.ok_or_else(|| FromValueError::new("variant requires a payload"))
}

macro_rules! field_required {
    ([required]) => {
        true
    };
    ($other:tt) => {
        false
    };
}
macro_rules! field_default {
    ($ty:ty, [default $value:expr]) => {{
        let value: $ty = $value;
        Some(IntoValue::into_value(value))
    }};
    ($ty:ty, $other:tt) => {
        None
    };
}
macro_rules! field_encode {
    ($object:ident, $name:expr, $value:expr, [presence]) => {
        if let Some(value) = $value {
            $object.insert($name, value);
        }
    };
    ($object:ident, $name:expr, $value:expr, $mode:tt) => {
        $object.insert($name, IntoValue::into_value($value));
    };
}
macro_rules! field_decode {
    ($object:ident, $name:expr, [required]) => {
        take(&mut $object, $name)?
    };
    ($object:ident, $name:expr, [default $value:expr]) => {
        match $object.remove($name) {
            Some(value) => FromValue::from_value(value).map_err(|error| error.in_field($name))?,
            None => $value,
        }
    };
    ($object:ident, $name:expr, [presence]) => {
        $object.remove($name)
    };
}
macro_rules! named_type {
    ($ty:ty, $name:literal, $definition:expr) => {
        impl SemanticType for $ty {
            fn semantic_type() -> Type {
                named($name)
            }
        }
        impl Definition for $ty {
            const NAME: &'static str = $name;
            fn definition() -> Type {
                let mut ty = $definition;
                normalize_dictionary_type(&mut ty);
                ty
            }
        }
    };
}
macro_rules! ast_record {
    ($ty:path, $name:literal, { $( $field:ident : $fty:ty => $wire:literal $mode:tt ),* $(,)? }) => {
        named_type!($ty, $name, Type::new(TypeKind::Record(record(vec![$(
            ($wire, <$fty>::semantic_type(), field_required!($mode), field_default!($fty, $mode)),
        )*]))));
        impl IntoValue for $ty {
            fn into_value(self) -> Value {
                let mut object = Object::new();
                $(field_encode!(object, $wire, self.$field, $mode);)*
                Value::Object(object)
            }
        }
        impl FromValue for $ty {
            fn from_value(value: Value) -> Result<Self, FromValueError> {
                let mut object = object(value)?;
                let result = Self { $($field: field_decode!(object, $wire, $mode)),* };
                finish(object)?;
                Ok(result)
            }
        }
    };
}
macro_rules! variant_payload_type {
    () => { schema::VariantPayload::Unit };
    (($value:ident : $ty:ty)) => { schema::VariantPayload::Newtype(Box::new(<$ty>::semantic_type())) };
    ({ $( $field:ident : $fty:ty => $wire:literal $mode:tt ),* $(,)? }) => {
        schema::VariantPayload::Record(record(vec![$(
            ($wire, <$fty>::semantic_type(), field_required!($mode), field_default!($fty, $mode)),
        )*]))
    };
}
macro_rules! variant_encode {
    () => { None };
    (($value:ident : $ty:ty)) => { Some($value.into_value()) };
    ({ $( $field:ident : $fty:ty => $wire:literal $mode:tt ),* $(,)? }) => {{
        let mut object = Object::new();
        $(field_encode!(object, $wire, $field, $mode);)*
        Some(Value::Object(object))
    }};
}
macro_rules! variant_decode {
    ($payload:ident, $variant:ident) => {{
        if $payload.is_some() { return Err(FromValueError::new("unit variant cannot have a payload")); }
        Ok(Self::$variant)
    }};
    ($payload:ident, $variant:ident, ($value:ident : $ty:ty)) => {
        <$ty>::from_value(payload($payload)?).map(Self::$variant)
    };
    ($payload:ident, $variant:ident, { $( $field:ident : $fty:ty => $wire:literal $mode:tt ),* $(,)? }) => {{
        let mut object = object(payload($payload)?)?;
        let result = Self::$variant { $($field: field_decode!(object, $wire, $mode)),* };
        finish(object)?;
        Ok(result)
    }};
}
macro_rules! ast_variant {
    ($ty:path, $name:literal, { $( $variant:ident => $wire:literal $( ($value:ident : $vty:ty) )? $( { $( $field:ident : $fty:ty => $fwire:literal $mode:tt ),* $(,)? } )? ),* $(,)? }) => {
        named_type!($ty, $name, Type::new(TypeKind::Variant(schema::VariantType {
            tag: schema::VariantTag::ExternallyTagged,
            variants: vec![$(schema::VariantCase {
                name: $wire.into(),
                payload: variant_payload_type!($(($value: $vty))? $({ $($field: $fty => $fwire $mode),* })?),
                discriminant: None,
                meta: schema::Meta::default(),
            }),*],
        })));
        impl IntoValue for $ty {
            fn into_value(self) -> Value {
                match self { $(Self::$variant $(($value))? $({ $($field),* })? =>
                    tagged($wire, variant_encode!($(($value: $vty))? $({ $($field: $fty => $fwire $mode),* })?)),)* }
            }
        }
        impl FromValue for $ty {
            fn from_value(value: Value) -> Result<Self, FromValueError> {
                let (name, value) = variant(value)?;
                match name.as_str() {
                    $($wire => variant_decode!(value, $variant $(, ($value: $vty))? $(, { $($field: $fty => $fwire $mode),* })?)
                        .map_err(|error: FromValueError| error.in_field($wire)),)*
                    _ => Err(FromValueError::new(format!("unknown variant '{name}'"))),
                }
            }
        }
    };
}
macro_rules! ast_enum {
    ($ty:path, $name:literal, { $( $variant:ident => $wire:literal ),* $(,)? }) => {
        named_type!($ty, $name, crate::value::convert::__private::enum_type(vec![$(($wire, None)),*]));
        impl IntoValue for $ty {
            fn into_value(self) -> Value { Value::String(match self { $(Self::$variant => $wire,)* }.into()) }
        }
        impl FromValue for $ty {
            fn from_value(value: Value) -> Result<Self, FromValueError> {
                let name = String::from_value(value)?;
                match name.as_str() { $($wire => Ok(Self::$variant),)*
                    _ => Err(FromValueError::new(format!("unknown enum value '{name}'"))),
                }
            }
        }
    };
}
macro_rules! ast_unit {
    ($ty:path, $name:literal) => {
        named_type!($ty, $name, Type::new(TypeKind::Record(record(Vec::new()))));
        impl IntoValue for $ty {
            fn into_value(self) -> Value {
                Value::Object(Object::new())
            }
        }
        impl FromValue for $ty {
            fn from_value(value: Value) -> Result<Self, FromValueError> {
                finish(object(value)?)?;
                Ok(Self)
            }
        }
    };
}
