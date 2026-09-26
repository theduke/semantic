//! Deterministic row generation.
//!
//! Every row is a pure function of its ordinal and the collection size, so
//! benchmarks can restore rows they deleted or rewrote without keeping a
//! copy of the data set.

use semantic_data::value::{DateTime, Object, Value};

use crate::schema::{ITEM_CLASS, OWNER_CLASS, attr};

/// Distinct [`attr::KIND`] values; each covers 1 % of the items.
pub const KINDS: u64 = 100;
/// Items sharing one [`attr::CODE`] value.
pub const ROWS_PER_CODE: u64 = 4;
/// Items per owner.
pub const ITEMS_PER_OWNER: u64 = 10;
/// Words in the full-text vocabulary.
pub const VOCABULARY: u64 = 1024;

const SEED: u64 = 0x5eed_beac_4d0c_0001;
/// Unix time of the first item's `created_at` (2024-01-01T00:00:00Z).
const EPOCH: i64 = 1_704_067_200;
/// Multiplier of the score permutation; a prime that does not divide any
/// benchmark size, so `i -> i * P mod size` is a bijection.
const SCORE_PRIME: u64 = 1_000_003;
const SYLLABLES: [&str; 16] = [
    "ka", "lo", "mi", "ne", "ru", "sa", "ti", "vo", "ze", "pa", "do", "fu", "gi", "ha", "je", "wo",
];
const TAGS: [&str; 12] = [
    "new", "sale", "popular", "limited", "eco", "premium", "bundle", "gift", "outdoor", "kids",
    "office", "travel",
];
const REGIONS: [&str; 6] = [
    "eu-west",
    "eu-central",
    "us-east",
    "us-west",
    "ap-south",
    "sa-east",
];

/// Deterministic SplitMix64 generator.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Word `index` of the synthetic full-text vocabulary.
pub fn word(index: u64) -> String {
    let mut out = String::new();
    let mut rest = index;
    for _ in 0..3 {
        out.push_str(SYLLABLES[(rest % 16) as usize]);
        rest /= 16;
    }
    out
}

/// Words are skewed towards low indexes, like natural language.
fn text(rng: &mut Rng, words: usize) -> String {
    (0..words)
        .map(|_| {
            let u = rng.unit();
            word((u * u * VOCABULARY as f64) as u64)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn item_id(ordinal: u64) -> String {
    format!("item-{ordinal:07}")
}

pub fn owner_id(ordinal: u64) -> String {
    format!("owner-{ordinal:06}")
}

pub fn kind(kind: u64) -> String {
    format!("kind-{:02}", kind % KINDS)
}

pub fn code(ordinal: u64) -> String {
    format!("code-{:07}", ordinal / ROWS_PER_CODE)
}

/// Owners referenced by a collection of `size` items.
pub fn owner_count(size: u64) -> u64 {
    (size / ITEMS_PER_OWNER).max(1)
}

/// Unique score of item `ordinal` (a permutation of `0..size`).
pub fn score(ordinal: u64, size: u64) -> i64 {
    ((ordinal % size) * SCORE_PRIME % size) as i64
}

/// Item `ordinal` of a collection of `size` items.
pub fn item(ordinal: u64, size: u64) -> Object {
    let mut rng = Rng::new(SEED ^ ordinal.wrapping_mul(0x2545_F491_4F6C_DD1D));
    let mut row = Object::new();
    row.insert("id", Value::String(item_id(ordinal)));
    row.insert("type", Value::String(ITEM_CLASS.to_string()));
    row.insert(attr::TITLE, Value::String(text(&mut rng, 4)));
    row.insert(attr::BODY, Value::String(text(&mut rng, 20)));
    row.insert(attr::KIND, Value::String(kind(ordinal)));
    row.insert(attr::CODE, Value::String(code(ordinal)));
    row.insert(attr::SCORE, Value::I64(score(ordinal, size)));
    let cents = rng.below(100_000) as f64;
    row.insert(attr::PRICE, Value::F64((cents / 100.0).into()));
    row.insert(attr::RATING, Value::I64(rng.below(6) as i64));
    row.insert(attr::ACTIVE, Value::Bool(rng.below(4) != 0));
    let created = EPOCH + ordinal as i64 * 60 + rng.below(60) as i64;
    let created = time::OffsetDateTime::from_unix_timestamp(created).expect("valid timestamp");
    row.insert(attr::CREATED_AT, Value::DateTime(DateTime::from(created)));
    let tags = (0..rng.below(4))
        .map(|_| Value::String(TAGS[rng.below(TAGS.len() as u64) as usize].to_string()))
        .collect();
    row.insert(attr::TAGS, Value::List(tags));
    let mut dimensions = Object::new();
    dimensions.insert("width", Value::F64((rng.below(2000) as f64 / 10.0).into()));
    dimensions.insert("height", Value::F64((rng.below(2000) as f64 / 10.0).into()));
    dimensions.insert("unit", Value::String("cm".to_string()));
    row.insert(attr::DIMENSIONS, Value::Object(dimensions));
    row.insert(
        attr::OWNER,
        Value::String(owner_id(ordinal % owner_count(size))),
    );
    row
}

/// Owner `ordinal`.
pub fn owner(ordinal: u64) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(owner_id(ordinal)));
    row.insert("type", Value::String(OWNER_CLASS.to_string()));
    row.insert(
        attr::NAME,
        Value::String(format!("{} {}", word(ordinal % 97), word(ordinal % 89))),
    );
    row.insert(
        attr::REGION,
        Value::String(REGIONS[(ordinal % REGIONS.len() as u64) as usize].to_string()),
    );
    row.insert(
        attr::EMAIL,
        Value::String(format!("owner{ordinal}@example.com")),
    );
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_are_a_permutation() {
        for size in [100, 10_000] {
            let mut scores = (0..size).map(|i| score(i, size)).collect::<Vec<_>>();
            scores.sort_unstable();
            assert_eq!(scores, (0..size as i64).collect::<Vec<_>>());
        }
    }

    #[test]
    fn rows_are_deterministic() {
        assert_eq!(item(42, 1000), item(42, 1000));
        assert_ne!(item(42, 1000), item(43, 1000));
    }
}
