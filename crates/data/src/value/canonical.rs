use std::io::Write;

use super::{Value, ValueRef};

const MAGIC: &[u8] = b"semantic-data-canonical-v1\0";

const TAG_VOID: u8 = 0x00;
const TAG_NULL: u8 = 0x01;
const TAG_BOOL_FALSE: u8 = 0x02;
const TAG_BOOL_TRUE: u8 = 0x03;

const TAG_I8: u8 = 0x10;
const TAG_I16: u8 = 0x11;
const TAG_I32: u8 = 0x12;
const TAG_I64: u8 = 0x13;
const TAG_I128: u8 = 0x14;

const TAG_U8: u8 = 0x18;
const TAG_U16: u8 = 0x19;
const TAG_U32: u8 = 0x1a;
const TAG_U64: u8 = 0x1b;
const TAG_U128: u8 = 0x1c;

const TAG_F32: u8 = 0x20;
const TAG_F64: u8 = 0x21;

const TAG_UUID: u8 = 0x30;
const TAG_IPV4: u8 = 0x31;
const TAG_IPV6: u8 = 0x32;

const TAG_DURATION: u8 = 0x40;
const TAG_TIME: u8 = 0x41;
const TAG_DATE: u8 = 0x42;
const TAG_DATETIME: u8 = 0x43;

const TAG_BYTES: u8 = 0x50;
const TAG_STRING: u8 = 0x51;
const TAG_LIST: u8 = 0x52;
const TAG_MAP: u8 = 0x53;
const TAG_OBJECT: u8 = 0x54;
const TAG_VARIANT: u8 = 0x55;

const VARIANT_TYPE_ABSENT: u8 = 0x00;
const VARIANT_TYPE_PRESENT: u8 = 0x01;

const CANONICAL_F32_NAN: u32 = 0x7fc00000;
const CANONICAL_F64_NAN: u64 = 0x7ff8000000000000;

pub fn write_canonical_value<W: Write>(writer: &mut W, value: &Value) -> std::io::Result<()> {
    write_canonical_value_ref(writer, value.as_value_ref())
}

pub fn write_canonical_value_ref<W: Write>(
    writer: &mut W,
    value: ValueRef<'_>,
) -> std::io::Result<()> {
    write_magic(writer)?;
    write_value_body(writer, value)
}

pub fn canonical_value_bytes(value: &Value) -> Vec<u8> {
    canonical_value_ref_bytes(value.as_value_ref())
}

pub fn canonical_value_ref_bytes(value: ValueRef<'_>) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_canonical_value_ref(&mut bytes, value).expect("writing to Vec cannot fail");
    bytes
}

fn write_magic<W: Write>(writer: &mut W) -> std::io::Result<()> {
    writer.write_all(MAGIC)
}

fn write_value_body<W: Write>(writer: &mut W, value: ValueRef<'_>) -> std::io::Result<()> {
    match value {
        ValueRef::Owned(value) => write_value_body(writer, value.as_value_ref()),
        ValueRef::Ref(value) => write_value_body(writer, value.as_value_ref()),

        ValueRef::Void => writer.write_all(&[TAG_VOID]),
        ValueRef::Null => writer.write_all(&[TAG_NULL]),
        ValueRef::Bool(false) => writer.write_all(&[TAG_BOOL_FALSE]),
        ValueRef::Bool(true) => writer.write_all(&[TAG_BOOL_TRUE]),

        ValueRef::I8(value) => {
            writer.write_all(&[TAG_I8])?;
            writer.write_all(&value.to_be_bytes())
        }
        ValueRef::I16(value) => {
            writer.write_all(&[TAG_I16])?;
            writer.write_all(&value.to_be_bytes())
        }
        ValueRef::I32(value) => {
            writer.write_all(&[TAG_I32])?;
            writer.write_all(&value.to_be_bytes())
        }
        ValueRef::I64(value) => {
            writer.write_all(&[TAG_I64])?;
            writer.write_all(&value.to_be_bytes())
        }
        ValueRef::I128(value) => {
            writer.write_all(&[TAG_I128])?;
            writer.write_all(&value.to_be_bytes())
        }
        ValueRef::U8(value) => {
            writer.write_all(&[TAG_U8])?;
            writer.write_all(&value.to_be_bytes())
        }
        ValueRef::U16(value) => {
            writer.write_all(&[TAG_U16])?;
            writer.write_all(&value.to_be_bytes())
        }
        ValueRef::U32(value) => {
            writer.write_all(&[TAG_U32])?;
            writer.write_all(&value.to_be_bytes())
        }
        ValueRef::U64(value) => {
            writer.write_all(&[TAG_U64])?;
            writer.write_all(&value.to_be_bytes())
        }
        ValueRef::U128(value) => {
            writer.write_all(&[TAG_U128])?;
            writer.write_all(&value.to_be_bytes())
        }

        ValueRef::F32(value) => {
            writer.write_all(&[TAG_F32])?;
            writer.write_all(&canonical_f32_bits(value.into_inner()).to_be_bytes())
        }
        ValueRef::F64(value) => {
            writer.write_all(&[TAG_F64])?;
            writer.write_all(&canonical_f64_bits(value.into_inner()).to_be_bytes())
        }

        ValueRef::Uuid(value) => {
            let uuid: uuid::Uuid = value.into();
            writer.write_all(&[TAG_UUID])?;
            writer.write_all(uuid.as_bytes())
        }
        ValueRef::IpAddr(value) => match value {
            std::net::IpAddr::V4(value) => {
                writer.write_all(&[TAG_IPV4])?;
                writer.write_all(&value.octets())
            }
            std::net::IpAddr::V6(value) => {
                writer.write_all(&[TAG_IPV6])?;
                writer.write_all(&value.octets())
            }
        },

        ValueRef::Duration(value) => {
            let duration: time::Duration = value.into();
            writer.write_all(&[TAG_DURATION])?;
            writer.write_all(&duration.whole_nanoseconds().to_be_bytes())
        }
        ValueRef::Time(value) => {
            let time: time::Time = value.into();
            writer.write_all(&[TAG_TIME])?;
            writer.write_all(&[time.hour(), time.minute(), time.second()])?;
            writer.write_all(&time.nanosecond().to_be_bytes())
        }
        ValueRef::Date(value) => {
            let date: time::Date = value.into();
            writer.write_all(&[TAG_DATE])?;
            writer.write_all(&date.year().to_be_bytes())?;
            writer.write_all(&[u8::from(date.month()), date.day()])
        }
        ValueRef::DateTime(value) => {
            let datetime: time::OffsetDateTime = value.into();
            writer.write_all(&[TAG_DATETIME])?;
            writer.write_all(&datetime.unix_timestamp().to_be_bytes())?;
            writer.write_all(&datetime.nanosecond().to_be_bytes())
        }

        ValueRef::Bytes(value) => {
            writer.write_all(&[TAG_BYTES])?;
            write_len(writer, value.len())?;
            writer.write_all(value)
        }
        ValueRef::String(value) => {
            writer.write_all(&[TAG_STRING])?;
            write_str_payload(writer, value)
        }
        ValueRef::List(values) => {
            writer.write_all(&[TAG_LIST])?;
            write_len(writer, values.len())?;
            for value in values {
                write_value_body(writer, value.as_value_ref())?;
            }
            Ok(())
        }
        ValueRef::Map(map) => {
            writer.write_all(&[TAG_MAP])?;
            write_len(writer, map.len())?;

            let mut entries = Vec::with_capacity(map.len());
            for (key, value) in map.iter() {
                let key_bytes = value_body_bytes(key.as_value_ref());
                let value_bytes = value_body_bytes(value.as_value_ref());
                entries.push((key_bytes, value_bytes));
            }
            entries.sort_by(|(left_key, left_value), (right_key, right_value)| {
                left_key
                    .cmp(right_key)
                    .then_with(|| left_value.cmp(right_value))
            });

            for (key_bytes, value_bytes) in entries {
                writer.write_all(&key_bytes)?;
                writer.write_all(&value_bytes)?;
            }
            Ok(())
        }
        ValueRef::Object(object) => {
            writer.write_all(&[TAG_OBJECT])?;
            write_len(writer, object.len())?;

            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|(left_key, _), (right_key, _)| {
                left_key.as_bytes().cmp(right_key.as_bytes())
            });

            for (key, value) in entries {
                write_str_payload(writer, key)?;
                write_value_body(writer, value.as_value_ref())?;
            }
            Ok(())
        }
        ValueRef::Variant(variant) => {
            writer.write_all(&[TAG_VARIANT])?;
            match variant.r#type {
                Some(r#type) => {
                    writer.write_all(&[VARIANT_TYPE_PRESENT])?;
                    write_str_payload(writer, r#type)?;
                }
                None => writer.write_all(&[VARIANT_TYPE_ABSENT])?,
            }
            write_str_payload(writer, variant.variant)?;
            write_value_body(writer, variant.value.as_value_ref())
        }
    }
}

fn write_len<W: Write>(writer: &mut W, len: usize) -> std::io::Result<()> {
    let len = u64::try_from(len).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "canonical value length exceeds u64",
        )
    })?;
    writer.write_all(&len.to_be_bytes())
}

fn write_str_payload<W: Write>(writer: &mut W, value: &str) -> std::io::Result<()> {
    write_len(writer, value.len())?;
    writer.write_all(value.as_bytes())
}

fn value_body_bytes(value: ValueRef<'_>) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_value_body(&mut bytes, value).expect("writing to Vec cannot fail");
    bytes
}

fn canonical_f32_bits(value: f32) -> u32 {
    if value.is_nan() {
        CANONICAL_F32_NAN
    } else if value == 0.0 {
        0.0f32.to_bits()
    } else {
        value.to_bits()
    }
}

fn canonical_f64_bits(value: f64) -> u64 {
    if value.is_nan() {
        CANONICAL_F64_NAN
    } else if value == 0.0 {
        0.0f64.to_bits()
    } else {
        value.to_bits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Map, Object, OrderedF32, OrderedF64, VariantValue};

    fn body(value: &Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_value_body(&mut bytes, value.as_value_ref()).expect("write value body");
        bytes
    }

    #[test]
    fn same_value_produces_identical_bytes() {
        let value = Value::String("stable".to_owned());
        assert_eq!(value.canonical_bytes(), value.canonical_bytes());
    }

    #[test]
    fn void_and_null_are_distinct() {
        assert_ne!(Value::Void.canonical_bytes(), Value::Null.canonical_bytes());
    }

    #[test]
    fn numeric_and_string_types_are_distinct() {
        let values = [
            Value::I64(1).canonical_bytes(),
            Value::U64(1).canonical_bytes(),
            Value::F64(OrderedF64::from(1.0)).canonical_bytes(),
            Value::String("1".to_owned()).canonical_bytes(),
        ];

        for (index, left) in values.iter().enumerate() {
            for right in values.iter().skip(index + 1) {
                assert_ne!(left, right);
            }
        }
    }

    #[test]
    fn signed_zero_is_canonicalized() {
        assert_eq!(
            Value::F32(OrderedF32::from(-0.0)).canonical_bytes(),
            Value::F32(OrderedF32::from(0.0)).canonical_bytes()
        );
        assert_eq!(
            Value::F64(OrderedF64::from(-0.0)).canonical_bytes(),
            Value::F64(OrderedF64::from(0.0)).canonical_bytes()
        );
    }

    #[test]
    fn nan_payloads_are_canonicalized() {
        let f32_a = f32::from_bits(0x7fc00001);
        let f32_b = f32::from_bits(0x7fffffff);
        assert_eq!(
            Value::F32(OrderedF32::from(f32_a)).canonical_bytes(),
            Value::F32(OrderedF32::from(f32_b)).canonical_bytes()
        );

        let f64_a = f64::from_bits(0x7ff8000000000001);
        let f64_b = f64::from_bits(0x7fffffffffffffff);
        assert_eq!(
            Value::F64(OrderedF64::from(f64_a)).canonical_bytes(),
            Value::F64(OrderedF64::from(f64_b)).canonical_bytes()
        );
    }

    #[test]
    fn string_and_bytes_are_distinct() {
        assert_ne!(
            Value::String("abc".to_owned()).canonical_bytes(),
            Value::Bytes(bytes::Bytes::from_static(b"abc")).canonical_bytes()
        );
    }

    #[test]
    fn object_order_is_canonical() {
        let mut left = Object::new();
        left.insert("b", Value::U8(2));
        left.insert("a", Value::U8(1));

        let mut right = Object::new();
        right.insert("a", Value::U8(1));
        right.insert("b", Value::U8(2));

        assert_eq!(
            Value::Object(left).canonical_bytes(),
            Value::Object(right).canonical_bytes()
        );
    }

    #[test]
    fn map_order_is_canonical() {
        let mut map = Map::new();
        map.insert(Value::String("z".to_owned()), Value::U8(1));
        map.insert(Value::I8(-1), Value::U8(2));
        map.insert(Value::Bool(true), Value::U8(3));

        let bytes = body(&Value::Map(map));
        let first_key = body(&Value::Bool(true));
        let second_key = body(&Value::I8(-1));
        let third_key = body(&Value::String("z".to_owned()));

        let first_pos = find_subslice(&bytes, &first_key).expect("first key");
        let second_pos = find_subslice(&bytes, &second_key).expect("second key");
        let third_pos = find_subslice(&bytes, &third_key).expect("third key");

        assert!(first_pos < second_pos);
        assert!(second_pos < third_pos);
    }

    #[test]
    fn list_order_is_preserved() {
        assert_ne!(
            Value::List(vec![Value::U8(1), Value::U8(2)]).canonical_bytes(),
            Value::List(vec![Value::U8(2), Value::U8(1)]).canonical_bytes()
        );
    }

    #[test]
    fn variant_parts_affect_bytes() {
        let base = Value::Variant(Box::new(VariantValue {
            r#type: Some("example::Result".to_owned()),
            variant: "ok".to_owned(),
            value: Value::String("done".to_owned()),
        }));
        let changed_type = Value::Variant(Box::new(VariantValue {
            r#type: None,
            variant: "ok".to_owned(),
            value: Value::String("done".to_owned()),
        }));
        let changed_variant = Value::Variant(Box::new(VariantValue {
            r#type: Some("example::Result".to_owned()),
            variant: "err".to_owned(),
            value: Value::String("done".to_owned()),
        }));
        let changed_value = Value::Variant(Box::new(VariantValue {
            r#type: Some("example::Result".to_owned()),
            variant: "ok".to_owned(),
            value: Value::String("other".to_owned()),
        }));

        assert_ne!(base.canonical_bytes(), changed_type.canonical_bytes());
        assert_ne!(base.canonical_bytes(), changed_variant.canonical_bytes());
        assert_ne!(base.canonical_bytes(), changed_value.canonical_bytes());
    }

    #[test]
    fn temporal_values_have_expected_snapshots() {
        let duration = Value::Duration(crate::value::Duration::from(time::Duration::new(1, 2)));
        let time = Value::Time(crate::value::Time::from(
            time::Time::from_hms_nano(1, 2, 3, 4).expect("time"),
        ));
        let date = Value::Date(crate::value::Date::from(
            time::Date::from_calendar_date(2024, time::Month::February, 3).expect("date"),
        ));
        let datetime = Value::DateTime(crate::value::DateTime::from(
            time::OffsetDateTime::from_unix_timestamp(1)
                .expect("datetime")
                .replace_nanosecond(2)
                .expect("nanosecond"),
        ));

        let mut expected_duration = vec![TAG_DURATION];
        expected_duration.extend_from_slice(&1_000_000_002i128.to_be_bytes());
        assert_eq!(body(&duration), expected_duration);

        assert_eq!(body(&time), vec![TAG_TIME, 1, 2, 3, 0, 0, 0, 4]);

        let mut expected_date = vec![TAG_DATE];
        expected_date.extend_from_slice(&2024i32.to_be_bytes());
        expected_date.extend_from_slice(&[2, 3]);
        assert_eq!(body(&date), expected_date);

        let mut expected_datetime = vec![TAG_DATETIME];
        expected_datetime.extend_from_slice(&1i64.to_be_bytes());
        expected_datetime.extend_from_slice(&2u32.to_be_bytes());
        assert_eq!(body(&datetime), expected_datetime);
    }

    #[test]
    fn value_ref_and_owned_value_match() {
        let value = Value::List(vec![Value::String("x".to_owned()), Value::U8(7)]);
        assert_eq!(
            value.canonical_bytes(),
            value.as_value_ref().canonical_bytes()
        );
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }
}
