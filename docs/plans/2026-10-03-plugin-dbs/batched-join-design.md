# T5.2 design for review: opt-in bounded virtual join probes

Status: implemented after user approval. Review 1 clarified that the existing
membership-list contract needs one accepted plan, without duplicate negotiation.

## Existing behavior and scope

`execute_index_nested_loop_join_stream` in
`crates/db_core/src/plan/execute.rs` currently collects **all** outer rows,
deduplicates their non-null keys globally, performs one
`index_lookup_many_stream`, collects its results, then executes the existing
hash join. The generic many-lookup default combines individual filtered
lookups. The embedded implementation in `crates/db_core/src/embedded/db.rs`
instead collects indexed entity IDs into one set and materializes them once.
Embedded is already optimized; replacing this path with row-by-row probes
would change its cost, deduplication, and row ordering.

The proposed change enables bounded probes only for an explicitly opted-in
virtual source on the right of an **inner equality index nested loop join**.
Local probes, other algorithms, non-equality conditions, and left/right/full
joins retain the existing executor path. Existing schema, parser, planner
types, and persisted indexes remain unchanged.

## Additive data-source interface

Add two object-safe methods to `AsyncPhysicalDataSource`:

```rust
/// None preserves the existing executor path for this particular source.
fn index_lookup_batch_size(
    &self,
    source: &SourceRef,
) -> Option<std::num::NonZeroUsize> {
    None
}

/// Each distinct key is looked up with the same residual predicate.
/// Ordering is the source's existing lookup ordering; no order is promised.
fn index_lookup_batch_stream(
    &self,
    source: SourceRef,
    field: FieldRef,
    values: Vec<Value>,
    residual_predicate: Option<Expr>,
) -> SendableRecordBatchStream {
    self.index_lookup_many_stream(source, field, values, residual_predicate)
}
```

The default deliberately delegates to the existing hook. Its generic default
already loops `index_lookup_filtered_stream`, while specialized sources keep
their overrides. No implementer must change. The borrowed data-source adapter
must forward both new methods so a wrapper preserves the source's policy.

`CompositeDataSource` returns `Some(64)` only when the referenced fragment is
virtual and has an exact accepted membership binding plan. It returns `None`
for local fragments and ordinary virtual scans. A global
source flag or backend-tag substring test is insufficient: this decision
must resolve the precise `SourceRef`/fragment being probed.

## Virtual negotiation and binding

T5.1 provides the parameterized membership request and its synthetic
equality index. As clarified by review 1, one accepted plan uses
`parameters = ["__keys"]` and a canonical membership predicate for the probe
field. Each batch scan passes `bindings["__keys"] = Value::List(keys)`; keys
are concrete, distinct, non-null values from that batch. List membership
binding must have explicit tested semantics in the adapter/plugin; a list
must not accidentally become one scalar equality key.

The binding candidate has no limit or offset: truncating the inner lookup
could lose matches for later outer rows. Projection remains a hint. The
candidate carries only ordering already valid for the existing fragment;
the host promises no new join ordering. Validate accepted filter support,
ordering, and schema revision as for every other negotiation. Preserve and
echo the accepted candidate's token for every list, including singleton lists.
There is no second negotiation for a different cardinality: the protocol
already defines `__keys` as a list. Rejection retains an accepted ordinary scan.
Protocol violations and
revision changes fail normally, without treating them as rejection.

Batch enablement requires exact membership support. Other inexact or
unsupported conjuncts are checked by the host, preserving their existing
semantics. Explain exposes the actual negotiated batch request/support and
batch size; it must not claim batching for an ordinary scan.

## Executor behavior, memory, and ordering

For an eligible join, consume at most N=64 outer **rows** at a time, across
upstream record-batch boundaries, then deduplicate keys within that group.
Bounding rows as well as keys avoids accumulating unbounded duplicate-key
outer rows. Skip lookups for an all-null-key group, preserving SQL equality
null behavior. Call the batch hook once for each nonempty key group.

Collect that group's inner rows and use the existing hash join/binding and
residual-condition logic on just this outer group. Emit output in the
existing outer-row order, preserving inner-stream order within each outer
row's matches, and split output with `ExecutionOptions::batch_size` (currently
1024 by default). Duplicate outer keys still produce every corresponding
join pair. Repeat keys across groups may be fetched again: there is no
unbounded cross-group cache and no new global entity deduplication rule.
The virtual scan contract already requires unique entity IDs per scan.
Explicit downstream sorting continues to determine SQL `ORDER BY` results.

Only one group/lookup is active; do not introduce concurrent remote batches.
The outer-row/key memory is bounded by N. Inner fanout and the materialized
output of one group remain potentially large, as does any upstream Sort or
Aggregate. This is **not** a claim of constant-memory query execution.
Existing embedded joins remain globally materialized with exactly their
current ordering, deduplication, and cost behavior.

On a source/outer-stream error, yield the error and stop; do not fetch the
next group. Earlier output may have streamed, but collecting APIs fail the
query. Dropping the join stream drops the current lookup and outer stream,
preventing subsequent groups and cancelling active plugin scans through
existing stream-drop semantics. A host LIMIT can therefore stop after the
needed groups. Record probe/build/access metrics per group with their
existing meanings; estimates remain plugin estimates, not measured costs.

## Validation after approval

Executor tests use independent expected pairs for zero keys, null keys,
duplicate outer keys, duplicate keys across boundaries, one-to-many matches,
residual predicates, and 63/64/65 outer rows. Compare eligible results with
the existing single-key path, including explicit ordering and alias binding.
Count remote batch requests and verify exact `__keys` lists. Cover rejected
binding negotiation, token reuse, schema changes, first/later batch errors,
stream cancellation, and LIMIT avoiding later groups. Verify all local and
outer-join cases still call the existing many hook, even through the borrowed
adapter. No production change is authorized by this document.

Run through Nix with the shared main-clone target:

```sh
export CARGO_TARGET_DIR=/home/theduke/dev/github.com/theduke/semantic/target
nix develop -c cargo check --quiet --message-format=short
nix develop -c cargo test --quiet --message-format=short -p semantic_db_core
nix develop -c cargo test --quiet --message-format=short -p semantic_db_bench
nix develop -c cargo fmt --check
```

Capture `join_ref` Criterion results before and after on the same machine,
Rust/Nix environment, engine settings, and fixture sizes; serialize runs
against other users of the shared target. The existing fixture asserts use
of the indexed join path. Run both memory and redb at 10k and 100k items:

```sh
SEMANTIC_BENCH_SIZES=10000,100000 SEMANTIC_BENCH_ENGINES=memory,redb \
  nix develop -c cargo bench -p semantic_db_bench --bench db -- \
  join_ref --save-baseline plugin-dbs-before-batching
# After the approved implementation:
SEMANTIC_BENCH_SIZES=10000,100000 SEMANTIC_BENCH_ENGINES=memory,redb \
  nix develop -c cargo bench -p semantic_db_bench --bench db -- \
  join_ref --baseline plugin-dbs-before-batching
```

Embedded correctness and dispatch must stay unchanged. Investigate any
statistically significant embedded regression before merging; do not infer
virtual speedups from embedded benchmarks. A counting/delayed virtual source
separately demonstrates the reduction from one call per key to one per group.
