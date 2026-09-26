//! Lowered, storable data types.
//!
//! The surface [`TypeKind`](crate::schema::TypeKind) is a universal type
//! language: it describes package contracts, interfaces, functions, handles
//! and streams as well as the shapes of stored data. Every surface type can be
//! persisted as a *definition*, but only a subset can be inhabited by stored
//! [`Value`](crate::value::Value)s.
//!
//! [`DataType`] is that subset. It is never persisted; it is derived from a
//! surface [`Type`](crate::schema::Type) by [`lower_type`] /
//! [`lower_type_def`], which resolve named types through a [`TypeResolver`]
//! and reject kinds that have no stored-value counterpart with a typed
//! [`LowerError`].
//!
//! # Representation
//!
//! A lowered node is a [`DataType`]: a [`DataKind`] plus the value
//! constraints that apply to it. Keeping constraints on every node (instead
//! of a separate constraint tree) lets consumers match on `kind` and then
//! check `constraints` uniformly.
//!
//! Lowering normalizes the surface language so that downstream matches stay
//! small:
//!
//! - `Named` references are resolved and inlined. Constraints declared at
//!   the reference site are appended to the resolved node. The only named
//!   node left is [`DataKind::Recursive`], a back-reference emitted when a
//!   definition refers to itself through a data constructor (list, map,
//!   tuple, record field or variant payload). Consumers resolve it through
//!   the catalog's lowered definition of that name.
//! - Inline `Attribute` types are replaced by the attribute's value type,
//!   with the attribute constraints appended.
//! - `Array` and `Set` become [`DataKind::List`] carrying the fixed length and
//!   distinctness requirements.
//! - `Result` becomes an externally tagged two-case [`DataKind::Variant`]
//!   (`ok` / `err`).
//! - Record-only `Intersection`s are merged into a single
//!   [`DataKind::Record`].
//! - Embedded class shapes (`TypeKind::Class` in a data position) become a
//!   [`DataKind::Record`] keyed by canonical attribute ID, including inherited
//!   attributes. [`RecordData::class`] records the origin so consumers can keep
//!   class-specific rules (for example accepting built-in fields on strict
//!   classes).
//!
//! See `docs/schema.md` for the full disposition table.

mod lower;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

pub use lower::{
    LowerError, LowerErrorKind, LowerPathSegment, TypeResolver, lower_type, lower_type_def,
};

use crate::schema::{
    BytesType, Constraint, EntityRef, EnumType, ExtensionType, FloatWidth, IntWidth, IpAddrType,
    LengthSpec, StringType, TypeName, UIntWidth, VariantTag,
};
use crate::value::Value;

/// A lowered, storable type node: shape plus value constraints.
#[derive(Clone, Debug, PartialEq)]
pub struct DataType {
    pub kind: DataKind,
    /// Constraints that apply to a value of this node, in declaration order.
    pub constraints: Vec<Constraint>,
}

impl DataType {
    pub fn new(kind: DataKind) -> Self {
        Self {
            kind,
            constraints: Vec::new(),
        }
    }
}

/// Shapes that can be inhabited by stored values.
#[derive(Clone, Debug, PartialEq)]
pub enum DataKind {
    /// Any stored value.
    Any,
    Null,
    Bool,
    /// A single-character string.
    Char,
    Number(NumberRepr),
    String(StringType),
    Bytes(BytesType),
    Temporal(TemporalRepr),
    Uuid,
    IpAddr(IpAddrType),
    Json,
    /// A value of the inner type, or null.
    Optional(Box<DataType>),
    List(ListData),
    Tuple(TupleData),
    Map(MapData),
    Record(RecordData),
    /// A value matching at least one of the listed types.
    Union(Vec<DataType>),
    Enum(EnumType),
    Variant(VariantData),
    /// A stored entity ID with foreign-key semantics.
    Ref(EntityRef),
    /// Extension types are opaque to the core; their values are not
    /// interpreted.
    Extension(ExtensionType),
    /// Back-reference to the named definition currently being lowered.
    Recursive(TypeName),
}

/// Number representations that have a stored-value counterpart.
///
/// Integer widths up to 128 bits and binary floats up to 64 bits are
/// representable; narrower declared widths are stored in the next larger
/// value representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NumberRepr {
    Int(IntWidth),
    UInt(UIntWidth),
    Float(FloatWidth),
    /// Any numeric value.
    Unspecified,
}

/// Temporal representations that have a stored-value counterpart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporalRepr {
    Date,
    Time,
    DateTime,
    Duration,
}

/// A homogeneous list. Also the lowered form of surface arrays and sets.
#[derive(Clone, Debug, PartialEq)]
pub struct ListData {
    pub items: Box<DataType>,
    /// Required item count (from surface `Array` types).
    pub length: Option<LengthSpec>,
    /// Whether items must be distinct (from surface `Set` types).
    pub distinct: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TupleData {
    pub items: Vec<DataType>,
    pub rest: Option<Box<DataType>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapData {
    pub keys: Box<DataType>,
    pub values: Box<DataType>,
    pub ordered: bool,
}

/// A resolved record shape.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordData {
    pub fields: BTreeMap<String, FieldData>,
    pub rest: RecordRest,
    /// Class ID when this record was lowered from an embedded class shape.
    pub class: Option<TypeName>,
}

/// How a record treats fields that are not declared.
#[derive(Clone, Debug, PartialEq)]
pub enum RecordRest {
    /// Undeclared fields are rejected.
    Closed,
    /// Undeclared fields are accepted without validation.
    Open,
    /// Undeclared fields are accepted and must match this type.
    Additional(Box<DataType>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldData {
    pub ty: DataType,
    pub required: bool,
    /// Computed class attributes are derived at read time and never stored.
    pub computed: bool,
    pub default: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariantData {
    pub tag: VariantTag,
    pub cases: Vec<VariantCaseData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariantCaseData {
    pub name: String,
    pub payload: VariantPayloadData,
    pub discriminant: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VariantPayloadData {
    Unit,
    Tuple(Vec<DataType>),
    Record(RecordData),
    Newtype(Box<DataType>),
}
