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
            (Self::Void, _) => Ordering::Less,

            (Self::Null, Self::Null) => Ordering::Equal,
            (Self::Null, _) => Ordering::Less,

            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Bool(_), _) => Ordering::Less,

            (Self::I8(a), Self::I8(b)) => a.cmp(b),
            (Self::I8(_), _) => Ordering::Less,

            (Self::I16(a), Self::I16(b)) => a.cmp(b),
            (Self::I16(_), _) => Ordering::Less,

            (Self::I32(a), Self::I32(b)) => a.cmp(b),
            (Self::I32(_), _) => Ordering::Less,

            (Self::I64(a), Self::I64(b)) => a.cmp(b),
            (Self::I64(_), _) => Ordering::Less,

            (Self::I128(a), Self::I128(b)) => a.cmp(b),
            (Self::I128(_), _) => Ordering::Less,

            (Self::U8(a), Self::U8(b)) => a.cmp(b),
            (Self::U8(_), _) => Ordering::Less,

            (Self::U16(a), Self::U16(b)) => a.cmp(b),
            (Self::U16(_), _) => Ordering::Less,

            (Self::U32(a), Self::U32(b)) => a.cmp(b),
            (Self::U32(_), _) => Ordering::Less,

            (Self::U64(a), Self::U64(b)) => a.cmp(b),
            (Self::U64(_), _) => Ordering::Less,

            (Self::U128(a), Self::U128(b)) => a.cmp(b),
            (Self::U128(_), _) => Ordering::Less,

            (Self::F32(a), Self::F32(b)) => a.cmp(b),
            (Self::F32(_), _) => Ordering::Less,

            (Self::F64(a), Self::F64(b)) => a.cmp(b),
            (Self::F64(_), _) => Ordering::Less,

            (Self::Uuid(a), Self::Uuid(b)) => a.cmp(b),
            (Self::Uuid(_), _) => Ordering::Less,

            (Self::IpAddr(a), Self::IpAddr(b)) => a.cmp(b),
            (Self::IpAddr(_), _) => Ordering::Less,
            (Self::Duration(a), Self::Duration(b)) => a.cmp(b),
            (Self::Duration(_), _) => Ordering::Less,

            (Self::Time(a), Self::Time(b)) => a.cmp(b),
            (Self::Time(_), _) => Ordering::Less,

            (Self::Date(a), Self::Date(b)) => a.cmp(b),
            (Self::Date(_), _) => Ordering::Less,

            (Self::DateTime(a), Self::DateTime(b)) => a.cmp(b),
            (Self::DateTime(_), _) => Ordering::Less,

            (Self::Bytes(a), Self::Bytes(b)) => a.cmp(b),
            (Self::Bytes(_), _) => Ordering::Less,

            (Self::String(a), Self::String(b)) => a.cmp(b),
            (Self::String(_), _) => Ordering::Less,

            (Self::List(a), Self::List(b)) => a.cmp(b),
            (Self::List(_), _) => Ordering::Less,

            (Self::Map(a), Self::Map(b)) => a.cmp(b),
            (Self::Map(_), _) => Ordering::Less,

            (Self::Object(a), Self::Object(b)) => a.cmp(b),
            (Self::Object(_), _) => Ordering::Less,

            (Self::Variant(a), Self::Variant(b)) => a.cmp(b),
            (Self::Variant(_), _) => Ordering::Less,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,

            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::I8(a), Self::I8(b)) => a == b,
            (Self::I16(a), Self::I16(b)) => a == b,
            (Self::I32(a), Self::I32(b)) => a == b,
            (Self::I64(a), Self::I64(b)) => a == b,
            (Self::I128(a), Self::I128(b)) => a == b,
            (Self::U8(a), Self::U8(b)) => a == b,
            (Self::U16(a), Self::U16(b)) => a == b,
            (Self::U32(a), Self::U32(b)) => a == b,
            (Self::U64(a), Self::U64(b)) => a == b,
            (Self::U128(a), Self::U128(b)) => a == b,
            (Self::F32(a), Self::F32(b)) => a == b,
            (Self::F64(a), Self::F64(b)) => a == b,

            (Self::Uuid(a), Self::Uuid(b)) => a == b,
            // (Self::IpAddr(a), Self::IpAddr(b)) => a == b,
            (Self::Duration(a), Self::Duration(b)) => a == b,
            (Self::Time(a), Self::Time(b)) => a == b,
            (Self::Date(a), Self::Date(b)) => a == b,
            (Self::DateTime(a), Self::DateTime(b)) => a == b,

            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,

            (Self::List(a), Self::List(b)) => a == b,

            (Self::Map(a), Self::Map(b)) => a == b,

            (Self::Object(a), Self::Object(b)) => a == b,
            (Self::Variant(a), Self::Variant(b)) => a == b,

            _ => false,
        }
    }
}

impl Eq for Value {}
