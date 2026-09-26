# Testing the database

Besides the unit tests of each crate and the shared backend suite
(`crates/db_test/src/suite`, run by the memory, redb and logfs backends),
the database has randomised property, fuzz, differential, crash and
concurrency tests. They are seeded and reproducible; the cheap variants run
with the regular `cargo test`, the expensive ones are `#[ignore]`d.

Run everything through the Nix devshell, for example:

```sh
nix develop -c cargo test --quiet -p semantic_db_kv -p semantic_db_redb -p semantic_db_log
# including the expensive tests and the known-bug tests:
nix develop -c cargo test --quiet -p semantic_db_redb -- --include-ignored
```

## Seeds and knobs

| Variable | Effect |
| --- | --- |
| `SEMANTIC_TEST_SEED` | Seed(s) of the randomised tests. Differential, concurrency and crash tests take a comma-separated list of `u64` seeds and run each; the `db_kv` property tests mix the (single) value into their fixed seeds. |
| `SEMANTIC_TEST_STEPS` | Operations per differential run (default 300 for redb, 200 for logfs and the memory engines). |
| `SEMANTIC_TEST_CHECK_EVERY` | Run the differential query checks and `verify` every N operations (default 10). |
| `SEMANTIC_TEST_CONCURRENCY_TXS` | Transactions per writer in the concurrency tests (default 25). |
| `SEMANTIC_CRASH_ITERATIONS` | Child processes killed by the `kill -9` test (default 5). |

A failing randomised test prints its seed, for example
`differential test failed with seed 2; reproduce with SEMANTIC_TEST_SEED=2`.
Rerun just that test with the printed value.

## What runs where

### Property and fuzz tests (`crates/db_kv/src/property_tests`)

- `key_layout`: entity and index keys round-trip through their parsers, sort
  by their `(collection, id)` and `(index, path, value, id)` tuples, and the
  prefixes of different collections, indexes, paths and values never
  overlap; index value ranges contain exactly the values within bounds.
- `payloads`: compact (v2) and self-contained (v1) entity payloads round-trip
  deep random objects with all value variants, empty containers,
  `Void`/`Null`, extreme integers and floats and long strings; the
  memcomparable order equals `Value::cmp` for nested values. The compact
  payload keeps float bits exactly (NaN payloads, `-0.0`); the key encoding
  canonicalises NaN and `-0.0`, which `Value::cmp` treats as equal.
- `field_dicts`: random writes with new and known field names across
  collections stay decodable through borrowed and owned snapshots taken at
  any point, after reopening with a cold dictionary cache and with mixed
  payload formats.
- `decode_fuzz`: random and mutated (bit flips, truncation, extension,
  splices, huge lengths) inputs to the payload decoders, the memcomparable
  decoder, the key parsers, the layout migration and the stats and
  dictionary meta decoders must return errors, never panic, take longer than
  a per-case limit or allocate out of proportion to the input (a tracking
  global allocator of the `db_kv` test binary records the largest
  allocation). Inputs nested far beyond the decoders' depth limits
  (`MAX_VALUE_DEPTH`, `memcmp::MAX_DEPTH`) must fail with an error on a
  regular test thread, and input at the limits must decode.
- `engines`: the differential and concurrency tests below on the memory
  engine with and without MVCC snapshots.

### Differential tests (`crates/db_test/src/differential`)

`run_differential` generates a random schema (two polymorphic collections
with a random subset of equality, unique, range, composite, partial,
full-text and reference indexes; three classes with validated references)
and a random sequence of operations: creates, upserts and deletes by id,
multi-operation batches, predicate updates and deletes, class rows with
valid and dangling references, interactive transactions with savepoints,
rollbacks to savepoints and final commits or rollbacks, adding indexes
(unique ones over existing duplicate values must fail) and dropping them,
invalid writes and reopens. Each operation runs on every target
and must produce the same result or the same error kind everywhere, and
match the prediction of a trivial model (a `BTreeMap` of rows per
collection, predicates evaluated with `evaluate_filter_expr`). Every
`SEMANTIC_TEST_CHECK_EVERY` operations, full scans, a query per index shape,
text matches, counts, grouped counts and a join must return identical rows
on every target and match the model, and `verify` must be clean.

Targets: memory vs redb (`crates/db_redb/src/differential_tests.rs`, three
seeds), memory vs logfs (`crates/db_log/src/differential_tests.rs`) and
memory vs memory with MVCC (`db_kv`). The ignored
`memory_and_redb_agree_on_many_long_random_runs` runs 10 seeds with 1000
operations each.

### Concurrency tests (`crates/db_test/src/concurrency.rs`)

Writers commit interactive transactions (random upserts and deletes of
overlapping ids and read-modify-write counter increments), retrying
conflicts, while readers run full scans and index queries and a subscriber
follows the change feed. Replaying the committed transactions in revision
order must yield the final state without lost increments, every read must
match the state after some prefix of that history (never going backwards
per reader), the change feed must deliver every commit in order with the
replayed rows (gaps only after `Lagged`), and `verify` must be clean. Runs on
memory, memory with MVCC, redb and logfs; the ignored
`concurrency_invariants_hold_on_many_runs` repeats it.

### Crash consistency (`crates/db_redb/src/crash_tests.rs`)

- A `KvEngine` wrapper around the redb engine fails (with an error or a
  panic) before the commit, after the n-th put or delete inside a write, or
  after the commit before returning. After each failure the database file is
  reopened: the batch or DDL change is entirely visible or entirely absent,
  indexes, reverse references and stats counters verify clean. A handle that
  saw an error (not a panic) keeps working without a reopen. After a panic
  redb repairs its file on the next integrity check; the test accepts that
  one reported repair and requires a clean check afterwards.
- The same injection interrupts every write of opening a legacy database
  (layout migration, stats backfill, index rebuilds); the next open must
  complete the migration with the data intact, and opening a migrated
  database must not write.
- `killed_writer_keeps_every_reported_commit` (ignored) spawns the test
  binary as a child process writing batches with durable commits, kills it
  with `SIGKILL` at random moments and checks that every commit the child
  reported is present, no batch is partial and `verify` is clean.

## Known bugs

Tests for bugs found by these tests are `#[ignore]`d with a `bug: ...`
reason; run them with `--ignored` to check whether they are fixed. There
are currently none.
