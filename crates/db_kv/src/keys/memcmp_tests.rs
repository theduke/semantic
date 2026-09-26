use std::cmp::Ordering;

use semantic_data::value::Value;

use super::{
    MAX_DEPTH, decode, decode_prefix, encode, encode_string_prefix, encoded_len, try_encode_into,
};
use crate::test_values::Rng;

fn assert_order_matches(a: &Value, b: &Value) {
    let (ea, eb) = (encode(a), encode(b));
    assert_eq!(
        ea.cmp(&eb),
        a.cmp(b),
        "byte order differs from value order\n a = {a:?}\n b = {b:?}\n ea = {ea:02x?}\n eb = {eb:02x?}"
    );
}

fn assert_round_trip(value: &Value) {
    let encoded = encode(value);
    let decoded = decode(&encoded).unwrap_or_else(|err| panic!("{value:?}: {err}"));
    assert_eq!(
        decoded.cmp(value),
        Ordering::Equal,
        "{value:?} vs {decoded:?}"
    );
    assert_eq!(encode(&decoded), encoded, "{value:?}");
    assert_eq!(encoded_len(&encoded).unwrap(), encoded.len());
    let mut with_suffix = encoded.clone();
    with_suffix.extend_from_slice(b"\x00\xffsuffix");
    assert_eq!(encoded_len(&with_suffix).unwrap(), encoded.len());
    assert_eq!(decode_prefix(&with_suffix).unwrap().1, encoded.len());
}

#[test]
fn random_pairs_preserve_order_and_round_trip() {
    let mut rng = Rng(0x5EED);
    for _ in 0..20_000 {
        let a = rng.value(2);
        let b = if rng.below(4) == 0 {
            a.clone()
        } else {
            rng.value(2)
        };
        assert_order_matches(&a, &b);
        assert_round_trip(&a);
    }
}

#[test]
fn same_variant_pairs_preserve_order() {
    // Pairs of the same variant exercise the per-variant payload encodings
    // far more often than fully random pairs.
    let mut rng = Rng(0xC0FFEE);
    let mut samples = 0;
    while samples < 20_000 {
        let a = rng.value(2);
        let b = rng.value(2);
        if std::mem::discriminant(&a) != std::mem::discriminant(&b) {
            continue;
        }
        samples += 1;
        assert_order_matches(&a, &b);
    }
}

#[test]
fn concatenated_suffixes_compare_after_the_value() {
    let mut rng = Rng(42);
    for _ in 0..5_000 {
        let (a, b) = (rng.value(1), rng.value(1));
        let (sa, sb) = (rng.bytes(), rng.bytes());
        let mut ka = encode(&a);
        ka.extend_from_slice(&sa);
        let mut kb = encode(&b);
        kb.extend_from_slice(&sb);
        assert_eq!(ka.cmp(&kb), a.cmp(&b).then(sa.cmp(&sb)), "{a:?} {b:?}");
    }
}

#[test]
fn float_edge_cases_follow_ordered_float() {
    let values = [
        f64::NEG_INFINITY,
        f64::MIN,
        -1.0,
        -5e-324,
        -0.0,
        0.0,
        5e-324,
        1.0,
        f64::MAX,
        f64::INFINITY,
        f64::NAN,
        -f64::NAN,
    ];
    for a in values {
        for b in values {
            assert_order_matches(&Value::from(a), &Value::from(b));
            assert_order_matches(&Value::from(a as f32), &Value::from(b as f32));
        }
        assert_round_trip(&Value::from(a));
        assert_round_trip(&Value::from(a as f32));
    }
}

#[test]
fn string_prefix_encoding_matches_exactly_the_prefixed_strings() {
    let mut rng = Rng(7);
    for _ in 0..5_000 {
        let prefix = rng.string();
        let value = rng.value(1);
        let matches = matches!(&value, Value::String(s) if s.starts_with(&prefix));
        assert_eq!(
            encode(&value).starts_with(&encode_string_prefix(&prefix)),
            matches,
            "{prefix:?} {value:?}"
        );
    }
}

#[test]
fn rejects_malformed_input() {
    assert!(decode(&[]).is_err());
    assert!(decode(&[0xEE]).is_err());
    assert!(decode(&[0x12, 0x02]).is_err());
    assert!(decode(&[0x26, b'a', 0x00, 0x07]).is_err());
    assert!(decode(&[0x26, b'a']).is_err());
    assert!(decode(&[0x11, 0x11]).is_err());
    assert!(encoded_len(&[0x27, 0x11]).is_err());
}

#[test]
fn nesting_is_limited_on_checked_encode_and_decode() {
    let nested = |depth: usize| {
        let mut value = Value::I64(1);
        for _ in 0..depth {
            value = Value::List(vec![value, Value::Null]);
        }
        value
    };
    let at_limit = nested(MAX_DEPTH);
    let mut key = Vec::new();
    try_encode_into(&at_limit, &mut key).unwrap();
    assert_eq!(key, encode(&at_limit));
    assert_eq!(decode(&key).unwrap(), at_limit);
    assert_eq!(encoded_len(&key).unwrap(), key.len());

    let beyond = nested(MAX_DEPTH + 1);
    let err = try_encode_into(&beyond, &mut Vec::new()).unwrap_err();
    assert!(err.to_string().contains("maximum depth"), "{err}");
    // The unchecked encoder still encodes it (for lookups), but the
    // decoders reject the result.
    let key = encode(&beyond);
    for err in [
        decode(&key).unwrap_err(),
        decode_prefix(&key).unwrap_err(),
        encoded_len(&key).unwrap_err(),
    ] {
        assert!(err.to_string().contains("maximum depth"), "{err}");
    }
}
