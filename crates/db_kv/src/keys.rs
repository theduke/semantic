//! Binary, order-preserving key layout of [`EntityStore`](crate::EntityStore)
//! (layout version 2).
//!
//! Every key starts with a table tag byte selecting its key space, so each
//! space is one contiguous, ordered key range:
//!
//! | Tag    | Space          | Key                                                     | Value                      |
//! |--------|----------------|---------------------------------------------------------|----------------------------|
//! | `0x01` | meta           | `0x01 name`                                             | per entry                  |
//! | `0x02` | entity         | `0x02 lid(collection) id`                               | encoded entity payload     |
//! | `0x03` | index entry    | `0x03 lid(index) [path token] value token id`           | empty                      |
//! | `0x04` | index marker   | `0x04 lid(index)`                                       | index format version       |
//! | `0x05` | stats          | `0x05 0x01 lid(collection)` / `0x05 0x02 lid(index)`    | row / entry count (u64 BE) |
//!
//! Tags `0x06..=0x1F` are reserved for future binary key spaces. Leading
//! bytes `0x20..` belong to the legacy textual layout (see [`legacy`]) and
//! to engine-private keys such as the redb revision counter
//! (`__semantic/revision`); binary keys never start with them.
//!
//! Local ids (`lid`) use an order-preserving variable-length encoding: one
//! byte holding the number `n` of significant bytes (0 to 8), followed by
//! the `n` big-endian bytes. Shorter encodings belong to smaller numbers and
//! no encoding is a prefix of another, so `entity_prefix(1)` never matches
//! keys of collection 10 and entity keys sort by `(collection, id)`.
//!
//! Entity ids are appended as raw UTF-8 bytes; they are the key suffix, so
//! they need no terminator. Index value tokens (and path tokens of
//! path-equality indexes) use the self-delimiting [`memcmp`] encoding, so
//! index keys sort by `(index, path, value, id)`, every value (and every
//! string prefix) is one contiguous key range, and the id is the remainder
//! after the value token.
//!
//! Equality and range indexes share one key derivation
//! ([`IndexSchema::key_value`](semantic_db_core::catalog::IndexSchema::key_value)):
//! a single-column key is the column value; a composite key is the list of
//! column values (missing columns as `Void`), whose list token frames the
//! concatenated column tokens. Equality on leading columns plus a range on
//! the next is therefore one contiguous key range ([`index_scan_range`]).
//!
//! The layout version is stored under the meta key `0x01 "format"` as a
//! big-endian `u32`; see [`crate::storage::layout`]. The meta key
//! `0x01 "stats"` marks the stats counters as maintained; see
//! [`crate::storage::stats`].
//!
//! Per-collection field-name dictionaries of the compact entity payload
//! format live under `0x01 "fdict" lid(collection) id` (`id` a big-endian
//! `u32`), holding the UTF-8 field name; see
//! [`crate::storage::entity_codec`] and the `storage::field_dict` module.

use semantic_data::value::{FieldPath, PathSegment, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};
use std::ops::Bound;

use crate::storage::prefix_range_end;

pub mod legacy;
pub mod memcmp;

/// Meta key space: database-level settings such as the layout version.
pub const TAG_META: u8 = 0x01;
/// Entity rows.
pub const TAG_ENTITY: u8 = 0x02;
/// Index entries.
pub const TAG_INDEX: u8 = 0x03;
/// Per-index format markers (present once an index has been built).
pub const TAG_INDEX_MARKER: u8 = 0x04;
/// Maintained collection and index statistics.
pub const TAG_STATS: u8 = 0x05;
const _: () = assert!(TAG_STATS < TAG_RESERVED_END);
/// Tags below this value are reserved for binary key spaces.
pub const TAG_RESERVED_END: u8 = 0x20;

/// Name of the meta entry holding the layout version.
pub const META_FORMAT: &[u8] = b"format";
/// Name of the meta entry marking the stats counters as maintained.
pub const META_STATS: &[u8] = b"stats";
/// Name prefix of the meta entries holding field-name dictionaries.
pub const META_FIELD_DICT: &[u8] = b"fdict";

/// Stats kind of per-collection row counts.
pub const STATS_COLLECTION_ROWS: u8 = 0x01;
/// Stats kind of per-index entry counts.
pub const STATS_INDEX_ENTRIES: u8 = 0x02;

/// Append the order-preserving variable-length encoding of `value`.
pub fn encode_lid(out: &mut Vec<u8>, value: u64) {
    let bytes = value.to_be_bytes();
    let skip = (value.leading_zeros() / 8) as usize;
    out.push((bytes.len() - skip) as u8);
    out.extend_from_slice(&bytes[skip..]);
}

/// Decode a local id at the start of `bytes`, returning it with its length.
pub fn decode_lid(bytes: &[u8]) -> Option<(u64, usize)> {
    let len = *bytes.first()? as usize;
    if len > 8 {
        return None;
    }
    let payload = bytes.get(1..=len)?;
    if payload.first() == Some(&0) {
        // Not minimal; never produced by `encode_lid`.
        return None;
    }
    let value = payload
        .iter()
        .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
    Some((value, len + 1))
}

fn tagged_lid(tag: u8, lid: usize) -> Vec<u8> {
    let mut key = Vec::with_capacity(10);
    key.push(tag);
    encode_lid(&mut key, lid as u64);
    key
}

pub fn meta_key(name: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + name.len());
    key.push(TAG_META);
    key.extend_from_slice(name);
    key
}

/// Key holding the database layout version.
pub fn layout_version_key() -> Vec<u8> {
    meta_key(META_FORMAT)
}

/// Key holding the stats format version; present once counters are
/// maintained.
pub fn stats_version_key() -> Vec<u8> {
    meta_key(META_STATS)
}

/// Prefix of the field-name dictionary entries of `collection`.
pub fn field_dict_prefix(collection: LocalCollectionId) -> Vec<u8> {
    let mut key = meta_key(META_FIELD_DICT);
    encode_lid(&mut key, collection.0 as u64);
    key
}

/// Key of the field name with dictionary id `id` in `collection`.
pub fn field_dict_key(collection: LocalCollectionId, id: u32) -> Vec<u8> {
    let mut key = field_dict_prefix(collection);
    key.extend_from_slice(&id.to_be_bytes());
    key
}

/// Decode a field-name dictionary key into its collection and field id.
pub fn parse_field_dict_key(key: &[u8]) -> Option<(LocalCollectionId, u32)> {
    let rest = key
        .strip_prefix(&[TAG_META])?
        .strip_prefix(META_FIELD_DICT)?;
    let (collection, len) = decode_lid(rest)?;
    let id = u32::from_be_bytes(rest.get(len..)?.try_into().ok()?);
    Some((LocalCollectionId(usize::try_from(collection).ok()?), id))
}

fn stats_key(kind: u8, lid: usize) -> Vec<u8> {
    let mut key = Vec::with_capacity(11);
    key.push(TAG_STATS);
    key.push(kind);
    encode_lid(&mut key, lid as u64);
    key
}

/// Key of the row count of `collection`.
pub fn collection_rows_key(collection: LocalCollectionId) -> Vec<u8> {
    stats_key(STATS_COLLECTION_ROWS, collection.0)
}

/// Key of the entry count of `index`.
pub fn index_entries_key(index: LocalIndexId) -> Vec<u8> {
    stats_key(STATS_INDEX_ENTRIES, index.0)
}

/// Key of the counter that the existence of `key` contributes to: the row
/// count of an entity's collection or the entry count of an index entry's
/// index. Other keys are not counted.
pub fn counter_key_for(key: &[u8]) -> Option<Vec<u8>> {
    let (&tag, rest) = key.split_first()?;
    let kind = match tag {
        TAG_ENTITY => STATS_COLLECTION_ROWS,
        TAG_INDEX => STATS_INDEX_ENTRIES,
        _ => return None,
    };
    let (lid, _) = decode_lid(rest)?;
    Some(stats_key(kind, usize::try_from(lid).ok()?))
}

/// Prefix of all entity keys of `collection`.
pub fn entity_prefix(collection: LocalCollectionId) -> Vec<u8> {
    tagged_lid(TAG_ENTITY, collection.0)
}

pub fn entity_key(collection: LocalCollectionId, id: &str) -> Vec<u8> {
    let mut key = entity_prefix(collection);
    key.extend_from_slice(id.as_bytes());
    key
}

/// Decode an entity key into its collection and entity identifier.
pub fn parse_entity_key(key: &[u8]) -> Option<(LocalCollectionId, &str)> {
    let rest = key.strip_prefix(&[TAG_ENTITY])?;
    let (collection, len) = decode_lid(rest)?;
    let id = std::str::from_utf8(&rest[len..]).ok()?;
    Some((LocalCollectionId(usize::try_from(collection).ok()?), id))
}

/// Prefix of all entries of `index`.
pub fn index_prefix(index: LocalIndexId) -> Vec<u8> {
    tagged_lid(TAG_INDEX, index.0)
}

/// Prefix of the entries of `index` at `path` (path-equality indexes) or of
/// all entries (other indexes).
pub fn index_path_prefix(index: LocalIndexId, path: Option<&FieldPath>) -> Vec<u8> {
    let mut key = index_prefix(index);
    if let Some(path) = path {
        memcmp::encode_into(&field_path_to_value(path), &mut key);
    }
    key
}

/// Prefix of the entries of `index` (at `path`) whose value is `value`.
pub fn index_value_prefix(index: LocalIndexId, path: Option<&FieldPath>, value: &Value) -> Vec<u8> {
    let mut key = index_path_prefix(index, path);
    memcmp::encode_into(value, &mut key);
    key
}

pub fn index_key(
    index: LocalIndexId,
    path: Option<&FieldPath>,
    value: &Value,
    entity_id: &str,
) -> Vec<u8> {
    let mut key = index_value_prefix(index, path, value);
    key.extend_from_slice(entity_id.as_bytes());
    key
}

/// Like [`index_key`], but fails when `value` nests deeper than
/// [`memcmp::MAX_DEPTH`], so every stored key decodes.
pub fn try_index_key(
    index: LocalIndexId,
    path: Option<&FieldPath>,
    value: &Value,
    entity_id: &str,
) -> Result<Vec<u8>, DbError> {
    let mut key = index_path_prefix(index, path);
    memcmp::try_encode_into(value, &mut key)?;
    key.extend_from_slice(entity_id.as_bytes());
    Ok(key)
}

/// Prefix of the entries of `index` (at `path`) whose value is a string
/// starting with `prefix`.
pub fn index_string_prefix(index: LocalIndexId, path: Option<&FieldPath>, prefix: &str) -> Vec<u8> {
    let mut key = index_path_prefix(index, path);
    key.extend_from_slice(&memcmp::encode_string_prefix(prefix));
    key
}

/// Key range `start..end` of the entries of `index` (at `path`) whose value
/// lies within `lower..upper`.
///
/// Returns `None` when the range is empty.
pub fn index_range(
    index: LocalIndexId,
    path: Option<&FieldPath>,
    lower: Bound<&Value>,
    upper: Bound<&Value>,
) -> Option<(Vec<u8>, Vec<u8>)> {
    bounded_key_range(index_path_prefix(index, path), lower, upper)
}

/// Key range `start..end` of the entries of an equality or range index
/// within `range` (see [`semantic_db_core::IndexScanRange`]).
///
/// Composite keys are list tokens, so the fixed leading columns are a byte
/// prefix (the list tag and their tokens) and the range applies to the next
/// token. Returns `None` when the range is empty.
pub fn index_scan_range(
    index: LocalIndexId,
    range: &semantic_db_core::IndexScanRange,
) -> Option<(Vec<u8>, Vec<u8>)> {
    use semantic_db_core::IndexColumnRange;

    let mut base = index_prefix(index);
    if range.composite {
        memcmp::encode_list_prefix_into(&range.prefix, &mut base);
    } else {
        debug_assert!(
            range.prefix.is_empty(),
            "single-column scans have no prefix"
        );
    }
    match &range.column {
        IndexColumnRange::Bounds { lower, upper } => {
            bounded_key_range(base, lower.as_ref(), upper.as_ref())
        }
        IndexColumnRange::StringPrefix(prefix) => {
            base.extend_from_slice(&memcmp::encode_string_prefix(prefix));
            let end = prefix_range_end(&base)?;
            Some((base, end))
        }
    }
}

/// Key range of the entries below the key prefix `base` whose next value
/// token lies within `lower..upper`.
fn bounded_key_range(
    base: Vec<u8>,
    lower: Bound<&Value>,
    upper: Bound<&Value>,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let with_value = |value: &Value| {
        let mut key = base.clone();
        memcmp::encode_into(value, &mut key);
        key
    };
    // Keys of one value share the prefix `base ++ encode(value)`, so the
    // prefix's range end is the first key after all of them.
    let start = match lower {
        Bound::Unbounded => base.clone(),
        Bound::Included(value) => with_value(value),
        Bound::Excluded(value) => prefix_range_end(&with_value(value))?,
    };
    let end = match upper {
        Bound::Unbounded => prefix_range_end(&base)?,
        Bound::Included(value) => prefix_range_end(&with_value(value))?,
        Bound::Excluded(value) => with_value(value),
    };
    (start < end).then_some((start, end))
}

/// Entity id of the index entry `key` whose value token starts at
/// `value_offset`.
pub fn index_entry_id(key: &[u8], value_offset: usize) -> Option<&str> {
    let value = key.get(value_offset..)?;
    let len = memcmp::encoded_len(value).ok()?;
    std::str::from_utf8(&value[len..]).ok()
}

/// Entity id of the entry `key` of `index`; path-equality entries
/// (`path_token`) carry a path token before the value token.
pub fn index_key_entity_id(key: &[u8], index: LocalIndexId, path_token: bool) -> Option<&str> {
    let prefix = index_prefix(index);
    if !key.starts_with(&prefix) {
        return None;
    }
    let mut offset = prefix.len();
    if path_token {
        offset += memcmp::encoded_len(key.get(offset..)?).ok()?;
    }
    index_entry_id(key, offset)
}

/// Key of the format marker of `index`.
pub fn index_marker_key(index: LocalIndexId) -> Vec<u8> {
    tagged_lid(TAG_INDEX_MARKER, index.0)
}

/// Representation of a field path in path-equality index keys.
pub(crate) fn field_path_to_value(path: &FieldPath) -> Value {
    let mut segments = Vec::with_capacity(path.segments().len());
    for segment in path.segments() {
        match segment {
            PathSegment::Field(field) => segments.push(Value::List(vec![
                Value::String("f".to_string()),
                Value::String(field.clone()),
            ])),
            PathSegment::Index(index) => segments.push(Value::List(vec![
                Value::String("i".to_string()),
                Value::U64(*index as u64),
            ])),
        }
    }
    Value::List(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lids_preserve_order_and_are_prefix_free() {
        let values = [
            0u64,
            1,
            9,
            10,
            255,
            256,
            65_535,
            65_536,
            u64::MAX >> 8,
            u64::MAX,
        ];
        for a in values {
            let mut ea = Vec::new();
            encode_lid(&mut ea, a);
            assert_eq!(decode_lid(&ea), Some((a, ea.len())));
            for b in values {
                let mut eb = Vec::new();
                encode_lid(&mut eb, b);
                assert_eq!(ea.cmp(&eb), a.cmp(&b));
                assert!(a == b || !eb.starts_with(&ea));
            }
        }
    }

    #[test]
    fn entity_prefixes_are_exact_per_collection() {
        let one = entity_key(LocalCollectionId(1), "0");
        let ten = entity_key(LocalCollectionId(10), "0");
        assert!(one.starts_with(&entity_prefix(LocalCollectionId(1))));
        assert!(!ten.starts_with(&entity_prefix(LocalCollectionId(1))));
        assert!(entity_key(LocalCollectionId(1), "zzz") < entity_key(LocalCollectionId(2), ""));
        assert!(entity_key(LocalCollectionId(2), "a") < entity_key(LocalCollectionId(2), "b"));
    }

    #[test]
    fn parses_entity_key_with_delimiters_in_id() {
        let key = entity_key(LocalCollectionId(12), "path/with/e/delimiters\0");
        assert_eq!(
            parse_entity_key(&key),
            Some((LocalCollectionId(12), "path/with/e/delimiters\0"))
        );
        let index = index_key(LocalIndexId(12), None, &Value::String("x".into()), "id");
        assert_eq!(parse_entity_key(&index), None);
        assert_eq!(parse_entity_key(b"c/12/e/id"), None);
    }

    #[test]
    fn index_entry_id_follows_value_token() {
        let path = FieldPath::from_fields(["a", "b"]);
        for path in [None, Some(&path)] {
            for value in [
                Value::String("a\0b".into()),
                Value::I64(-3),
                Value::List(vec![]),
            ] {
                let key = index_key(LocalIndexId(3), path, &value, "id\0/x");
                let offset = index_path_prefix(LocalIndexId(3), path).len();
                assert_eq!(index_entry_id(&key, offset), Some("id\0/x"));
            }
        }
    }

    #[test]
    fn key_spaces_do_not_overlap() {
        let spaces = [
            layout_version_key(),
            entity_key(LocalCollectionId(u32::MAX as usize), "\u{10ffff}"),
            index_key(LocalIndexId(0), None, &Value::Void, ""),
            index_marker_key(LocalIndexId(usize::MAX)),
        ];
        for pair in spaces.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert!(spaces.iter().all(|key| key[0] < TAG_RESERVED_END));
    }
}
