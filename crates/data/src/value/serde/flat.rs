use ::serde::de::{self, MapAccess, SeqAccess, Visitor};
use ::serde::ser::{SerializeMap, SerializeSeq};
use ::serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use crate::value::{Map, Object, Value, VariantValue};

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize(deserializer)
    }
}

pub fn serialize<S>(value: &Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Value::Void | Value::Null => serializer.serialize_unit(),
        Value::Bool(v) => serializer.serialize_bool(*v),
        Value::I8(v) => serializer.serialize_i8(*v),
        Value::I16(v) => serializer.serialize_i16(*v),
        Value::I32(v) => serializer.serialize_i32(*v),
        Value::I64(v) => serializer.serialize_i64(*v),
        Value::I128(v) => serializer.serialize_i128(*v),
        Value::U8(v) => serializer.serialize_u8(*v),
        Value::U16(v) => serializer.serialize_u16(*v),
        Value::U32(v) => serializer.serialize_u32(*v),
        Value::U64(v) => serializer.serialize_u64(*v),
        Value::U128(v) => serializer.serialize_u128(*v),
        Value::F32(v) => serializer.serialize_f32(v.into_inner()),
        Value::F64(v) => serializer.serialize_f64(v.into_inner()),
        Value::Uuid(v) => {
            let raw: uuid::Uuid = (*v).into();
            serializer.serialize_str(&raw.to_string())
        }
        Value::IpAddr(v) => serializer.serialize_str(&v.to_string()),
        Value::Duration(v) => {
            let raw: time::Duration = (*v).into();
            serializer.serialize_i64(raw.whole_seconds())
        }
        Value::Time(v) => {
            let raw: time::Time = (*v).into();
            serializer.serialize_str(&raw.to_string())
        }
        Value::Date(v) => {
            let raw: time::Date = (*v).into();
            serializer.serialize_str(&raw.to_string())
        }
        Value::DateTime(v) => {
            let raw: time::OffsetDateTime = (*v).into();
            serializer.serialize_str(&raw.to_string())
        }
        Value::Bytes(v) => serializer.serialize_bytes(v),
        Value::String(v) => serializer.serialize_str(v),
        Value::List(values) => {
            let mut seq = serializer.serialize_seq(Some(values.len()))?;
            for entry in values {
                seq.serialize_element(entry)?;
            }
            seq.end()
        }
        Value::Map(entries) => {
            let mut map = serializer.serialize_map(Some(entries.len()))?;
            for (key, value) in entries.iter() {
                map.serialize_entry(key, value)?;
            }
            map.end()
        }
        Value::Object(fields) => {
            let mut map = serializer.serialize_map(Some(fields.len()))?;
            for (field, value) in fields.iter() {
                map.serialize_entry(field, value)?;
            }
            map.end()
        }
        Value::Variant(variant) => {
            let mut map =
                serializer.serialize_map(Some(2 + usize::from(variant.r#type.is_some())))?;
            if let Some(type_name) = variant.r#type.as_deref() {
                map.serialize_entry("type", type_name)?;
            }
            map.serialize_entry("variant", variant.variant.as_str())?;
            map.serialize_entry("value", &variant.value)?;
            map.end()
        }
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(ValueVisitor)
}

pub fn serialize_value<S>(value: &Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serialize(value, serializer)
}

pub fn deserialize_value<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize(deserializer)
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a semantic value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i8<E>(self, value: i8) -> Result<Self::Value, E> {
        Ok(Value::I8(value))
    }

    fn visit_i16<E>(self, value: i16) -> Result<Self::Value, E> {
        Ok(Value::I16(value))
    }

    fn visit_i32<E>(self, value: i32) -> Result<Self::Value, E> {
        Ok(Value::I32(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::I64(value))
    }

    fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E> {
        Ok(Value::I128(value))
    }

    fn visit_u8<E>(self, value: u8) -> Result<Self::Value, E> {
        Ok(Value::U8(value))
    }

    fn visit_u16<E>(self, value: u16) -> Result<Self::Value, E> {
        Ok(Value::U16(value))
    }

    fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E> {
        Ok(Value::U32(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::U64(value))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E> {
        Ok(Value::U128(value))
    }

    fn visit_f32<E>(self, value: f32) -> Result<Self::Value, E> {
        Ok(Value::F32(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(Value::F64(value.into()))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E> {
        Ok(Value::Bytes(bytes::Bytes::copy_from_slice(value)))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E> {
        Ok(Value::Bytes(value.into()))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        Value::deserialize(deserializer)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element::<Value>()? {
            values.push(value);
        }
        Ok(Value::List(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        while let Some((key, value)) = map.next_entry::<Value, Value>()? {
            entries.push((key, value));
        }

        if entries
            .iter()
            .all(|(key, _)| matches!(key, Value::String(_)))
        {
            let mut object = Object::new();
            for (key, value) in entries {
                if let Value::String(field) = key {
                    object.insert(field, value);
                }
            }
            if let Some(variant) = decode_variant_object(&object)? {
                Ok(variant)
            } else {
                Ok(Value::Object(object))
            }
        } else {
            let mut out = Map::new();
            for (key, value) in entries {
                out.insert(key, value);
            }
            Ok(Value::Map(out))
        }
    }
}

fn decode_variant_object<E>(object: &Object) -> Result<Option<Value>, E>
where
    E: de::Error,
{
    if !object.contains_key("variant") || !object.contains_key("value") {
        return Ok(None);
    }

    if !object
        .keys()
        .all(|field| matches!(field.as_str(), "type" | "variant" | "value"))
    {
        return Ok(None);
    }

    let variant = match object.get("variant") {
        Some(Value::String(name)) => name.clone(),
        Some(other) => {
            return Err(E::custom(format!(
                "invalid variant name field for VariantValue: {other:?}"
            )));
        }
        None => return Ok(None),
    };

    let value = match object.get("value") {
        Some(value) => value.clone(),
        None => return Ok(None),
    };

    let r#type = match object.get("type") {
        Some(Value::String(name)) => Some(name.clone()),
        Some(Value::Null) | None => None,
        Some(other) => {
            return Err(E::custom(format!(
                "invalid type field for VariantValue: {other:?}"
            )));
        }
    };

    Ok(Some(Value::Variant(Box::new(VariantValue {
        r#type,
        variant,
        value,
    }))))
}

#[cfg(test)]
mod tests {
    use crate::value::{Value, VariantValue};

    #[test]
    fn flat_bool_roundtrip_json() {
        let value = Value::Bool(true);
        let encoded = ::serde_json::to_string(&value).expect("serialize bool");
        assert_eq!(encoded, "true");

        let decoded: Value = ::serde_json::from_str(&encoded).expect("deserialize bool");
        assert_eq!(decoded, value);
    }

    #[test]
    fn flat_object_roundtrip_json() {
        let mut object = crate::value::Object::new();
        object.insert("x", Value::I64(123));
        object.insert("ok", Value::Bool(true));
        let value = Value::Object(object);

        let encoded = ::serde_json::to_string(&value).expect("serialize object");
        let decoded: Value = ::serde_json::from_str(&encoded).expect("deserialize object");
        assert_eq!(
            decoded,
            Value::Object(
                [
                    ("ok".to_owned(), Value::Bool(true)),
                    ("x".to_owned(), Value::U64(123))
                ]
                .into_iter()
                .collect()
            )
        );
    }

    #[test]
    fn flat_void_is_lossy_in_json() {
        let encoded = ::serde_json::to_string(&Value::Void).expect("serialize void");
        assert_eq!(encoded, "null");

        let decoded: Value = ::serde_json::from_str(&encoded).expect("deserialize null");
        assert_eq!(decoded, Value::Null);
    }

    #[test]
    fn flat_variant_roundtrip_json() {
        let value = Value::Variant(Box::new(VariantValue {
            r#type: Some("example::Result".to_owned()),
            variant: "ok".to_owned(),
            value: Value::Bool(true),
        }));

        let encoded = ::serde_json::to_string(&value).expect("serialize variant");
        let decoded: Value = ::serde_json::from_str(&encoded).expect("deserialize variant");
        assert_eq!(decoded, value);
    }
}
