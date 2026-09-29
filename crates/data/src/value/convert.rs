//! Conversions between Rust types and [`Value`]s, plus the [`Type`] they declare.
//!
//! [`SemanticType`], [`IntoValue`] and [`FromValue`] can be derived with the
//! macros of the same name (see `semantic_macros`).

use std::collections::BTreeMap;
use std::fmt;

use super::{DateTime, Object, OrderedF64, Value};
use crate::schema::{
    AnyType, BoolType, FloatWidth, IntWidth, ListType, NeverType, NullType, NumberType,
    OptionalType, RecordType, StringType, TemporalType, Type, TypeKind, UIntWidth,
};

/// A Rust type with a declared semantic [`Type`].
pub trait SemanticType {
    fn semantic_type() -> Type;
}

/// Infallible conversion into a [`Value`].
pub trait IntoValue {
    fn into_value(self) -> Value;
}

/// Fallible conversion from a [`Value`].
pub trait FromValue: Sized {
    fn from_value(value: Value) -> Result<Self, FromValueError>;
}

/// Failure to convert a [`Value`], with the path to the offending value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FromValueError {
    /// Innermost segment first; see [`Self::path`].
    path: Vec<PathElement>,
    message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PathElement {
    Field(String),
    Index(usize),
}

impl FromValueError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            path: Vec::new(),
            message: message.into(),
        }
    }

    /// An error for a value that is not of the `expected` kind.
    pub fn expected(expected: &str, found: &Value) -> Self {
        Self::new(format!("expected {expected}, found {}", value_kind(found)))
    }

    pub fn missing_field(name: &str) -> Self {
        Self::new("missing required field").in_field(name)
    }

    /// Prefix the path with an object field.
    pub fn in_field(mut self, name: &str) -> Self {
        self.path.push(PathElement::Field(name.to_owned()));
        self
    }

    /// Prefix the path with a list index.
    pub fn at_index(mut self, index: usize) -> Self {
        self.path.push(PathElement::Index(index));
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// The path to the offending value, like `operations[0].id`; empty for the root.
    pub fn path(&self) -> String {
        let mut out = String::new();
        for element in self.path.iter().rev() {
            match element {
                PathElement::Field(name) => {
                    if !out.is_empty() {
                        out.push('.');
                    }
                    out.push_str(name);
                }
                PathElement::Index(index) => {
                    out.push_str(&format!("[{index}]"));
                }
            }
        }
        out
    }

    /// Describe the error relative to a named root, like `payload.name: expected string`.
    pub fn describe(&self, root: &str) -> String {
        let path = self.path();
        if path.is_empty() {
            format!("{root}: {}", self.message)
        } else if path.starts_with('[') {
            format!("{root}{path}: {}", self.message)
        } else {
            format!("{root}.{path}: {}", self.message)
        }
    }
}

impl fmt::Display for FromValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path();
        if path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{path}: {}", self.message)
        }
    }
}

impl std::error::Error for FromValueError {}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Void => "void",
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::I8(_)
        | Value::I16(_)
        | Value::I32(_)
        | Value::I64(_)
        | Value::I128(_)
        | Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::U64(_)
        | Value::U128(_) => "integer",
        Value::F32(_) | Value::F64(_) => "float",
        Value::Uuid(_) => "uuid",
        Value::IpAddr(_) => "ip address",
        Value::Duration(_) => "duration",
        Value::Time(_) => "time",
        Value::Date(_) => "date",
        Value::DateTime(_) => "datetime",
        Value::Bytes(_) => "bytes",
        Value::String(_) => "string",
        Value::List(_) => "list",
        Value::Map(_) => "map",
        Value::Object(_) => "object",
        Value::Variant(_) => "variant",
    }
}

// Value

impl SemanticType for Value {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Any(AnyType))
    }
}

impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}

impl FromValue for Value {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        Ok(value)
    }
}

// Unit: the void value.

impl SemanticType for () {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Never(NeverType))
    }
}

impl IntoValue for () {
    fn into_value(self) -> Value {
        Value::Void
    }
}

impl FromValue for () {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::Void | Value::Null => Ok(()),
            other => Err(FromValueError::expected("void", &other)),
        }
    }
}

/// The explicit null value, for results that are always null.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Null;

impl SemanticType for Null {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Null(NullType))
    }
}

impl IntoValue for Null {
    fn into_value(self) -> Value {
        Value::Null
    }
}

impl FromValue for Null {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::Void | Value::Null => Ok(Null),
            other => Err(FromValueError::expected("null", &other)),
        }
    }
}

// Scalars

impl SemanticType for bool {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Bool(BoolType))
    }
}

impl IntoValue for bool {
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
}

impl FromValue for bool {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::Bool(value) => Ok(value),
            other => Err(FromValueError::expected("boolean", &other)),
        }
    }
}

impl SemanticType for String {
    fn semantic_type() -> Type {
        Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        }))
    }
}

impl IntoValue for String {
    fn into_value(self) -> Value {
        Value::String(self)
    }
}

impl FromValue for String {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::String(value) => Ok(value),
            other => Err(FromValueError::expected("string", &other)),
        }
    }
}

/// The value of any integer variant, if it fits into an `i128`.
fn integer(value: &Value) -> Option<Result<i128, ()>> {
    Some(Ok(match *value {
        Value::I8(v) => v.into(),
        Value::I16(v) => v.into(),
        Value::I32(v) => v.into(),
        Value::I64(v) => v.into(),
        Value::I128(v) => v,
        Value::U8(v) => v.into(),
        Value::U16(v) => v.into(),
        Value::U32(v) => v.into(),
        Value::U64(v) => v.into(),
        Value::U128(v) => return Some(i128::try_from(v).map_err(|_| ())),
        _ => return None,
    }))
}

/// Integers are encoded with their own width and decoded from any integer
/// variant whose value fits.
macro_rules! integer_impls {
    ($($ty:ty => $variant:ident, $kind:expr;)*) => {$(
        impl SemanticType for $ty {
            fn semantic_type() -> Type {
                Type::new(TypeKind::Number($kind))
            }
        }

        impl IntoValue for $ty {
            fn into_value(self) -> Value {
                Value::$variant(self as _)
            }
        }

        impl FromValue for $ty {
            fn from_value(value: Value) -> Result<Self, FromValueError> {
                match integer(&value) {
                    Some(Ok(number)) => <$ty>::try_from(number).map_err(|_| {
                        FromValueError::new(format!(
                            "integer {number} is out of range for {}",
                            stringify!($ty)
                        ))
                    }),
                    Some(Err(())) => Err(FromValueError::new(format!(
                        "integer is out of range for {}",
                        stringify!($ty)
                    ))),
                    None => Err(FromValueError::expected("integer", &value)),
                }
            }
        }
    )*};
}

integer_impls! {
    i8 => I8, NumberType::Int(IntWidth::I8);
    i16 => I16, NumberType::Int(IntWidth::I16);
    i32 => I32, NumberType::Int(IntWidth::I32);
    i64 => I64, NumberType::Int(IntWidth::I64);
    u8 => U8, NumberType::UInt(UIntWidth::U8);
    u16 => U16, NumberType::UInt(UIntWidth::U16);
    u32 => U32, NumberType::UInt(UIntWidth::U32);
    u64 => U64, NumberType::UInt(UIntWidth::U64);
    usize => U64, NumberType::UInt(UIntWidth::U64);
}

impl SemanticType for f64 {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Number(NumberType::Float(FloatWidth::F64)))
    }
}

impl IntoValue for f64 {
    fn into_value(self) -> Value {
        Value::F64(OrderedF64::from(self))
    }
}

impl FromValue for f64 {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::F64(value) => Ok(value.0),
            Value::F32(value) => Ok(value.0.into()),
            other => Err(FromValueError::expected("float", &other)),
        }
    }
}

impl SemanticType for DateTime {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Temporal(TemporalType::DateTime))
    }
}

impl IntoValue for DateTime {
    fn into_value(self) -> Value {
        Value::DateTime(self)
    }
}

impl FromValue for DateTime {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::DateTime(value) => Ok(value),
            other => Err(FromValueError::expected("datetime", &other)),
        }
    }
}

// Collections

/// `None` is encoded as null; null and void decode to `None`.
impl<T: SemanticType> SemanticType for Option<T> {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Optional(OptionalType {
            inner: Box::new(T::semantic_type()),
        }))
    }
}

impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Value {
        self.map_or(Value::Null, T::into_value)
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::Void | Value::Null => Ok(None),
            value => T::from_value(value).map(Some),
        }
    }
}

impl<T: SemanticType> SemanticType for Vec<T> {
    fn semantic_type() -> Type {
        Type::new(TypeKind::List(ListType {
            items: Box::new(T::semantic_type()),
        }))
    }
}

impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> Value {
        Value::List(self.into_iter().map(T::into_value).collect())
    }
}

impl<T: FromValue> FromValue for Vec<T> {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::List(items) => items
                .into_iter()
                .enumerate()
                .map(|(index, item)| T::from_value(item).map_err(|err| err.at_index(index)))
                .collect(),
            other => Err(FromValueError::expected("list", &other)),
        }
    }
}

/// String-keyed maps are encoded as objects: a record of arbitrary keys.
impl<T: SemanticType> SemanticType for BTreeMap<String, T> {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Record(RecordType {
            fields: BTreeMap::new(),
            open: false,
            additional: Some(Box::new(T::semantic_type())),
            required_order: None,
        }))
    }
}

impl<T: IntoValue> IntoValue for BTreeMap<String, T> {
    fn into_value(self) -> Value {
        Value::Object(
            self.into_iter()
                .map(|(key, value)| (key, value.into_value()))
                .collect(),
        )
    }
}

impl<T: FromValue> FromValue for BTreeMap<String, T> {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::Object(object) => object
                .into_btree()
                .into_iter()
                .map(|(key, value)| match T::from_value(value) {
                    Ok(value) => Ok((key, value)),
                    Err(err) => Err(err.in_field(&key)),
                })
                .collect(),
            other => Err(FromValueError::expected("object", &other)),
        }
    }
}

/// An object with arbitrary fields: an open record.
impl SemanticType for Object {
    fn semantic_type() -> Type {
        Type::new(TypeKind::Record(RecordType {
            fields: BTreeMap::new(),
            open: true,
            additional: None,
            required_order: None,
        }))
    }
}

impl IntoValue for Object {
    fn into_value(self) -> Value {
        Value::Object(self)
    }
}

impl FromValue for Object {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        match value {
            Value::Object(object) => Ok(object),
            other => Err(FromValueError::expected("object", &other)),
        }
    }
}

/// Support code for the derive macros. Not a public API.
#[doc(hidden)]
pub mod __private {
    use super::*;
    use crate::schema::{
        EnumRepr, EnumType, EnumVariant, Field, Meta, VariantCase, VariantPayload, VariantTag,
        VariantType,
    };

    pub use crate::schema::Type;
    pub use crate::value::{Object, Value};

    pub struct FieldDef {
        pub name: &'static str,
        pub ty: Type,
        pub required: bool,
        pub default: Option<Value>,
        pub description: Option<&'static str>,
    }

    fn meta(description: Option<&'static str>) -> Meta {
        Meta {
            description: description.map(str::to_owned),
            ..Meta::default()
        }
    }

    /// Fields of a flattened record type.
    pub fn flattened<T: SemanticType>() -> Vec<(String, Field)> {
        match T::semantic_type().kind {
            TypeKind::Record(record) => record.fields.into_iter().collect(),
            _ => panic!("flattened field types must be records"),
        }
    }

    pub fn record(fields: Vec<FieldDef>, flattened: Vec<Vec<(String, Field)>>) -> RecordType {
        let mut out: BTreeMap<String, Field> = flattened.into_iter().flatten().collect();
        for field in fields {
            out.insert(
                field.name.to_owned(),
                Field {
                    ty: field.ty,
                    required: field.required,
                    readonly: false,
                    writeonly: false,
                    default: field.default,
                    meta: meta(field.description),
                },
            );
        }
        RecordType {
            fields: out,
            open: false,
            additional: None,
            required_order: None,
        }
    }

    pub fn record_type(fields: Vec<FieldDef>, flattened: Vec<Vec<(String, Field)>>) -> Type {
        Type::new(TypeKind::Record(record(fields, flattened)))
    }

    pub fn enum_type(variants: Vec<(&'static str, Option<&'static str>)>) -> Type {
        Type::new(TypeKind::Enum(EnumType {
            repr: EnumRepr::String,
            variants: variants
                .into_iter()
                .map(|(name, description)| EnumVariant {
                    name: name.to_owned(),
                    value: None,
                    symbol: Some(name.to_owned()),
                    meta: meta(description),
                })
                .collect(),
        }))
    }

    /// An internally tagged variant; `None` records are unit variants.
    pub fn tagged_type(
        tag: &'static str,
        variants: Vec<(&'static str, Option<&'static str>, Option<RecordType>)>,
    ) -> Type {
        Type::new(TypeKind::Variant(VariantType {
            tag: VariantTag::InternallyTagged { field: tag.into() },
            variants: variants
                .into_iter()
                .map(|(name, description, record)| VariantCase {
                    name: name.to_owned(),
                    payload: record.map_or(VariantPayload::Unit, VariantPayload::Record),
                    discriminant: None,
                    meta: meta(description),
                })
                .collect(),
        }))
    }

    pub fn default_value<T: Default + IntoValue>() -> Option<Value> {
        Some(T::default().into_value())
    }

    pub fn object(value: Value) -> Result<Object, FromValueError> {
        match value {
            Value::Object(object) => Ok(object),
            other => Err(FromValueError::expected("object", &other)),
        }
    }

    pub fn string(value: Value) -> Result<String, FromValueError> {
        String::from_value(value)
    }

    pub fn unknown_variant(name: &str, expected: &[&str]) -> FromValueError {
        FromValueError::new(format!(
            "unknown variant '{name}', expected one of: {}",
            expected.join(", ")
        ))
    }

    pub fn required<T: FromValue>(object: &mut Object, name: &str) -> Result<T, FromValueError> {
        match object.remove(name) {
            Some(value) => T::from_value(value).map_err(|err| err.in_field(name)),
            None => Err(FromValueError::missing_field(name)),
        }
    }

    pub fn or_else<T: FromValue>(
        object: &mut Object,
        name: &str,
        default: impl FnOnce() -> T,
    ) -> Result<T, FromValueError> {
        match object.remove(name) {
            Some(value) => T::from_value(value).map_err(|err| err.in_field(name)),
            None => Ok(default()),
        }
    }

    /// Decode a flattened field from the fields not consumed by the outer type.
    pub fn flatten<T: FromValue>(object: &Object) -> Result<T, FromValueError> {
        T::from_value(Value::Object(object.clone()))
    }

    pub fn merge(object: &mut Object, value: Value) {
        match value {
            Value::Object(fields) => object.extend(fields.into_btree()),
            _ => panic!("flattened field values must be objects"),
        }
    }
}
