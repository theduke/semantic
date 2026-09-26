use std::cmp::Ordering;

use semantic_data::value::{Object, Value};
use semantic_db_core::DbError;

use super::{CodecError, FieldNames, Reader, encode_value, push_varint};
use crate::test_values::Rng;

/// In-memory dictionary assigning ids in first-use order.
#[derive(Default)]
struct Names(Vec<String>);

impl Names {
    fn id(&mut self, name: &str) -> Result<u32, DbError> {
        if let Some(id) = self.0.iter().position(|known| known == name) {
            return Ok(id as u32);
        }
        self.0.push(name.to_string());
        Ok(self.0.len() as u32 - 1)
    }
}

impl FieldNames for Names {
    fn field_name(&self, id: u32) -> Option<&str> {
        self.0.get(id as usize).map(String::as_str)
    }
}

fn encode(value: &Value, names: &mut Names) -> Vec<u8> {
    let mut out = Vec::new();
    encode_value(value, &mut out, &mut |name: &str| names.id(name)).unwrap();
    out
}

fn decode(bytes: &[u8], names: &Names) -> Result<Value, CodecError> {
    let mut reader = Reader::new(bytes);
    let value = reader.value(names)?;
    assert!(reader.is_empty(), "trailing bytes");
    Ok(value)
}

fn assert_round_trip(value: &Value, names: &mut Names) {
    let encoded = encode(value, names);
    let decoded = decode(&encoded, names).unwrap_or_else(|err| panic!("{value:?}: {err:?}"));
    // DateTime offsets are normalised to UTC; everything else is exact.
    assert_eq!(
        decoded.cmp(value),
        Ordering::Equal,
        "{value:?} vs {decoded:?}"
    );
    assert_eq!(encode(&decoded, names), encoded, "{value:?}");
    // Truncated encodings fail cleanly instead of panicking.
    for len in 0..encoded.len() {
        assert!(
            Reader::new(&encoded[..len]).value(names).is_err(),
            "truncated encoding of {value:?} decoded"
        );
    }
}

#[test]
fn random_values_round_trip() {
    let mut rng = Rng(0xC0DEC);
    let mut names = Names::default();
    for _ in 0..20_000 {
        let value = rng.value(3);
        assert_round_trip(&value, &mut names);
    }
}

#[test]
fn boundary_values_round_trip() {
    let mut names = Names::default();
    let values = [
        Value::I8(i8::MIN),
        Value::I16(i16::MIN),
        Value::I32(i32::MIN),
        Value::I64(i64::MIN),
        Value::I64(i64::MAX),
        Value::I128(i128::MIN),
        Value::I128(i128::MAX),
        Value::U64(u64::MAX),
        Value::U128(u128::MAX),
        Value::from(-0.0f64),
        Value::from(f64::NAN),
        Value::from(-0.0f32),
    ];
    for value in values {
        assert_round_trip(&value, &mut names);
    }
    // Float bits are kept exactly (including the sign of zero).
    let encoded = encode(&Value::from(-0.0f64), &mut names);
    let Value::F64(decoded) = decode(&encoded, &names).unwrap() else {
        panic!("expected f64");
    };
    assert!(decoded.0.is_sign_negative());
}

#[test]
fn small_values_are_compact() {
    let mut names = Names::default();
    assert_eq!(encode(&Value::I64(5), &mut names).len(), 2);
    assert_eq!(encode(&Value::I64(-64), &mut names).len(), 2);
    assert_eq!(encode(&Value::Bool(true), &mut names).len(), 1);
    assert_eq!(encode(&Value::String("abc".into()), &mut names).len(), 5);
    let mut object = Object::new();
    object.insert("a_rather_long_field_name", Value::Null);
    // Tag, count, field id, value.
    assert_eq!(encode(&Value::Object(object), &mut names).len(), 4);
}

#[test]
fn nested_object_fields_use_the_dictionary() {
    let mut names = Names::default();
    let mut inner = Object::new();
    inner.insert("inner", Value::I32(1));
    let mut outer = Object::new();
    outer.insert("outer", Value::Object(inner));
    let encoded = encode(&Value::Object(outer.clone()), &mut names);
    assert_eq!(names.0, ["outer", "inner"]);
    assert!(!encoded.windows(5).any(|window| window == b"inner"));

    // A dictionary missing an id reports it, so callers can reload.
    let partial = Names(vec!["outer".to_string()]);
    assert!(matches!(
        decode(&encoded, &partial),
        Err(CodecError::UnknownField(1))
    ));
    assert_eq!(decode(&encoded, &names).unwrap(), Value::Object(outer));
}

#[test]
fn rejects_malformed_input() {
    let names = Names::default();
    assert!(matches!(
        decode(&[0xFF], &names),
        Err(CodecError::Malformed(_))
    ));
    // A string length beyond the payload.
    let mut bytes = vec![0x18];
    push_varint(&mut bytes, 1 << 40);
    assert!(matches!(
        decode(&bytes, &names),
        Err(CodecError::Malformed(_))
    ));
    // An overlong varint.
    let bytes = [
        0x0D, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFF, 0xFF, 0xFF, 0xFF, 0x7F,
    ];
    assert!(matches!(
        decode(&bytes, &names),
        Err(CodecError::Malformed(_))
    ));
    // An out-of-range u16.
    let mut bytes = vec![0x0A];
    push_varint(&mut bytes, 1 << 16);
    assert!(matches!(
        decode(&bytes, &names),
        Err(CodecError::Malformed(_))
    ));
}
