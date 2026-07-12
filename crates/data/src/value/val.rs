use std::cmp::Ordering;

use facet::Facet;

pub type OrderedF32 = ordered_float::OrderedFloat<f32>;
pub type OrderedF64 = ordered_float::OrderedFloat<f64>;

use super::{Date, DateTime, Duration, Map, Object, Time, Uuid, ValueRef, VariantValue};

#[derive(Facet)]
#[facet(transparent)]
struct FacetProxyF32(f32);

impl TryFrom<FacetProxyF32> for OrderedF32 {
    type Error = &'static str;

    fn try_from(proxy: FacetProxyF32) -> Result<Self, Self::Error> {
        Ok(Self::from(proxy.0))
    }
}

impl From<&OrderedF32> for FacetProxyF32 {
    fn from(key: &OrderedF32) -> Self {
        Self(key.into_inner())
    }
}

#[derive(Facet)]
#[facet(transparent)]
struct FacetProxyF64(f64);

impl TryFrom<FacetProxyF64> for OrderedF64 {
    type Error = &'static str;

    fn try_from(proxy: FacetProxyF64) -> Result<Self, Self::Error> {
        Ok(Self::from(proxy.0))
    }
}

impl From<&OrderedF64> for FacetProxyF64 {
    fn from(key: &OrderedF64) -> Self {
        Self(key.into_inner())
    }
}

#[derive(Facet, Debug)]
#[facet(transparent)]
struct FacetProxyIpAddr(String);

impl TryFrom<FacetProxyIpAddr> for std::net::IpAddr {
    type Error = &'static str;

    fn try_from(proxy: FacetProxyIpAddr) -> Result<Self, Self::Error> {
        proxy.0.parse().map_err(|_| "Invalid IP address")
    }
}

impl From<&std::net::IpAddr> for FacetProxyIpAddr {
    fn from(key: &std::net::IpAddr) -> Self {
        Self(key.to_string())
    }
}

#[allow(clippy::derived_hash_with_manual_eq)]
#[derive(Facet, Debug, Clone, Hash)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Value {
    /// Bottom/never value: no value exists or is produced. Unlike [`Self::Null`], which is an
    /// explicit data null, this is the value-level counterpart of `TypeKind::Never`.
    ///
    /// Both `Void` and `Null` are nullish. Even under bottom semantics, `Void == Void` holds at
    /// the representation level: `Value` implements `Eq`, `Ord`, and `Hash`, and `Void` is used
    /// as a concrete marker value (for example, in RPC command payloads).
    Void,
    Null,

    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(i128),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    F32(#[facet(opaque, proxy = FacetProxyF32)] OrderedF32),
    F64(#[facet(opaque, proxy = FacetProxyF64)] OrderedF64),

    Uuid(Uuid),
    IpAddr(#[facet(opaque, proxy = FacetProxyIpAddr)] std::net::IpAddr),

    Duration(Duration),
    Time(Time),
    Date(Date),
    DateTime(DateTime),

    Bytes(bytes::Bytes),
    String(String),
    List(Vec<Value>),

    Map(Map),
    Object(Object),
    Variant(Box<VariantValue>),
}

// Generic methods.
impl Value {
    fn variant_rank(&self) -> u8 {
        match self {
            Self::Void => 0,
            Self::Null => 1,
            Self::Bool(_) => 2,
            Self::I8(_) => 3,
            Self::I16(_) => 4,
            Self::I32(_) => 5,
            Self::I64(_) => 6,
            Self::I128(_) => 7,
            Self::U8(_) => 8,
            Self::U16(_) => 9,
            Self::U32(_) => 10,
            Self::U64(_) => 11,
            Self::U128(_) => 12,
            Self::F32(_) => 13,
            Self::F64(_) => 14,
            Self::Uuid(_) => 15,
            Self::IpAddr(_) => 16,
            Self::Duration(_) => 17,
            Self::Time(_) => 18,
            Self::Date(_) => 19,
            Self::DateTime(_) => 20,
            Self::Bytes(_) => 21,
            Self::String(_) => 22,
            Self::List(_) => 23,
            Self::Map(_) => 24,
            Self::Object(_) => 25,
            Self::Variant(_) => 26,
        }
    }

    pub fn as_value_ref<'a>(&'a self) -> ValueRef<'a> {
        match self {
            Self::Void => ValueRef::Void,
            Self::Null => ValueRef::Null,

            Self::Bool(b) => ValueRef::Bool(*b),
            Self::I8(i) => ValueRef::I8(*i),
            Self::I16(i) => ValueRef::I16(*i),
            Self::I32(i) => ValueRef::I32(*i),
            Self::I64(i) => ValueRef::I64(*i),
            Self::I128(i) => ValueRef::I128(*i),
            Self::U8(u) => ValueRef::U8(*u),
            Self::U16(u) => ValueRef::U16(*u),
            Self::U32(u) => ValueRef::U32(*u),
            Self::U64(u) => ValueRef::U64(*u),
            Self::U128(u) => ValueRef::U128(*u),
            Self::F32(f) => ValueRef::F32(*f),
            Self::F64(f) => ValueRef::F64(*f),

            Self::Uuid(uuid) => ValueRef::Uuid(*uuid),
            Self::IpAddr(ip_addr) => ValueRef::IpAddr(*ip_addr),
            Self::Duration(duration) => ValueRef::Duration(*duration),
            Self::Time(time) => ValueRef::Time(*time),
            Self::Date(date) => ValueRef::Date(*date),
            Self::DateTime(datetime) => ValueRef::DateTime(*datetime),

            Self::Bytes(bytes) => ValueRef::Bytes(bytes.as_ref()),
            Self::String(string) => ValueRef::String(string.as_str()),
            Self::List(list) => ValueRef::List(list.as_slice()),

            Self::Map(map) => ValueRef::Map(map),
            Self::Object(object) => ValueRef::Object(object),
            Self::Variant(variant) => ValueRef::Variant(super::variant::VariantValueRef {
                r#type: variant.r#type.as_deref(),
                variant: variant.variant.as_str(),
                value: &variant.value,
            }),
        }
    }

    pub fn get_field(&self, field: &str) -> Option<&Value> {
        match self {
            Self::Object(object) => object.get(field),
            _ => None,
        }
    }

    pub fn get_index(&self, index: usize) -> Option<&Value> {
        match self {
            Self::List(values) => values.get(index),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I8(value) => Some((*value).into()),
            Self::I16(value) => Some((*value).into()),
            Self::I32(value) => Some((*value).into()),
            Self::I64(value) => Some(*value),
            Self::U8(value) => Some((*value).into()),
            Self::U16(value) => Some((*value).into()),
            Self::U32(value) => Some((*value).into()),
            Self::U64(value) => i64::try_from(*value).ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::F32(value) => Some(value.into_inner().into()),
            Self::F64(value) => Some(value.into_inner()),
            Self::I8(value) => Some((*value).into()),
            Self::I16(value) => Some((*value).into()),
            Self::I32(value) => Some((*value).into()),
            Self::I64(value) => Some(*value as f64),
            Self::U8(value) => Some((*value).into()),
            Self::U16(value) => Some((*value).into()),
            Self::U32(value) => Some((*value).into()),
            Self::U64(value) => Some(*value as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            _ => None,
        }
    }

    pub fn is_nullish(&self) -> bool {
        matches!(self, Self::Null | Self::Void)
    }

    pub fn write_canonical<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        super::canonical::write_canonical_value(writer, self)
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        super::canonical::canonical_value_bytes(self)
    }
}

// INT methods.
impl Value {}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Value {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Void, Self::Void) => Ordering::Equal,
            (Self::Null, Self::Null) => Ordering::Equal,
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::I8(a), Self::I8(b)) => a.cmp(b),
            (Self::I16(a), Self::I16(b)) => a.cmp(b),
            (Self::I32(a), Self::I32(b)) => a.cmp(b),
            (Self::I64(a), Self::I64(b)) => a.cmp(b),
            (Self::I128(a), Self::I128(b)) => a.cmp(b),
            (Self::U8(a), Self::U8(b)) => a.cmp(b),
            (Self::U16(a), Self::U16(b)) => a.cmp(b),
            (Self::U32(a), Self::U32(b)) => a.cmp(b),
            (Self::U64(a), Self::U64(b)) => a.cmp(b),
            (Self::U128(a), Self::U128(b)) => a.cmp(b),
            (Self::F32(a), Self::F32(b)) => a.cmp(b),
            (Self::F64(a), Self::F64(b)) => a.cmp(b),
            (Self::Uuid(a), Self::Uuid(b)) => a.cmp(b),
            (Self::IpAddr(a), Self::IpAddr(b)) => a.cmp(b),
            (Self::Duration(a), Self::Duration(b)) => a.cmp(b),
            (Self::Time(a), Self::Time(b)) => a.cmp(b),
            (Self::Date(a), Self::Date(b)) => a.cmp(b),
            (Self::DateTime(a), Self::DateTime(b)) => a.cmp(b),
            (Self::Bytes(a), Self::Bytes(b)) => a.cmp(b),
            (Self::String(a), Self::String(b)) => a.cmp(b),
            (Self::List(a), Self::List(b)) => a.cmp(b),
            (Self::Map(a), Self::Map(b)) => a.cmp(b),
            (Self::Object(a), Self::Object(b)) => a.cmp(b),
            (Self::Variant(a), Self::Variant(b)) => a.cmp(b),
            _ => self.variant_rank().cmp(&other.variant_rank()),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Value {}

macro_rules! impl_primitive {
    ($($ty:ty => $variant:ident ,)*) => {
        $(
            impl From<$ty> for Value {
                fn from(value: $ty) -> Self {
                    Self::$variant(value)
                }
            }
        )*
    };
}

impl_primitive!(
    bool => Bool,
    i8 => I8,
    i16 => I16,
    i32 => I32,
    i64 => I64,
    i128 => I128,
    u8 => U8,
    u16 => U16,
    u32 => U32,
    u64 => U64,
    u128 => U128,

    OrderedF32 => F32,
    OrderedF64 => F64,

    Uuid => Uuid,
    std::net::IpAddr => IpAddr,
    Duration => Duration,
    Time => Time,
    Date => Date,
    DateTime => DateTime,
    bytes::Bytes => Bytes,
    String => String,
    Map => Map,
    Object => Object,
);

impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Self::F32(OrderedF32::from(value))
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::F64(OrderedF64::from(value))
    }
}
