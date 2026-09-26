use std::sync::Arc;
use std::time::Instant;

use semantic_data::value::{Object, Value};
use semantic_db_core::DbError;
use semantic_db_core::catalog::LocalCollectionId;
use semantic_db_core::embedded::{StoredEntity, StoredEntityKind};

use super::{
    ENTITY_FORMAT_VERSION_V2_COMPACT, decode_entity, decode_stored, encode_compact, encode_entity,
    payload_version,
};
use crate::storage::field_dict::FieldDict;
use crate::test_values::Rng;

const COLLECTION: LocalCollectionId = LocalCollectionId(7);

fn datetime(unix_millis: i64) -> Value {
    Value::DateTime(
        time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(unix_millis) * 1_000_000)
            .unwrap()
            .into(),
    )
}

/// A realistic typed row: 12 fields with qualified names, a nested object,
/// a list and datetimes.
fn realistic_entity(n: u64) -> StoredEntity {
    let id = format!("note-{n:08}");
    let mut payload = Object::new();
    payload.insert("example:kind", Value::String("markdown".to_string()));
    payload.insert("example:revision", Value::I64(n as i64 % 17));
    payload.insert(
        "example:checksum",
        Value::String(format!("{:016x}", n.wrapping_mul(0x9E37_79B9_7F4A_7C15))),
    );
    let mut object = Object::new();
    object.insert("id", Value::String(id.clone()));
    object.insert("semantic:type", Value::String("semantic:Note".to_string()));
    object.insert(
        "semantic:title",
        Value::String(format!("Meeting notes for project {n}")),
    );
    object.insert(
        "semantic:created_at",
        datetime(1_758_000_000_000 + n as i64),
    );
    object.insert(
        "semantic:updated_at",
        datetime(1_758_000_500_000 + n as i64),
    );
    object.insert(
        "semantic:tags",
        Value::List(vec![
            Value::String("work".to_string()),
            Value::String("planning".to_string()),
            Value::String("q3".to_string()),
        ]),
    );
    object.insert("semantic:parent", Value::String("folder-0001".to_string()));
    object.insert("example:payload", Value::Object(payload));
    object.insert("example:priority", Value::I64(3));
    object.insert("example:score", Value::from(0.75f64));
    object.insert("example:archived", Value::Bool(false));
    object.insert(
        "example:owner",
        Value::Uuid(uuid::Uuid::from_u128(0x1234_5678_9ABC_DEF0_u128 + u128::from(n)).into()),
    );
    assert_eq!(object.len(), 12);
    StoredEntity {
        id,
        collection: COLLECTION.0,
        kind: StoredEntityKind::Class,
        object,
    }
}

fn encode_v2(entity: &StoredEntity, dict: &mut FieldDict) -> Vec<u8> {
    encode_compact(entity, &mut |name: &str| match dict.id(name) {
        Some(id) => Ok(id),
        None => dict.push(name),
    })
    .unwrap()
}

fn decode_v2(entity: &StoredEntity, payload: &[u8], dict: &Arc<FieldDict>) -> StoredEntity {
    decode_stored(
        LocalCollectionId(entity.collection),
        &entity.id,
        payload,
        &mut |_| Ok(dict.clone()),
    )
    .unwrap()
}

#[test]
fn compact_payload_is_at_least_40_percent_smaller() {
    let entity = realistic_entity(42);
    let v1 = encode_entity(&entity).unwrap();
    let mut dict = FieldDict::default();
    let v2 = encode_v2(&entity, &mut dict);
    let saved = 100.0 * (1.0 - v2.len() as f64 / v1.len() as f64);
    println!(
        "entity payload sizes: v1 {} bytes, v2 {} bytes ({saved:.1}% smaller)",
        v1.len(),
        v2.len()
    );
    assert!(
        v2.len() * 10 <= v1.len() * 6,
        "v1 {} bytes, v2 {} bytes",
        v1.len(),
        v2.len()
    );
    assert_eq!(decode_v2(&entity, &v2, &Arc::new(dict)), entity);
    assert_eq!(decode_entity(&v1).unwrap(), entity);
}

/// Sanity comparison of decode speed; prints timings, asserts nothing about
/// time.
#[test]
fn decode_speed_comparison() {
    const ROWS: u64 = 10_000;
    let entities = (0..ROWS).map(realistic_entity).collect::<Vec<_>>();
    let mut dict = FieldDict::default();
    let v1 = entities
        .iter()
        .map(|entity| encode_entity(entity).unwrap())
        .collect::<Vec<_>>();
    let v2 = entities
        .iter()
        .map(|entity| encode_v2(entity, &mut dict))
        .collect::<Vec<_>>();
    let dict = Arc::new(dict);

    let start = Instant::now();
    let decoded_v1 = v1
        .iter()
        .map(|payload| decode_entity(payload).unwrap())
        .collect::<Vec<_>>();
    let v1_time = start.elapsed();
    let start = Instant::now();
    let decoded_v2 = entities
        .iter()
        .zip(&v2)
        .map(|(entity, payload)| decode_v2(entity, payload, &dict))
        .collect::<Vec<_>>();
    let v2_time = start.elapsed();

    println!(
        "decoded {ROWS} rows: v1 {v1_time:?} ({} bytes total), v2 {v2_time:?} ({} bytes total)",
        v1.iter().map(Vec::len).sum::<usize>(),
        v2.iter().map(Vec::len).sum::<usize>(),
    );
    assert_eq!(decoded_v1, entities);
    assert_eq!(decoded_v2, entities);
}

#[test]
fn compact_payload_round_trips_kinds_and_ids() {
    let mut dict = FieldDict::default();
    let mut matching = Object::new();
    matching.insert("id", Value::String("one".to_string()));
    matching.insert("x", Value::I64(1));
    let mut different = Object::new();
    different.insert("id", Value::String("other".to_string()));
    let mut non_string = Object::new();
    non_string.insert("id", Value::I64(1));
    let cases = [
        (StoredEntityKind::Untyped, matching.clone()),
        (StoredEntityKind::Record, different),
        (StoredEntityKind::Class, non_string),
        (StoredEntityKind::Untyped, Object::new()),
    ];
    for (kind, object) in cases {
        let entity = StoredEntity {
            id: "one".to_string(),
            collection: COLLECTION.0,
            kind,
            object,
        };
        let payload = encode_v2(&entity, &mut dict);
        assert_eq!(
            payload_version(&payload).unwrap(),
            ENTITY_FORMAT_VERSION_V2_COMPACT
        );
        assert_eq!(
            decode_v2(&entity, &payload, &Arc::new(dict.clone())),
            entity
        );
    }
    // The id field is elided when it matches the key: only `x` is stored.
    let entity = StoredEntity {
        id: "one".to_string(),
        collection: COLLECTION.0,
        kind: StoredEntityKind::Untyped,
        object: matching,
    };
    assert_eq!(encode_v2(&entity, &mut dict), [2, 0, 0x80, 1, 0, 0x07, 2]);
}

#[test]
fn random_objects_round_trip() {
    let mut rng = Rng(0xE7717);
    let mut dict = FieldDict::default();
    for n in 0..2_000 {
        let mut object = Object::new();
        for _ in 0..rng.below(6) {
            object.insert(rng.string(), rng.value(2));
        }
        let entity = StoredEntity {
            id: format!("e{n}"),
            collection: COLLECTION.0,
            kind: StoredEntityKind::Untyped,
            object,
        };
        let payload = encode_v2(&entity, &mut dict);
        let decoded = decode_v2(&entity, &payload, &Arc::new(dict.clone()));
        assert_eq!(decoded.id, entity.id);
        assert_eq!(decoded.kind, entity.kind);
        assert_eq!(
            Value::Object(decoded.object).cmp(&Value::Object(entity.object)),
            std::cmp::Ordering::Equal
        );
    }
}

#[test]
fn decoding_reloads_a_stale_dictionary_once() {
    let entity = realistic_entity(1);
    let mut dict = FieldDict::default();
    let payload = encode_v2(&entity, &mut dict);
    let full = Arc::new(dict);
    let mut calls = Vec::new();
    let decoded = decode_stored(COLLECTION, &entity.id, &payload, &mut |fresh| {
        calls.push(fresh);
        Ok(if fresh {
            full.clone()
        } else {
            Arc::new(FieldDict::default())
        })
    })
    .unwrap();
    assert_eq!(decoded, entity);
    assert_eq!(calls, [false, true]);

    let err = decode_stored(COLLECTION, &entity.id, &payload, &mut |_| {
        Ok(Arc::new(FieldDict::default()))
    })
    .unwrap_err();
    assert!(matches!(err, DbError::Deserialization(_)), "{err:?}");
}

#[test]
fn self_contained_decoder_rejects_compact_payloads() {
    let entity = realistic_entity(3);
    let payload = encode_v2(&entity, &mut FieldDict::default());
    assert!(decode_entity(&payload).is_err());
    assert!(decode_entity(&[9, 0]).is_err());
    assert!(decode_entity(&[1]).is_err());
}
