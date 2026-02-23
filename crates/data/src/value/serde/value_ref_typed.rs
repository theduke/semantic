use ::serde::de::{self, EnumAccess, SeqAccess, VariantAccess, Visitor};
use ::serde::ser::{SerializeMap, SerializeSeq, SerializeTuple};
use ::serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use crate::value::serde::typed::{TypedRef, TypedValue};
use crate::value::{Map, Object, ValueRef};

#[derive(Debug, Clone)]
pub struct TypedValueRef<'a>(pub ValueRef<'a>);

impl Serialize for TypedValueRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for TypedValueRef<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize(deserializer).map(Self)
    }
}

pub fn serialize<S>(value: &ValueRef<'_>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        ValueRef::Owned(v) => super::typed::serialize(v, serializer),
        ValueRef::Ref(v) => super::typed::serialize(v, serializer),
        ValueRef::Void => serializer.serialize_unit_variant("Value", 0, "void"),
        ValueRef::Null => serializer.serialize_unit_variant("Value", 1, "null"),
        ValueRef::Bool(v) => serializer.serialize_newtype_variant("Value", 2, "bool", v),
        ValueRef::I8(v) => serializer.serialize_newtype_variant("Value", 3, "i8", v),
        ValueRef::I16(v) => serializer.serialize_newtype_variant("Value", 4, "i16", v),
        ValueRef::I32(v) => serializer.serialize_newtype_variant("Value", 5, "i32", v),
        ValueRef::I64(v) => serializer.serialize_newtype_variant("Value", 6, "i64", v),
        ValueRef::I128(v) => serializer.serialize_newtype_variant("Value", 7, "i128", v),
        ValueRef::U8(v) => serializer.serialize_newtype_variant("Value", 8, "u8", v),
        ValueRef::U16(v) => serializer.serialize_newtype_variant("Value", 9, "u16", v),
        ValueRef::U32(v) => serializer.serialize_newtype_variant("Value", 10, "u32", v),
        ValueRef::U64(v) => serializer.serialize_newtype_variant("Value", 11, "u64", v),
        ValueRef::U128(v) => serializer.serialize_newtype_variant("Value", 12, "u128", v),
        ValueRef::F32(v) => {
            serializer.serialize_newtype_variant("Value", 13, "f32", &v.into_inner())
        }
        ValueRef::F64(v) => {
            serializer.serialize_newtype_variant("Value", 14, "f64", &v.into_inner())
        }
        ValueRef::Uuid(v) => {
            let raw: uuid::Uuid = (*v).into();
            serializer.serialize_newtype_variant("Value", 15, "uuid", &raw.to_string())
        }
        ValueRef::IpAddr(v) => {
            serializer.serialize_newtype_variant("Value", 16, "ip_addr", &v.to_string())
        }
        ValueRef::Duration(v) => {
            let raw: time::Duration = (*v).into();
            serializer.serialize_newtype_variant("Value", 17, "duration", &raw.whole_seconds())
        }
        ValueRef::Time(v) => {
            let raw: time::Time = (*v).into();
            let nanos = (raw - time::Time::MIDNIGHT).whole_nanoseconds();
            serializer.serialize_newtype_variant("Value", 18, "time", &(nanos as i64))
        }
        ValueRef::Date(v) => {
            let raw: time::Date = (*v).into();
            serializer.serialize_newtype_variant("Value", 19, "date", &raw.to_julian_day())
        }
        ValueRef::DateTime(v) => {
            let raw: time::OffsetDateTime = (*v).into();
            serializer.serialize_newtype_variant(
                "Value",
                20,
                "date_time",
                &raw.unix_timestamp_nanos(),
            )
        }
        ValueRef::Bytes(v) => serializer.serialize_newtype_variant("Value", 21, "bytes", v),
        ValueRef::String(v) => serializer.serialize_newtype_variant("Value", 22, "string", v),
        ValueRef::List(values) => {
            serializer.serialize_newtype_variant("Value", 23, "list", &TypedListRef(values))
        }
        ValueRef::Map(entries) => {
            serializer.serialize_newtype_variant("Value", 24, "map", &TypedMapRef(entries))
        }
        ValueRef::Object(fields) => {
            serializer.serialize_newtype_variant("Value", 25, "object", &TypedObjectRef(fields))
        }
        ValueRef::Variant(variant) => serializer.serialize_newtype_variant(
            "Value",
            26,
            "variant",
            &TypedRawVariantRef(variant),
        ),
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<ValueRef<'de>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_enum("Value", VARIANTS, TypedValueRefVisitor)
}

const VARIANTS: &[&str] = &[
    "void",
    "null",
    "bool",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "f32",
    "f64",
    "uuid",
    "ip_addr",
    "duration",
    "time",
    "date",
    "date_time",
    "bytes",
    "string",
    "list",
    "map",
    "object",
    "variant",
];

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum VariantTag {
    Void,
    Null,
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    Uuid,
    IpAddr,
    Duration,
    Time,
    Date,
    DateTime,
    Bytes,
    String,
    List,
    Map,
    Object,
    Variant,
}

struct TypedValueRefVisitor;

impl<'de> Visitor<'de> for TypedValueRefVisitor {
    type Value = ValueRef<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an externally tagged semantic value reference")
    }

    fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        let (tag, variant) = data.variant::<VariantTag>()?;
        match tag {
            VariantTag::Void => {
                variant.unit_variant()?;
                Ok(ValueRef::Void)
            }
            VariantTag::Null => {
                variant.unit_variant()?;
                Ok(ValueRef::Null)
            }
            VariantTag::Bool => Ok(ValueRef::Bool(variant.newtype_variant()?)),
            VariantTag::I8 => Ok(ValueRef::I8(variant.newtype_variant()?)),
            VariantTag::I16 => Ok(ValueRef::I16(variant.newtype_variant()?)),
            VariantTag::I32 => Ok(ValueRef::I32(variant.newtype_variant()?)),
            VariantTag::I64 => Ok(ValueRef::I64(variant.newtype_variant()?)),
            VariantTag::I128 => Ok(ValueRef::I128(variant.newtype_variant()?)),
            VariantTag::U8 => Ok(ValueRef::U8(variant.newtype_variant()?)),
            VariantTag::U16 => Ok(ValueRef::U16(variant.newtype_variant()?)),
            VariantTag::U32 => Ok(ValueRef::U32(variant.newtype_variant()?)),
            VariantTag::U64 => Ok(ValueRef::U64(variant.newtype_variant()?)),
            VariantTag::U128 => Ok(ValueRef::U128(variant.newtype_variant()?)),
            VariantTag::F32 => Ok(ValueRef::F32(variant.newtype_variant::<f32>()?.into())),
            VariantTag::F64 => Ok(ValueRef::F64(variant.newtype_variant::<f64>()?.into())),
            VariantTag::Uuid => {
                let raw = variant.newtype_variant::<Cow<'de, str>>()?;
                let parsed = uuid::Uuid::parse_str(raw.as_ref()).map_err(|err| {
                    de::Error::custom(format!("invalid uuid value '{raw}': {err}"))
                })?;
                Ok(ValueRef::Uuid(parsed.into()))
            }
            VariantTag::IpAddr => {
                let raw = variant.newtype_variant::<Cow<'de, str>>()?;
                let parsed = raw.parse::<std::net::IpAddr>().map_err(|err| {
                    de::Error::custom(format!("invalid ip_addr value '{raw}': {err}"))
                })?;
                Ok(ValueRef::IpAddr(parsed))
            }
            VariantTag::Duration => {
                let seconds = variant.newtype_variant::<i64>()?;
                Ok(ValueRef::Duration(time::Duration::seconds(seconds).into()))
            }
            VariantTag::Time => {
                let nanos = variant.newtype_variant::<i64>()?;
                let raw = time::Time::MIDNIGHT + time::Duration::nanoseconds(nanos);
                Ok(ValueRef::Time(raw.into()))
            }
            VariantTag::Date => {
                let julian_day = variant.newtype_variant::<i32>()?;
                let raw = time::Date::from_julian_day(julian_day).map_err(|err| {
                    de::Error::custom(format!("invalid date julian day {julian_day}: {err}"))
                })?;
                Ok(ValueRef::Date(raw.into()))
            }
            VariantTag::DateTime => {
                let unix_nanos = variant.newtype_variant::<i128>()?;
                let raw =
                    time::OffsetDateTime::from_unix_timestamp_nanos(unix_nanos).map_err(|err| {
                        de::Error::custom(format!(
                            "invalid datetime unix nanos {unix_nanos}: {err}"
                        ))
                    })?;
                Ok(ValueRef::DateTime(raw.into()))
            }
            VariantTag::Bytes => {
                let bytes = variant.newtype_variant::<Cow<'de, [u8]>>()?;
                match bytes {
                    Cow::Borrowed(raw) => Ok(ValueRef::Bytes(raw)),
                    Cow::Owned(raw) => Ok(ValueRef::Owned(crate::value::Value::Bytes(raw.into()))),
                }
            }
            VariantTag::String => {
                let value = variant.newtype_variant::<Cow<'de, str>>()?;
                match value {
                    Cow::Borrowed(raw) => Ok(ValueRef::String(raw)),
                    Cow::Owned(raw) => Ok(ValueRef::Owned(crate::value::Value::String(raw))),
                }
            }
            VariantTag::List => {
                let values = variant.newtype_variant::<TypedListOwned>()?;
                Ok(ValueRef::Owned(crate::value::Value::List(values.0)))
            }
            VariantTag::Map => {
                let entries = variant.newtype_variant::<TypedMapOwned>()?;
                Ok(ValueRef::Owned(crate::value::Value::Map(entries.0)))
            }
            VariantTag::Object => {
                let fields = variant.newtype_variant::<TypedObjectOwned>()?;
                Ok(ValueRef::Owned(crate::value::Value::Object(fields.0)))
            }
            VariantTag::Variant => {
                let typed_variant = variant.newtype_variant::<TypedVariantOwned>()?;
                Ok(ValueRef::Owned(crate::value::Value::Variant(Box::new(
                    typed_variant.0,
                ))))
            }
        }
    }
}

struct TypedRawVariantRef<'a, 'b>(&'a crate::value::variant::VariantValueRef<'b>);

impl Serialize for TypedRawVariantRef<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2 + usize::from(self.0.r#type.is_some())))?;
        if let Some(type_name) = self.0.r#type {
            map.serialize_entry("type", type_name)?;
        }
        map.serialize_entry("variant", self.0.variant)?;
        map.serialize_entry("value", &TypedRef(self.0.value))?;
        map.end()
    }
}

struct TypedListOwned(Vec<crate::value::Value>);

impl<'de> Deserialize<'de> for TypedListOwned {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ListVisitor;

        impl<'de> Visitor<'de> for ListVisitor {
            type Value = TypedListOwned;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a list of typed values")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<TypedValueRef<'de>>()? {
                    values.push(value.0.into_owned());
                }
                Ok(TypedListOwned(values))
            }
        }

        deserializer.deserialize_seq(ListVisitor)
    }
}

struct TypedMapOwned(Map);

impl<'de> Deserialize<'de> for TypedMapOwned {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MapVisitor;

        impl<'de> Visitor<'de> for MapVisitor {
            type Value = TypedMapOwned;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a list of typed key/value pairs")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut map = Map::new();
                while let Some((key, value)) =
                    seq.next_element::<(TypedValueRef<'de>, TypedValueRef<'de>)>()?
                {
                    map.insert(key.0.into_owned(), value.0.into_owned());
                }
                Ok(TypedMapOwned(map))
            }
        }

        deserializer.deserialize_seq(MapVisitor)
    }
}

struct TypedObjectOwned(Object);

impl<'de> Deserialize<'de> for TypedObjectOwned {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = BTreeMap::<String, TypedValueRef<'de>>::deserialize(deserializer)?;
        let mut object = Object::new();
        for (field, value) in raw {
            object.insert(field, value.0.into_owned());
        }
        Ok(TypedObjectOwned(object))
    }
}

struct TypedVariantOwned(crate::value::VariantValue);

impl<'de> Deserialize<'de> for TypedVariantOwned {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawTypedVariant {
            #[serde(default)]
            r#type: Option<String>,
            variant: String,
            value: TypedValue,
        }

        let raw = RawTypedVariant::deserialize(deserializer)?;
        Ok(TypedVariantOwned(crate::value::VariantValue {
            r#type: raw.r#type,
            variant: raw.variant,
            value: raw.value.0,
        }))
    }
}

struct TypedListRef<'a>(&'a [crate::value::Value]);

impl Serialize for TypedListRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&TypedRef(value))?;
        }
        seq.end()
    }
}

struct TypedMapRef<'a>(&'a Map);

impl Serialize for TypedMapRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for (key, value) in self.0.iter() {
            seq.serialize_element(&TypedPairRef(key, value))?;
        }
        seq.end()
    }
}

struct TypedObjectRef<'a>(&'a Object);

impl Serialize for TypedObjectRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (field, value) in self.0.iter() {
            map.serialize_entry(field, &TypedRef(value))?;
        }
        map.end()
    }
}

struct TypedPairRef<'a>(&'a crate::value::Value, &'a crate::value::Value);

impl Serialize for TypedPairRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&TypedRef(self.0))?;
        tuple.serialize_element(&TypedRef(self.1))?;
        tuple.end()
    }
}

#[cfg(test)]
mod tests {
    use super::TypedValueRef;
    use crate::value::{Map, Value, ValueRef, VariantValue};

    #[test]
    fn typed_value_ref_bool_json_shape() {
        let value = TypedValueRef(ValueRef::Bool(true));
        let encoded = ::serde_json::to_string(&value).expect("serialize typed bool");
        assert_eq!(encoded, r#"{"bool":true}"#);
    }

    #[test]
    fn typed_value_ref_roundtrip_preserves_variant() {
        let value = TypedValueRef(ValueRef::I8(12));
        let encoded = ::serde_json::to_string(&value).expect("serialize typed");
        let decoded: TypedValueRef<'_> =
            ::serde_json::from_str(&encoded).expect("deserialize typed");
        assert_eq!(decoded.0.into_owned(), Value::I8(12));
    }

    #[test]
    fn typed_value_ref_map_roundtrip_allows_non_string_keys() {
        let mut map = Map::new();
        map.insert(Value::Bool(true), Value::String("yes".to_owned()));
        let value = TypedValueRef(ValueRef::Owned(Value::Map(map.clone())));

        let encoded = ::serde_json::to_string(&value).expect("serialize typed");
        let decoded: TypedValueRef<'_> =
            ::serde_json::from_str(&encoded).expect("deserialize typed");
        assert_eq!(decoded.0.into_owned(), Value::Map(map));
    }

    #[test]
    fn typed_value_ref_deserialize_string_variant() {
        let decoded: TypedValueRef<'_> =
            ::serde_json::from_str(r#"{"string":"hello"}"#).expect("deserialize typed string");
        match decoded.0 {
            ValueRef::String(value) => assert_eq!(value, "hello"),
            ValueRef::Owned(Value::String(value)) => assert_eq!(value, "hello"),
            other => panic!("expected string value, got {other:?}"),
        }
    }

    #[test]
    fn typed_value_ref_variant_roundtrip_json() {
        let value = Value::Variant(Box::new(VariantValue {
            r#type: Some("example::Result".to_owned()),
            variant: "ok".to_owned(),
            value: Value::I64(7),
        }));

        let encoded = ::serde_json::to_string(&TypedValueRef(ValueRef::Ref(&value)))
            .expect("serialize typed variant ref");
        let decoded: TypedValueRef<'_> =
            ::serde_json::from_str(&encoded).expect("deserialize typed variant ref");
        assert_eq!(decoded.0.into_owned(), value);
    }
}
