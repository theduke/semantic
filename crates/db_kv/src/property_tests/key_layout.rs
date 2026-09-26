//! Entity and index keys round-trip through their parsers, sort by their
//! logical tuples, and the key prefixes of different collections, indexes,
//! paths and values never overlap.

use std::cmp::Ordering;

use semantic_data::value::{FieldPath, Value};
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};

use super::{any_string, rng};
use crate::keys::{self, memcmp};
use crate::storage::prefix_range_end;
use crate::test_values::Rng;

/// Local ids biased towards the encoding's length boundaries.
fn lid(rng: &mut Rng) -> usize {
    match rng.below(4) {
        0 => rng.pick(&[0, 1, 0xFF, 0x100, 0xFFFF, 0x1_0000, usize::MAX]),
        1 => rng.below(4) as usize,
        2 => 1 << rng.below(64),
        _ => rng.next() as usize >> rng.below(64),
    }
}

fn id(rng: &mut Rng) -> String {
    match rng.below(6) {
        0 => rng
            .pick(&["", "a", "a\0", "a/e/b", "\u{10ffff}", "ä"])
            .to_string(),
        1 => any_string(rng),
        _ => rng.string(),
    }
}

fn path(rng: &mut Rng) -> FieldPath {
    let mut path = FieldPath::new();
    for _ in 0..1 + rng.below(3) {
        if rng.below(4) == 0 {
            path.push_index(rng.below(3) as usize);
        } else {
            path.push_field(rng.pick(&["a", "b", "", "a\0"]).to_string());
        }
    }
    path
}

#[derive(Debug, Clone)]
struct IndexEntry {
    index: usize,
    path: Option<FieldPath>,
    value: Value,
    id: String,
}

impl IndexEntry {
    /// Path-equality indexes (odd local ids here) carry a path token.
    fn random(rng: &mut Rng) -> Self {
        let index = rng.below(6) as usize * 0x7F;
        Self {
            index,
            path: (index % 2 == 1).then(|| path(rng)),
            value: if rng.below(3) == 0 {
                rng.value(0)
            } else {
                rng.value(2)
            },
            id: id(rng),
        }
    }

    fn key(&self) -> Vec<u8> {
        keys::index_key(
            LocalIndexId(self.index),
            self.path.as_ref(),
            &self.value,
            &self.id,
        )
    }

    fn path_value(&self) -> Option<Value> {
        self.path.as_ref().map(keys::field_path_to_value)
    }

    fn cmp_tuple(&self, other: &Self) -> Ordering {
        self.index
            .cmp(&other.index)
            .then_with(|| self.path_value().cmp(&other.path_value()))
            .then_with(|| self.value.cmp(&other.value))
            .then_with(|| self.id.as_bytes().cmp(other.id.as_bytes()))
    }
}

/// Parse an index key back into `(index, path value, value, id)`.
fn parse_index_key(key: &[u8], path_token: bool) -> (usize, Option<Value>, Value, String) {
    let rest = key.strip_prefix(&[keys::TAG_INDEX]).expect("index tag");
    let (lid, len) = keys::decode_lid(rest).expect("index lid");
    let mut rest = &rest[len..];
    let path = if path_token {
        let (path, len) = memcmp::decode_prefix(rest).expect("path token");
        rest = &rest[len..];
        Some(path)
    } else {
        None
    };
    let (value, len) = memcmp::decode_prefix(rest).expect("value token");
    let id = std::str::from_utf8(&rest[len..]).expect("utf-8 id");
    (lid as usize, path, value, id.to_string())
}

#[test]
fn local_ids_round_trip_and_sort() {
    let (mut rng, _seed) = rng(0x11D5);
    for _ in 0..20_000 {
        let (a, b) = (lid(&mut rng) as u64, lid(&mut rng) as u64);
        let (mut ea, mut eb) = (Vec::new(), Vec::new());
        keys::encode_lid(&mut ea, a);
        keys::encode_lid(&mut eb, b);
        assert_eq!(keys::decode_lid(&ea), Some((a, ea.len())));
        assert_eq!(ea.cmp(&eb), a.cmp(&b), "{a} {b}");
        if a != b {
            assert!(!eb.starts_with(&ea), "{a} encoding prefixes {b}");
        }
    }
}

#[test]
fn entity_keys_round_trip_and_sort_by_collection_and_id() {
    let (mut rng, _seed) = rng(0xE17);
    let mut entries = Vec::new();
    for _ in 0..4_000 {
        let collection = LocalCollectionId(lid(&mut rng));
        let id = id(&mut rng);
        let key = keys::entity_key(collection, &id);
        assert_eq!(
            keys::parse_entity_key(&key),
            Some((collection, id.as_str()))
        );
        assert!(key.starts_with(&keys::entity_prefix(collection)));
        assert_eq!(
            keys::counter_key_for(&key),
            Some(keys::collection_rows_key(collection))
        );
        entries.push((collection.0, id, key));
    }
    let mut by_key = entries.clone();
    by_key.sort_by(|a, b| a.2.cmp(&b.2));
    let mut by_tuple = entries;
    by_tuple.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
    });
    let tuples = |entries: &[(usize, String, Vec<u8>)]| {
        entries
            .iter()
            .map(|(lid, id, _)| (*lid, id.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(tuples(&by_key), tuples(&by_tuple));
}

#[test]
fn entity_prefixes_never_overlap_across_collections() {
    let (mut rng, _seed) = rng(0xC011);
    for _ in 0..10_000 {
        let (a, b) = (
            LocalCollectionId(lid(&mut rng)),
            LocalCollectionId(lid(&mut rng)),
        );
        let key = keys::entity_key(b, &id(&mut rng));
        let prefix = keys::entity_prefix(a);
        let in_range = key.as_slice() >= prefix.as_slice()
            && prefix_range_end(&prefix).is_none_or(|end| key < end);
        assert_eq!(key.starts_with(&prefix), a == b, "{a:?} {b:?}");
        assert_eq!(in_range, a == b, "{a:?} {b:?}");
        // Entity keys never fall into another key space.
        assert!(!key.starts_with(&keys::index_prefix(LocalIndexId(a.0))));
        assert!(!key.starts_with(&keys::field_dict_prefix(a)));
    }
}

#[test]
fn index_keys_round_trip_and_sort_by_index_path_value_and_id() {
    let (mut rng, _seed) = rng(0x1DE);
    let entries = (0..4_000)
        .map(|_| IndexEntry::random(&mut rng))
        .collect::<Vec<_>>();
    for entry in &entries {
        let key = entry.key();
        let path_token = entry.path.is_some();
        let (index, path, value, id) = parse_index_key(&key, path_token);
        assert_eq!(index, entry.index);
        assert_eq!(path, entry.path_value());
        assert_eq!(value.cmp(&entry.value), Ordering::Equal, "{entry:?}");
        assert_eq!(id, entry.id);
        assert_eq!(
            keys::index_key_entity_id(&key, LocalIndexId(entry.index), path_token),
            Some(entry.id.as_str())
        );
        assert_eq!(
            keys::counter_key_for(&key),
            Some(keys::index_entries_key(LocalIndexId(entry.index)))
        );
    }
    let mut by_key = entries.iter().map(|e| (e.key(), e)).collect::<Vec<_>>();
    by_key.sort_by(|a, b| a.0.cmp(&b.0));
    for pair in by_key.windows(2) {
        let (a, b) = (pair[0].1, pair[1].1);
        assert_ne!(
            a.cmp_tuple(b),
            Ordering::Greater,
            "key order differs from tuple order\n a = {a:?}\n b = {b:?}"
        );
    }
}

#[test]
fn index_prefixes_never_overlap_across_indexes_paths_and_values() {
    let (mut rng, _seed) = rng(0x0FF);
    for _ in 0..10_000 {
        let a = IndexEntry::random(&mut rng);
        // Share components often so near-collisions are common.
        let mut b = IndexEntry::random(&mut rng);
        if rng.below(2) == 0 {
            b.index = a.index;
            b.path = a.path.clone();
        }
        if rng.below(3) == 0 {
            b.value = a.value.clone();
        }
        let key = b.key();
        let same_index = a.index == b.index;
        let index_prefix = keys::index_prefix(LocalIndexId(a.index));
        assert_eq!(key.starts_with(&index_prefix), same_index, "{a:?} {b:?}");

        let same_path = same_index && a.path_value() == b.path_value();
        let path_prefix = keys::index_path_prefix(LocalIndexId(a.index), a.path.as_ref());
        assert_eq!(key.starts_with(&path_prefix), same_path, "{a:?} {b:?}");

        let same_value = same_path && a.value.cmp(&b.value) == Ordering::Equal;
        let value_prefix =
            keys::index_value_prefix(LocalIndexId(a.index), a.path.as_ref(), &a.value);
        assert_eq!(key.starts_with(&value_prefix), same_value, "{a:?} {b:?}");

        // Index entries never fall into the marker or entity key spaces.
        assert!(!key.starts_with(&keys::index_marker_key(LocalIndexId(a.index))));
        assert!(!key.starts_with(&keys::entity_prefix(LocalCollectionId(a.index))));
    }
}

#[test]
fn index_value_ranges_contain_exactly_the_values_within_bounds() {
    use std::ops::Bound;

    let (mut rng, _seed) = rng(0xB0D);
    for _ in 0..5_000 {
        let entry = IndexEntry::random(&mut rng);
        let (low, high) = (rng.value(1), rng.value(1));
        let bound = |rng: &mut Rng, value: &Value| match rng.below(3) {
            0 => Bound::Unbounded,
            1 => Bound::Included(value.clone()),
            _ => Bound::Excluded(value.clone()),
        };
        let (lower, upper) = (bound(&mut rng, &low), bound(&mut rng, &high));
        let within = match &lower {
            Bound::Unbounded => true,
            Bound::Included(low) => entry.value >= *low,
            Bound::Excluded(low) => entry.value > *low,
        } && match &upper {
            Bound::Unbounded => true,
            Bound::Included(high) => entry.value <= *high,
            Bound::Excluded(high) => entry.value < *high,
        };
        let key = entry.key();
        let in_range = keys::index_range(
            LocalIndexId(entry.index),
            entry.path.as_ref(),
            lower.as_ref(),
            upper.as_ref(),
        )
        .is_some_and(|(start, end)| key >= start && key < end);
        assert_eq!(in_range, within, "{entry:?} {lower:?} {upper:?}");
    }
}
