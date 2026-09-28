# Fast registration of an unchanged package

Follow-up: [durable certificate implementation](implementation.md) supersedes the
withdrawn design below after authorization to add persisted bookkeeping.

Date: 2026-09-27. Status: implementation reviewed; the unsafe shortcut was
withdrawn. Diagnostics, regression tests, and baseline benchmarks remain. See
[the implementation review](review.md) for the blocking evidence and validation.

## Objective and scope

Make registration of an already installed, healthy package effectively instant,
including the first registration after reopening a large database. In particular,
`semantic.base` must not perform work proportional to stored entity count or to
unrelated installed schema. Preserve the existing ability to repair schema drift
even when every migration has already been recorded.

The reported event is:

```text
Package registration completed operation="package_registration" package="semantic.base" elapsed=36.144187043s migrations_executed=0 attempts=1 conflicts=0
```

Zero executed migrations does **not** currently imply zero catalog changes or zero
database scans. This event proves that retries did not cause the delay, but does
not identify which phase consumed the 36 seconds. The findings below come from
the call graph; phase measurements on the affected database are still required
before attributing that particular duration to CPU replay versus row scanning.

“No-op” here means identical package metadata, identical recorded migrations, and
healthy effective schema. Metadata changes and actual repairs remain mutations,
even if `executed_migrations` is empty. Making a necessary repair constant time is
not part of this change.

## Findings and root cause

### Actual startup path

`crates/app/src/scope.rs::initialize_default_db` registers default packages in
sequence; `default_packages` includes base and filestore. The embedded backend's
`upsert_package` calls `EmbeddedBackend::write`, which schedules blocking work and
takes the database write lock. It then calls
`crates/db_core/src/embedded/db.rs::EmbeddedDb::upsert_package`.

The registration timer starts inside `EmbeddedDb::upsert_package`, after the
backend lock was acquired. Its duration therefore excludes the async scheduling
and lock wait. The current inner path is:

1. Normalize the incoming package.
2. For each transaction attempt, take an `Arc` catalog snapshot and call
   `validate_package_migrations_with_catalog`.
3. Read the storage revision and call `apply_package_update`.
4. Only after replay, compare two complete `CatalogStorageSnapshot` values to
   decide whether this was a no-op.
5. Either check the revision and return, or perform the ordinary schema-change
   persistence/validation work and commit.

### Work paid before the current no-op check

| Location | Work | Scaling |
| --- | --- | --- |
| `managed_schema.rs::validate_package_migrations_with_catalog` | Normalizes again; builds a fresh core catalog; visits all installed classes and named aliases; replays incoming migrations; scans the validation catalog for each module's final definitions | Package history plus unrelated catalog size |
| Dependency seeding in that validator | Inserts each foreign class separately with `upsert_class`; each insertion reaches `refresh_type_def_data` | Repeated lowering of a growing catalog, potentially quadratic |
| `db.rs::apply_package_update` | Deep-clones the entire catalog, including packages and recorded migration definitions | Entire catalog size |
| `db.rs::reconcile_applied_migration_schema` | Replays the **stored** DDL for every applied migration, even when it is already effective | All historical package DDL |
| `catalog/catalog.rs::Catalog::apply_batch` and `flush_pending_type_operations` | Collects tolerated unstorable definitions; rebuilds all type and collection projections for type batches; lowers all type definitions | Entire catalog, repeatedly |
| `rebuild_collection_projections` / `build_collection_schema_for_lid` | Rebuilds every collection using registered attributes and computed fields from all classes | Collections multiplied by schema size |
| `catalog/id_map.rs::insert_fixed` / `clear_id_aliases` | Clears aliases with `retain` scans while projections are rebuilt | Additional repeated full-map scans |
| `Catalog::to_storage_snapshot`, called on both catalogs | Clones every persistent schema representation, package, and applied migration before equality comparison | Entire catalog size |

The `SharedCatalog` snapshot itself is cheap; it is not the deep catalog clone.
The pre-return work above is schema-size dependent and does not itself scan user
rows. A large entity database with a fixed catalog needs a separate explanation
if registration takes longer than the same catalog with few entities.

### How zero migrations can still scan the database

If the post-replay snapshots differ, registration takes the mutation path even
with zero migrations:

- `backfill_new_indexes` can scan collections for new or changed indexes.
- `embedded/schema_store.rs::catalog_write_ops` clears and rewrites the catalog
  collection and its indexes; it does not write just the changed package.
- `embedded/db/compact.rs::rebuild_reverse_references` scans every collection
  except the reference collection when that derived collection exists.
- `embedded/db/validation.rs::validate_catalog_rows`, when validation is enabled,
  scans every noninternal collection and can perform reference lookups per row.
- `persist_dataset_delta` commits the resulting operations and catalog change.

Possible reasons for this branch include genuine legacy repairs, changed package
metadata, or an unintended difference introduced by replay. Do not claim that
snapshot ordering or local-ID churn caused the reported event without measuring
the difference. Instrument the first differing category and operation counts to
establish that distinction.

### Existing behavior that is intentional

- `unchanged_package_still_reconciles_missing_schema` deletes a class through DDL,
  registers the same package, and expects the class to return with no migration
  executions.
- Registration compares incoming migrations with recorded definitions, including
  descriptions and metadata. `Fail` rejects a mismatch. `Log` reports it but
  reconciles the **stored** migration definition, not the changed incoming DDL.
- `package_metadata_changes_are_persisted_without_new_migrations` requires the
  stored package to update even with no new migrations.
- Complete type definitions carry behavior that their class/record projections
  alone do not represent. The reconciliation comment explicitly calls out legacy
  behavioral typedef constraints.
- `redb_reopen_and_unchanged_package_preserve_revision` already requires a healthy
  filestore registration after reopen to leave the revision unchanged.
- The base history contains superseded definitions, including the historical
  `Web Bookmark` title and later `WebBookmark` definition, and later UI metadata
  changes. Comparing every historical upsert to current schema would always
  reject a healthy base package.

## Invariants

1. Never edit existing core/base migrations, historical helpers, persisted
   migration definitions, or their snapshots. This optimization requires no
   schema migration or new persisted catalog field.
2. Preserve package normalization, full migration identity/content comparison,
   `Fail`/`Log` semantics, migration ordering, and transactional atomicity.
3. Applied Insert/Update/Delete operations must never execute again. In particular,
   base's label-group data update remains historical data work.
4. Preserve schema repair, including complete typedef constraints/annotations,
   module metadata, missing projections, collections, indexes, and relationships.
5. A successful fast return performs no catalog rewrite, data write, revision
   increment, change-feed event, index backfill, or full collection scan.
6. Keep the revision fence and retry behavior. Do not return from an unchecked
   observation or cache a result across a catalog change.
7. Do not use a package name/version, applied-migration count, or stored package
   equality alone as proof that the installed schema is healthy.
8. Performance is bounded by this package's definition/history and the foreign
   schema it actually references. Strict O(1) independent of input package size
   is neither realistic nor necessary.

## Design alternatives

| Approach | Decision |
| --- | --- |
| Return when the package/version or all migration keys are present | Reject: conceals drift and changed migration contents |
| Skip all reconciliation after adding a schema-version marker | Reject for this change: persisted compatibility, invalidation, and repair semantics become broader work |
| Cache successful registration in `EmbeddedDb` | Optional later optimization only; does not solve first startup after reopen and can become stale after DDL |
| Only optimize alias maps, batch dependency insertion, or remove snapshot allocations | Useful follow-ups, but historical replay still scales with unrelated catalog size |
| General copy-on-write catalog or incremental projection engine | Defer: too broad for a startup fix |
| Read-only proof of effective package schema, then existing fallback | Choose: bounded healthy path and established repair path remain separate |

## Chosen approach

Add a conservative, read-only preflight before whole-catalog validation and
`apply_package_update`. It must prove that the same normalized package is already
installed, all supplied migrations match recorded definitions, and replay's
effective schema changes are already present. Otherwise run the existing path.

The first implementation must cover all migration shapes used by the default
startup packages, including base, filestore, and packages registered during app
initialization. It may conservatively fall back for unusual DDL histories whose
side effects cannot yet be proved cheaply. Log that reason. Do not silently
advertise arbitrary-package constant time while such cases remain unsupported.

### 1. Build a package-local effective schema description

Introduce a private module such as
`crates/db_core/src/managed_schema/registration.rs`, shared by registration and
its tests. Proposed types:

- `PackageRegistrationPreflight::{Unchanged, NeedsFullRegistration(reason)}`.
- `EffectivePackageSchema`, containing canonical expected definitions, relevant
  dependency requirements, collection/index/relationship effects, and a proof
  capability classification for the history.

The compiler walks migrations in their original order and only interprets DDL.
Use the same normalization/conversion helpers as normal catalog application.
Expose small `pub(crate)` helpers where necessary; do not duplicate the rules for
module qualification, class/attribute-to-typedef conversion, or index canonical
fields/predicates.

For type operations, all Attribute/Class/RecordType/TypeDef forms share the
canonical typedef key. The later definition wins for that key. Retain the full
`TypeDef`, including wrapper constraints, annotations, params, visibility, module,
and metadata. Retain deletion expectations as explicit absence, not as omitted
entries. Collections, indexes keyed by collection/name, and relationships need
their own effective state and dependency/side-effect description.

Do not simply compare every historical operation to the live catalog. For
example, base's early bookmark class must be replaced in the description by its
later bookmark class. Likewise, applying an old class upsert must not erase a
later full typedef's constraints in the expected final state.

A last-operation map alone is also insufficient for every DDL history. Model or
reject these cases conservatively:

- Delete/recreate cycles can change local IDs and collection/index identity.
- A type changing between attribute/class/record can change projections.
- Upserting a collection also restores builtin ID/type indexes, the automatic
  path index when enabled, and its builtin parent relationship.
- Deleting a collection cascades to its indexes/relationships.
- `SetAutoIndex` affects collections beyond the package. Do not scan them to
  force a supposed fast path; fall back unless an existing authoritative bounded
  invariant can prove the global effect.
- Intermediate schema states can allocate/remove fields, introduce errors, or
  affect subsequent operations. Only fold operations when replay equivalence is
  established. Treat uncertain histories as a fallback, not a successful proof.

For the initial fast path, explicitly recognize stable-kind upserts and the
collection/relationship operations used by the default packages. Already
applied data operations are ignored for schema reconciliation exactly as today.
Before enabling more complex histories, add differential tests against current
replay; the slow path remains available indefinitely.

### 2. Keep validation independent of installed package-owned definitions

The preflight must not let installed schema conceal an invalid package. Preserve
the invariant in
`package_validation_resolves_installed_reference_targets_without_reusing_owned_schema`.

Build/validate the effective description in an isolated catalog containing the
core baseline plus only referenced foreign definitions. Discover foreign
requirements by walking the normalized package's declarations and historical
DDL; include references that occur only in old migrations. Resolve transitively
required named aliases and preserve existing ambiguity/error behavior. Foreign
class stubs must retain today's deliberate stripping of fields, constraints,
inheritance, and extensions, so consumers do not contaminate historical replay.

Do not enumerate `dependencies.classes()` or `dependencies.type_defs()` to seed
this catalog. Use targeted canonical/alias lookups. Unsupported resolution cases
fall back to the existing validator. Batch dependency insertion rather than
repeatedly lowering the scratch catalog after each inserted class.

Replay the package's DDL on this bounded scratch catalog and perform the current
module-versus-migrations validation. This preserves independent validation while
avoiding full installed-catalog replay. Capture the effective expected definitions
from this replay and its operation effects. A process-local compiled-package
cache may avoid repeating this fixed package work, but the uncached path must
already meet the startup budget. Any cache key must include complete package
content and relevant resolved dependency content; no persisted hash is needed.

Keep `validate_package_migrations_with_catalog` as the authoritative fallback
while this restricted dependency path is introduced. Differential validation
tests must establish that the bounded validator accepts/rejects the same supported
packages. Do not broaden accepted schema as part of the optimization.

### 3. Check live schema using targeted comparisons

Inside each transaction attempt:

1. Capture catalog snapshot and storage revision.
2. Compare normalized incoming package to `package_by_name`; changed metadata,
   missing package, or unsupported package shape selects the full path.
3. Look up each migration by package/module/name and compare its full definition.
   Missing entries or mismatches select the existing path, which retains error
   precedence and `Fail`/`Log` reporting. Do not shortcut `Log` mismatches merely
   because the stored package was previously updated to the changed input.
4. Compile/validate the bounded effective description and check each expected
   definition via catalog lookup. Compare full typedefs and their relevant
   attribute/record/class projections; checking class existence alone is invalid.
5. Compare explicitly affected collection properties and required builtin
   structures, final index definitions, and relationships. Compare semantic
   fields, not scratch-catalog local IDs, hash-map iteration order, or defaults
   reconstructed differently from normal application.
6. Return unchanged only when every required condition is proven. Keep
   `storage.ensure_revision(read_revision)` immediately before returning, inside
   `run_with_transaction_retries`.

Projection consistency deserves an explicit implementation review. Normal catalog
load and DDL reconstruct derived projections, but public low-level catalog
mutation/replacement can produce states that were not checked in the same way.
The preflight must not assume arbitrary `Catalog` instances are consistent. Use
targeted projection checks wherever possible. If proving a supported operation's
global derived effects would require unrelated-catalog traversal, select the
fallback for that state. Do not introduce a broad “trusted catalog” flag without
enumerating and testing every constructor and mutation that could invalidate it.

This proof obligation is a release gate: demonstrate that the normal reopened
base/filestore state is provable without global work. If that cannot be done with
small read-only helpers, reassess the design before introducing pervasive catalog
invalidation bookkeeping; do not hide an O(catalog) audit inside the fast path.

### 4. Preserve the existing repair/migration path

On any negative or uncertain preflight result, call the existing validator,
`apply_package_update`, snapshot comparison, index backfill, reverse-reference
rebuild, row validation, and commit path. Keep stored migration replay under the
`Log` policy. This preserves drift repair without trying to implement repairs in
the preflight.

Initially retain the full snapshot comparison on this fallback. It is no longer
on the healthy path and remains a useful correctness guard. Optimizing metadata
updates, repair scans, or catalog persistence is separate work, with separate
validation of changed behavior.

## Detailed implementation sequence

1. **Characterize and instrument.** Add registration phase timings/reasons and
   counting-storage tests before changing the branch structure. Capture the slow
   event on a representative copied database. Record whether the storage revision
   changes and whether fallback scans occur.
2. **Extract the bounded description/validation helpers.** Add
   `managed_schema/registration.rs`; factor reusable normalization/effect helpers
   from `managed_schema.rs` and `catalog/catalog.rs` without changing existing
   public method behavior. Unit-test superseded upserts and full typedef equality.
3. **Add read-only catalog helpers as needed.** Prefer existing keyed lookups.
   For indexes, add a lookup by collection/name using the existing `IdMap` key;
   `indexes_for_collection` currently filters all indexes and must not become
   hidden global work. Keep alias/canonical-name semantics unchanged.
4. **Integrate the preflight in `EmbeddedDb::upsert_package`.** Normalize once,
   retain revision/retry fencing, and select fast return versus existing fallback.
   Keep `PackageRegistrationOutcome` unchanged; reason/timing data can be private.
5. **Add backend and real-package regression coverage.** Put generic cases in a
   dedicated `embedded/db/package_registration_tests.rs` module; extend redb
   reopen coverage; put base-specific tests in `crates/base` to avoid a db_core
   dependency cycle.
6. **Add benchmarks.** Extend `crates/db_bench` with a dedicated
   `benches/package_registration.rs` target and fixture helpers. Add base as a
   benchmark dependency if needed. Keep fixture setup and DB open timing separate
   from registration timing.
7. **Validate and compare.** Run focused tests, checks, formatting, and cold/warm
   benchmarks. Enable the fast branch only after differential replay and default
   package coverage pass.

Do not modify `crates/data` or `crates/base` schema definitions/migrations for this
optimization. Small catalog helper additions are justified; redesigning core
types, local-ID allocation, or transaction semantics is outside this plan.

## Tests and performance evidence

### Deterministic healthy-path tests

Add a test storage wrapper with counters for reads, collection scans (both eager
and streaming), index scans, commit calls, and submitted write operations. Count
both storage and snapshot interfaces so a scan cannot move behind a snapshot and
escape the assertion. Reset counters after seeding/opening.

For unchanged registration, assert zero collection/index scans, zero writes and
commits, unchanged storage revision and catalog version, empty migration outcome,
and no change-feed event. Allow the fixed revision reads/fence. Assert the same
after unrelated data writes and after unrelated schema additions: an unrelated
change must not force full replay of a healthy package.

Use private preflight diagnostics/test instrumentation to assert no live-catalog
clone, full snapshot construction, global dependency enumeration, or live catalog
projection rebuild on the healthy branch. Storage counters alone cannot catch
schema-only regressions.

### Correctness and fallback matrix

- Keep all existing package metadata, missing-schema, mismatch, redb revision,
  managed-schema validation, and typedef round-trip tests.
- Delete an owned class, attribute, typedef, relationship, index, or collection;
  re-register and verify repair where current replay repairs it. Reopen and
  verify repaired state persists. A subsequent healthy registration must be fast.
- Remove only typedef wrapper constraints/annotations while leaving the projected
  class/record equal. Confirm fallback restores complete behavior and rejects
  invalid data afterward.
- Exercise stale projection state using controlled test fixtures; preflight must
  decline or detect it. Preserve existing behavior for invalid rows during repair,
  including atomic failure without partial catalog writes.
- Test superseded upserts with base's real title/UI metadata migrations. Compare
  both first post-reopen registration and repeated calls. Test type-form changes,
  deletion/recreation, and global auto-index operations as either proven fast
  cases or explicit, correct fallbacks.
- Change migration description only and DDL content separately under `Fail` and
  `Log`. For `Log`, remove affected live schema and verify repair follows the old
  stored definition. Repeat registration after the changed package metadata was
  persisted; a cache/equality check must not bypass mismatch handling.
- Add a new migration after historical data migrations; only the new migration
  executes. Existing inserted/updated/deleted data must not be replayed on no-op
  or repair calls.
- Keep metadata-only changes durable. Test zero-migration packages and packages
  whose stored state is ahead of the incoming migration list according to current
  behavior; do not add a new downgrade policy.
- Validate missing/ambiguous foreign targets, named-alias dependencies, historical
  references, and mutually recursive batches. Installed package-owned definitions
  must not mask missing migration DDL.
- Inject a revision change during the fast proof/fence and assert normal retries
  or exhaustion, with no stale success or write. Preserve attempts/conflicts logs.

Add a differential test oracle: run the current full registration/reconciliation
on a cloned fixture state and compare its outcome/error and persistent catalog to
the proposed preflight decision. Whenever preflight returns `Unchanged`, the
oracle must produce no data/catalog change and no error. Use generated small DDL
histories in addition to actual base/filestore histories; never run the oracle in
the production fast path.

### Benchmark matrix and acceptance

Measure axes independently:

| Axis | Suggested fixtures |
| --- | --- |
| Entity count, fixed schema | 0, 1k, 100k, 1M rows |
| Unrelated catalog size, fixed target package | 0, 100, 1k unrelated classes/attributes; separately vary collections/indexes |
| Target package | Real base, real filestore, a synthetic package with superseded DDL and a foreign dependency |
| Lifetime | Repeated in-process call; first call after close/reopen; fresh process with cold compilation cache |
| Storage | Memory and persistent redb; label durability mode |
| State | Healthy; one missing schema item; metadata update; one pending migration |

Seed and open outside the measured registration interval. Record open separately
so work cannot be moved to opening and called a startup improvement. Include
package clone/normalization consistently in both baseline and candidate. Record
median/p95 and allocation/operation counts; use release builds and fixed hardware.

Acceptance: default-package healthy registration has zero scans/writes and no
whole live-catalog passes; latency has no material upward trend with entity count
or unrelated catalog size. Initial target is sub-10 ms median and sub-25 ms p95
per package after reopen on the reference development machine. Establish an
honest baseline before fixing these budgets; a cache-only improvement is not
sufficient. Use counters as deterministic CI guards, not tight wall-clock unit
assertions. The slow repair case may scale with data but must run once and be
followed by the healthy fast path.

## Logging and validation on the affected database

Retain the current structured start/completion events and their existing fields.
Add a completion `registration_path` (`unchanged`, `repair`, `metadata_update`,
`migration`, or conservative fallback), plus a bounded `fallback_reason` where
applicable. Distinguish `catalog_changed` from `migrations_executed`.

Add debug phase measurements under `operation="package_registration"` for:
normalization; preflight/identity checks; bounded validation/description compile;
schema comparison; full validation; reconciliation; snapshot comparison; index
backfill; catalog encoding; reverse-reference rebuild; row validation; commit.
Include attempt number and counts already available from that phase, such as
examined definitions, migrations replayed, scanned collections/rows, and writes.
Do not enumerate the catalog just to populate a log field. Preserve low overhead
when tracing is disabled. Log a failure outcome as well as successful completion.

For slow-path snapshot differences, emit changed categories/counts at debug/trace
level, without dumping schemas or user rows. Keep expensive diagnostics behind
explicit tracing enablement and off the fast branch. If app-level startup remains
slow after the inner fix, separately measure backend scheduling/lock wait and
database open; neither is included in the quoted registration timer.

Use a disposable copy of the affected database: capture baseline startup, run the
candidate, verify no revision change on healthy registration, restart again, and
compare phase timings. A genuine one-time legacy repair is acceptable; repeated
repairs on every startup are a failing regression to investigate.

## Backend limits and rollout

The immediate target is embedded/redb registration and the quoted inner event.
`db_postgres/src/backend.rs::with_semantic_db` additionally loads state into an
embedded database and calls `save` for write operations, including registration.
An inner fast return does not prove PostgreSQL end-to-end startup is constant time.
Measure that adapter separately before claiming backend-wide results; redesigning
its transaction/load/save strategy is not implicitly part of this fix.

Land characterization/diagnostics first, then the private compiler and comparisons,
then the guarded fast return and benchmarks. Keep the full path callable in tests
as the comparison oracle. No persisted format change or database migration is
needed, so rollback is ordinary binary rollback. Do not add a production switch
that disables repair by default.

Suggested implementation validation, always through the Nix devshell:

```sh
nix develop -c cargo test --quiet --message-format=short -p semantic_db_core
nix develop -c cargo test --quiet --message-format=short -p semantic_base
nix develop -c cargo test --quiet --message-format=short -p semantic_db_redb
nix develop -c cargo check --quiet --message-format=short
nix develop -c cargo fmt
nix develop -c cargo bench -p semantic_db_bench --bench package_registration
git diff --check
```

Run the targeted registration tests first during development, then the listed
crate suites and final checks. Retain historical migration-definition tests,
including base's bookmark and label-group compatibility tests. This document-only
task did not claim those runtime tests or benchmarks had been executed; the
implementation review records subsequent validation separately.

## Risks and review gates

- **False healthy proof:** the main risk. Full typedef and side-effect checks,
  conservative fallback, and differential replay tests are mandatory.
- **False fallback:** safe but can preserve the startup delay. Assert that every
  real default startup package hits the fast branch after reopen, not just a toy
  package or a warmed in-memory cache.
- **Duplicated schema semantics:** share conversion/normalization helpers; do not
  maintain a separate simplified type system in the optimization.
- **Dependency closure mistakes:** preserve isolated validation and alias rules;
  a dependency map assembled by scanning all catalog entries defeats the goal.
- **Hidden projection work:** review catalog helper internals and performance
  counters. A function named “check” can still hide a clone or full rebuild.
- **Broader catalog redesign pressure:** if proving unchanged state requires
  pervasive invalidation flags, persistent fingerprints, or changing core type
  semantics, stop and review that expansion separately rather than treating it as
  routine optimization.
- **Overstated attribution:** the 36-second event alone cannot prove row scanning;
  phase timings and mutation counts determine the actual cause on the affected DB.
