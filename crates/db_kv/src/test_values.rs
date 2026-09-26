//! Seeded random [`Value`] generator shared by the codec property tests.

use semantic_data::value::{Map, Object, Value, VariantValue};

/// Deterministic SplitMix64 generator; keeps the property tests reproducible
/// without an external property-testing dependency.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub(crate) fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    pub(crate) fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }

    /// Integers biased towards boundaries and small magnitudes so that equal
    /// values and near-collisions are common.
    pub(crate) fn int(&mut self) -> u128 {
        match self.below(4) {
            0 => self.pick(&[0, 1, 2, u128::MAX, u128::MAX - 1, 1 << 63, 1 << 127]),
            1 => self.below(8) as u128,
            2 => (self.next() as i8) as i128 as u128,
            _ => ((self.next() as u128) << 64) | self.next() as u128,
        }
    }

    pub(crate) fn f64(&mut self) -> f64 {
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

    pub(crate) fn bytes(&mut self) -> Vec<u8> {
        let len = self.below(5) as usize;
        (0..len)
            .map(|_| self.pick(&[0x00, 0x01, 0x02, b'a', b'b', 0xFE, 0xFF]))
            .collect()
    }

    pub(crate) fn string(&mut self) -> String {
        let len = self.below(5) as usize;
        (0..len)
            .map(|_| self.pick(&['\0', '\u{1}', 'a', 'b', 'z', '\u{ff}', '\u{10ffff}']))
            .collect()
    }

    pub(crate) fn value(&mut self, depth: u32) -> Value {
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
