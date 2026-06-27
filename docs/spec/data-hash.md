# Semantic Data Canonical Hash Input

## Status

This document specifies version 1 of the canonical byte encoding for
`semantic_data` values.

The encoding defined here is hash input, not a hash algorithm. Implementations
MUST first encode a value exactly as specified in this document, then feed the
resulting bytes to the caller's chosen digest algorithm.

This version applies only to the `semantic_data::Value` data model and its
borrowed equivalent, `semantic_data::ValueRef`. It does not define canonical
hashing for arbitrary Rust structs, serde values, facet shapes, schema
definitions, query ASTs, or transport formats.

The key words "MUST", "MUST NOT", "SHOULD", and "MAY" are to be interpreted as
normative requirements for independent implementations of this encoding.

## Goals

The canonical encoding is designed to be:

- implementation-independent
- byte-for-byte deterministic
- stable over long time periods
- independent of any concrete hash algorithm
- distinct across all `Value` variants unless this spec explicitly normalizes
  a representation, such as floating-point NaN payloads

The canonical encoding is not intended to be compact, human-readable, or tied to
Rust's internal `Hash`, `Ord`, serde, or facet behavior.

## Top-Level Encoding

Every top-level canonical byte stream MUST start with this exact ASCII
magic/version prefix:

```text
semantic-data-canonical-v1\0
```

In bytes:

```text
73 65 6d 61 6e 74 69 63 2d 64 61 74 61 2d 63 61
6e 6f 6e 69 63 61 6c 2d 76 31 00
```

After the prefix, the stream MUST contain exactly one encoded value body.

Nested values inside lists, maps, objects, and variants MUST be encoded as value
bodies only. They MUST NOT repeat the top-level magic/version prefix.

## Value Body Format

Every value body begins with a one-byte type tag. Depending on the tag, the tag
is followed by either no payload, a fixed-width payload, or a length-prefixed
payload.

All multi-byte numeric payloads MUST use big-endian byte order.

All length fields MUST be unsigned 64-bit big-endian integers. A length is
either a byte count or an item count, as specified by the containing type.

Strings are raw UTF-8 bytes. Implementations MUST NOT apply Unicode
normalization, case folding, locale-specific ordering, escaping, or re-encoding.

## Type Tags

| Tag | Value kind |
|---:|---|
| `0x00` | void |
| `0x01` | null |
| `0x02` | bool false |
| `0x03` | bool true |
| `0x10` | i8 |
| `0x11` | i16 |
| `0x12` | i32 |
| `0x13` | i64 |
| `0x14` | i128 |
| `0x18` | u8 |
| `0x19` | u16 |
| `0x1a` | u32 |
| `0x1b` | u64 |
| `0x1c` | u128 |
| `0x20` | f32 |
| `0x21` | f64 |
| `0x30` | UUID |
| `0x31` | IPv4 address |
| `0x32` | IPv6 address |
| `0x40` | duration |
| `0x41` | time |
| `0x42` | date |
| `0x43` | datetime |
| `0x50` | bytes |
| `0x51` | string |
| `0x52` | list |
| `0x53` | map |
| `0x54` | object |
| `0x55` | variant |

Tags not listed in this table are reserved. Implementations of version 1 MUST
NOT emit reserved tags.

## Primitive Values

### Void and Null

`Void` MUST encode as the single byte:

```text
00
```

`Null` MUST encode as the single byte:

```text
01
```

`Void` and `Null` are distinct values and MUST produce distinct canonical
streams.

### Bool

`false` MUST encode as:

```text
02
```

`true` MUST encode as:

```text
03
```

There is no additional boolean payload.

### Signed Integers

Signed integers MUST encode as:

```text
tag + big-endian two's-complement integer bytes
```

The payload width is fixed by the value kind:

| Kind | Tag | Payload bytes |
|---|---:|---:|
| `i8` | `0x10` | 1 |
| `i16` | `0x11` | 2 |
| `i32` | `0x12` | 4 |
| `i64` | `0x13` | 8 |
| `i128` | `0x14` | 16 |

Different integer widths are distinct. For example, `i8(1)` and `i64(1)` MUST
produce different canonical streams.

### Unsigned Integers

Unsigned integers MUST encode as:

```text
tag + big-endian unsigned integer bytes
```

The payload width is fixed by the value kind:

| Kind | Tag | Payload bytes |
|---|---:|---:|
| `u8` | `0x18` | 1 |
| `u16` | `0x19` | 2 |
| `u32` | `0x1a` | 4 |
| `u64` | `0x1b` | 8 |
| `u128` | `0x1c` | 16 |

Signed and unsigned values are distinct even when they represent the same
mathematical integer.

### Floating-Point Values

Floating-point values MUST encode as:

```text
tag + canonical IEEE-754 bits in big-endian order
```

`f32` uses tag `0x20` and a 4-byte payload.

`f64` uses tag `0x21` and an 8-byte payload.

Before writing the payload, implementations MUST canonicalize the floating-point
value:

- If the value is NaN, encode the canonical quiet NaN bit pattern.
  - `f32`: `0x7fc00000`
  - `f64`: `0x7ff8000000000000`
- If the value compares equal to zero, encode positive zero.
  - `f32`: `0x00000000`
  - `f64`: `0x0000000000000000`
- Otherwise, encode the value's IEEE-754 bit pattern unchanged.

As a consequence, all NaN payloads hash identically within the same float width,
and `-0.0` hashes identically to `+0.0`.

`f32` and `f64` remain distinct value kinds.

### UUID

A UUID MUST encode as:

```text
30 + 16 UUID bytes
```

The UUID bytes are the RFC 4122 byte representation.

### IP Addresses

An IPv4 address MUST encode as:

```text
31 + 4 address octets
```

An IPv6 address MUST encode as:

```text
32 + 16 address octets
```

IPv4-mapped IPv6 addresses MUST remain IPv6 values unless the source value is
actually an IPv4 address.

### Duration

A duration MUST encode as:

```text
40 + i128 total_nanoseconds
```

The payload is the signed total number of nanoseconds, encoded as a 16-byte
big-endian two's-complement integer.

If a source implementation has sub-nanosecond precision, it MUST convert to the
nearest representable `semantic_data::Duration` before applying this encoding.
Version 1 does not encode sub-nanosecond information.

### Time

A time of day MUST encode as:

```text
41 + hour:u8 + minute:u8 + second:u8 + nanosecond:u32
```

The nanosecond field is a 4-byte big-endian unsigned integer.

The encoded time is a civil time without a date or time zone.

### Date

A date MUST encode as:

```text
42 + year:i32 + month:u8 + day:u8
```

The year is the proleptic Gregorian year, encoded as a 4-byte big-endian
two's-complement integer.

The month is `1` through `12`. The day is `1` through `31`, constrained by the
valid calendar date.

### DateTime

A datetime MUST encode as:

```text
43 + unix_timestamp_seconds:i64 + nanosecond:u32
```

The Unix timestamp seconds field is an 8-byte big-endian two's-complement
integer. The nanosecond field is a 4-byte big-endian unsigned integer.

The encoded value is the absolute UTC instant represented by the datetime.
Offset or time zone presentation details MUST NOT affect the encoding.

## Byte Strings and Text Strings

### Bytes

Bytes MUST encode as:

```text
50 + byte_length:u64 + bytes
```

The length is the number of payload bytes.

### String

Strings MUST encode as:

```text
51 + byte_length:u64 + utf8_bytes
```

The length is the number of UTF-8 bytes, not the number of Unicode scalar values
or grapheme clusters.

Bytes and strings are distinct value kinds. A byte string and a UTF-8 string
with identical bytes MUST produce different canonical streams because their type
tags differ.

## Containers

### List

A list MUST encode as:

```text
52 + item_count:u64 + item_0 + item_1 + ...
```

Each item is encoded as a value body without the top-level magic/version prefix.

List order is significant and MUST be preserved.

Tuple-shaped data is represented as `Value::List` in version 1 because
`semantic_data::Value` has no distinct tuple variant.

### Object

An object is a map from UTF-8 string field names to values.

An object MUST encode as:

```text
54 + field_count:u64 + field_0 + field_1 + ...
```

Each field MUST encode as:

```text
field_name_byte_length:u64 + field_name_utf8_bytes + value_body
```

Fields MUST be sorted by the raw UTF-8 bytes of their field names in ascending
lexicographic byte order.

This ordering is byte ordering. Implementations MUST NOT use locale-aware
comparison, Unicode collation, normalized string comparison, insertion order, or
host language map ordering.

### Map

A map is a map from arbitrary `Value` keys to `Value` values.

A map MUST encode as:

```text
53 + entry_count:u64 + entry_0 + entry_1 + ...
```

Each entry MUST encode as:

```text
canonical_key_body + canonical_value_body
```

The key and value bodies MUST NOT include the top-level magic/version prefix.

Entries MUST be sorted by the full canonical key body bytes in ascending
lexicographic byte order. If two entries have identical canonical key body bytes,
they MUST be sorted by the full canonical value body bytes in ascending
lexicographic byte order.

The tie-breaker is required because version 1 normalizes some representations,
such as NaN payloads and signed zero. A host data model SHOULD avoid storing
multiple distinct keys that normalize to the same canonical key bytes, but if it
does, the byte stream remains deterministic.

Implementations MUST NOT use Rust's `Ord` for `Value`, insertion order, hash map
bucket order, JSON object order, or any other host-specific ordering to encode
maps.

### Variant

A variant is a tagged value with:

- optional type string
- variant name string
- payload value

A variant MUST encode as:

```text
55 + type_presence:u8 + optional_type_payload + variant_payload + value_body
```

`type_presence` MUST be:

| Byte | Meaning |
|---:|---|
| `0x00` | no type string |
| `0x01` | type string present |

If `type_presence` is `0x01`, the type string payload MUST be encoded as:

```text
byte_length:u64 + utf8_bytes
```

If `type_presence` is `0x00`, no type string payload is present.

The variant name payload MUST always be encoded as:

```text
byte_length:u64 + utf8_bytes
```

The payload value MUST be encoded as a value body without the top-level
magic/version prefix.

Changing the optional type string, variant name, or payload value MUST change
the canonical stream unless the changed payload normalizes to the same canonical
value body under this spec.

## Canonical Hash Procedure

To compute a canonical hash:

1. Encode the top-level value using this specification.
2. Feed the resulting bytes, unchanged, into the chosen hash algorithm.
3. Interpret or label the digest according to that hash algorithm's own rules.

This specification does not choose or recommend a digest algorithm. SHA-256,
BLAKE3, multihash, CIDs, or other digest/container systems MAY be used by
callers, but their semantics are outside the scope of this document.

## Rust API

The Rust implementation exposes the version 1 encoding through these functions:

```rust
pub fn write_canonical_value<W: std::io::Write>(
    writer: &mut W,
    value: &Value,
) -> std::io::Result<()>;

pub fn write_canonical_value_ref<W: std::io::Write>(
    writer: &mut W,
    value: ValueRef<'_>,
) -> std::io::Result<()>;

pub fn canonical_value_bytes(value: &Value) -> Vec<u8>;

pub fn canonical_value_ref_bytes(value: ValueRef<'_>) -> Vec<u8>;
```

`Value` and `ValueRef` also provide convenience methods:

```rust
impl Value {
    pub fn write_canonical<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()>;
    pub fn canonical_bytes(&self) -> Vec<u8>;
}

impl<'a> ValueRef<'a> {
    pub fn write_canonical<W: std::io::Write>(self, writer: &mut W) -> std::io::Result<()>;
    pub fn canonical_bytes(self) -> Vec<u8>;
}
```

The writer-based functions are the normative API for streaming bytes into a
caller-owned digest implementation.

## Versioning

The magic/version prefix is part of every top-level canonical stream. A future
incompatible change MUST use a different prefix.

Implementations MUST NOT silently treat another prefix as version 1.

Version 1 decoders, if implemented, SHOULD reject reserved type tags and
malformed payload lengths.

## Non-Goals

Version 1 does not define:

- a default digest algorithm
- textual hashes
- CIDs or multihash wrapping
- serde or facet canonicalization
- schema canonicalization
- query canonicalization
- Unicode normalization
- lossy coercions between numeric kinds
- canonicalization between objects and maps

