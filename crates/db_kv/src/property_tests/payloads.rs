//! Entity payload round trips (compact version 2 and self-contained
//! version 1) for deep random objects, and memcomparable order of nested
//! values.
//!
//! Documented float behaviour: the compact payload stores the exact bits of
//! every float (NaN payloads and `-0.0` survive), while the memcomparable
//! key encoding canonicalises NaN and `-0.0` (they compare equal to the
//! canonical NaN and `0.0` under `Value::cmp`, which the key order follows).

use std::cmp::Ordering;
use std::sync::Arc;

use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::LocalCollectionId;
use semantic_db_core::embedded::{StoredEntity, StoredEntityKind};

use super::{deep_object, deep_value, edge_value, mutate, nested_value, rng};
use crate::keys::memcmp;
use crate::storage::entity_codec::{decode_entity, decode_stored, encode_compact, encode_entity};
use crate::storage::field_dict::FieldDict;
use crate::test_values::Rng;

const COLLECTION: LocalCollectionId = LocalCollectionId(3);

fn random_entity(rng: &mut Rng) -> StoredEntity {
    let id = if rng.below(8) == 0 {
        String::new()
    } else {
        format!("e{}", rng.below(1000))
    };
    let mut object = deep_object(rng, 4);
    match rng.below(4) {
        // The id field equals the key: elided from compact payloads.
        0 | 1 => {
            object.insert("id", Value::String(id.clone()));
        }
        2 => {
            object.insert("id", deep_value(rng, 1));
        }
        _ => {}
    }
    for _ in 0..rng.below(3) {
        object.insert(format!("edge{}", rng.below(4)), edge_value(rng));
    }
    StoredEntity {
        id,
        collection: COLLECTION.0,
        kind: [
            StoredEntityKind::Untyped,
            StoredEntityKind::Record,
            StoredEntityKind::Class,
        ][rng.below(3) as usize]
            .clone(),
        object,
    }
}

/// Encode `entity` compactly, growing `dict` with its field names.
pub(super) fn encode_v2(entity: &StoredEntity, dict: &mut FieldDict) -> Vec<u8> {
    encode_compact(entity, &mut |name: &str| match dict.id(name) {
        Some(id) => Ok(id),
        None => dict.push(name),
    })
    .unwrap()
}

pub(super) fn decode_v2(
    id: &str,
    payload: &[u8],
    dict: &Arc<FieldDict>,
) -> Result<StoredEntity, semantic_db_core::DbError> {
    decode_stored(COLLECTION, id, payload, &mut |_| Ok(dict.clone()))
}

/// Values equal under `Value::cmp`, with floats compared bitwise.
fn assert_same_value(expected: &Value, actual: &Value, context: &str) {
    assert_eq!(expected.cmp(actual), Ordering::Equal, "{context}");
    match (expected, actual) {
        (Value::F64(a), Value::F64(b)) => {
            assert_eq!(a.0.to_bits(), b.0.to_bits(), "{context}: f64 bits")
        }
        (Value::F32(a), Value::F32(b)) => {
            assert_eq!(a.0.to_bits(), b.0.to_bits(), "{context}: f32 bits")
        }
        (Value::List(a), Value::List(b)) => {
            for (a, b) in a.iter().zip(b) {
                assert_same_value(a, b, context);
            }
        }
        (Value::Object(a), Value::Object(b)) => assert_same_object(a, b, context),
        (Value::Map(a), Value::Map(b)) => {
            for ((_, a), (_, b)) in a.iter().zip(b.iter()) {
                assert_same_value(a, b, context);
            }
        }
        (Value::Variant(a), Value::Variant(b)) => {
            assert_eq!(a.r#type, b.r#type, "{context}");
            assert_eq!(a.variant, b.variant, "{context}");
            assert_same_value(&a.value, &b.value, context);
        }
        _ => {}
    }
}

fn assert_same_object(expected: &Object, actual: &Object, context: &str) {
    assert_eq!(
        expected.keys().collect::<Vec<_>>(),
        actual.keys().collect::<Vec<_>>(),
        "{context}"
    );
    for (key, value) in expected.iter() {
        assert_same_value(value, actual.get(key).unwrap(), context);
    }
}

#[test]
fn compact_payloads_round_trip_deep_random_entities() {
    let (mut rng, _seed) = rng(0xD1C7);
    let mut dict = FieldDict::default();
    for case in 0..1_500 {
        let entity = random_entity(&mut rng);
        let payload = encode_v2(&entity, &mut dict);
        let snapshot = Arc::new(dict.clone());
        let decoded = decode_v2(&entity.id, &payload, &snapshot)
            .unwrap_or_else(|err| panic!("case {case}: {err}"));
        assert_eq!(decoded.id, entity.id);
        assert_eq!(decoded.collection, entity.collection);
        assert_eq!(decoded.kind, entity.kind);
        assert_same_object(&entity.object, &decoded.object, &format!("case {case}"));
        // Re-encoding is stable and a larger (newer) dictionary decodes it
        // too.
        assert_eq!(encode_v2(&decoded, &mut dict), payload, "case {case}");
    }
    let newest = Arc::new(dict);
    assert!(newest.len() > 12, "field names repeat and grow");
}

#[test]
fn compact_payloads_with_unknown_field_ids_fail_cleanly() {
    let (mut rng, _seed) = rng(0x0DD);
    for _ in 0..500 {
        let mut entity = random_entity(&mut rng);
        let mut dict = FieldDict::default();
        encode_v2(&entity, &mut dict);
        let older = Arc::new(dict.clone());
        entity
            .object
            .insert("unknown-to-an-older-dictionary", Value::Null);
        let payload = encode_v2(&entity, &mut dict);
        assert_eq!(dict.len(), older.len() + 1);
        // A dictionary missing a name used by the payload cannot decode it.
        assert!(decode_v2(&entity.id, &payload, &older).is_err());
        assert!(decode_v2(&entity.id, &payload, &Arc::new(dict)).is_ok());
    }
}

#[test]
fn self_contained_payloads_round_trip_deep_random_entities() {
    let (mut rng, _seed) = rng(0x5E1F);
    for case in 0..300 {
        let mut entity = random_entity(&mut rng);
        // Version 1 stores durations as wrapping whole milliseconds (see
        // `self_contained_payloads_preserve_durations`).
        entity.object = match to_v1_durations(Value::Object(entity.object)) {
            Value::Object(object) => object,
            _ => unreachable!(),
        };
        let payload = encode_entity(&entity).unwrap();
        let decoded = decode_entity(&payload).unwrap_or_else(|err| panic!("case {case}: {err}"));
        assert_eq!(decoded.id, entity.id);
        assert_eq!(decoded.kind, entity.kind);
        if decoded.object.cmp(&entity.object) != Ordering::Equal {
            panic!(
                "case {case}: {}",
                first_difference(
                    &Value::Object(entity.object),
                    &Value::Object(decoded.object)
                )
            );
        }
    }
}

/// Debug output of `value`, shortened.
fn short(value: &Value) -> String {
    let mut text = format!("{value:?}");
    if text.len() > 300 {
        let mut end = 300;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...");
    }
    text
}

/// The innermost differing pair of values of `expected` and `actual`.
pub(super) fn first_difference(expected: &Value, actual: &Value) -> String {
    match (expected, actual) {
        (Value::Object(a), Value::Object(b)) if a.len() == b.len() => {
            for ((ka, va), (kb, vb)) in a.iter().zip(b.iter()) {
                if ka != kb {
                    return format!("field {ka:?} vs {kb:?}");
                }
                if va.cmp(vb) != Ordering::Equal {
                    return format!("in {ka:?}: {}", first_difference(va, vb));
                }
            }
        }
        (Value::List(a), Value::List(b)) if a.len() == b.len() => {
            for (index, (va, vb)) in a.iter().zip(b).enumerate() {
                if va.cmp(vb) != Ordering::Equal {
                    return format!("at [{index}]: {}", first_difference(va, vb));
                }
            }
        }
        (Value::Map(a), Value::Map(b)) if a.len() == b.len() => {
            for ((ka, va), (kb, vb)) in a.iter().zip(b.iter()) {
                if ka.cmp(kb) != Ordering::Equal {
                    return format!("map key {} vs {}", short(ka), short(kb));
                }
                if va.cmp(vb) != Ordering::Equal {
                    return format!("at map key {}: {}", short(ka), first_difference(va, vb));
                }
            }
        }
        (Value::Variant(a), Value::Variant(b))
            if a.r#type == b.r#type && a.variant == b.variant =>
        {
            return format!("in variant: {}", first_difference(&a.value, &b.value));
        }
        _ => {}
    }
    format!("expected {}\n  actual {}", short(expected), short(actual))
}

/// `value` with every duration reduced to what version 1 payloads keep.
pub(super) fn to_v1_durations(value: Value) -> Value {
    match value {
        Value::Duration(duration) => {
            let raw: time::Duration = duration.into();
            Value::Duration(time::Duration::milliseconds(raw.whole_milliseconds() as i64).into())
        }
        Value::List(items) => Value::List(items.into_iter().map(to_v1_durations).collect()),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, to_v1_durations(value)))
                .collect(),
        ),
        Value::Map(map) => {
            let mut out = semantic_data::value::Map::new();
            for (key, value) in map.into_btree() {
                out.insert(to_v1_durations(key), to_v1_durations(value));
            }
            Value::Map(out)
        }
        Value::Variant(mut variant) => {
            variant.value = to_v1_durations(variant.value);
            Value::Variant(variant)
        }
        other => other,
    }
}

#[test]
#[ignore = "bug: self-contained (version 1) payloads, still written by the postgres managed \
            store, keep durations as `whole_milliseconds() as i64`: sub-millisecond precision \
            is lost and durations beyond ~292 million years wrap around"]
fn self_contained_payloads_preserve_durations() {
    for duration in [
        time::Duration::new(3, 68_338_061),
        time::Duration::new(i64::MAX / 2, 0),
    ] {
        let mut object = Object::new();
        object.insert("d", Value::Duration(duration.into()));
        let entity = StoredEntity {
            id: "e".into(),
            collection: COLLECTION.0,
            kind: StoredEntityKind::Untyped,
            object,
        };
        let decoded = decode_entity(&encode_entity(&entity).unwrap()).unwrap();
        assert_eq!(decoded.object, entity.object);
    }
}

#[test]
fn memcmp_order_matches_value_order_for_nested_values() {
    let (mut rng, _seed) = rng(0x0E5);
    for _ in 0..10_000 {
        let a = nested_value(&mut rng, 3);
        // Mutations share long prefixes with `a`, so the comparison is
        // decided deep inside nested containers.
        let b = match rng.below(3) {
            0 => nested_value(&mut rng, 3),
            _ => mutate(&mut rng, &a),
        };
        let (ea, eb) = (memcmp::encode(&a), memcmp::encode(&b));
        assert_eq!(
            ea.cmp(&eb),
            a.cmp(&b),
            "byte order differs from value order\n a = {a:?}\n b = {b:?}"
        );
        let decoded = memcmp::decode(&ea).unwrap_or_else(|err| panic!("{a:?}: {err}"));
        assert_eq!(decoded.cmp(&a), Ordering::Equal);
        assert_eq!(memcmp::encoded_len(&ea).unwrap(), ea.len());
    }
}

#[test]
fn memcmp_canonicalises_nan_and_negative_zero() {
    for (value, canonical) in [
        (Value::from(-0.0f64), Value::from(0.0f64)),
        (Value::from(-f64::NAN), Value::from(f64::NAN)),
        (Value::from(-0.0f32), Value::from(0.0f32)),
    ] {
        assert_eq!(value.cmp(&canonical), Ordering::Equal);
        assert_eq!(memcmp::encode(&value), memcmp::encode(&canonical));
    }
}
