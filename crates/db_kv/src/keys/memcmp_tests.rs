use std::cmp::Ordering;

use semantic_data::value::{Map, Object, Value, VariantValue};

use super::{decode, decode_prefix, encode, encode_string_prefix, encoded_len};

/// Deterministic SplitMix64 generator; keeps the property tests reproducible
/// without an external property-testing dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }

    /// Integers biased towards boundaries and small magnitudes so that equal
    /// values and near-collisions are common.
    fn int(&mut self) -> u128 {
        match self.below(4) {
            0 => self.pick(&[0, 1, 2, u128::MAX, u128::MAX - 1, 1 << 63, 1 << 127]),
            1 => self.below(8) as u128,
            2 => (self.next() as i8) as i128 as u128,
            _ => ((self.next() as u128) << 64) | self.next() as u128,
        }
    }

    fn f64(&mut self) -> f64 {
        match self.below(3) {
            0 => self.pick(&[
                0.0,
                -0.0,
                1.0,
                -1.0,
                f64::NAN,
                -f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::MIN_POSITIVE,
                -f64::MIN_POSITIVE,
                f64::MAX,
                f64::MIN,
                5e-324,
                -5e-324,
            ]),
            1 => self.below(5) as f64 - 2.0,
            _ => f64::from_bits(self.next()),
        }
    }

    fn bytes(&mut self) -> Vec<u8> {
        let len = self.below(5) as usize;
        (0..len)
            .map(|_| self.pick(&[0x00, 0x01, 0x02, b'a', b'b', 0xFE, 0xFF]))
            .collect()
    }

    fn string(&mut self) -> String {
        let len = self.below(5) as usize;
        (0..len)
            .map(|_| self.pick(&['\0', '\u{1}', 'a', 'b', 'z', '\u{ff}', '\u{10ffff}']))
            .collect()
    }

    fn value(&mut self, depth: u32) -> Value {
        let variants = if depth == 0 { 23 } else { 27 };
        match self.below(variants) {
            0 => Value::Void,
            1 => Value::Null,
            2 => Value::Bool(self.below(2) == 1),
            3 => Value::I8(self.int() as i8),
            4 => Value::I16(self.int() as i16),
            5 => Value::I32(self.int() as i32),
            6 => Value::I64(self.int() as i64),
            7 => Value::I128(self.int() as i128),
            8 => Value::U8(self.int() as u8),
            9 => Value::U16(self.int() as u16),
            10 => Value::U32(self.int() as u32),
            11 => Value::U64(self.int() as u64),
            12 => Value::U128(self.int()),
            13 => Value::from(self.f64() as f32),
            14 => Value::from(self.f64()),
            15 => Value::Uuid(uuid::Uuid::from_u128(self.int()).into()),
            16 => Value::IpAddr(if self.below(2) == 0 {
                std::net::IpAddr::from((self.int() as u32).to_be_bytes())
            } else {
                std::net::IpAddr::from(self.int().to_be_bytes())
            }),
            17 => {
                let seconds = self.int() as i64 / 2;
                let nanos = (self.below(1_000_000_000) as i32).copysign_like(seconds);
                Value::Duration(time::Duration::new(seconds, nanos).into())
            }
            18 => Value::Time(
                time::Time::from_hms_nano(
                    self.below(24) as u8,
                    self.below(60) as u8,
                    self.below(60) as u8,
                    self.below(3) as u32 * 499_999_999,
                )
                .unwrap()
                .into(),
            ),
            19 => {
                let min = time::Date::MIN.to_julian_day();
                let max = time::Date::MAX.to_julian_day();
                let day = if self.below(2) == 0 {
                    2_451_545 + self.below(4) as i32
                } else {
                    min + self.below((max - min + 1) as u64) as i32
                };
                Value::Date(time::Date::from_julian_day(day).unwrap().into())
            }
            20 => {
                let seconds = self.below(4_000_000_000) as i64 - 2_000_000_000;
                let offset =
                    time::UtcOffset::from_whole_seconds(self.pick(&[0, 3600, -7200, 45 * 60]))
                        .unwrap();
                Value::DateTime(
                    time::OffsetDateTime::from_unix_timestamp(seconds)
                        .unwrap()
                        .replace_nanosecond(self.below(3) as u32)
                        .unwrap()
                        .to_offset(offset)
                        .into(),
                )
            }
            21 => Value::Bytes(self.bytes().into()),
            22 => Value::String(self.string()),
            23 => Value::List((0..self.below(4)).map(|_| self.value(depth - 1)).collect()),
            24 => {
                let mut map = Map::new();
                for _ in 0..self.below(4) {
                    map.insert(self.value(depth - 1), self.value(depth - 1));
                }
                Value::Map(map)
            }
            25 => {
                let mut object = Object::new();
                for _ in 0..self.below(4) {
                    object.insert(self.string(), self.value(depth - 1));
                }
                Value::Object(object)
            }
            _ => Value::Variant(Box::new(VariantValue {
                r#type: (self.below(2) == 0).then(|| self.string()),
                variant: self.string(),
                value: self.value(depth - 1),
            })),
        }
    }
}

trait CopySignLike {
    fn copysign_like(self, sign_of: i64) -> Self;
}

impl CopySignLike for i32 {
    fn copysign_like(self, sign_of: i64) -> Self {
        if sign_of < 0 { -self } else { self }
    }
}

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
