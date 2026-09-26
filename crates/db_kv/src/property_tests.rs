//! Seeded property and fuzz tests of the key layout, the entity payload
//! codecs and the field-name dictionaries (see `docs/testing.md`).
//!
//! Every test derives its generator from a fixed seed, optionally mixed with
//! `SEMANTIC_TEST_SEED`, and reports the effective seed when it fails.

use semantic_data::value::{Map, Object, Value, VariantValue};

use crate::test_values::Rng;

mod decode_fuzz;
mod engines;
mod field_dicts;
mod key_layout;
mod payloads;

/// Seed of a test: `base`, mixed with `SEMANTIC_TEST_SEED` when set.
fn seed(base: u64) -> u64 {
    match std::env::var("SEMANTIC_TEST_SEED") {
        Ok(value) => {
            let extra = value
                .trim()
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("SEMANTIC_TEST_SEED must be a u64, got {value:?}"));
            base ^ extra.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        }
        Err(_) => base,
    }
}

/// Prints the seed of a failing test while it unwinds.
struct SeedReport(u64);

impl Drop for SeedReport {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "property test failed with generator seed {:#x} (SEMANTIC_TEST_SEED={})",
                self.0,
                std::env::var("SEMANTIC_TEST_SEED").unwrap_or_else(|_| "<unset>".into())
            );
        }
    }
}

/// A generator seeded by [`seed`], and a guard reporting its seed.
fn rng(base: u64) -> (Rng, SeedReport) {
    let seed = seed(base);
    (Rng(seed), SeedReport(seed))
}

/// Strings including empty, long and non-ASCII ones.
fn any_string(rng: &mut Rng) -> String {
    match rng.below(12) {
        0 => String::new(),
        1 => "x".repeat(1 + rng.below(70_000) as usize),
        2 => "é☃𝄞\u{0}/\\\"'".repeat(1 + rng.below(8) as usize),
        _ => rng.string(),
    }
}

/// Values at the edges of their types.
fn edge_value(rng: &mut Rng) -> Value {
    let values = [
        Value::Void,
        Value::Null,
        Value::String(String::new()),
        Value::Bytes(Vec::new().into()),
        Value::List(Vec::new()),
        Value::Map(Map::new()),
        Value::Object(Object::new()),
        Value::I8(i8::MIN),
        Value::I128(i128::MIN),
        Value::I128(i128::MAX),
        Value::U128(u128::MAX),
        Value::I64(i64::MIN),
        Value::U64(u64::MAX),
        Value::from(f64::NAN),
        Value::from(-0.0f64),
        Value::from(f64::NEG_INFINITY),
        Value::from(f64::MAX),
        Value::from(f32::INFINITY),
        Value::from(-0.0f32),
        Value::from(f32::MIN_POSITIVE),
    ];
    values[rng.below(values.len() as u64) as usize].clone()
}

/// Random values nested up to `depth` levels with wider containers than
/// [`Rng::value`], empty containers, edge values and occasional long
/// strings and byte strings.
fn deep_value(rng: &mut Rng, depth: u32) -> Value {
    deep_value_with(rng, depth, true)
}

/// [`deep_value`] without long strings and byte strings, for tests that
/// encode many values.
fn nested_value(rng: &mut Rng, depth: u32) -> Value {
    deep_value_with(rng, depth, false)
}

fn deep_value_with(rng: &mut Rng, depth: u32, long: bool) -> Value {
    if depth == 0 || rng.below(3) == 0 {
        return match rng.below(10) {
            0 if long => Value::String(any_string(rng)),
            1 if long => Value::Bytes(vec![0xFF; rng.below(40_000) as usize].into()),
            2 | 3 => edge_value(rng),
            _ => rng.value(0),
        };
    }
    let width = rng.below(6);
    match rng.below(4) {
        0 => Value::List(
            (0..width)
                .map(|_| deep_value_with(rng, depth - 1, long))
                .collect(),
        ),
        1 => {
            let mut map = Map::new();
            for _ in 0..width {
                map.insert(
                    deep_value_with(rng, depth - 1, long),
                    deep_value_with(rng, depth - 1, long),
                );
            }
            Value::Map(map)
        }
        2 => {
            let mut object = Object::new();
            for _ in 0..rng.below(6) {
                object.insert(field_name(rng), deep_value_with(rng, depth - 1, long));
            }
            Value::Object(object)
        }
        _ => Value::Variant(Box::new(VariantValue {
            r#type: (rng.below(2) == 0).then(|| if long { any_string(rng) } else { rng.string() }),
            variant: rng.string(),
            value: deep_value_with(rng, depth - 1, long),
        })),
    }
}

/// A random object with field names from a small pool (so names repeat
/// across objects) plus occasional fresh names.
fn deep_object(rng: &mut Rng, depth: u32) -> Object {
    let mut object = Object::new();
    for _ in 0..rng.below(6) {
        object.insert(field_name(rng), deep_value(rng, depth));
    }
    object
}

fn field_name(rng: &mut Rng) -> String {
    match rng.below(10) {
        0 => rng.string(),
        1 => String::new(),
        _ => format!("f{}", rng.below(12)),
    }
}

/// `value` with one randomly chosen leaf or container element replaced,
/// inserted or removed, so the pair shares a long common prefix.
fn mutate(rng: &mut Rng, value: &Value) -> Value {
    let mut value = value.clone();
    mutate_in_place(rng, &mut value);
    value
}

fn mutate_in_place(rng: &mut Rng, value: &mut Value) {
    let descend = rng.below(5) != 0;
    match value {
        Value::List(items) if descend && !items.is_empty() => {
            let index = rng.below(items.len() as u64) as usize;
            match rng.below(4) {
                0 => {
                    items.remove(index);
                }
                1 => items.insert(index, rng.value(1)),
                _ => mutate_in_place(rng, &mut items[index]),
            }
        }
        Value::Object(object) if descend && !object.is_empty() => {
            let key = object
                .keys()
                .nth(rng.below(object.len() as u64) as usize)
                .cloned()
                .unwrap();
            if rng.below(3) == 0 {
                object.remove(&key);
            } else {
                mutate_in_place(rng, object.get_mut(&key).unwrap());
            }
        }
        Value::Map(map) if descend && !map.is_empty() => {
            let key = map
                .keys()
                .nth(rng.below(map.len() as u64) as usize)
                .cloned()
                .unwrap();
            let mut entry = map.remove(&key).unwrap();
            mutate_in_place(rng, &mut entry);
            map.insert(key, entry);
        }
        Value::Variant(variant) if descend => mutate_in_place(rng, &mut variant.value),
        _ => *value = rng.value(1),
    }
}
