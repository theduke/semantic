# Database benchmarks

`crates/db_bench` (`semantic_db_bench`, not published) contains criterion
benchmarks of the embedded database against the in-memory KV engine and redb
(a temporary file). They use only public APIs (`Db`, `semantic_db_kv`,
`semantic_db_redb`).

## Data set

One strictly validated polymorphic collection (`bench_entities`) holds
`size` items plus `size / 10` owners (references resolve within a
collection). The `bench:item` class has twelve attributes with qualified
names: title and body text, kind (100 values, 1 % each), code (4 rows per
value), a unique score, price, rating, active flag, `created_at` datetime,
a tag list, a nested `dimensions` record and a `Ref` to `bench:owner`.
Indexes: equality on code, range on score, composite range on
(kind, created_at) and full-text on (title, body). Rows are generated
deterministically from their ordinal (SplitMix64), so every run sees the
same data.

Each (engine, size) database is seeded once and shared by all workloads.
Queries are built as ASTs up front, so SQL parsing is not measured.
Mutating workloads either rewrite rows in place or undo their effect
outside the measured section (criterion `iter_custom`), so the collection
size stays constant. redb commits with `Durability::Eventual` (engine cost,
no fsync), except `insert_immediate`, which reopens the file with
`Durability::Immediate`.

## Workloads

Benchmark ids are `<workload>/<engine>[/<variant>]/<size>`.

| Workload | What one iteration does |
|----------|-------------------------|
| `point_get` | `Db::get` by id |
| `eq_select` | `WHERE code = ?` (equality index, 4 rows) |
| `range_limit` | `WHERE score >= ? AND score < ? + size/10 LIMIT 50` (range index, unordered) |
| `ordered_index_scan` | `WHERE score >= 0 ORDER BY score LIMIT 50` (ordered range index scan) |
| `keyset_page` | `WHERE score > ? ORDER BY score LIMIT 50` (one keyset pagination page) |
| `full_text` | `text_match(title, body, 'a b')`, mode All, two terms |
| `full_scan_filter` | `WHERE price > 990` (non-indexed, ~1 % of the rows) |
| `count` | `SELECT COUNT(*)` |
| `top_n_unindexed` | `ORDER BY price DESC LIMIT 10` (top-N over a full scan) |
| `join_ref` | items of one kind joined to their owners via the `Ref` (composite index + index nested loop) |
| `feed` | `WHERE kind = ? ORDER BY created_at DESC LIMIT 20` (composite index) |
| `insert/<engine>/batch_N` | `Db::execute_batch` of N new rows (replies with a `Dataset`) |
| `insert_stats/<engine>/batch_N` | the same batch via `execute_batch_returning(.., BatchReturn::Stats)` |
| `insert_immediate/redb/batch_100` | `insert_stats` with fsync on every commit |
| `update_by_kind` | `UPDATE .. SET rating = ? WHERE kind = ?` (1 % of the rows) |
| `update_by_id` | `UPDATE .. SET rating = ? WHERE id = ?` |
| `delete_by_id` | `Db::delete` (replies with a `Dataset`) |
| `delete_by_id_stats` | one `DeleteById` batch returning stats |
| `delete_by_kind` | `DELETE .. WHERE kind = ?` (1 % of the rows) |
| `transaction_10r_10w` | interactive transaction: 10 `get`s, 10 `upsert`s, commit |
| `concurrent_read/<engine>/4_readers_1_writer` | mean `feed` latency of 4 concurrent reader tasks while one task inserts 100-row batches |

## Running

Always build and run through the Nix devshell (`nix develop -c ...`).

```sh
# Compile only.
cargo bench -p semantic_db_bench --no-run

# Everything (10k and 100k rows, both engines).
cargo bench -p semantic_db_bench

# Quick smoke run: 10k rows, redb only.
SEMANTIC_BENCH_SIZES=10000 SEMANTIC_BENCH_ENGINES=redb \
  cargo bench -p semantic_db_bench -- --quick

# Only some workloads (criterion name filter; the regex matches ids).
SEMANTIC_BENCH_SIZES=10000 cargo bench -p semantic_db_bench -- 'point_get|feed'

# Every workload once at 100 rows (plus plan checks), as part of the tests.
cargo test -p semantic_db_bench
```

Environment variables (read before seeding, so unselected databases are
never built):

| Variable | Default | Meaning |
|----------|---------|---------|
| `SEMANTIC_BENCH_SIZES` | `10000,100000` | comma-separated item counts (min 100) |
| `SEMANTIC_BENCH_LARGE` | unset | `1` also runs 1,000,000 items (slow to seed) |
| `SEMANTIC_BENCH_ENGINES` | `memory,redb` | engines to run |

Criterion options go after `--`: `--quick` stops once results are
significant, `--sample-size N`, `--measurement-time S`, `--warm-up-time S`.
Workloads whose iterations take milliseconds use 10 samples.

### Comparing runs

```sh
git checkout main
cargo bench -p semantic_db_bench -- --save-baseline main
git checkout my-branch
cargo bench -p semantic_db_bench -- --baseline main
```

Reports are written to `target/criterion/`. Compare runs made with the same
sizes and on the same machine only.

There is no CI configuration in the repository. When one is added, the
smoke job should run `cargo bench -p semantic_db_bench --no-run` and then
the quick 10k redb run above. The run took about 22 s on the reference
machine, not counting the build.

## Reference numbers

Indicative only: one `--quick` run with `SEMANTIC_BENCH_SIZES=10000`
on 2026-09-26, AMD Ryzen 7 5800X (8 cores / 16 threads), 32 GiB RAM,
NVMe SSD, Linux 7.2, rustc 1.96 (bench profile). Median time per
iteration (criterion's middle estimate); 10,000 items + 1,000 owners.

| Benchmark | memory | redb |
|-----------|-------:|-----:|
| `point_get` | 9.8 µs | 11.3 µs |
| `eq_select` (4 rows) | 57 µs | 54 µs |
| `range_limit` (LIMIT 50) | 5.40 ms | 3.02 ms |
| `ordered_index_scan` (LIMIT 50) | 318 µs | 244 µs |
| `keyset_page` (LIMIT 50) | 353 µs | 248 µs |
| `full_text` | 1.33 ms | 872 µs |
| `full_scan_filter` | 19.2 ms | 17.4 ms |
| `count` | 12.5 µs | 14.0 µs |
| `top_n_unindexed` | 47.5 ms | 24.1 ms |
| `join_ref` (100 items) | 1.46 ms | 872 µs |
| `feed` (LIMIT 20) | 162 µs | 132 µs |
| `insert` batch 100 (`execute_batch`) | 227 ms (441 rows/s) | 207 ms (484 rows/s) |
| `insert` batch 1000 (`execute_batch`) | 459 ms (2.2k rows/s) | 494 ms (2.0k rows/s) |
| `insert_stats` batch 100 | 34.2 ms (2.9k rows/s) | 42.3 ms (2.4k rows/s) |
| `insert_stats` batch 1000 | 409 ms (2.4k rows/s) | 395 ms (2.5k rows/s) |
| `insert_immediate` batch 100 | - | 42.2 ms (2.4k rows/s) |
| `update_by_kind` (100 rows) | 25.5 ms | 20.0 ms |
| `update_by_id` | 271 µs | 1.63 ms |
| `delete_by_id` (`Db::delete`) | 668 ms | 678 ms |
| `delete_by_id_stats` | 192 µs | 2.40 ms |
| `delete_by_kind` (100 rows) | 15.7 ms | 60.8 ms |
| `transaction_10r_10w` | 2.44 ms | 3.75 ms |
| `concurrent_read` (4 readers, 1 writer) | 14.5 ms | 458 µs (24 ms in a rerun) |

What these numbers show:

- `Db::execute_batch` and `Db::delete` reply with a `Dataset` of the
  touched collection, so they cost O(collection size) (about 190 ms at 10k
  rows, and a single `Db::delete` about 670 ms). The `BatchReturn::Stats`
  path is bounded.
- Inserts cost about 350-400 µs per row even on the bounded path (strict
  validation, four indexes including full-text, reverse references).
- `range_limit` reads the whole range (about 1,000 rows) before applying
  `LIMIT`, while the ordered `keyset_page` stops after 50 rows.
- A bare `ORDER BY score LIMIT 50` cannot use the range index, because
  rows without a score (the owners) are not in it. The benchmark adds
  `score >= 0`.
- Reads slow down a lot while a writer commits (`feed` alone takes
  130-160 µs). `concurrent_read` is bimodal: a read either overlaps a
  write commit (tens of ms) or it does not, so short `--quick` runs vary
  widely (redb gave 458 µs and 24 ms in two runs). Use longer runs for
  this benchmark.
- redb commits cost about 1.3 ms even with `Eventual` durability, and fsync
  (`Immediate`) adds little on this machine.
