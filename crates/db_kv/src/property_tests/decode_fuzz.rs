//! Decoder robustness: random and mutated (bit-flipped, truncated,
//! extended, spliced) inputs to every decoder of stored bytes must yield
//! `Ok` or `Err`, never panic, hang or allocate out of proportion to the
//! input.
//!
//! A tracking global allocator records the largest single allocation of the
//! fuzzing thread; each case must stay below a generous cap derived from the
//! input size, and must finish within a per-case time limit.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::{LocalCollectionId, LocalIndexId};
use semantic_db_core::embedded::{EntityStorage, StorageWriteOp, StoredEntity, StoredEntityKind};

use super::payloads::{decode_v2, encode_v2};
use super::rng;
use crate::keys::{self, legacy, memcmp};
use crate::storage::entity_codec::{decode_entity, encode_entity};
use crate::storage::field_dict::FieldDict;
use crate::storage::layout::decode_layout_version;
use crate::test_values::Rng;
use crate::{EntityStore, KvEngine, MemoryKvEngine};

struct Tracking;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static LARGEST: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
    let _ = TRACKING.try_with(|tracking| {
        if tracking.get() {
            let _ = LARGEST.try_with(|largest| largest.set(largest.get().max(size)));
        }
    });
}

// SAFETY: forwards to the system allocator; only records sizes.
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Tracking = Tracking;

/// Per-case limits: generous for debug builds, far below a hang or an
/// allocation driven by a corrupt length.
const CASE_TIME_LIMIT: Duration = Duration::from_secs(2);
const ALLOCATION_FLOOR: usize = 4 << 20;
const ALLOCATION_PER_INPUT_BYTE: usize = 128;

/// Run `decode` on `input` and check it neither panics nor exceeds the
/// limits.
fn check_case(target: &str, input: &[u8], decode: &dyn Fn(&[u8])) {
    LARGEST.with(|largest| largest.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    let started = Instant::now();
    let outcome = catch_unwind(AssertUnwindSafe(|| decode(input)));
    let elapsed = started.elapsed();
    TRACKING.with(|tracking| tracking.set(false));
    let largest = LARGEST.with(Cell::get);
    let shown = &input[..input.len().min(96)];
    if outcome.is_err() {
        panic!(
            "{target}: decoder panicked on {} bytes {shown:02x?}",
            input.len()
        );
    }
    assert!(
        elapsed < CASE_TIME_LIMIT,
        "{target}: decoding {} bytes took {elapsed:?}: {shown:02x?}",
        input.len()
    );
    let cap = ALLOCATION_FLOOR.max(input.len() * ALLOCATION_PER_INPUT_BYTE);
    assert!(
        largest <= cap,
        "{target}: decoding {} bytes allocated {largest} bytes at once: {shown:02x?}",
        input.len()
    );
}

/// A random mutation of `valid`.
fn mutated(rng: &mut Rng, valid: &[u8]) -> Vec<u8> {
    let mut bytes = valid.to_vec();
    for _ in 0..1 + rng.below(3) {
        match rng.below(7) {
            0 if !bytes.is_empty() => {
                let bit = rng.below(bytes.len() as u64 * 8);
                bytes[(bit / 8) as usize] ^= 1 << (bit % 8);
            }
            1 if !bytes.is_empty() => {
                let len = rng.below(bytes.len() as u64) as usize;
                bytes.truncate(len);
            }
            2 => bytes.extend((0..1 + rng.below(16)).map(|_| rng.next() as u8)),
            3 if !bytes.is_empty() => {
                let at = rng.below(bytes.len() as u64) as usize;
                bytes[at] = rng.pick(&[0x00, 0x7F, 0x80, 0xFF]);
            }
            4 if !bytes.is_empty() => {
                // Corrupt a length or varint into a huge value.
                let at = rng.below(bytes.len() as u64) as usize;
                let run = (1 + rng.below(18) as usize).min(bytes.len() - at);
                bytes[at..at + run].fill(0xFF);
            }
            5 if !bytes.is_empty() => {
                let at = rng.below(bytes.len() as u64) as usize;
                let chunk = bytes[at..].to_vec();
                bytes.splice(at..at, chunk.into_iter().take(32));
            }
            _ => {
                let at = rng.below(bytes.len() as u64 + 1) as usize;
                bytes.splice(at..at, (0..rng.below(8)).map(|_| rng.next() as u8));
            }
        }
    }
    bytes
}

fn random_bytes(rng: &mut Rng) -> Vec<u8> {
    (0..rng.below(80)).map(|_| rng.next() as u8).collect()
}

/// Run `cases` random and mutated inputs derived from `corpus` through
/// `decode`.
fn fuzz(target: &str, base_seed: u64, cases: usize, corpus: &[Vec<u8>], decode: &dyn Fn(&[u8])) {
    let (mut rng, _seed) = rng(base_seed);
    for valid in corpus {
        check_case(target, valid, decode);
    }
    for _ in 0..cases {
        let input = if corpus.is_empty() || rng.below(5) == 0 {
            random_bytes(&mut rng)
        } else {
            let valid = &corpus[rng.below(corpus.len() as u64) as usize];
            mutated(&mut rng, valid)
        };
        check_case(target, &input, decode);
    }
}

fn entity(rng: &mut Rng, id: &str) -> StoredEntity {
    let mut object = Object::new();
    for _ in 0..rng.below(6) {
        object.insert(super::field_name(rng), super::nested_value(rng, 3));
    }
    object.insert("id", Value::String(id.to_string()));
    StoredEntity {
        id: id.to_string(),
        collection: 1,
        kind: StoredEntityKind::Untyped,
        object,
    }
}

#[test]
fn entity_payload_decoders_reject_corrupt_input() {
    let (mut rng, _seed) = rng(0xF022);
    let mut dict = FieldDict::default();
    let entities = (0..40)
        .map(|index| entity(&mut rng, &format!("e{index}")))
        .collect::<Vec<_>>();
    let compact = entities
        .iter()
        .map(|entity| encode_v2(entity, &mut dict))
        .collect::<Vec<_>>();
    let dict = Arc::new(dict);
    let self_contained = entities
        .iter()
        .map(|entity| encode_entity(entity).unwrap())
        .collect::<Vec<_>>();

    fuzz("compact payload", 1, 4_000, &compact, &|bytes| {
        let _ = decode_v2("e0", bytes, &dict);
    });
    fuzz(
        "self-contained payload",
        2,
        4_000,
        &self_contained,
        &|bytes| {
            let _ = decode_entity(bytes);
        },
    );
}

#[test]
fn memcmp_decoder_rejects_corrupt_input() {
    let (mut rng, _seed) = rng(0xF0CC);
    let corpus = (0..60)
        .map(|_| memcmp::encode(&super::nested_value(&mut rng, 3)))
        .collect::<Vec<_>>();
    fuzz("memcmp", 3, 5_000, &corpus, &|bytes| {
        let _ = memcmp::decode(bytes);
        let _ = memcmp::decode_prefix(bytes);
        let _ = memcmp::encoded_len(bytes);
    });
}

#[test]
fn key_parsers_reject_corrupt_keys() {
    let (mut rng, _seed) = rng(0xF0EE);
    let path = semantic_data::value::FieldPath::from_fields(["a", "b"]);
    let mut corpus = Vec::new();
    for index in 0..30usize {
        let value = rng.value(2);
        corpus.push(keys::entity_key(LocalCollectionId(index * 37), "some/id"));
        corpus.push(keys::index_key(LocalIndexId(index), None, &value, "id"));
        corpus.push(keys::index_key(
            LocalIndexId(index),
            Some(&path),
            &value,
            "id",
        ));
        corpus.push(keys::field_dict_key(LocalCollectionId(index), index as u32));
        corpus.push(keys::collection_rows_key(LocalCollectionId(index)));
        corpus.push(legacy::entity_key(LocalCollectionId(index), "some/e/id"));
        corpus.push(legacy::index_key(LocalIndexId(index), Some(&path), &value, "id").unwrap());
    }
    corpus.push(vec![0, 0, 0, 2]);
    fuzz("key parsers", 4, 8_000, &corpus, &|bytes| {
        let _ = keys::parse_entity_key(bytes);
        let _ = keys::parse_field_dict_key(bytes);
        let _ = keys::decode_lid(bytes);
        let _ = keys::counter_key_for(bytes);
        let _ = keys::index_key_entity_id(bytes, LocalIndexId(3), true);
        let _ = keys::index_key_entity_id(bytes, LocalIndexId(3), false);
        let _ = keys::index_entry_id(bytes, 0);
        let _ = legacy::parse_entity_key(bytes);
        let _ = decode_layout_version(bytes);
        let _ = legacy::downgrade_entries([(bytes.to_vec(), Vec::new())], &BTreeSet::new());
    });
}

/// A store holding `entries` verbatim.
fn store_with(entries: &[(Vec<u8>, Vec<u8>)]) -> EntityStore<MemoryKvEngine> {
    let mut engine = MemoryKvEngine::new();
    for (key, value) in entries {
        engine.put(key.clone(), value.clone()).unwrap();
    }
    EntityStore::new(engine)
}

/// Read everything a store can decode; errors are fine, panics are not.
fn read_everything(store: &EntityStore<MemoryKvEngine>) {
    for lid in 0..4 {
        let collection = LocalCollectionId(lid);
        let _ = store.collection_row_count(collection);
        let _ = store.index_entry_count(LocalIndexId(lid));
        if let Ok(snapshot) = store.snapshot() {
            let _ = snapshot.scan_collection(collection);
            let _ = snapshot.get_entity(collection, "e0");
            let _ = snapshot.collection_row_count(collection);
        }
        let _ = store.field_dictionary(collection, true);
    }
}

#[test]
fn layout_migration_and_meta_decoders_reject_corrupt_entries() {
    let (mut rng, _seed) = rng(0xF0AA);
    // Valid entries of a small database in both layouts.
    let mut store = EntityStore::new(MemoryKvEngine::new());
    store.ensure_stats().unwrap();
    let rows = (0..6)
        .map(|index| entity(&mut rng, &format!("e{index}")))
        .map(StorageWriteOp::PutEntity)
        .collect::<Vec<_>>();
    store.apply_batch(&rows).unwrap();
    let current = store.scan_raw_prefix(&[]).unwrap();
    let legacy = legacy::downgrade_entries(current.clone(), &BTreeSet::new()).unwrap();

    let mut cases = 0;
    while cases < 300 {
        let base = if rng.below(2) == 0 { &current } else { &legacy };
        let mut entries = base.clone();
        for _ in 0..1 + rng.below(3) {
            let at = rng.below(entries.len() as u64) as usize;
            let (key, value) = &mut entries[at];
            if rng.below(2) == 0 {
                *key = mutated(&mut rng, key);
            } else {
                *value = mutated(&mut rng, value);
            }
        }
        cases += 1;
        let input = entries
            .iter()
            .flat_map(|(key, value)| key.iter().chain(value))
            .copied()
            .collect::<Vec<_>>();
        check_case("stored entries", &input, &|_| {
            let mut store = store_with(&entries);
            let _ = store.layout_version();
            let _ = store.migrate_layout();
            let _ = store.ensure_stats();
            read_everything(&store);
            let _ = store.rebuild_stats();
            let _ = FieldDict::load_all(&entries);
        });
    }
}

/// Deeply nested input: `leaf` wrapped in `depth` single-element lists by
/// repeating the framing `encode` adds around a nested value.
fn nested(encode: &dyn Fn(&Value) -> Vec<u8>, depth: usize) -> Vec<u8> {
    let leaf = encode(&Value::Null);
    let one = encode(&Value::List(vec![Value::Null]));
    let common_prefix = leaf.iter().zip(&one).take_while(|(a, b)| a == b).count();
    let common_suffix = leaf
        .iter()
        .rev()
        .zip(one.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(leaf.len() - common_prefix);
    let (pre, post) = (&leaf[..common_prefix], &leaf[leaf.len() - common_suffix..]);
    let mid_leaf = &leaf[common_prefix..leaf.len() - common_suffix];
    let mid_one = &one[common_prefix..one.len() - common_suffix];
    let at = (0..=mid_one.len() - mid_leaf.len())
        .find(|at| mid_one[*at..].starts_with(mid_leaf))
        .unwrap();
    let (level_pre, level_post) = (&mid_one[..at], &mid_one[at + mid_leaf.len()..]);
    let mut out = pre.to_vec();
    for _ in 0..depth {
        out.extend_from_slice(level_pre);
    }
    out.extend_from_slice(mid_leaf);
    for _ in 0..depth {
        out.extend_from_slice(level_post);
    }
    out.extend_from_slice(post);
    out
}

const DEEP_CHILD_ENV: &str = "SEMANTIC_FUZZ_DEEP_NESTING_CHILD";

/// Child half of [`deeply_nested_input_is_rejected_without_overflowing`]:
/// decodes deeply nested payloads on a regular test thread. A stack
/// overflow aborts the process, so it must run in its own process.
#[test]
fn deeply_nested_input_child() {
    let Ok(target) = std::env::var(DEEP_CHILD_ENV) else {
        return;
    };
    let depth = std::env::var("SEMANTIC_FUZZ_DEEP_NESTING_DEPTH")
        .ok()
        .and_then(|depth| depth.parse().ok())
        .unwrap_or(1_000_000);
    let dict = std::cell::RefCell::new(FieldDict::default());
    let compact = |value: &Value| {
        let mut object = Object::new();
        object.insert("a", value.clone());
        encode_v2(
            &StoredEntity {
                id: "e".into(),
                collection: 1,
                kind: StoredEntityKind::Untyped,
                object,
            },
            &mut dict.borrow_mut(),
        )
    };
    match target.as_str() {
        "compact" => {
            let input = nested(&compact, depth);
            let dict = Arc::new(dict.borrow().clone());
            // Leak the result: dropping a deeply nested value recurses too.
            std::mem::forget(decode_v2("e", &input, &dict));
        }
        "memcmp" => {
            let input = nested(&memcmp::encode, depth);
            std::mem::forget(memcmp::decode(&input));
            std::mem::forget(memcmp::encoded_len(&input));
        }
        "self_contained" => {
            let encode = |value: &Value| {
                let mut object = Object::new();
                object.insert("a", value.clone());
                encode_entity(&StoredEntity {
                    id: "e".into(),
                    collection: 1,
                    kind: StoredEntityKind::Untyped,
                    object,
                })
                .unwrap()
            };
            std::mem::forget(decode_entity(&nested(&encode, depth)));
        }
        other => panic!("unknown target {other}"),
    }
}

fn run_deep_child(target: &str) -> std::process::ExitStatus {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "property_tests::decode_fuzz::deeply_nested_input_child",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(DEEP_CHILD_ENV, target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
}

#[test]
fn moderately_nested_values_round_trip_in_every_codec() {
    let mut value = Value::String("leaf".into());
    for level in 0..32 {
        value = if level % 2 == 0 {
            Value::List(vec![value, Value::Null])
        } else {
            let mut object = Object::new();
            object.insert("nested", value);
            Value::Object(object)
        };
    }
    let entity = StoredEntity {
        id: "e".into(),
        collection: 1,
        kind: StoredEntityKind::Untyped,
        object: Object::from_iter([("a".to_string(), value.clone())]),
    };
    let mut dict = FieldDict::default();
    let compact = encode_v2(&entity, &mut dict);
    assert_eq!(
        decode_v2("e", &compact, &Arc::new(dict)).unwrap().object,
        entity.object
    );
    assert_eq!(
        decode_entity(&encode_entity(&entity).unwrap())
            .unwrap()
            .object,
        entity.object
    );
    assert_eq!(memcmp::decode(&memcmp::encode(&value)).unwrap(), value);
}

// The decoders recurse once per nesting level without a depth limit. On a
// 2 MiB test-thread stack in debug builds the memcomparable decoder
// overflows at ~100 levels, the compact and self-contained payload decoders
// at ~500 levels (rmp_serde's recursion limit does not trigger first).
// Corrupt stored bytes can therefore abort the process instead of failing
// with an error.

#[test]
#[ignore = "bug: the self-contained (version 1) payload decoder has no effective nesting \
            limit; a corrupt payload nested ~500 levels overflows the stack and aborts the \
            process"]
fn deeply_nested_self_contained_payloads_are_rejected() {
    assert!(run_deep_child("self_contained").success());
}

#[test]
#[ignore = "bug: the compact (version 2) value decoder recurses per nesting level without a \
            depth limit; a corrupt payload nested ~500 levels overflows the stack and aborts \
            the process"]
fn deeply_nested_compact_payloads_are_rejected() {
    assert!(run_deep_child("compact").success());
}

#[test]
#[ignore = "bug: the memcomparable key decoder recurses per nesting level without a depth \
            limit; a key nested ~100 levels (debug build, 2 MiB stack) overflows the stack and \
            aborts the process"]
fn deeply_nested_memcmp_keys_are_rejected() {
    assert!(run_deep_child("memcmp").success());
}
