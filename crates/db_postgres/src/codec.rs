use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use semantic_data::value::{Map, Object, Value, VariantValue};
use semantic_db_core::DbError;
use serde_json::{Map as JsonMap, Value as JsonValue, json};

const FORMAT_VERSION: u64 = 1;

pub(crate) fn encode_value(value: &Value) -> JsonValue {
    json!({ "format": FORMAT_VERSION, "value": encode_inner(value) })
}

pub(crate) fn decode_value(document: &JsonValue) -> Result<Value, DbError> {
    let object = document
        .as_object()
        .ok_or_else(|| malformed("document must be an object"))?;
    if object.get("format").and_then(JsonValue::as_u64) != Some(FORMAT_VERSION) {
        return Err(malformed("unsupported or missing format version"));
    }
    decode_inner(
        object
            .get("value")
            .ok_or_else(|| malformed("missing value"))?,
    )
}

pub(crate) fn encode_object(object: &Object) -> JsonValue {
    encode_value(&Value::Object(object.clone()))
}

pub(crate) fn decode_object(document: &JsonValue) -> Result<Object, DbError> {
    match decode_value(document)? {
        Value::Object(object) => Ok(object),
        _ => Err(malformed("document does not contain an object")),
    }
}

fn tagged(tag: &str, value: impl Into<JsonValue>) -> JsonValue {
    json!({ "t": tag, "v": value.into() })
}

fn encode_inner(value: &Value) -> JsonValue {
    match value {
        Value::Void => json!({ "t": "void" }),
        Value::Null => json!({ "t": "null" }),
        Value::Bool(value) => tagged("bool", *value),
        Value::I8(value) => tagged("i8", value.to_string()),
        Value::I16(value) => tagged("i16", value.to_string()),
        Value::I32(value) => tagged("i32", value.to_string()),
        Value::I64(value) => tagged("i64", value.to_string()),
        Value::I128(value) => tagged("i128", value.to_string()),
        Value::U8(value) => tagged("u8", value.to_string()),
        Value::U16(value) => tagged("u16", value.to_string()),
        Value::U32(value) => tagged("u32", value.to_string()),
        Value::U64(value) => tagged("u64", value.to_string()),
        Value::U128(value) => tagged("u128", value.to_string()),
        Value::F32(value) => tagged("f32", format!("{:08x}", value.into_inner().to_bits())),
        Value::F64(value) => tagged("f64", format!("{:016x}", value.into_inner().to_bits())),
        Value::Uuid(value) => {
            let value: uuid::Uuid = (*value).into();
            tagged("uuid", value.to_string())
        }
        Value::IpAddr(value) => tagged("ip_addr", value.to_string()),
        Value::Duration(value) => {
            let value: time::Duration = (*value).into();
            tagged("duration", value.whole_nanoseconds().to_string())
        }
        Value::Time(value) => {
            let value: time::Time = (*value).into();
            tagged(
                "time",
                (value - time::Time::MIDNIGHT)
                    .whole_nanoseconds()
                    .to_string(),
            )
        }
        Value::Date(value) => {
            let value: time::Date = (*value).into();
            tagged("date", value.to_julian_day().to_string())
        }
        Value::DateTime(value) => {
            let value: time::OffsetDateTime = (*value).into();
            tagged(
                "date_time",
                json!({
                    "unix_nanos": value.unix_timestamp_nanos().to_string(),
                    "offset_seconds": value.offset().whole_seconds().to_string(),
                }),
            )
        }
        Value::Bytes(value) => tagged("bytes", URL_SAFE_NO_PAD.encode(value)),
        Value::String(value) => tagged("string", value.clone()),
        Value::List(values) => tagged(
            "list",
            JsonValue::Array(values.iter().map(encode_inner).collect()),
        ),
        Value::Map(values) => tagged(
            "map",
            JsonValue::Array(
                values
                    .iter()
                    .map(|(key, value)| json!([encode_inner(key), encode_inner(value)]))
                    .collect(),
            ),
        ),
        Value::Object(values) => {
            let values = values
                .iter()
                .map(|(key, value)| (key.clone(), encode_inner(value)))
                .collect::<JsonMap<_, _>>();
            tagged("object", JsonValue::Object(values))
        }
        Value::Variant(value) => tagged(
            "variant",
            json!({
                "type": value.r#type,
                "variant": value.variant,
                "value": encode_inner(&value.value),
            }),
        ),
    }
}

fn decode_inner(encoded: &JsonValue) -> Result<Value, DbError> {
    let object = encoded
        .as_object()
        .ok_or_else(|| malformed("tagged value must be an object"))?;
    let tag = object
        .get("t")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| malformed("tagged value is missing string tag"))?;
    let payload = || {
        object
            .get("v")
            .ok_or_else(|| malformed(format!("tag '{tag}' is missing a payload")))
    };
    let string = || {
        payload()?
            .as_str()
            .ok_or_else(|| malformed(format!("tag '{tag}' requires a string payload")))
    };
    macro_rules! integer {
        ($variant:ident, $type:ty) => {
            Value::$variant(
                string()?
                    .parse::<$type>()
                    .map_err(|err| malformed(format!("invalid {tag}: {err}")))?,
            )
        };
    }
    Ok(match tag {
        "void" => Value::Void,
        "null" => Value::Null,
        "bool" => Value::Bool(
            payload()?
                .as_bool()
                .ok_or_else(|| malformed("bool requires a Boolean payload"))?,
        ),
        "i8" => integer!(I8, i8),
        "i16" => integer!(I16, i16),
        "i32" => integer!(I32, i32),
        "i64" => integer!(I64, i64),
        "i128" => integer!(I128, i128),
        "u8" => integer!(U8, u8),
        "u16" => integer!(U16, u16),
        "u32" => integer!(U32, u32),
        "u64" => integer!(U64, u64),
        "u128" => integer!(U128, u128),
        "f32" => Value::F32(
            f32::from_bits(
                u32::from_str_radix(string()?, 16)
                    .map_err(|err| malformed(format!("invalid f32: {err}")))?,
            )
            .into(),
        ),
        "f64" => Value::F64(
            f64::from_bits(
                u64::from_str_radix(string()?, 16)
                    .map_err(|err| malformed(format!("invalid f64: {err}")))?,
            )
            .into(),
        ),
        "uuid" => Value::Uuid(
            uuid::Uuid::parse_str(string()?)
                .map_err(|err| malformed(format!("invalid uuid: {err}")))?
                .into(),
        ),
        "ip_addr" => Value::IpAddr(
            string()?
                .parse()
                .map_err(|err| malformed(format!("invalid IP address: {err}")))?,
        ),
        "duration" => Value::Duration(duration_from_nanos(parse_i128(string()?, tag)?)?.into()),
        "time" => Value::Time(
            (time::Time::MIDNIGHT + duration_from_nanos(parse_i128(string()?, tag)?)?).into(),
        ),
        "date" => Value::Date(
            time::Date::from_julian_day(
                string()?
                    .parse()
                    .map_err(|err| malformed(format!("invalid date: {err}")))?,
            )
            .map_err(|err| malformed(format!("invalid date: {err}")))?
            .into(),
        ),
        "date_time" => {
            let payload = payload()?
                .as_object()
                .ok_or_else(|| malformed("date_time requires an object payload"))?;
            let nanos = required_string(payload, "unix_nanos")?;
            let offset = required_string(payload, "offset_seconds")?
                .parse::<i32>()
                .map_err(|err| malformed(format!("invalid datetime offset: {err}")))?;
            let offset = time::UtcOffset::from_whole_seconds(offset)
                .map_err(|err| malformed(format!("invalid datetime offset: {err}")))?;
            Value::DateTime(
                time::OffsetDateTime::from_unix_timestamp_nanos(parse_i128(nanos, tag)?)
                    .map_err(|err| malformed(format!("invalid datetime: {err}")))?
                    .to_offset(offset)
                    .into(),
            )
        }
        "bytes" => Value::Bytes(
            URL_SAFE_NO_PAD
                .decode(string()?)
                .map_err(|err| malformed(format!("invalid bytes: {err}")))?
                .into(),
        ),
        "string" => Value::String(string()?.to_string()),
        "list" => Value::List(decode_array(payload()?)?),
        "map" => {
            let entries = payload()?
                .as_array()
                .ok_or_else(|| malformed("map requires an array payload"))?;
            let mut map = Map::new();
            for entry in entries {
                let pair = entry
                    .as_array()
                    .filter(|pair| pair.len() == 2)
                    .ok_or_else(|| malformed("map entries must be two-element arrays"))?;
                map.insert(decode_inner(&pair[0])?, decode_inner(&pair[1])?);
            }
            Value::Map(map)
        }
        "object" => {
            let fields = payload()?
                .as_object()
                .ok_or_else(|| malformed("object requires an object payload"))?;
            let mut object = BTreeMap::new();
            for (key, value) in fields {
                object.insert(key.clone(), decode_inner(value)?);
            }
            Value::Object(Object::from(object))
        }
        "variant" => {
            let variant = payload()?
                .as_object()
                .ok_or_else(|| malformed("variant requires an object payload"))?;
            let r#type = variant
                .get("type")
                .and_then(JsonValue::as_str)
                .map(ToString::to_string);
            Value::Variant(Box::new(VariantValue {
                r#type,
                variant: required_string(variant, "variant")?.to_string(),
                value: decode_inner(
                    variant
                        .get("value")
                        .ok_or_else(|| malformed("variant is missing value"))?,
                )?,
            }))
        }
        _ => return Err(malformed(format!("unknown value tag '{tag}'"))),
    })
}

fn decode_array(value: &JsonValue) -> Result<Vec<Value>, DbError> {
    value
        .as_array()
        .ok_or_else(|| malformed("list requires an array payload"))?
        .iter()
        .map(decode_inner)
        .collect()
}

fn required_string<'a>(
    object: &'a JsonMap<String, JsonValue>,
    key: &str,
) -> Result<&'a str, DbError> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| malformed(format!("missing string field '{key}'")))
}

fn parse_i128(value: &str, tag: &str) -> Result<i128, DbError> {
    value
        .parse()
        .map_err(|err| malformed(format!("invalid {tag}: {err}")))
}

fn duration_from_nanos(value: i128) -> Result<time::Duration, DbError> {
    let seconds = value.div_euclid(1_000_000_000);
    let nanoseconds = value.rem_euclid(1_000_000_000);
    Ok(time::Duration::new(
        i64::try_from(seconds).map_err(|_| malformed("duration seconds exceed i64"))?,
        i32::try_from(nanoseconds).map_err(|_| malformed("invalid duration nanoseconds"))?,
    ))
}

fn malformed(message: impl Into<String>) -> DbError {
    DbError::Deserialization(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_codec_round_trips_lossless_values() {
        let mut map = Map::new();
        map.insert(Value::I16(-2), Value::String("mapped".into()));
        let values = vec![
            Value::Void,
            Value::Null,
            Value::Bool(true),
            Value::I8(i8::MIN),
            Value::I16(i16::MIN),
            Value::I32(i32::MIN),
            Value::I64(i64::MIN),
            Value::I128(i128::MIN),
            Value::U8(u8::MAX),
            Value::U16(u16::MAX),
            Value::U32(u32::MAX),
            Value::U64(u64::MAX),
            Value::U128(u128::MAX),
            Value::F32(f32::from_bits(0x8000_0000).into()),
            Value::F64(f64::from_bits(0x7ff8_0000_0000_0042).into()),
            Value::Uuid(uuid::Uuid::nil().into()),
            Value::IpAddr("::1".parse().unwrap()),
            Value::Duration(time::Duration::nanoseconds(17).into()),
            Value::Time((time::Time::MIDNIGHT + time::Duration::nanoseconds(19)).into()),
            Value::Date(time::Date::from_julian_day(2_460_000).unwrap().into()),
            Value::DateTime(time::OffsetDateTime::UNIX_EPOCH.into()),
            Value::Bytes(vec![0, 1, 255].into()),
            Value::String("value".into()),
            Value::List(vec![Value::U128(u128::MAX)]),
            Value::Map(map),
            Value::Object(Object::from_iter([("x".into(), Value::I8(1))])),
            Value::Variant(Box::new(VariantValue {
                r#type: Some("example".into()),
                variant: "case".into(),
                value: Value::Null,
            })),
        ];
        for value in values {
            let encoded = encode_value(&value);
            assert_eq!(decode_value(&encoded).unwrap(), value, "{encoded}");
        }
    }

    #[test]
    fn tagged_codec_rejects_malformed_documents() {
        for malformed in [
            json!(null),
            json!({}),
            json!({"format": 2, "value": {"t": "null"}}),
            json!({"format": 1, "value": {"t": "u64", "v": "nope"}}),
            json!({"format": 1, "value": {"t": "unknown"}}),
        ] {
            assert!(decode_value(&malformed).is_err());
        }
    }

    #[test]
    fn object_helpers_require_object_documents() {
        let object = Object::from_iter([("field".into(), Value::Void)]);
        assert_eq!(decode_object(&encode_object(&object)).unwrap(), object);
        assert!(decode_object(&encode_value(&Value::Null)).is_err());
    }
}
