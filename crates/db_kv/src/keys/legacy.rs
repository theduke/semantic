//! Legacy textual key layout (layout version 1).
//!
//! Databases written before the binary layout used UTF-8 keys:
//!
//! - entities: `c/{collection lid}/e/{id}`
//! - index entries: `i/{index lid}/v/{token}/e/{id}` and, for path-equality
//!   indexes, `i/{index lid}/p/{path token}/v/{token}/e/{id}`, where tokens
//!   are URL-safe base64 of the MessagePack-encoded typed value
//! - index format markers: `i/{index lid}/fmt`
//!
//! Base64 tokens do not preserve value order, so this layout only supports
//! equality lookups. It is kept for the on-open migration
//! ([`crate::storage::layout`]) and for building legacy fixtures in tests.

use std::collections::BTreeSet;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use semantic_data::value::serde::typed::TypedRef;
use semantic_data::value::{FieldPath, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};

use super::{TAG_ENTITY, TAG_INDEX, TAG_INDEX_MARKER, decode_lid, memcmp};

/// Prefix shared by all legacy entity keys.
pub const ENTITY_SPACE: &[u8] = b"c/";
/// Prefix shared by all legacy index entries and index format markers.
pub const INDEX_SPACE: &[u8] = b"i/";

pub fn entity_key(collection: LocalCollectionId, id: &str) -> Vec<u8> {
    format!("c/{}/e/{}", collection.0, id).into_bytes()
}

/// Decode a legacy entity key into its collection and entity identifier.
pub fn parse_entity_key(key: &[u8]) -> Option<(LocalCollectionId, &str)> {
    let key = std::str::from_utf8(key).ok()?;
    let key = key.strip_prefix("c/")?;
    let (collection, id) = key.split_once("/e/")?;
    Some((LocalCollectionId(collection.parse().ok()?), id))
}

pub fn index_key(
    index: LocalIndexId,
    path: Option<&FieldPath>,
    value: &Value,
    entity_id: &str,
) -> Result<Vec<u8>, DbError> {
    let mut key = match path {
        Some(path) => format!(
            "i/{}/p/{}/",
            index.0,
            value_token(&super::field_path_to_value(path))?
        ),
        None => format!("i/{}/", index.0),
    };
    key.push_str("v/");
    key.push_str(&value_token(value)?);
    key.push_str("/e/");
    key.push_str(entity_id);
    Ok(key.into_bytes())
}

pub fn index_format_key(index: LocalIndexId) -> Vec<u8> {
    format!("i/{}/fmt", index.0).into_bytes()
}

fn value_token(value: &Value) -> Result<String, DbError> {
    let bytes = rmp_serde::to_vec(&TypedRef(value))
        .map_err(|err| DbError::Serialization(err.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Re-encode entries of the current layout in the legacy layout.
///
/// Meta entries are dropped (the legacy layout has none) and keys outside the
/// binary key spaces are kept. `path_indexes` lists the path-equality
/// indexes, whose entries carry a path token. Intended for building legacy
/// fixtures in tests.
#[doc(hidden)]
pub fn downgrade_entries(
    entries: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>,
    path_indexes: &BTreeSet<LocalIndexId>,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
    let invalid = || DbError::Deserialization("invalid current-layout key".to_string());
    let mut out = Vec::new();
    for (key, value) in entries {
        let Some((&tag, rest)) = key.split_first() else {
            continue;
        };
        match tag {
            TAG_ENTITY => {
                let (collection, id) = super::parse_entity_key(&key).ok_or_else(invalid)?;
                out.push((entity_key(collection, id), value));
            }
            TAG_INDEX => {
                let (lid, len) = decode_lid(rest).ok_or_else(invalid)?;
                let index = LocalIndexId(lid as usize);
                let mut rest = &rest[len..];
                let path = if path_indexes.contains(&index) {
                    let (path, len) = memcmp::decode_prefix(rest)?;
                    rest = &rest[len..];
                    Some(value_to_field_path(&path).ok_or_else(invalid)?)
                } else {
                    None
                };
                let (indexed, len) = memcmp::decode_prefix(rest)?;
                let id = std::str::from_utf8(&rest[len..]).map_err(|_| invalid())?;
                out.push((index_key(index, path.as_ref(), &indexed, id)?, value));
            }
            TAG_INDEX_MARKER => {
                let (lid, _) = decode_lid(rest).ok_or_else(invalid)?;
                out.push((index_format_key(LocalIndexId(lid as usize)), value));
            }
            tag if tag < super::TAG_RESERVED_END => {}
            _ => out.push((key, value)),
        }
    }
    Ok(out)
}

fn value_to_field_path(value: &Value) -> Option<FieldPath> {
    let Value::List(segments) = value else {
        return None;
    };
    let mut path = FieldPath::new();
    for segment in segments {
        match segment {
            Value::List(parts) => match parts.as_slice() {
                [Value::String(kind), Value::String(field)] if kind == "f" => {
                    path.push_field(field.clone());
                }
                [Value::String(kind), Value::U64(index)] if kind == "i" => {
                    path.push_index(usize::try_from(*index).ok()?);
                }
                _ => return None,
            },
            _ => return None,
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_entity_key_with_delimiters_in_id() {
        let key = entity_key(LocalCollectionId(12), "path/with/e/delimiters");
        assert_eq!(
            parse_entity_key(&key),
            Some((LocalCollectionId(12), "path/with/e/delimiters"))
        );
        assert_eq!(parse_entity_key(b"i/12/v/token/e/id"), None);
    }

    #[test]
    fn downgrades_current_index_keys() {
        let path = FieldPath::from_fields(["a"]);
        let current = [
            (
                super::super::index_key(LocalIndexId(1), None, &Value::String("x".into()), "one"),
                Vec::new(),
            ),
            (
                super::super::index_key(LocalIndexId(2), Some(&path), &Value::I64(3), "two"),
                Vec::new(),
            ),
            (super::super::layout_version_key(), vec![0, 0, 0, 2]),
        ];
        let legacy = downgrade_entries(current, &BTreeSet::from([LocalIndexId(2)])).unwrap();
        assert_eq!(
            legacy,
            vec![
                (
                    index_key(LocalIndexId(1), None, &Value::String("x".into()), "one").unwrap(),
                    Vec::new()
                ),
                (
                    index_key(LocalIndexId(2), Some(&path), &Value::I64(3), "two").unwrap(),
                    Vec::new()
                ),
            ]
        );
    }
}
