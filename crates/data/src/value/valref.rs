use crate::value::variant::VariantValueRef;

use super::{Date, DateTime, Duration, Map, Object, OrderedF32, OrderedF64, Time, Uuid, Value};

#[derive(Clone, Debug)]
pub enum ValueRef<'a> {
    Owned(Value),
    Ref(&'a Value),

    // Copy types
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
    F32(OrderedF32),
    F64(OrderedF64),

    Uuid(Uuid),
    IpAddr(std::net::IpAddr),

    Duration(Duration),
    Time(Time),
    Date(Date),
    DateTime(DateTime),

    Bytes(&'a [u8]),
    List(&'a [Value]),
    String(&'a str),
    Map(&'a Map),
    Object(&'a Object),
    Variant(VariantValueRef<'a>),
}

impl<'a> ValueRef<'a> {
    pub fn into_owned(self) -> Value {
        match self {
            Self::Owned(value) => value,
            Self::Ref(value) => value.clone(),

            Self::Void => Value::Void,
            Self::Null => Value::Null,
            Self::Bool(b) => Value::Bool(b),
            Self::I8(i) => Value::I8(i),
            Self::I16(i) => Value::I16(i),
            Self::I32(i) => Value::I32(i),
            Self::I64(i) => Value::I64(i),
            Self::I128(i) => Value::I128(i),
            Self::U8(u) => Value::U8(u),
            Self::U16(u) => Value::U16(u),
            Self::U32(u) => Value::U32(u),
            Self::U64(u) => Value::U64(u),
            Self::U128(u) => Value::U128(u),
            Self::F32(f) => Value::F32(f),
            Self::F64(f) => Value::F64(f),

            Self::Uuid(uuid) => Value::Uuid(uuid),
            Self::IpAddr(ip_addr) => Value::IpAddr(ip_addr),

            Self::Duration(duration) => Value::Duration(duration),
            Self::Time(time) => Value::Time(time),
            Self::Date(date) => Value::Date(date),
            Self::DateTime(datetime) => Value::DateTime(datetime),

            Self::Bytes(bytes) => Value::Bytes(bytes::Bytes::copy_from_slice(bytes)),
            Self::List(list) => Value::List(list.to_vec()),
            Self::String(string) => Value::String(string.to_owned()),
            Self::Map(map) => Value::Map(map.clone()),
            Self::Object(object) => Value::Object(object.clone()),
            Self::Variant(variant) => Value::Variant(Box::new(variant.into_owned())),
        }
    }
}
