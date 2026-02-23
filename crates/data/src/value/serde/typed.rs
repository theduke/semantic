use ::serde::de::{self, EnumAccess, SeqAccess, VariantAccess, Visitor};
use ::serde::ser::{SerializeMap, SerializeSeq, SerializeTuple};
use ::serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;

use crate::value::{Map, Object, Value, VariantValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedValue(pub Value);

#[derive(Debug, Clone, Copy)]
pub struct TypedRef<'a>(pub &'a Value);

impl Serialize for TypedValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for TypedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize(deserializer).map(Self)
    }
}

impl<'a> Serialize for TypedRef<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize(self.0, serializer)
    }
}

pub fn serialize<S>(value: &Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Value::Void => serializer.serialize_unit_variant("Value", 0, "void"),
        Value::Null => serializer.serialize_unit_variant("Value", 1, "null"),
        Value::Bool(v) => serializer.serialize_newtype_variant("Value", 2, "bool", v),
        Value::I8(v) => serializer.serialize_newtype_variant("Value", 3, "i8", v),
        Value::I16(v) => serializer.serialize_newtype_variant("Value", 4, "i16", v),
        Value::I32(v) => serializer.serialize_newtype_variant("Value", 5, "i32", v),
        Value::I64(v) => serializer.serialize_newtype_variant("Value", 6, "i64", v),
        Value::I128(v) => serializer.serialize_newtype_variant("Value", 7, "i128", v),
        Value::U8(v) => serializer.serialize_newtype_variant("Value", 8, "u8", v),
        Value::U16(v) => serializer.serialize_newtype_variant("Value", 9, "u16", v),
        Value::U32(v) => serializer.serialize_newtype_variant("Value", 10, "u32", v),
        Value::U64(v) => serializer.serialize_newtype_variant("Value", 11, "u64", v),
        Value::U128(v) => serializer.serialize_newtype_variant("Value", 12, "u128", v),
        Value::F32(v) => serializer.serialize_newtype_variant("Value", 13, "f32", &v.into_inner()),
        Value::F64(v) => serializer.serialize_newtype_variant("Value", 14, "f64", &v.into_inner()),
        Value::Uuid(v) => {
            let raw: uuid::Uuid = (*v).into();
            serializer.serialize_newtype_variant("Value", 15, "uuid", &raw.to_string())
        }
        Value::IpAddr(v) => {
            serializer.serialize_newtype_variant("Value", 16, "ip_addr", &v.to_string())
        }
        Value::Duration(v) => {
            let raw: time::Duration = (*v).into();
            serializer.serialize_newtype_variant("Value", 17, "duration", &raw.whole_seconds())
        }
        Value::Time(v) => {
            let raw: time::Time = (*v).into();
            let nanos = (raw - time::Time::MIDNIGHT).whole_nanoseconds();
            serializer.serialize_newtype_variant("Value", 18, "time", &(nanos as i64))
        }
        Value::Date(v) => {
            let raw: time::Date = (*v).into();
            serializer.serialize_newtype_variant("Value", 19, "date", &raw.to_julian_day())
        }
        Value::DateTime(v) => {
            let raw: time::OffsetDateTime = (*v).into();
            serializer.serialize_newtype_variant(
                "Value",
                20,
                "date_time",
                &raw.unix_timestamp_nanos(),
            )
        }
        Value::Bytes(v) => serializer.serialize_newtype_variant("Value", 21, "bytes", &v.as_ref()),
        Value::String(v) => serializer.serialize_newtype_variant("Value", 22, "string", v),
        Value::List(values) => {
            serializer.serialize_newtype_variant("Value", 23, "list", &TypedListRef(values))
        }
        Value::Map(entries) => {
            serializer.serialize_newtype_variant("Value", 24, "map", &TypedMapRef(entries))
        }
        Value::Object(fields) => {
            serializer.serialize_newtype_variant("Value", 25, "object", &TypedObjectRef(fields))
        }
        Value::Variant(variant) => serializer.serialize_newtype_variant(
            "Value",
            26,
            "variant",
            &TypedVariantRef(variant.as_ref()),
        ),
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_enum("Value", VARIANTS, TypedValueVisitor)
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

struct TypedValueVisitor;

impl<'de> Visitor<'de> for TypedValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an externally tagged semantic value")
    }

    fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        let (tag, variant) = data.variant::<VariantTag>()?;
        match tag {
            VariantTag::Void => {
                variant.unit_variant()?;
                Ok(Value::Void)
            }
            VariantTag::Null => {
                variant.unit_variant()?;
                Ok(Value::Null)
            }
            VariantTag::Bool => Ok(Value::Bool(variant.newtype_variant()?)),
            VariantTag::I8 => Ok(Value::I8(variant.newtype_variant()?)),
            VariantTag::I16 => Ok(Value::I16(variant.newtype_variant()?)),
            VariantTag::I32 => Ok(Value::I32(variant.newtype_variant()?)),
            VariantTag::I64 => Ok(Value::I64(variant.newtype_variant()?)),
            VariantTag::I128 => Ok(Value::I128(variant.newtype_variant()?)),
            VariantTag::U8 => Ok(Value::U8(variant.newtype_variant()?)),
            VariantTag::U16 => Ok(Value::U16(variant.newtype_variant()?)),
            VariantTag::U32 => Ok(Value::U32(variant.newtype_variant()?)),
            VariantTag::U64 => Ok(Value::U64(variant.newtype_variant()?)),
            VariantTag::U128 => Ok(Value::U128(variant.newtype_variant()?)),
            VariantTag::F32 => Ok(Value::F32(variant.newtype_variant::<f32>()?.into())),
            VariantTag::F64 => Ok(Value::F64(variant.newtype_variant::<f64>()?.into())),
            VariantTag::Uuid => {
                let raw = variant.newtype_variant::<String>()?;
                let parsed = uuid::Uuid::parse_str(&raw).map_err(|err| {
                    de::Error::custom(format!("invalid uuid value '{raw}': {err}"))
                })?;
                Ok(Value::Uuid(parsed.into()))
            }
            VariantTag::IpAddr => {
                let raw = variant.newtype_variant::<String>()?;
                let parsed = raw.parse::<std::net::IpAddr>().map_err(|err| {
                    de::Error::custom(format!("invalid ip_addr value '{raw}': {err}"))
                })?;
                Ok(Value::IpAddr(parsed))
            }
            VariantTag::Duration => {
                let seconds = variant.newtype_variant::<i64>()?;
                Ok(Value::Duration(time::Duration::seconds(seconds).into()))
            }
            VariantTag::Time => {
                let nanos = variant.newtype_variant::<i64>()?;
                let raw = time::Time::MIDNIGHT + time::Duration::nanoseconds(nanos);
                Ok(Value::Time(raw.into()))
            }
            VariantTag::Date => {
                let julian_day = variant.newtype_variant::<i32>()?;
                let raw = time::Date::from_julian_day(julian_day).map_err(|err| {
                    de::Error::custom(format!("invalid date julian day {julian_day}: {err}"))
                })?;
                Ok(Value::Date(raw.into()))
            }
            VariantTag::DateTime => {
                let unix_nanos = variant.newtype_variant::<i128>()?;
                let raw =
                    time::OffsetDateTime::from_unix_timestamp_nanos(unix_nanos).map_err(|err| {
                        de::Error::custom(format!(
                            "invalid datetime unix nanos {unix_nanos}: {err}"
                        ))
                    })?;
                Ok(Value::DateTime(raw.into()))
            }
            VariantTag::Bytes => {
                let bytes = variant.newtype_variant::<Vec<u8>>()?;
                Ok(Value::Bytes(bytes.into()))
            }
            VariantTag::String => Ok(Value::String(variant.newtype_variant()?)),
            VariantTag::List => {
                let values = variant.newtype_variant::<TypedList>()?;
                Ok(Value::List(values.0))
            }
            VariantTag::Map => {
                let entries = variant.newtype_variant::<TypedMap>()?;
                Ok(Value::Map(entries.0))
            }
            VariantTag::Object => {
                let fields = variant.newtype_variant::<TypedObject>()?;
                Ok(Value::Object(fields.0))
            }
            VariantTag::Variant => {
                let variant = variant.newtype_variant::<TypedVariant>()?;
                Ok(Value::Variant(Box::new(variant.0)))
            }
        }
    }
}

struct TypedVariantRef<'a>(&'a VariantValue);

impl Serialize for TypedVariantRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2 + usize::from(self.0.r#type.is_some())))?;
        if let Some(type_name) = self.0.r#type.as_deref() {
            map.serialize_entry("type", type_name)?;
        }
        map.serialize_entry("variant", self.0.variant.as_str())?;
        map.serialize_entry("value", &TypedRef(&self.0.value))?;
        map.end()
    }
}

struct TypedListRef<'a>(&'a [Value]);

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

struct TypedPairRef<'a>(&'a Value, &'a Value);

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

struct TypedList(Vec<Value>);

impl<'de> Deserialize<'de> for TypedList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ListVisitor;

        impl<'de> Visitor<'de> for ListVisitor {
            type Value = TypedList;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a list of typed values")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<TypedValue>()? {
                    values.push(value.0);
                }
                Ok(TypedList(values))
            }
        }

        deserializer.deserialize_seq(ListVisitor)
    }
}

struct TypedMap(Map);

impl<'de> Deserialize<'de> for TypedMap {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MapVisitor;

        impl<'de> Visitor<'de> for MapVisitor {
            type Value = TypedMap;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a list of typed key/value pairs")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut map = Map::new();
                while let Some((key, value)) = seq.next_element::<(TypedValue, TypedValue)>()? {
                    map.insert(key.0, value.0);
                }
                Ok(TypedMap(map))
            }
        }

        deserializer.deserialize_seq(MapVisitor)
    }
}

struct TypedObject(Object);

impl<'de> Deserialize<'de> for TypedObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = BTreeMap::<String, TypedValue>::deserialize(deserializer)?;
        let mut object = Object::new();
        for (field, value) in raw {
            object.insert(field, value.0);
        }
        Ok(TypedObject(object))
    }
}

struct TypedVariant(VariantValue);

impl<'de> Deserialize<'de> for TypedVariant {
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
        Ok(TypedVariant(VariantValue {
            r#type: raw.r#type,
            variant: raw.variant,
            value: raw.value.0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::TypedValue;
    use crate::value::{Map, Value, VariantValue};

    #[test]
    fn typed_bool_json_shape() {
        let value = Value::Bool(true);
        let encoded = ::serde_json::to_string(&TypedValue(value)).expect("serialize typed bool");
        assert_eq!(encoded, r#"{"bool":true}"#);
    }

    #[test]
    fn typed_unit_uses_external_tagging_shape() {
        let encoded =
            ::serde_json::to_string(&TypedValue(Value::Void)).expect("serialize typed void");
        assert_eq!(encoded, r#""void""#);
    }

    #[test]
    fn typed_roundtrip_preserves_variant() {
        let value = Value::I8(12);
        let encoded = ::serde_json::to_string(&TypedValue(value.clone())).expect("serialize typed");
        let decoded: TypedValue = ::serde_json::from_str(&encoded).expect("deserialize typed");
        assert_eq!(decoded.0, value);
    }

    #[test]
    fn typed_map_roundtrip_allows_non_string_keys() {
        let mut map = Map::new();
        map.insert(Value::Bool(true), Value::String("yes".to_owned()));
        let value = Value::Map(map);

        let encoded = ::serde_json::to_string(&TypedValue(value.clone())).expect("serialize typed");
        let decoded: TypedValue = ::serde_json::from_str(&encoded).expect("deserialize typed");
        assert_eq!(decoded.0, value);
    }

    #[test]
    fn typed_variant_roundtrip_json() {
        let value = Value::Variant(Box::new(VariantValue {
            r#type: Some("example::Result".to_owned()),
            variant: "ok".to_owned(),
            value: Value::String("done".to_owned()),
        }));

        let encoded =
            ::serde_json::to_string(&TypedValue(value.clone())).expect("serialize typed variant");
        let decoded: TypedValue =
            ::serde_json::from_str(&encoded).expect("deserialize typed variant");
        assert_eq!(decoded.0, value);
    }
}
