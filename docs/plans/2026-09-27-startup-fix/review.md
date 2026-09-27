# Startup registration implementation review

Date: 2026-09-27.

The proposed fast return does not satisfy the plan's correctness gate. It was
removed, together with the package-local proof module and its production catalog
helpers. Registration again always performs the existing validation and stored
DDL reconciliation. **The startup performance objective remains unresolved.**

No schema definitions, historical migrations, persisted formats, or transaction
semantics were changed. The plan remains the proposed design, subject to the
additional proof obligations below.

## Blocking findings

1. **Unrelated derived projections are not covered by a package-local proof.**
   Public `Catalog::upsert_attribute` updates the typedef and attribute projection
   without rebuilding collection fields. A catalog installed through
   `EmbeddedDb::replace_catalog_snapshot` can therefore contain an unrelated
   attribute missing from collection projections. Full package replay rebuilds
   those fields. The proposed comparison inspected only package-owned attributes
   in the default/package collections and could skip that repair. The retained
   regression uses the attribute's resolved canonical identifier when inspecting
   field IDs; aliases are not canonical field IDs.
2. **A normal collection-kind history has intermediate allocation effects.**
   Upserting the same collection as `Schema` and then `Untyped` allocates schema
   field IDs during replay, advancing persisted `next_field_id`. The proof stored
   only the collection name and final properties, returning unchanged despite a
   different full-replay snapshot. The differential regression failed on the
   candidate and passes after withdrawal. Such histories must conservatively fall
   back unless their intermediate effects are modeled.
3. **Freshly opened catalogs are not a sufficient trusted baseline.**
   A persisted unrelated class with self-inheritance successfully reopens because
   `Catalog::from_stored_rows` reconstructs projections without running all the
   global invariants checked by `rebuild_type_projections`. Full registration
   rejects its inheritance cycle; the candidate accepted it. The retained test
   checks both the full-replay error and public registration's error. It failed
   on the candidate and passes after withdrawal.

An ephemeral EmbeddedDb-only provenance gate was explicitly investigated. The
mutation inventory is small: one construction site in `open_with_config`, public
catalog replacement, externally accessible `SharedCatalog` replacement/CAS, and
three internal catalog commit sites (internal collection marking, DDL, package
registration). A version/identity gate could detect external replacement, but
cannot establish validity of a freshly loaded legacy catalog in finding 3.
Authorizing a new validation/provenance design or auditing the whole catalog on
open would be additional work. No such flag, new global audit, or persisted marker
was introduced.

## Retained changes and corrections

- Registration phase timings and failure/completion events remain. Completion
  distinguishes `unchanged_reconciled`, `migration`, `metadata_update`, and
  `repair`, and includes `catalog_changed`. There is no fast-path claim.
- Diagnostic catalog comparisons moved into a small private helper. Expensive
  category diagnostics and metadata classification run only with debug tracing;
  new migrations avoid extra diagnostic snapshots when tracing is disabled.
- Difference logs now include `next_field_id` and auto-index state. Metadata
  classification uses the actual persisted delta, so simultaneous metadata edits
  and repairs are reported as repairs. Catalog encoding counts only its added
  operations; commit counts include data operations as well as extra catalog ops.
- Validation again precedes the storage revision read, preserving the original
  error precedence. Revision fencing, retry handling, Fail/Log migration mismatch
  policy, replay of stored DDL, and omission of applied data operations remain on
  the original path.
- New registration tests live in `embedded/db/package_registration_tests.rs`.
  They cover the blocking cases, wrapper-only metadata and class-projection
  repair, filestore index repair, unchanged revision/catalog version, zero
  storage reads/scans/writes, no change-feed event, and revision-fence retry.
  A repeated Log-policy mismatch still repairs deleted schema after changed
  package metadata has already been persisted.
- Counting storage's borrowed snapshots forward through the wrapper and owned
  snapshots retain its shared counters. A dedicated test exercises both snapshot
  interfaces, eager/streaming collection scans, and equality/range/prefix index
  scans. This verifies that the zero-work assertions cannot be bypassed merely
  by reading through snapshots. It does not measure catalog CPU work.
- Base's reopen test now explicitly checks full reconciliation in tracing. Its
  relationship-repair/reopen regression remains. Existing base compatibility
  tests and redb reopen tests remain unchanged.

## Benchmark limits

The four Criterion fixtures measure real base and filestore registration in warm
memory databases and the first registration after a real redb close/reopen. Setup
and open are excluded from registration timing, and package cloning is included
consistently. Revision assertions prevent timing a repeated catalog mutation as
an unchanged fixture.

These are empty-database **baseline** fixtures after withdrawal of the shortcut.
They do not establish entity-count or unrelated-catalog scaling, p95 budgets,
allocations, cold-process behavior, database-open latency, or PostgreSQL startup
performance. No copy of the reported slow database was available in this review.
The plan's full performance acceptance matrix has not been satisfied.

## Validation

All Rust commands ran through the Nix development shell with Rust 1.96.0.

- `nix develop -c cargo test --quiet --message-format=short -p semantic_db_core -p semantic_base -p semantic_db_redb`:
  passed; core 387, base 32 unit + 2 integration, redb 42 passed / 3 ignored.
  All three crates' doc-test runs completed successfully (zero doc tests).
- `nix develop -c cargo check --quiet --message-format=short --workspace --all-targets`:
  passed, including the benchmark target.
- `nix develop -c cargo bench --quiet --message-format=short -p semantic_db_bench --bench package_registration -- --test`:
  passed all four baseline fixtures. This was a smoke run, not a latency study.
- `nix develop -c cargo fmt`: passed.
- `git diff --check`: passed.

The collection-kind and reopened-inheritance differential regressions were first
observed failing against the candidate shortcut and now pass on full registration.
The only environment warnings were the dirty Git tree and the existing
`flake-parts` override for a nonexistent `nixpkgs` input. No commits or pushes were
made, and no unrelated UI files were changed.
