//! Order-preserving ("memcomparable") binary encoding of [`Value`]s.
//!
//! For all values `a` and `b`, `encode(a).cmp(&encode(b)) == a.cmp(&b)`:
//! comparing encodings as byte strings yields the order of `Value::cmp`,
//! including the rank order between different variants.
//!
//! Every encoding is self-delimiting: no encoding is a proper prefix of
//! another. Encodings can therefore be concatenated with other data, and
//! `encode(a) ++ x` compares to `encode(b) ++ y` like the tuple `(a, x)` to
//! `(b, y)`. Index keys rely on this to append the entity id directly after
//! the value.
//!
//! # Format
//!
//! Every value starts with a type tag. Tags follow the variant order of
//! `Value::cmp` and are all greater than [`SEQUENCE_END`]:
//!
//! | Tag    | Variant    | Payload                                                  |
//! |--------|------------|----------------------------------------------------------|
//! | `0x10` | `Void`     | none                                                     |
//! | `0x11` | `Null`     | none                                                     |
//! | `0x12` | `Bool`     | `0x00` / `0x01`                                          |
//! | `0x13`-`0x17` | `I8`-`I128` | big-endian two's complement with the sign bit flipped |
//! | `0x18`-`0x1C` | `U8`-`U128` | big-endian                                        |
//! | `0x1D` | `F32`      | IEEE-754 total-order transform, big-endian (4 bytes)     |
//! | `0x1E` | `F64`      | IEEE-754 total-order transform, big-endian (8 bytes)     |
//! | `0x1F` | `Uuid`     | 16 bytes                                                 |
//! | `0x20` | `IpAddr`   | `0x00` + 4 octets (v4) or `0x01` + 16 octets (v6)        |
//! | `0x21` | `Duration` | whole seconds (`i64`), subsecond nanoseconds (`i32`), both sign-flipped |
//! | `0x22` | `Time`     | hour, minute, second (`u8` each), nanosecond (`u32`)     |
//! | `0x23` | `Date`     | Julian day (`i32`, sign-flipped)                         |
//! | `0x24` | `DateTime` | Unix timestamp in nanoseconds (`i128`, sign-flipped)     |
//! | `0x25` | `Bytes`    | escaped bytes                                            |
//! | `0x26` | `String`   | escaped UTF-8 bytes                                      |
//! | `0x27` | `List`     | element encodings, then `0x00`                           |
//! | `0x28` | `Map`      | key and value encoding per entry, then `0x00`            |
//! | `0x29` | `Object`   | key (as a `String` value) and value per entry, then `0x00` |
//! | `0x2A` | `Variant`  | type name (`0x00` when absent, else a `String` value), variant name (`String` value), value |
//!
//! Escaped byte strings replace every `0x00` with `0x00 0xFF` and end with
//! the terminator `0x00 0x01`, so shorter strings sort before their
//! extensions and the escaped form of a prefix (without terminator, see
//! [`encode_string_prefix`]) is a byte prefix of every string starting with
//! it.
//!
//! Floats are canonicalized first, matching `OrderedFloat` equality: `-0.0`
//! encodes like `0.0`, and every NaN encodes as the positive quiet NaN, which
//! sorts above positive infinity.
//!
//! # Decoding
//!
//! [`decode`] supports every variant. It returns a value equal (by
//! `Value::eq`) to the encoded one, but canonicalizes what the encoding does
//! not distinguish: `-0.0` decodes as `0.0`, NaN payloads are not preserved,
//! and `DateTime` values decode with a UTC offset (ordering and equality of
//! `DateTime` only consider the instant).

use std::collections::BTreeMap;

use semantic_data::value::{Map, Object, OrderedF32, OrderedF64, Value, VariantValue};
use semantic_db_core::DbError;

/// Terminates lists, maps and objects. Lower than every type tag.
pub const SEQUENCE_END: u8 = 0x00;
/// Marks an absent variant type name. Lower than every type tag.
const VARIANT_NO_TYPE: u8 = 0x00;
const ESCAPE: u8 = 0x00;
const ESCAPED_ZERO: u8 = 0xFF;
const ESCAPED_END: u8 = 0x01;

const TAG_VOID: u8 = 0x10;
const TAG_NULL: u8 = 0x11;
const TAG_BOOL: u8 = 0x12;
const TAG_I8: u8 = 0x13;
const TAG_I16: u8 = 0x14;
const TAG_I32: u8 = 0x15;
const TAG_I64: u8 = 0x16;
const TAG_I128: u8 = 0x17;
const TAG_U8: u8 = 0x18;
const TAG_U16: u8 = 0x19;
const TAG_U32: u8 = 0x1A;
const TAG_U64: u8 = 0x1B;
const TAG_U128: u8 = 0x1C;
const TAG_F32: u8 = 0x1D;
const TAG_F64: u8 = 0x1E;
const TAG_UUID: u8 = 0x1F;
const TAG_IP_ADDR: u8 = 0x20;
const TAG_DURATION: u8 = 0x21;
const TAG_TIME: u8 = 0x22;
const TAG_DATE: u8 = 0x23;
const TAG_DATE_TIME: u8 = 0x24;
const TAG_BYTES: u8 = 0x25;
const TAG_STRING: u8 = 0x26;
const TAG_LIST: u8 = 0x27;
const TAG_MAP: u8 = 0x28;
const TAG_OBJECT: u8 = 0x29;
const TAG_VARIANT: u8 = 0x2A;

const IP_V4: u8 = 0x00;
const IP_V6: u8 = 0x01;

const CANONICAL_NAN_F32: u32 = 0x7FC0_0000;
const CANONICAL_NAN_F64: u64 = 0x7FF8_0000_0000_0000;

/// Encode `value` into a new buffer.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(value, &mut out);
    out
}

/// Append the encoding of `value` to `out`.
pub fn encode_into(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Void => out.push(TAG_VOID),
        Value::Null => out.push(TAG_NULL),
        Value::Bool(value) => {
            out.push(TAG_BOOL);
            out.push(u8::from(*value));
        }
        Value::I8(value) => push_tagged(out, TAG_I8, &((*value as u8) ^ (1 << 7)).to_be_bytes()),
        Value::I16(value) => {
            push_tagged(out, TAG_I16, &((*value as u16) ^ (1 << 15)).to_be_bytes())
        }
        Value::I32(value) => {
            push_tagged(out, TAG_I32, &((*value as u32) ^ (1 << 31)).to_be_bytes())
        }
        Value::I64(value) => {
            push_tagged(out, TAG_I64, &((*value as u64) ^ (1 << 63)).to_be_bytes())
        }
        Value::I128(value) => push_tagged(
            out,
            TAG_I128,
            &((*value as u128) ^ (1 << 127)).to_be_bytes(),
        ),
        Value::U8(value) => push_tagged(out, TAG_U8, &value.to_be_bytes()),
        Value::U16(value) => push_tagged(out, TAG_U16, &value.to_be_bytes()),
        Value::U32(value) => push_tagged(out, TAG_U32, &value.to_be_bytes()),
        Value::U64(value) => push_tagged(out, TAG_U64, &value.to_be_bytes()),
        Value::U128(value) => push_tagged(out, TAG_U128, &value.to_be_bytes()),
        Value::F32(value) => push_tagged(out, TAG_F32, &f32_key(value.0).to_be_bytes()),
        Value::F64(value) => push_tagged(out, TAG_F64, &f64_key(value.0).to_be_bytes()),
        Value::Uuid(value) => {
            push_tagged(out, TAG_UUID, uuid::Uuid::from(*value).as_bytes());
        }
        Value::IpAddr(std::net::IpAddr::V4(addr)) => {
            out.extend_from_slice(&[TAG_IP_ADDR, IP_V4]);
            out.extend_from_slice(&addr.octets());
        }
        Value::IpAddr(std::net::IpAddr::V6(addr)) => {
            out.extend_from_slice(&[TAG_IP_ADDR, IP_V6]);
            out.extend_from_slice(&addr.octets());
        }
        Value::Duration(value) => {
            let duration = time::Duration::from(*value);
            out.push(TAG_DURATION);
            out.extend_from_slice(&((duration.whole_seconds() as u64) ^ (1 << 63)).to_be_bytes());
            out.extend_from_slice(
                &((duration.subsec_nanoseconds() as u32) ^ (1 << 31)).to_be_bytes(),
            );
        }
        Value::Time(value) => {
            let (hour, minute, second, nanosecond) = time::Time::from(*value).as_hms_nano();
            out.extend_from_slice(&[TAG_TIME, hour, minute, second]);
            out.extend_from_slice(&nanosecond.to_be_bytes());
        }
        Value::Date(value) => {
            let day = time::Date::from(*value).to_julian_day();
            push_tagged(out, TAG_DATE, &((day as u32) ^ (1 << 31)).to_be_bytes());
        }
        Value::DateTime(value) => {
            let nanos = time::OffsetDateTime::from(*value).unix_timestamp_nanos();
            push_tagged(
                out,
                TAG_DATE_TIME,
                &((nanos as u128) ^ (1 << 127)).to_be_bytes(),
            );
        }
        Value::Bytes(bytes) => {
            out.push(TAG_BYTES);
            push_escaped(out, bytes);
            push_escaped_end(out);
        }
        Value::String(value) => push_string(out, value),
        Value::List(items) => {
            out.push(TAG_LIST);
            for item in items {
                encode_into(item, out);
            }
            out.push(SEQUENCE_END);
        }
        Value::Map(map) => {
            out.push(TAG_MAP);
            for (key, value) in map.iter() {
                encode_into(key, out);
                encode_into(value, out);
            }
            out.push(SEQUENCE_END);
        }
        Value::Object(object) => {
            out.push(TAG_OBJECT);
            for (key, value) in object.iter() {
                push_string(out, key);
                encode_into(value, out);
            }
            out.push(SEQUENCE_END);
        }
        Value::Variant(variant) => {
            out.push(TAG_VARIANT);
            match &variant.r#type {
                Some(type_name) => push_string(out, type_name),
                None => out.push(VARIANT_NO_TYPE),
            }
            push_string(out, &variant.variant);
            encode_into(&variant.value, out);
        }
    }
}

/// Encode the strings starting with `prefix`.
///
/// The result is a byte prefix of the encoding of every `Value::String`
/// starting with `prefix`, and of no other value.
pub fn encode_string_prefix(prefix: &str) -> Vec<u8> {
    let mut out = vec![TAG_STRING];
    push_escaped(&mut out, prefix.as_bytes());
    out
}

/// Encode the lists starting with the elements `items`, appending to `out`.
///
/// The result is a byte prefix of the encoding of every `Value::List` whose
/// first elements are `items`, and of no other value.
pub fn encode_list_prefix_into(items: &[Value], out: &mut Vec<u8>) {
    out.push(TAG_LIST);
    for item in items {
        encode_into(item, out);
    }
}

/// Decode one value that spans all of `bytes`.
pub fn decode(bytes: &[u8]) -> Result<Value, DbError> {
    let mut reader = Reader::new(bytes);
    let value = reader.value()?;
    if reader.pos != bytes.len() {
        return Err(decode_error("trailing bytes after encoded value"));
    }
    Ok(value)
}

/// Decode the value at the start of `bytes`, returning it with the number of
/// bytes it occupies.
pub fn decode_prefix(bytes: &[u8]) -> Result<(Value, usize), DbError> {
    let mut reader = Reader::new(bytes);
    let value = reader.value()?;
    Ok((value, reader.pos))
}

/// Length of the encoded value at the start of `bytes`, without decoding it.
pub fn encoded_len(bytes: &[u8]) -> Result<usize, DbError> {
    let mut reader = Reader::new(bytes);
    reader.skip()?;
    Ok(reader.pos)
}

fn push_tagged(out: &mut Vec<u8>, tag: u8, payload: &[u8]) {
    out.push(tag);
    out.extend_from_slice(payload);
}

fn push_string(out: &mut Vec<u8>, value: &str) {
    out.push(TAG_STRING);
    push_escaped(out, value.as_bytes());
    push_escaped_end(out);
}

fn push_escaped(out: &mut Vec<u8>, bytes: &[u8]) {
    for byte in bytes {
        if *byte == ESCAPE {
            out.extend_from_slice(&[ESCAPE, ESCAPED_ZERO]);
        } else {
            out.push(*byte);
        }
    }
}

fn push_escaped_end(out: &mut Vec<u8>) {
    out.extend_from_slice(&[ESCAPE, ESCAPED_END]);
}

fn f32_key(value: f32) -> u32 {
    let bits = if value.is_nan() {
        CANONICAL_NAN_F32
    } else if value == 0.0 {
        0
    } else {
        value.to_bits()
    };
    if bits >> 31 == 1 {
        !bits
    } else {
        bits | (1 << 31)
    }
}

fn f32_from_key(key: u32) -> f32 {
    let bits = if key >> 31 == 1 {
        key & !(1 << 31)
    } else {
        !key
    };
    f32::from_bits(bits)
}

fn f64_key(value: f64) -> u64 {
    let bits = if value.is_nan() {
        CANONICAL_NAN_F64
    } else if value == 0.0 {
        0
    } else {
        value.to_bits()
    };
    if bits >> 63 == 1 {
        !bits
    } else {
        bits | (1 << 63)
    }
}

fn f64_from_key(key: u64) -> f64 {
    let bits = if key >> 63 == 1 {
        key & !(1 << 63)
    } else {
        !key
    };
    f64::from_bits(bits)
}

fn decode_error(message: impl std::fmt::Display) -> DbError {
    DbError::Deserialization(format!("invalid memcomparable value: {message}"))
}

/// Payload size of fixed-width variants.
fn fixed_payload_len(tag: u8) -> Option<usize> {
    Some(match tag {
        TAG_VOID | TAG_NULL => 0,
        TAG_BOOL | TAG_I8 | TAG_U8 => 1,
        TAG_I16 | TAG_U16 => 2,
        TAG_I32 | TAG_U32 | TAG_F32 | TAG_DATE => 4,
        TAG_TIME => 7,
        TAG_I64 | TAG_U64 | TAG_F64 => 8,
        TAG_DURATION => 12,
        TAG_I128 | TAG_U128 | TAG_UUID | TAG_DATE_TIME => 16,
        _ => return None,
    })
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn peek(&self) -> Result<u8, DbError> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or_else(|| decode_error("unexpected end of input"))
    }

    fn byte(&mut self) -> Result<u8, DbError> {
        let byte = self.peek()?;
        self.pos += 1;
        Ok(byte)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], DbError> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| decode_error("unexpected end of input"))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DbError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    /// Consume an escaped byte string including its terminator.
    fn escaped(&mut self, mut sink: Option<&mut Vec<u8>>) -> Result<(), DbError> {
        loop {
            let byte = self.byte()?;
            if byte != ESCAPE {
                if let Some(sink) = sink.as_deref_mut() {
                    sink.push(byte);
                }
                continue;
            }
            match self.byte()? {
                ESCAPED_ZERO => {
                    if let Some(sink) = sink.as_deref_mut() {
                        sink.push(0);
                    }
                }
                ESCAPED_END => return Ok(()),
                other => return Err(decode_error(format!("invalid escape byte {other:#04x}"))),
            }
        }
    }

    fn escaped_bytes(&mut self) -> Result<Vec<u8>, DbError> {
        let mut out = Vec::new();
        self.escaped(Some(&mut out))?;
        Ok(out)
    }

    fn string_payload(&mut self) -> Result<String, DbError> {
        String::from_utf8(self.escaped_bytes()?).map_err(decode_error)
    }

    fn string_value(&mut self) -> Result<String, DbError> {
        match self.byte()? {
            TAG_STRING => self.string_payload(),
            other => Err(decode_error(format!(
                "expected string tag, found {other:#04x}"
            ))),
        }
    }

    /// Consume a `SEQUENCE_END` byte if it is next.
    fn sequence_end(&mut self) -> Result<bool, DbError> {
        if self.peek()? == SEQUENCE_END {
            self.pos += 1;
            return Ok(true);
        }
        Ok(false)
    }

    fn skip(&mut self) -> Result<(), DbError> {
        let tag = self.byte()?;
        if let Some(len) = fixed_payload_len(tag) {
            self.take(len)?;
            return Ok(());
        }
        match tag {
            TAG_IP_ADDR => {
                let len = match self.byte()? {
                    IP_V4 => 4,
                    IP_V6 => 16,
                    other => return Err(decode_error(format!("invalid ip kind {other:#04x}"))),
                };
                self.take(len)?;
            }
            TAG_BYTES | TAG_STRING => self.escaped(None)?,
            TAG_LIST => {
                while !self.sequence_end()? {
                    self.skip()?;
                }
            }
            TAG_MAP | TAG_OBJECT => {
                while !self.sequence_end()? {
                    self.skip()?;
                    self.skip()?;
                }
            }
            TAG_VARIANT => {
                if self.peek()? == VARIANT_NO_TYPE {
                    self.pos += 1;
                } else {
                    self.skip()?;
                }
                self.skip()?;
                self.skip()?;
            }
            other => return Err(decode_error(format!("unknown tag {other:#04x}"))),
        }
        Ok(())
    }

    fn value(&mut self) -> Result<Value, DbError> {
        let tag = self.byte()?;
        Ok(match tag {
            TAG_VOID => Value::Void,
            TAG_NULL => Value::Null,
            TAG_BOOL => match self.byte()? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                other => return Err(decode_error(format!("invalid bool byte {other:#04x}"))),
            },
            TAG_I8 => Value::I8((u8::from_be_bytes(self.array()?) ^ (1 << 7)) as i8),
            TAG_I16 => Value::I16((u16::from_be_bytes(self.array()?) ^ (1 << 15)) as i16),
            TAG_I32 => Value::I32((u32::from_be_bytes(self.array()?) ^ (1 << 31)) as i32),
            TAG_I64 => Value::I64((u64::from_be_bytes(self.array()?) ^ (1 << 63)) as i64),
            TAG_I128 => Value::I128((u128::from_be_bytes(self.array()?) ^ (1 << 127)) as i128),
            TAG_U8 => Value::U8(u8::from_be_bytes(self.array()?)),
            TAG_U16 => Value::U16(u16::from_be_bytes(self.array()?)),
            TAG_U32 => Value::U32(u32::from_be_bytes(self.array()?)),
            TAG_U64 => Value::U64(u64::from_be_bytes(self.array()?)),
            TAG_U128 => Value::U128(u128::from_be_bytes(self.array()?)),
            TAG_F32 => Value::F32(OrderedF32::from(f32_from_key(u32::from_be_bytes(
                self.array()?,
            )))),
            TAG_F64 => Value::F64(OrderedF64::from(f64_from_key(u64::from_be_bytes(
                self.array()?,
            )))),
            TAG_UUID => Value::Uuid(uuid::Uuid::from_bytes(self.array()?).into()),
            TAG_IP_ADDR => match self.byte()? {
                IP_V4 => Value::IpAddr(std::net::Ipv4Addr::from(self.array::<4>()?).into()),
                IP_V6 => Value::IpAddr(std::net::Ipv6Addr::from(self.array::<16>()?).into()),
                other => return Err(decode_error(format!("invalid ip kind {other:#04x}"))),
            },
            TAG_DURATION => {
                let seconds = (u64::from_be_bytes(self.array()?) ^ (1 << 63)) as i64;
                let nanos = (u32::from_be_bytes(self.array()?) ^ (1 << 31)) as i32;
                let consistent_sign = seconds == 0 || nanos == 0 || (seconds < 0) == (nanos < 0);
                if nanos.unsigned_abs() >= 1_000_000_000 || !consistent_sign {
                    return Err(decode_error("invalid duration nanoseconds"));
                }
                Value::Duration(time::Duration::new(seconds, nanos).into())
            }
            TAG_TIME => {
                let [hour, minute, second] = self.array()?;
                let nanosecond = u32::from_be_bytes(self.array()?);
                let time = time::Time::from_hms_nano(hour, minute, second, nanosecond)
                    .map_err(decode_error)?;
                Value::Time(time.into())
            }
            TAG_DATE => {
                let day = (u32::from_be_bytes(self.array()?) ^ (1 << 31)) as i32;
                Value::Date(
                    time::Date::from_julian_day(day)
                        .map_err(decode_error)?
                        .into(),
                )
            }
            TAG_DATE_TIME => {
                let nanos = (u128::from_be_bytes(self.array()?) ^ (1 << 127)) as i128;
                Value::DateTime(
                    time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
                        .map_err(decode_error)?
                        .into(),
                )
            }
            TAG_BYTES => Value::Bytes(bytes::Bytes::from(self.escaped_bytes()?)),
            TAG_STRING => Value::String(self.string_payload()?),
            TAG_LIST => {
                let mut items = Vec::new();
                while !self.sequence_end()? {
                    items.push(self.value()?);
                }
                Value::List(items)
            }
            TAG_MAP => {
                let mut map = Map::new();
                while !self.sequence_end()? {
                    let key = self.value()?;
                    let value = self.value()?;
                    map.insert(key, value);
                }
                Value::Map(map)
            }
            TAG_OBJECT => {
                let mut object = BTreeMap::new();
                while !self.sequence_end()? {
                    let key = self.string_value()?;
                    let value = self.value()?;
                    object.insert(key, value);
                }
                Value::Object(Object::from(object))
            }
            TAG_VARIANT => {
                let r#type = if self.peek()? == VARIANT_NO_TYPE {
                    self.pos += 1;
                    None
                } else {
                    Some(self.string_value()?)
                };
                let variant = self.string_value()?;
                let value = self.value()?;
                Value::Variant(Box::new(VariantValue {
                    r#type,
                    variant,
                    value,
                }))
            }
            other => return Err(decode_error(format!("unknown tag {other:#04x}"))),
        })
    }
}

#[cfg(test)]
#[path = "memcmp_tests.rs"]
mod tests;
