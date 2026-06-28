use ::serde::de::{self, MapAccess, SeqAccess, Visitor};
use ::serde::ser::{SerializeMap, SerializeSeq};
use ::serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use crate::value::{Map, Object, Value, ValueRef, VariantValue};

#[derive(Debug, Clone)]
pub struct FlatValueRef<'a>(pub ValueRef<'a>);

impl Serialize for FlatValueRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for FlatValueRef<'de> {
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
        ValueRef::Owned(v) => super::flat::serialize(v, serializer),
        ValueRef::Ref(v) => super::flat::serialize(v, serializer),
        ValueRef::Void | ValueRef::Null => serializer.serialize_unit(),
        ValueRef::Bool(v) => serializer.serialize_bool(*v),
        ValueRef::I8(v) => serializer.serialize_i8(*v),
        ValueRef::I16(v) => serializer.serialize_i16(*v),
        ValueRef::I32(v) => serializer.serialize_i32(*v),
        ValueRef::I64(v) => serializer.serialize_i64(*v),
        ValueRef::I128(v) => serializer.serialize_i128(*v),
        ValueRef::U8(v) => serializer.serialize_u8(*v),
        ValueRef::U16(v) => serializer.serialize_u16(*v),
        ValueRef::U32(v) => serializer.serialize_u32(*v),
        ValueRef::U64(v) => serializer.serialize_u64(*v),
        ValueRef::U128(v) => serializer.serialize_u128(*v),
        ValueRef::F32(v) => serializer.serialize_f32(v.into_inner()),
        ValueRef::F64(v) => serializer.serialize_f64(v.into_inner()),
        ValueRef::Uuid(v) => {
            let raw: uuid::Uuid = (*v).into();
            serializer.serialize_str(&raw.to_string())
        }
        ValueRef::IpAddr(v) => serializer.serialize_str(&v.to_string()),
        ValueRef::Duration(v) => {
            let raw: time::Duration = (*v).into();
            serializer.serialize_i64(raw.whole_milliseconds() as i64)
        }
        ValueRef::Time(v) => {
            let raw: time::Time = (*v).into();
            serializer.serialize_str(&raw.to_string())
        }
        ValueRef::Date(v) => {
            let raw: time::Date = (*v).into();
            serializer.serialize_str(&raw.to_string())
        }
        ValueRef::DateTime(v) => {
            let raw: time::OffsetDateTime = (*v).into();
            serializer.serialize_str(&raw.to_string())
        }
        ValueRef::Bytes(v) => serializer.serialize_bytes(v),
        ValueRef::String(v) => serializer.serialize_str(v),
        ValueRef::List(values) => {
            let mut seq = serializer.serialize_seq(Some(values.len()))?;
            for entry in *values {
                seq.serialize_element(entry)?;
            }
            seq.end()
        }
        ValueRef::Map(entries) => {
            let mut map = serializer.serialize_map(Some(entries.len()))?;
            for (key, value) in entries.iter() {
                map.serialize_entry(key, value)?;
            }
            map.end()
        }
        ValueRef::Object(fields) => {
            let mut map = serializer.serialize_map(Some(fields.len()))?;
            for (field, value) in fields.iter() {
                map.serialize_entry(field, value)?;
            }
            map.end()
        }
        ValueRef::Variant(variant) => {
            let mut map =
                serializer.serialize_map(Some(2 + usize::from(variant.r#type.is_some())))?;
            if let Some(type_name) = variant.r#type {
                map.serialize_entry("type", type_name)?;
            }
            map.serialize_entry("variant", variant.variant)?;
            map.serialize_entry("value", variant.value)?;
            map.end()
        }
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<ValueRef<'de>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(ValueRefVisitor)
}

struct ValueRefVisitor;

impl<'de> Visitor<'de> for ValueRefVisitor {
    type Value = ValueRef<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a semantic value reference")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(ValueRef::Bool(value))
    }

    fn visit_i8<E>(self, value: i8) -> Result<Self::Value, E> {
        Ok(ValueRef::I8(value))
    }

    fn visit_i16<E>(self, value: i16) -> Result<Self::Value, E> {
        Ok(ValueRef::I16(value))
    }

    fn visit_i32<E>(self, value: i32) -> Result<Self::Value, E> {
        Ok(ValueRef::I32(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(ValueRef::I64(value))
    }

    fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E> {
        Ok(ValueRef::I128(value))
    }

    fn visit_u8<E>(self, value: u8) -> Result<Self::Value, E> {
        Ok(ValueRef::U8(value))
    }

    fn visit_u16<E>(self, value: u16) -> Result<Self::Value, E> {
        Ok(ValueRef::U16(value))
    }

    fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E> {
        Ok(ValueRef::U32(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(ValueRef::U64(value))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E> {
        Ok(ValueRef::U128(value))
    }

    fn visit_f32<E>(self, value: f32) -> Result<Self::Value, E> {
        Ok(ValueRef::F32(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(ValueRef::F64(value.into()))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(ValueRef::String(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(ValueRef::Owned(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(ValueRef::Owned(Value::String(value)))
    }

    fn visit_borrowed_bytes<E>(self, value: &'de [u8]) -> Result<Self::Value, E> {
        Ok(ValueRef::Bytes(value))
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E> {
        Ok(ValueRef::Owned(Value::Bytes(
            bytes::Bytes::copy_from_slice(value),
        )))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E> {
        Ok(ValueRef::Owned(Value::Bytes(value.into())))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(ValueRef::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize(deserializer)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(ValueRef::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element::<FlatValueRef<'de>>()? {
            values.push(value.0.into_owned());
        }
        Ok(ValueRef::Owned(Value::List(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        while let Some((key, value)) = map.next_entry::<FlatValueRef<'de>, FlatValueRef<'de>>()? {
            entries.push((key.0, value.0));
        }

        if entries
            .iter()
            .all(|(key, _)| matches!(key, ValueRef::String(_) | ValueRef::Owned(Value::String(_))))
        {
            let mut object = Object::new();
            for (key, value) in entries {
                match key {
                    ValueRef::String(field) => {
                        object.insert(field, value.into_owned());
                    }
                    ValueRef::Owned(Value::String(field)) => {
                        object.insert(field, value.into_owned());
                    }
                    _ => {}
                }
            }
            if let Some(variant) = decode_variant_object(&object)? {
                Ok(ValueRef::Owned(variant))
            } else {
                Ok(ValueRef::Owned(Value::Object(object)))
            }
        } else {
            let mut out = Map::new();
            for (key, value) in entries {
                out.insert(key.into_owned(), value.into_owned());
            }
            Ok(ValueRef::Owned(Value::Map(out)))
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
    use super::FlatValueRef;
    use crate::value::{Value, ValueRef, VariantValue};

    #[test]
    fn flat_value_ref_bool_roundtrip_json() {
        let value = FlatValueRef(ValueRef::Bool(true));
        let encoded = ::serde_json::to_string(&value).expect("serialize bool");
        assert_eq!(encoded, "true");

        let decoded: FlatValueRef<'_> = ::serde_json::from_str(&encoded).expect("deserialize bool");
        assert_eq!(decoded.0.into_owned(), Value::Bool(true));
    }

    #[test]
    fn flat_value_ref_ref_serializes_like_value() {
        let value = Value::I64(42);
        let encoded =
            ::serde_json::to_string(&FlatValueRef(ValueRef::Ref(&value))).expect("serialize ref");
        assert_eq!(encoded, "42");
    }

    #[test]
    fn flat_value_ref_duration_serializes_as_milliseconds() {
        let duration = crate::value::Duration::from(time::Duration::milliseconds(1500));
        let encoded = ::serde_json::to_string(&FlatValueRef(ValueRef::Duration(duration)))
            .expect("serialize duration");
        assert_eq!(encoded, "1500");
    }

    #[test]
    fn flat_value_ref_deserialize_borrowed_str() {
        let decoded: FlatValueRef<'_> =
            ::serde_json::from_str(r#""hello""#).expect("deserialize str");
        match decoded.0 {
            ValueRef::String(value) => assert_eq!(value, "hello"),
            other => panic!("expected borrowed string, got {other:?}"),
        }
    }

    #[test]
    fn flat_value_ref_variant_roundtrip_json() {
        let value = Value::Variant(Box::new(VariantValue {
            r#type: Some("example::Result".to_owned()),
            variant: "ok".to_owned(),
            value: Value::Bool(true),
        }));

        let encoded = ::serde_json::to_string(&FlatValueRef(ValueRef::Ref(&value)))
            .expect("serialize variant ref");
        let decoded: FlatValueRef<'_> =
            ::serde_json::from_str(&encoded).expect("deserialize variant ref");
        assert_eq!(decoded.0.into_owned(), value);
    }
}
