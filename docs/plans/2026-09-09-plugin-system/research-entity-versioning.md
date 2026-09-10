# Entity versioning audit

Date: 2026-09-10

## Conclusion

The current project does **not** implement first-class entity versioning. It has database/storage revision machinery that can support consistency and optimistic conflict detection, but there is no user-visible per-entity history, version identity, current-version pointer, version policy, or API for retrieving/restoring entity versions.

Consequently, an importer-level `versioning` setting has no real entity-versioning backend to delegate to today. Importers must not synthesize their own history model. Until a generic entity-versioning facility exists, the setting should either be absent/reserved or explicitly reject versioned mode as unsupported; ordinary import writes can use the existing upsert path and storage-level conflict protection independently.

## What exists

### Database/global revisions and optimistic concurrency

The embedded storage abstraction advertises three storage-wide capabilities: conflict detection, MVCC, and snapshot reads. A conditional commit compares an expected database revision with the actual database revision and returns either `Committed` or `Conflict`; neither type identifies an entity version ([`crates/db_core/src/embedded/storage.rs:49`](../../../crates/db_core/src/embedded/storage.rs#L49), [`crates/db_core/src/embedded/storage.rs:68`](../../../crates/db_core/src/embedded/storage.rs#L68)).

`KvEngine` exposes a single current revision, revision-scoped prefix scans, and conditional batch writes. The default revision scan simply scans current state, while the default conditional write does not itself enforce the expected revision, so callers must respect backend capabilities rather than infer uniform MVCC semantics ([`crates/db_kv/src/storage.rs:49`](../../../crates/db_kv/src/storage.rs#L49), [`crates/db_kv/src/storage.rs:53`](../../../crates/db_kv/src/storage.rs#L53), [`crates/db_kv/src/storage.rs:57`](../../../crates/db_kv/src/storage.rs#L57), [`crates/db_kv/src/storage.rs:74`](../../../crates/db_kv/src/storage.rs#L74)). `EntityStore` forwards these operations at storage scope ([`crates/db_kv/src/storage.rs:200`](../../../crates/db_kv/src/storage.rs#L200), [`crates/db_kv/src/storage.rs:365`](../../../crates/db_kv/src/storage.rs#L365)).

The in-memory engine demonstrates the intended meaning: every write/batch advances one global revision, optional snapshots clone the entire key-value map, only the last 64 snapshots are retained, and conditional writes compare against that global revision ([`crates/db_kv/src/storage/memory.rs:8`](../../../crates/db_kv/src/storage/memory.rs#L8), [`crates/db_kv/src/storage/memory.rs:29`](../../../crates/db_kv/src/storage/memory.rs#L29), [`crates/db_kv/src/storage/memory.rs:34`](../../../crates/db_kv/src/storage/memory.rs#L34), [`crates/db_kv/src/storage/memory.rs:140`](../../../crates/db_kv/src/storage/memory.rs#L140)). Its historic-read test reads a prefix at a database revision, not an entity version ([`crates/db_kv/src/storage/memory.rs:193`](../../../crates/db_kv/src/storage/memory.rs#L193)).

Backend support differs. Redb reports conflict detection but no MVCC/snapshot reads and its revision scan returns current state; it does enforce conditional commits against a global metadata revision ([`crates/db_redb/src/lib.rs:120`](../../../crates/db_redb/src/lib.rs#L120), [`crates/db_redb/src/lib.rs:153`](../../../crates/db_redb/src/lib.rs#L153), [`crates/db_redb/src/lib.rs:167`](../../../crates/db_redb/src/lib.rs#L167)). Managed PostgreSQL similarly stores one catalog revision and a `row_revision`, but its entity primary key is still `(collection_lid, entity_id)` and writes use `ON CONFLICT ... DO UPDATE`, replacing the document and row revision rather than retaining history ([`crates/db_postgres/src/managed.rs:146`](../../../crates/db_postgres/src/managed.rs#L146), [`crates/db_postgres/src/managed.rs:159`](../../../crates/db_postgres/src/managed.rs#L159), [`crates/db_postgres/src/managed.rs:349`](../../../crates/db_postgres/src/managed.rs#L349)). Thus `row_revision` is overwrite metadata, not a version-history key.

### WAL revisions

The log backend stores each committed key-value batch as an immutable event and rebuilds current state by replay. It explicitly lacks compaction and supports one writer per WAL namespace ([`crates/db_log/src/lib.rs:1`](../../../crates/db_log/src/lib.rs#L1), [`crates/db_log/src/lib.rs:14`](../../../crates/db_log/src/lib.rs#L14)). Commits append an event and then apply operations to the in-memory current-state map ([`crates/db_log/src/engine.rs:62`](../../../crates/db_log/src/engine.rs#L62)). This is an internal durability/replay log, not an entity-history API: there is no entity-version identity, current pointer, historical entity query, or restoration contract exposed from it.

### Format and identifier versions

Two uses of “version” are serialization compatibility markers:

- Stored entity payloads carry an entity **format** version and reject unsupported encodings ([`crates/db_kv/src/storage.rs:524`](../../../crates/db_kv/src/storage.rs#L524)).
- PostgreSQL's `pg1.` entity ID is a versioned encoding for an unambiguous typed primary-key tuple ([`crates/db_postgres/src/sql.rs:353`](../../../crates/db_postgres/src/sql.rs#L353), [`crates/db_postgres/src/sql.rs:399`](../../../crates/db_postgres/src/sql.rs#L399), [`crates/db_postgres/src/sql.rs:490`](../../../crates/db_postgres/src/sql.rs#L490)).

Neither represents versions of entity content.

## What is absent

The public entity types consist only of `id`, `collection`, and `object` ([`crates/db_core/src/backend.rs:88`](../../../crates/db_core/src/backend.rs#L88), [`crates/db_core/src/query.rs:1164`](../../../crates/db_core/src/query.rs#L1164)). The public backend offers current `insert`, `get`, and `delete` operations, with no version selector or history operation ([`crates/db_core/src/backend.rs:127`](../../../crates/db_core/src/backend.rs#L127)). Batch upsert likewise has only collection, ID, and object; `BatchOutcome` returns the resulting dataset and aggregate statistics, not entity version identifiers ([`crates/db_core/src/query.rs:1173`](../../../crates/db_core/src/query.rs#L1173), [`crates/db_core/src/query.rs:1224`](../../../crates/db_core/src/query.rs#L1224)).

Specifically absent are:

- a stable logical entity identity distinct from a version identity;
- immutable per-entity revisions or a version chain;
- a current/head pointer and rules for advancing it;
- version creation policies or a generic per-write versioning option;
- APIs to list, fetch, compare, restore, or delete versions;
- user-visible version metadata such as author, reason, timestamp, or provenance;
- retention/compaction semantics for entity histories;
- backend-independent historic reads (current snapshot support is optional and storage-scoped);
- per-entity compare-and-swap. Existing conflict checks guard a whole storage revision.

## Importer implications

Import integration can reuse the current batch upsert/delete operations, database-wide conditional commit where supported, and snapshot reads where a backend advertises them. These are useful for atomicity, detecting concurrent writes, and producing a consistent import view. They do not provide imported entity history.

The future import contract may include an importer default and per-run override for a generic entity-versioning policy, but it should be modeled as delegation to a future database/runtime facility. The import subsystem should not define version IDs, duplicate entities as revisions, maintain head pointers, or create import-specific history tables. Once generic versioning exists, import provenance and job/run identity can be supplied as version metadata through that generic API.

For the initial system, versioned mode should fail with a clear unsupported-capability error rather than silently behaving like an upsert. Non-versioned mode should perform normal deterministic-ID upserts. Optimistic concurrency should remain a separate import execution/commit concern and must not be described as entity versioning.
