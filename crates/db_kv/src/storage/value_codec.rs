//! Compact binary encoding of [`Value`]s for entity payloads (format v2).
//!
//! Unlike the order-preserving [`memcmp`](crate::keys::memcmp) encoding used
//! for keys, this encoding is optimised for size and decode speed: every
//! value is a one-byte tag followed by its payload, integers use LEB128
//! varints (zigzag-mapped for signed types), strings, bytes and collections
//! are length-prefixed, and object field names are replaced by ids from a
//! per-collection field-name dictionary (see
//! [`entity_codec`](super::entity_codec)). `Map` keys are arbitrary values
//! and are encoded as values.
//!
//! | Tag    | Variant    | Payload                                                  |
//! |--------|------------|----------------------------------------------------------|
//! | `0x00` | `Void`     | none                                                     |
//! | `0x01` | `Null`     | none                                                     |
//! | `0x02` | `Bool`     | none (`false`)                                           |
//! | `0x03` | `Bool`     | none (`true`)                                            |
//! | `0x04` | `I8`       | 1 byte                                                   |
//! | `0x05`-`0x08` | `I16`-`I128` | zigzag varint                                 |
//! | `0x09` | `U8`       | 1 byte                                                   |
//! | `0x0A`-`0x0D` | `U16`-`U128` | varint                                        |
//! | `0x0E` | `F32`      | IEEE-754 bits, little-endian (4 bytes)                   |
//! | `0x0F` | `F64`      | IEEE-754 bits, little-endian (8 bytes)                   |
//! | `0x10` | `Uuid`     | 16 bytes                                                 |
//! | `0x11` | `IpAddr`   | 4 octets (v4)                                            |
//! | `0x12` | `IpAddr`   | 16 octets (v6)                                           |
//! | `0x13` | `Duration` | whole seconds, subsecond nanoseconds (zigzag varints)    |
//! | `0x14` | `Time`     | hour, minute, second (1 byte each), nanosecond (varint)  |
//! | `0x15` | `Date`     | Julian day (zigzag varint)                               |
//! | `0x16` | `DateTime` | Unix timestamp in nanoseconds (zigzag varint)            |
//! | `0x17` | `Bytes`    | length (varint), bytes                                   |
//! | `0x18` | `String`   | length (varint), UTF-8 bytes                             |
//! | `0x19` | `List`     | count (varint), values                                   |
//! | `0x1A` | `Map`      | count (varint), key and value per entry                  |
//! | `0x1B` | `Object`   | count (varint), field id (varint) and value per field    |
//! | `0x1C` | `Variant`  | type name (varint `0` when absent, else length + 1, then bytes), variant name (length-prefixed), value |
//!
//! Floats keep their exact bits. `DateTime` values are stored as their UTC
//! instant, like the msgpack format (version 1), so they decode with a UTC
//! offset.
//!
//! Containers (lists, maps, objects, variants) nest at most
//! [`MAX_VALUE_DEPTH`] levels below an entity's top-level fields: the
//! encoder rejects deeper values, so everything written stays readable, and
//! the decoder, which recurses once per level, rejects deeper (corrupt)
//! input instead of overflowing the stack. Length prefixes are checked
//! against the remaining input before anything is allocated.

use std::collections::BTreeMap;

use semantic_data::value::{
    MAX_VALUE_DEPTH, Map, Object, OrderedF32, OrderedF64, Value, VariantValue,
};
use semantic_db_core::DbError;

const TAG_VOID: u8 = 0x00;
const TAG_NULL: u8 = 0x01;
const TAG_FALSE: u8 = 0x02;
const TAG_TRUE: u8 = 0x03;
const TAG_I8: u8 = 0x04;
const TAG_I16: u8 = 0x05;
const TAG_I32: u8 = 0x06;
const TAG_I64: u8 = 0x07;
const TAG_I128: u8 = 0x08;
const TAG_U8: u8 = 0x09;
const TAG_U16: u8 = 0x0A;
const TAG_U32: u8 = 0x0B;
const TAG_U64: u8 = 0x0C;
const TAG_U128: u8 = 0x0D;
const TAG_F32: u8 = 0x0E;
const TAG_F64: u8 = 0x0F;
const TAG_UUID: u8 = 0x10;
const TAG_IPV4: u8 = 0x11;
const TAG_IPV6: u8 = 0x12;
const TAG_DURATION: u8 = 0x13;
const TAG_TIME: u8 = 0x14;
const TAG_DATE: u8 = 0x15;
const TAG_DATE_TIME: u8 = 0x16;
const TAG_BYTES: u8 = 0x17;
const TAG_STRING: u8 = 0x18;
const TAG_LIST: u8 = 0x19;
const TAG_MAP: u8 = 0x1A;
const TAG_OBJECT: u8 = 0x1B;
const TAG_VARIANT: u8 = 0x1C;

/// Maps object field names to dictionary ids while encoding.
pub(crate) type FieldIds<'a> = dyn FnMut(&str) -> Result<u32, DbError> + 'a;

/// Resolves dictionary ids to object field names while decoding.
pub(crate) trait FieldNames {
    fn field_name(&self, id: u32) -> Option<&str>;
}

/// Decoding failure.
#[derive(Debug)]
pub(crate) enum CodecError {
    /// The payload references a field id the dictionary does not contain
    /// (yet); a newer dictionary may resolve it.
    UnknownField(u32),
    /// The payload is malformed.
    Malformed(String),
}

impl From<CodecError> for DbError {
    fn from(err: CodecError) -> Self {
        match err {
            CodecError::UnknownField(id) => {
                DbError::Deserialization(format!("entity payload references unknown field id {id}"))
            }
            CodecError::Malformed(message) => {
                DbError::Deserialization(format!("invalid entity payload: {message}"))
            }
        }
    }
}

fn malformed(message: impl std::fmt::Display) -> CodecError {
    CodecError::Malformed(message.to_string())
}

pub(crate) fn push_varint(out: &mut Vec<u8>, mut value: u128) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn zigzag(value: i128) -> u128 {
    ((value << 1) ^ (value >> 127)) as u128
}

fn unzigzag(value: u128) -> i128 {
    ((value >> 1) as i128) ^ -((value & 1) as i128)
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    push_varint(out, bytes.len() as u128);
    out.extend_from_slice(bytes);
}

fn depth_exceeded() -> String {
    format!("value nesting exceeds the maximum depth of {MAX_VALUE_DEPTH}")
}

/// Append the encoding of `value` to `out`.
///
/// Fails when containers nest deeper than [`MAX_VALUE_DEPTH`].
#[cfg(test)]
pub(crate) fn encode_value(
    value: &Value,
    out: &mut Vec<u8>,
    fields: &mut FieldIds<'_>,
) -> Result<(), DbError> {
    encode_value_at(value, out, fields, MAX_VALUE_DEPTH)
}

/// Encode `value`, whose containers may nest `depth` more levels.
fn encode_value_at(
    value: &Value,
    out: &mut Vec<u8>,
    fields: &mut FieldIds<'_>,
    depth: usize,
) -> Result<(), DbError> {
    let inner = || {
        depth
            .checked_sub(1)
            .ok_or_else(|| DbError::Serialization(depth_exceeded()))
    };
    match value {
        Value::Void => out.push(TAG_VOID),
        Value::Null => out.push(TAG_NULL),
        Value::Bool(false) => out.push(TAG_FALSE),
        Value::Bool(true) => out.push(TAG_TRUE),
        Value::I8(value) => out.extend_from_slice(&[TAG_I8, *value as u8]),
        Value::I16(value) => push_signed(out, TAG_I16, i128::from(*value)),
        Value::I32(value) => push_signed(out, TAG_I32, i128::from(*value)),
        Value::I64(value) => push_signed(out, TAG_I64, i128::from(*value)),
        Value::I128(value) => push_signed(out, TAG_I128, *value),
        Value::U8(value) => out.extend_from_slice(&[TAG_U8, *value]),
        Value::U16(value) => push_unsigned(out, TAG_U16, u128::from(*value)),
        Value::U32(value) => push_unsigned(out, TAG_U32, u128::from(*value)),
        Value::U64(value) => push_unsigned(out, TAG_U64, u128::from(*value)),
        Value::U128(value) => push_unsigned(out, TAG_U128, *value),
        Value::F32(value) => {
            out.push(TAG_F32);
            out.extend_from_slice(&value.0.to_bits().to_le_bytes());
        }
        Value::F64(value) => {
            out.push(TAG_F64);
            out.extend_from_slice(&value.0.to_bits().to_le_bytes());
        }
        Value::Uuid(value) => {
            out.push(TAG_UUID);
            out.extend_from_slice(uuid::Uuid::from(*value).as_bytes());
        }
        Value::IpAddr(std::net::IpAddr::V4(addr)) => {
            out.push(TAG_IPV4);
            out.extend_from_slice(&addr.octets());
        }
        Value::IpAddr(std::net::IpAddr::V6(addr)) => {
            out.push(TAG_IPV6);
            out.extend_from_slice(&addr.octets());
        }
        Value::Duration(value) => {
            let duration = time::Duration::from(*value);
            out.push(TAG_DURATION);
            push_varint(out, zigzag(i128::from(duration.whole_seconds())));
            push_varint(out, zigzag(i128::from(duration.subsec_nanoseconds())));
        }
        Value::Time(value) => {
            let (hour, minute, second, nanosecond) = time::Time::from(*value).as_hms_nano();
            out.extend_from_slice(&[TAG_TIME, hour, minute, second]);
            push_varint(out, u128::from(nanosecond));
        }
        Value::Date(value) => {
            push_signed(
                out,
                TAG_DATE,
                i128::from(time::Date::from(*value).to_julian_day()),
            );
        }
        Value::DateTime(value) => push_signed(
            out,
            TAG_DATE_TIME,
            time::OffsetDateTime::from(*value).unix_timestamp_nanos(),
        ),
        Value::Bytes(bytes) => {
            out.push(TAG_BYTES);
            push_bytes(out, bytes);
        }
        Value::String(value) => {
            out.push(TAG_STRING);
            push_bytes(out, value.as_bytes());
        }
        Value::List(items) => {
            let depth = inner()?;
            out.push(TAG_LIST);
            push_varint(out, items.len() as u128);
            for item in items {
                encode_value_at(item, out, fields, depth)?;
            }
        }
        Value::Map(map) => {
            let depth = inner()?;
            out.push(TAG_MAP);
            push_varint(out, map.len() as u128);
            for (key, value) in map.iter() {
                encode_value_at(key, out, fields, depth)?;
                encode_value_at(value, out, fields, depth)?;
            }
        }
        Value::Object(object) => {
            let depth = inner()?;
            out.push(TAG_OBJECT);
            encode_object_body_at(object.iter(), object.len(), out, fields, depth)?;
        }
        Value::Variant(variant) => {
            let depth = inner()?;
            out.push(TAG_VARIANT);
            match &variant.r#type {
                Some(type_name) => {
                    push_varint(out, type_name.len() as u128 + 1);
                    out.extend_from_slice(type_name.as_bytes());
                }
                None => push_varint(out, 0),
            }
            push_bytes(out, variant.variant.as_bytes());
            encode_value_at(&variant.value, out, fields, depth)?;
        }
    }
    Ok(())
}

/// Append the field count and the `(field id, value)` pairs of an entity's
/// top-level object.
pub(crate) fn encode_object_body<'v>(
    entries: impl Iterator<Item = (&'v String, &'v Value)>,
    len: usize,
    out: &mut Vec<u8>,
    fields: &mut FieldIds<'_>,
) -> Result<(), DbError> {
    encode_object_body_at(entries, len, out, fields, MAX_VALUE_DEPTH)
}

fn encode_object_body_at<'v>(
    entries: impl Iterator<Item = (&'v String, &'v Value)>,
    len: usize,
    out: &mut Vec<u8>,
    fields: &mut FieldIds<'_>,
    depth: usize,
) -> Result<(), DbError> {
    push_varint(out, len as u128);
    for (name, value) in entries {
        push_varint(out, u128::from(fields(name)?));
        encode_value_at(value, out, fields, depth)?;
    }
    Ok(())
}

fn push_signed(out: &mut Vec<u8>, tag: u8, value: i128) {
    out.push(tag);
    push_varint(out, zigzag(value));
}

fn push_unsigned(out: &mut Vec<u8>, tag: u8, value: u128) {
    out.push(tag);
    push_varint(out, value);
}

/// Decoder over one encoded buffer.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pos == self.bytes.len()
    }

    pub(crate) fn byte(&mut self) -> Result<u8, CodecError> {
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| malformed("unexpected end of input"))?;
        self.pos += 1;
        Ok(byte)
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], CodecError> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| malformed("unexpected end of input"))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub(crate) fn varint(&mut self) -> Result<u128, CodecError> {
        let mut value = 0u128;
        for shift in (0..128).step_by(7) {
            let byte = self.byte()?;
            let bits = u128::from(byte & 0x7F);
            if shift == 126 && bits > 0b11 {
                return Err(malformed("varint overflow"));
            }
            value |= bits << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(malformed("varint overflow"))
    }

    fn varint_as<T: TryFrom<u128>>(&mut self, what: &str) -> Result<T, CodecError> {
        T::try_from(self.varint()?).map_err(|_| malformed(format!("{what} out of range")))
    }

    fn signed_as<T: TryFrom<i128>>(&mut self, what: &str) -> Result<T, CodecError> {
        T::try_from(unzigzag(self.varint()?)).map_err(|_| malformed(format!("{what} out of range")))
    }

    fn read_len(&mut self) -> Result<usize, CodecError> {
        let len = self.varint_as::<usize>("length")?;
        // Every element occupies at least one byte, so a larger count is
        // malformed; this also bounds preallocations.
        if len > self.bytes.len() - self.pos {
            return Err(malformed("length exceeds payload"));
        }
        Ok(len)
    }

    fn bytes(&mut self) -> Result<&'a [u8], CodecError> {
        let len = self.read_len()?;
        self.take(len)
    }

    fn string(&mut self) -> Result<String, CodecError> {
        std::str::from_utf8(self.bytes()?)
            .map(str::to_string)
            .map_err(malformed)
    }

    /// Decode the field count and fields of an entity's top-level object
    /// into `object`.
    pub(crate) fn object_body(
        &mut self,
        names: &dyn FieldNames,
        object: &mut BTreeMap<String, Value>,
    ) -> Result<(), CodecError> {
        self.object_body_at(names, object, MAX_VALUE_DEPTH)
    }

    fn object_body_at(
        &mut self,
        names: &dyn FieldNames,
        object: &mut BTreeMap<String, Value>,
        depth: usize,
    ) -> Result<(), CodecError> {
        let len = self.read_len()?;
        for _ in 0..len {
            let id = self.varint_as::<u32>("field id")?;
            let name = names
                .field_name(id)
                .ok_or(CodecError::UnknownField(id))?
                .to_string();
            let value = self.value_at(names, depth)?;
            object.insert(name, value);
        }
        Ok(())
    }

    /// Decode one value whose containers may nest [`MAX_VALUE_DEPTH`]
    /// levels.
    #[cfg(test)]
    pub(crate) fn value(&mut self, names: &dyn FieldNames) -> Result<Value, CodecError> {
        self.value_at(names, MAX_VALUE_DEPTH)
    }

    /// Decode one value whose containers may nest `depth` more levels.
    ///
    /// Only containers recurse; scalars are decoded out of line, which keeps
    /// the stack use per nesting level small.
    fn value_at(&mut self, names: &dyn FieldNames, depth: usize) -> Result<Value, CodecError> {
        match self.byte()? {
            tag @ (TAG_LIST | TAG_MAP | TAG_OBJECT | TAG_VARIANT) => {
                let depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| malformed(depth_exceeded()))?;
                self.container(tag, names, depth)
            }
            tag => self.scalar(tag),
        }
    }

    fn container(
        &mut self,
        tag: u8,
        names: &dyn FieldNames,
        depth: usize,
    ) -> Result<Value, CodecError> {
        match tag {
            TAG_LIST => self.list(names, depth),
            TAG_MAP => self.map(names, depth),
            TAG_OBJECT => {
                let mut object = BTreeMap::new();
                self.object_body_at(names, &mut object, depth)?;
                Ok(Value::Object(Object::from(object)))
            }
            _ => self.variant(names, depth),
        }
    }

    fn list(&mut self, names: &dyn FieldNames, depth: usize) -> Result<Value, CodecError> {
        let len = self.read_len()?;
        let mut items = Vec::with_capacity(len);
        for _ in 0..len {
            items.push(self.value_at(names, depth)?);
        }
        Ok(Value::List(items))
    }

    fn map(&mut self, names: &dyn FieldNames, depth: usize) -> Result<Value, CodecError> {
        let len = self.read_len()?;
        let mut map = Map::new();
        for _ in 0..len {
            let key = self.value_at(names, depth)?;
            let value = self.value_at(names, depth)?;
            map.insert(key, value);
        }
        Ok(Value::Map(map))
    }

    fn variant(&mut self, names: &dyn FieldNames, depth: usize) -> Result<Value, CodecError> {
        let (r#type, variant) = self.variant_names()?;
        let value = self.value_at(names, depth)?;
        Ok(Value::Variant(Box::new(VariantValue {
            r#type,
            variant,
            value,
        })))
    }

    #[inline(never)]
    fn variant_names(&mut self) -> Result<(Option<String>, String), CodecError> {
        let r#type = match self.varint_as::<usize>("variant type length")? {
            0 => None,
            len => {
                let bytes = self.take(len - 1)?;
                Some(std::str::from_utf8(bytes).map_err(malformed)?.to_string())
            }
        };
        Ok((r#type, self.string()?))
    }

    #[inline(never)]
    fn scalar(&mut self, tag: u8) -> Result<Value, CodecError> {
        Ok(match tag {
            TAG_VOID => Value::Void,
            TAG_NULL => Value::Null,
            TAG_FALSE => Value::Bool(false),
            TAG_TRUE => Value::Bool(true),
            TAG_I8 => Value::I8(self.byte()? as i8),
            TAG_I16 => Value::I16(self.signed_as("i16")?),
            TAG_I32 => Value::I32(self.signed_as("i32")?),
            TAG_I64 => Value::I64(self.signed_as("i64")?),
            TAG_I128 => Value::I128(unzigzag(self.varint()?)),
            TAG_U8 => Value::U8(self.byte()?),
            TAG_U16 => Value::U16(self.varint_as("u16")?),
            TAG_U32 => Value::U32(self.varint_as("u32")?),
            TAG_U64 => Value::U64(self.varint_as("u64")?),
            TAG_U128 => Value::U128(self.varint()?),
            TAG_F32 => Value::F32(OrderedF32::from(f32::from_bits(u32::from_le_bytes(
                self.array()?,
            )))),
            TAG_F64 => Value::F64(OrderedF64::from(f64::from_bits(u64::from_le_bytes(
                self.array()?,
            )))),
            TAG_UUID => Value::Uuid(uuid::Uuid::from_bytes(self.array()?).into()),
            TAG_IPV4 => Value::IpAddr(std::net::Ipv4Addr::from(self.array::<4>()?).into()),
            TAG_IPV6 => Value::IpAddr(std::net::Ipv6Addr::from(self.array::<16>()?).into()),
            TAG_DURATION => {
                let seconds = self.signed_as::<i64>("duration seconds")?;
                let nanos = self.signed_as::<i32>("duration nanoseconds")?;
                let consistent_sign = seconds == 0 || nanos == 0 || (seconds < 0) == (nanos < 0);
                if nanos.unsigned_abs() >= 1_000_000_000 || !consistent_sign {
                    return Err(malformed("invalid duration nanoseconds"));
                }
                Value::Duration(time::Duration::new(seconds, nanos).into())
            }
            TAG_TIME => {
                let [hour, minute, second] = self.array()?;
                let nanosecond = self.varint_as::<u32>("time nanoseconds")?;
                let time = time::Time::from_hms_nano(hour, minute, second, nanosecond)
                    .map_err(malformed)?;
                Value::Time(time.into())
            }
            TAG_DATE => {
                let day = self.signed_as::<i32>("julian day")?;
                Value::Date(time::Date::from_julian_day(day).map_err(malformed)?.into())
            }
            TAG_DATE_TIME => {
                let nanos = unzigzag(self.varint()?);
                Value::DateTime(
                    time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
                        .map_err(malformed)?
                        .into(),
                )
            }
            TAG_BYTES => Value::Bytes(bytes::Bytes::copy_from_slice(self.bytes()?)),
            TAG_STRING => Value::String(self.string()?),
            other => return Err(malformed(format!("unknown value tag {other:#04x}"))),
        })
    }
}

#[cfg(test)]
#[path = "value_codec_tests.rs"]
mod tests;
