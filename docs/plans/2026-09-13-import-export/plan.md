# Streaming database import and export

Date: 2026-09-13

Status: implementation plan; no implementation changes made by the planner.

## Goal

Provide reusable, streaming entity export/import and a local CLI:

- JSONL contains one complete entity record per line, including its collection and ID.
- A tar archive contains the same JSONL plus the contents of blobs that exported entities reference through the file content-hash attribute.
- Import upserts in configurable batches, defaulting to 1,000 entities.
- A per-call database settings struct can disable foreign-key validation for unordered imports.
- `semantic db export` and `semantic db import` open local storage in-process, with no HTTP/RPC transport.
- The transfer layer never collects the entity dataset, blob inventory, or a blob's bytes in RAM. Buffers, queues, and batches are bounded. Temporary disk spooling is permitted and documented.

Preserve ordinary write validation, entity identity, existing storage keys, indexes, and relationship semantics. Do not weaken persistent validation configuration to make an import work.

## Repository findings that determine the design

1. `crates/db_core/src/backend.rs` contains `Backend`, `Db`, and `EntityRecord { id, collection, object }`. Query and select APIs return complete `Vec<Object>` results; they are unsuitable as the export implementation.
2. `EntityStorage::scan_collection_stream` in `crates/db_core/src/embedded/storage.rs` already returns a lazy `BoxEntityScan`. `EntityStore` forwards this to KV prefix iterators. Redb's `scan_prefix_stream` opens a read transaction and lazily decodes entries. Expose this capability instead of inventing SQL pagination or wrapping a collected query result in a stream.
3. `EmbeddedBackend` wraps the synchronous database in `Arc<RwLock<_>>` and runs operations through `spawn_blocking_on`. The export bridge must obey the existing runtime abstraction and apply backpressure.
4. `BatchReturn::Stats` and the point-addressed compact transaction path already exist. Ordinary `execute_batch` returns a `BatchOutcome` containing a dataset and loads touched collections. Import must not call that path.
5. Compact returning is not by itself a memory guarantee. `try_compact_batch` falls back for indexed/transitive relationship changes, missing reverse-reference infrastructure, predicate mutations, and backends without revision conflicts. `update_relationship_edges` and `compute_relationship_edges` also contain collection/graph materialization. Base entity-label links use an indexed relationship, so this is a real import case.
6. Recursive stored validation combines value-shape/constraint checks with reference existence and reference target-class checks. Legacy reference validation is a separate path. The FK option must reach both, including compact and dataset transactions.
7. The file content hash is `semantic_data::filestore::ATTR_FILE_CONTENT_HASH_SHA256`, whose qualified name is `semantic:filestore:file:content_hash_sha256`. Blob bytes are addressed separately by `ATTR_FILE_FILESTORE_LOCATOR`, commonly `file-sha256-<hash>/<uuid>`. The hash is not generally an object-store key. Full restore must preserve entity locators and restore bytes at those exact logical keys.
8. `semantic_app::storage::{resolve_storage, open_app}` already resolve redb/log/logfs URIs, filesystem/logfs blobs, shared `log:<blob>` storage, passwords, and local defaults. Reuse that wiring without starting a server or dispatching RPC commands.
9. Existing serde serialization of `Value` is flat and loses distinctions such as temporal types, bytes versus strings, and numeric widths. `semantic_data::value::serde::typed::{TypedValue, TypedRef}` already provides explicit typed JSON. Use that encoding rather than changing `Value` serialization globally.
10. Internal collections contain catalog records, validation state, relationship indexes, and reverse-reference indexes. Direct writes to internal collections are explicitly prohibited. These are implementation state rather than portable application entities.
11. The log backend explicitly keeps database state in memory. Transfer-layer streaming cannot turn it into a disk-resident backend. Redb is the backend for the end-to-end bounded-memory acceptance test.

## Scope decisions

### Entities and schema

Export every entity in every non-internal collection, including non-default collections, job/import/file entities, custom classes, and file subclasses. Use stored objects with canonical/qualified field names, without query projection or computed output formatting.

Do not export/replay internal catalog or derived index rows as ordinary upserts. This format is a logical entity transfer, not a raw storage image. The destination must already have compatible custom collections/packages installed; normal local application initialization supplies the built-in packages. Unknown collections/classes are explicit errors. Do not auto-create permissive collections or overwrite catalog local IDs to suppress such errors.

Document this assumption in the CLI help and format documentation. A self-contained custom-schema backup is a separate extension requiring a schema format and merge/conflict semantics; it must not be implemented by bypassing internal-collection protections.

### Defaults

- Export format: `jsonl`; `--full` selects tar.
- Import format: `auto`, with explicit `jsonl` and `tar` overrides.
- Import batch size: 1,000; reject zero before opening or mutating the DB.
- Existing DB write APIs: FK validation enabled.
- Transfer import options: FK validation disabled by default, because exported order is not dependency order. `--validate-foreign-keys` opts into per-batch validation; library callers can choose either setting explicitly.
- No implicit post-import validation pass that collects the complete DB. Explain that disabling FK validation permits dangling references and that FK checks resume for later ordinary writes. A future streaming validation command may provide a separate audit.
- No compression in v1; the additional format is an ordinary tar file. Compression can be piped externally.
- Export files must not silently overwrite existing files. Use a temporary sibling and a no-clobber publication step; stdout can contain partial output if writing fails.
- Import is an upsert/merge, not replacement. Unmentioned destination entities and blobs remain.

## Public format

### JSONL v1

Each nonblank line is a versioned entity envelope:

```json
{"version":1,"collection":"entities","id":"example","object":{"object":{"id":{"string":"example"},"title":{"string":"Example"}}}}
```

The default collection is `entities`. The inner `object` is the existing typed `Value::Object` representation, not flat arbitrary JSON.

Define a transfer-specific serde DTO rather than adding serde derives to core types just for this format. Require `object` to decode to `Value::Object`. Require a nonempty collection and ID; reject envelope/object identity mismatches instead of silently normalizing the wrong record. Preserve the database's established canonical identity conventions.

Use `TypedRef` when serializing borrowed values and `TypedValue` when deserializing. An allocation for one record is acceptable; a full-file string or `Vec<EntityRecord>` is not. Keep `version` per line so the plain JSONL has no non-entity header record. Reject unsupported versions with a line number.

Support LF, CRLF, a final record without a trailing newline, and blank lines. Blank lines still increment physical line numbers. Empty input is a successful no-op. Parse one line at a time, not a stream of adjacent JSON values that accidentally accepts multiline objects. Include source name and 1-based line number in errors.

Bound line growth explicitly: expose `max_record_bytes` in library import options, with a generous documented default such as 64 MiB and a CLI override. This bounds malformed/unbounded lines; it does not imply that every record requires a 64 MiB buffer. Reject non-finite floats or other existing typed-serde non-roundtrippable values clearly rather than emitting silently lossy JSON. Add tests before deciding whether a narrowly scoped transfer encoding extension is necessary.

### Tar v1

Archive layout:

```text
entities.jsonl
blobs/<64-character-lowercase-sha256>
blobs/<another-sha256>
```

There is exactly one regular `entities.jsonl` entry. Blob content is deduplicated by hash. The JSONL is byte-for-byte the same encoding as standalone export and retains every original `filestore_locator`. One hash may map to multiple destination locators; restore the verified bytes to each referenced locator.

Export writes `entities.jsonl` first, then hashes in lexical order. Import accepts entries in any order through disk staging. Optional directory entries for `blobs/` may be ignored, but no symlinks, hardlinks, devices, absolute paths, parent traversal, or unknown regular entries are accepted. Never call unrestricted `tar::Archive::unpack`.

An entity contributes a blob reference only if its canonical content-hash attribute is present. Recognize the established short alias when processing legacy/noncanonical input through a dedicated helper, and reject conflicting qualified/short values. This applies regardless of the entity's class and collection. A locator without the hash does not cause a blob to be included. A hash reference with invalid hash syntax, a missing/invalid locator, or conflicting hashes for one locator makes full export/import fail with entity context; it must not be silently dropped.

Do not enumerate all object-store keys and then filter. Build the reference inventory only while reading exported/imported entity records. Do not include cleanup candidates, abandoned uploads, superseded locators, or blobs merely listed by the store. Unknown extra tar blobs are rejected, and missing referenced hashes are detected before reporting success.

## Component and API changes

### 1. Add per-write settings without breaking existing callers

Files:

- `crates/db_core/src/config.rs` or a focused new `write_settings.rs`, reexported by `lib.rs`.
- `crates/db_core/src/backend.rs`.
- `crates/db_core/src/embedded/backend.rs`.
- `crates/db_core/src/embedded/db.rs`.
- `crates/db_core/src/embedded/db/compact.rs`.
- `crates/db_core/src/validation/stored.rs` and `validation.rs`.
- `crates/app/src/db.rs`.

Suggested settings:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteSettings {
    pub validate_foreign_keys: bool,
}

impl Default for WriteSettings {
    fn default() -> Self {
        Self { validate_foreign_keys: true }
    }
}
```

Add settings-aware variants while preserving existing methods as default-settings wrappers:

```rust
execute_batch_with_settings(batch, settings)
execute_batch_returning_with_settings(batch, returning, settings)
```

Expose them on synchronous `EmbeddedDb`, async `Backend`/`Db`, and application `SemanticDb`. `Db` continues to accept public query batch types and converts them as today. `SemanticDb` continues to accept core batches. Thread settings as explicit arguments through each transaction attempt; never store a temporarily disabled flag on the DB and never mutate persistent validation state.

Traits may supply backwards-compatible defaults that forward default settings to existing methods and return an explicit unsupported-settings error for non-default settings. This keeps PostgreSQL, federation, mocks, and downstream implementers compiling without pretending they honored an option. Implement the local embedded capability fully. Do not disable actual PostgreSQL constraints globally or change RPC payloads in this task.

Add a settings-aware stored validation entry point while retaining `validate_stored_object` as the enabled wrapper. When FK validation is disabled:

1. Still verify the reference value's representation and nonempty ID.
2. Still resolve and record reverse-reference information.
3. Skip the target lookup, missing-target error, and target-class validation.
4. Keep recursive type checks, required fields, enum/string/range constraints, primary IDs, and uniqueness checks active.

Legacy validation must honor the same bypass, preserving its existing non-FK behavior. Compact incoming-dependent validation should skip FK-only dependency checks in this mode, but still maintain reverse-reference indexes for later writes. Validate both disabled and enabled settings through compact and fallback paths. Do not equate this setting with the database's recursive-validation activation flag.

### 2. Expose a real entity stream

Add a boxed `Stream<Item = Result<EntityRecord, DbError>> + Send` type and `scan_entities` method on `Backend`, `Db`, and `SemanticDb`, defaulting to unsupported on providers without a true streaming implementation. Do not implement a default by collecting a query.

Implement the embedded scan over a catalog snapshot and each non-internal collection's `BoxEntityScan`, converting local collection IDs to stable names and returning canonical stored objects. Keep only the catalog metadata, current iterator, and a small fixed queue in memory.

Bridge synchronous iterator work to async consumers using a bounded channel and a blocking producer, or bounded page pulls over an owned iterator. Use existing runtime facilities; with a channel, schedule the producer independently before returning the stream, propagate producer errors, and close promptly when the consumer drops. Do not await producer completion before returning the receiver, as that deadlocks a bounded queue. Do not hold a non-Send lock guard over `.await`.

A local offline export must be internally consistent: retain the database read lock for the scan, or use a revision/catalog fence and fail if it changes. Use a single logical snapshot across collection iteration, not separate unfenced snapshots. Clarify that log stores require their existing single-writer discipline. Do not claim consistency against unsupported concurrent cross-process writers.

Unit tests must prove that requesting the first record does not exhaust the source, a slow consumer bounds producer progress, cancellation stops work, and source failures propagate.

### 3. Ensure batched imports do not secretly materialize the DB

Use point-addressed `BatchOperation::Upsert` and `BatchReturn::Stats`, with the settings-aware API. Flush when the configured batch size is reached and at EOF. Maintain only the current parsed record, batch, and counters. Duplicate IDs have sequential upsert semantics, including duplicates inside one batch. A failed batch is atomic; earlier batches remain committed.

Add a strict bounded-execution path or requirement used by import. A compact fallback that calls `load_dataset_for_batch` is not acceptable. Before implementing the transfer, test and address these cases:

- ordinary inserts and overwrites;
- FK-disabled imports with forward references and cycles;
- indexed relationship changes, including base label assignments;
- incoming references on changed target types;
- unavailable compact support and backfill requirements.

For ordinary point writes, reuse `TxView`, per-key uniqueness checks, and incremental row/reverse-reference writes. Return an explicit unsupported/bounded-execution error before mutation if the backend cannot honor the memory contract. Do not silently fall back to dataset returning.

Indexed/transitive relationships are an implementation gate, not permission to disable or corrupt their indexes. First inspect whether a local, semantics-preserving streaming update can reuse contributor/count storage and avoid loading source entities. A complete general bounded solution needs disk-backed graph work tables (adjacency, BFS queue/visited, old/new edge comparison) and a bounded way to commit derived writes while preserving batch atomicity. Simply deleting the existing compact fallback is incorrect: the current closure code expects complete source rows in its `after` dataset and can lose unchanged edges.

If the required fix turns into a broad rewrite of core transaction/storage/index APIs, stop and present this concrete constraint under `AGENTS.md` before implementing that expansion. An initial explicit error is safer than violating the memory requirement, but support for base indexed relationships remains an incomplete acceptance item until resolved. Do not mark the overall feature complete while concealing that limitation.

Also state the scope of the guarantee: streaming transfer adds bounded working memory; the existing log database is intrinsically memory-resident. End-to-end bounded-memory verification uses redb.

### 4. Implement a reusable transfer module

Add `crates/app/src/transfer/mod.rs`, `jsonl.rs`, `archive.rs`, and a small private `blob_index.rs` as useful boundaries, with `pub mod transfer` in `app/src/lib.rs`. Keep the existing content-ingestion `imports` module unchanged; database restore is a distinct operation.

Suggested API shape:

```rust
pub enum TransferFormat { Jsonl, Tar }
pub enum ImportFormat { Auto, Jsonl, Tar }

pub struct ImportOptions {
    pub format: ImportFormat,
    pub batch_size: usize,
    pub write_settings: WriteSettings,
    pub max_record_bytes: usize,
    pub temp_dir: Option<PathBuf>,
}

pub struct ExportOptions {
    pub format: TransferFormat,
    pub temp_dir: Option<PathBuf>,
}

pub struct TransferStats {
    pub entities: u64,
    pub blobs: u64,
    pub blob_bytes: u64,
    pub batches: u64,
}
```

Provide `export` and `import` against a local `SemanticDb` plus optional `DynObjStore` and generic buffered/async I/O adapters. Keep the format engine testable without a CLI or `SemanticApp`. A thin scope/context wrapper may obtain the DB and default file store inside the app crate, where the existing default-file-store accessor is already available.

Use full `Result<T, E>` signatures, never new convenience result aliases. Define a focused `TransferError` carrying I/O, parse line, archive entry, blob hash/locator, database, and progress context. Library methods must return stats, not print.

Dependencies: promote `tempfile` from dev-only to runtime where used; add `tar` and, if using sync tar behind async I/O, `tokio-util` I/O bridge support. Use redb as a bounded-cache temporary index or an equivalently bounded disk-backed structure. Do not replace it with an unbounded `HashSet` merely because the expected data is small. Update `Cargo.lock` normally.

### 5. JSONL streaming

Export: consume `scan_entities`, serialize each envelope with typed values into the buffered writer, append a newline, update counters, and drop the entity. Flush and propagate flush failures before success.

Import: bounded `read_until`/equivalent with an explicit size check, parse one envelope, validate identity/version, append an upsert, and flush batches with stats-only settings-aware execution. Do not reuse `cli::shared::FileInputArgs::read`, which calls `read_to_string`.

Do not pre-scan plain JSONL solely to order entities. With validation disabled, record order is irrelevant to referential existence. With validation enabled, the committing batch sees its final rows and existing DB state; references into later batches fail normally.

### 6. Full export

Tar headers require entry sizes before writing. Spool JSONL to a temporary file while streaming entities once, and simultaneously build a disk-backed reference index:

- unique hash -> a candidate source locator and optional MIME metadata;
- `(hash, locator)` -> restore destination association;
- locator -> hash, to reject conflicting content claims.

Use a bounded cache/transaction size for that index. Do not store every locator for a popular hash in a single growing value; make each association a separate key.

After the scan completes, append the known-size JSONL temp file to tar. Iterate hashes lazily. For each hash, resolve a referenced source locator, fetch with `get_stream`, and stream bytes through SHA-256 verification. Missing bytes and mismatched hashes are errors. Never use whole-object `get` or stream `try_collect`.

If reliable byte size is available, stream directly into the tar entry while checking the actual byte count and digest. Otherwise spool just that blob to a temporary file, measuring and hashing it, then append that file and immediately remove it through RAII. This bounds peak temporary disk to JSONL/index plus at most one source blob. Candidate locators for duplicate hashes can be tried without a whole-store listing if a referenced candidate is absent; otherwise report the missing reference clearly.

Use deterministic header metadata (regular file, fixed permissions, zero uid/gid/mtime) and finish the tar writer explicitly. Publish a path output only after success; preserve the original error on cleanup failure.

### 7. Full import

For robust entry ordering and blob-before-entity publication, stream tar entries into a private temporary directory:

1. Validate every entry name/type before writing bytes; only use generated safe local paths derived from validated hashes or fixed filenames.
2. Spool the one JSONL entry while parsing records incrementally into the disk reference index. Do not upsert application entities yet.
3. For each `blobs/<hash>` entry, stream it to a staging file and calculate SHA-256; enforce its declared length and verify the name against the computed digest.
4. Record received hashes in the disk index. Reject duplicate entity JSONL and duplicate blob entries. Entry metadata and duplicate tracking must not become unbounded RAM collections.
5. After archive parsing, verify every referenced hash exists and every received blob is referenced. Reject malformed JSONL, conflicting locator hashes, extra blobs, or missing blobs before writing destination entities.
6. Iterate `(hash, locator)` references and restore verified bytes to exact original locators with object-store streaming writes. Preserve MIME metadata where the store supports it. A locator already containing matching content is a no-op. A locator containing different content is an explicit conflict, not a silent overwrite of potentially live bytes.
7. Rewind the staged JSONL and run the same batched import implementation as plain JSONL.

Full import may use temporary disk proportional to archive size but not RAM proportional to it. This is a deliberate tradeoff: it supports pipes, arbitrary tar entry order, known-size object-store writes, digest verification before publication, and records whose locators differ from content hashes.

Reuse object-store streaming and atomic-create/copy behavior from the existing `persist_content` implementation without invoking ordinary file creation (which generates new locators/IDs and can run media analysis). Honor provider capabilities; reject a provider that requires whole-object buffering rather than silently collecting bytes. Do not run jobs or media analysis for imported entities.

An archive failure leaves no new application entities. Blob publication or later batch failures can leave new unreferenced destination blobs; they are safe for normal retention-based maintenance. Do not automatically delete destination blobs or earlier committed entities as rollback. Include committed batch/entity counts in an import failure.

### 8. Local CLI

Add `crates/cli/src/cmd/db/mod.rs` (plus `import.rs`/`export.rs` if useful), register `SubCmd::Db`, and dispatch in `crates/cli/src/lib.rs`.

Interface:

```text
semantic db --db-uri redb:/path/to/db export entities.jsonl
semantic db --db-uri redb:/path/to/db --blob-uri file:///path/to/blobs export --full backup.tar
semantic db --db-uri redb:/path/to/db import entities.jsonl
semantic db --db-uri redb:/path/to/db --blob-uri file:///path/to/blobs import backup.tar --batch-size 1000
semantic db --data-dir /path/to/data export --format tar -
semantic db --data-dir /path/to/data import --format tar -
```

Shared local options: `--db-uri`, `--blob-uri`, `--data-dir`, `--temp-dir`, using the established environment variables. Export accepts `--format jsonl|tar` and an ergonomic `--full` alias selecting tar; reject contradictory options. Import accepts `--format auto|jsonl|tar`, `--batch-size`, `--validate-foreign-keys`, and the record-size limit. Input/output positional paths accept `-` for standard streams.

Auto-detection reads a small prefix and replays those bytes to the chosen parser; do not require seek. Prefer actual content recognition (JSONL first non-whitespace `{`, valid tar header/checksum) to extension-only guesses. Empty input is JSONL. A damaged recognized tar remains a tar error. Explicit format always wins.

Reuse local storage resolution and logfs password prompting. Export uses the existing non-creating open mode; import can use auto-create consistent with local CLI conventions. Verify actual `DbOpenMode` names in code. For plain redb JSONL, avoid requiring a live blob store if practical; `log:<blob>` necessarily needs its backing store. Reuse provider/opening logic rather than duplicating URI parsing.

Keep stdout exclusively for export bytes. Errors, progress/final stats, password prompts, and URI-selection notices go to stderr. Return nonzero on parse/read/write/hash/database failures. Opening a DB held by the server may fail under the backend's normal lock; explain offline/local operation instead of attempting to connect to that server.

Before opening an output, guard against aliasing the input database file or using a destination that would truncate source storage. A temporary sibling plus no-clobber final publication avoids the ordinary collision case.

## Verification plan

Write behavior tests that catch streaming and correctness errors, rather than tests that only mirror helper code.

### Database settings and bounded execution

- Enabled/default FK validation rejects missing and wrong-class targets.
- Disabled validation permits forward/cyclic references across batches while later ordinary writes still validate.
- Disabled FK validation still rejects malformed reference values, unrelated recursive constraints, ID errors, and uniqueness violations.
- Reverse-reference bookkeeping survives disabled writes and reopen; later target deletion/type-change validation works.
- Settings are isolated per call and per retry; a failed import does not change future defaults.
- Unsupported providers reject nondefault settings instead of ignoring them.
- Point-upsert imports on a large redb collection show reads/work proportional to batch/touched rows, with no collection scan fallback.
- Include actual base label relationship assignments and overwrites. This test is required to resolve the indexed-relationship implementation gate.

### JSONL

- Multiple collections, arbitrary record order, duplicate IDs, overwrite semantics, empty input, CRLF, missing final newline, and blank lines.
- Exact typed-value round trips: null/void, numeric widths and boundaries, UUID, date/time/datetime/duration, bytes, lists, nested objects, map and variant values.
- Malformed/version/identity errors include line/source; oversized records stop with a bounded buffer.
- Batch sizes 1, default, and a non-divisor produce the expected commits; zero is rejected before side effects.
- An invalid later record or failed batch preserves earlier committed batches and reports their counts.
- Instrumented reader/source demonstrates progress before EOF and bounded read-ahead; lazy stream cancellation stops the producer.

### Tar/blobs

- Export/import with filesystem object storage, blobs larger than copy buffers, duplicate hash references under multiple locators, and file subclasses/custom classes.
- Hash-qualified references include bytes; locator-only entities and abandoned/unreferenced/superseded blobs do not.
- Original entity objects and locator values remain equal after full restore.
- Missing source blob, hash mismatch, conflicting locator hashes, malformed hash/locator, and short/qualified alias conflict fail explicitly.
- Entities-first, blobs-first, empty archive data, zero-length blob, missing JSONL, duplicate JSONL/blob, missing/extra blob, truncation, invalid checksum, and traversal/link/device entries.
- Existing matching destination locator is idempotent; different content is a conflict.
- Nonseekable input/output work; unknown blob size uses a file spool; a store test double fails if a whole-object API is called.
- Temporary files are removed on success, parser errors, DB errors, and consumer cancellation.
- Large input test uses a generated reader/blob stream and a small batch/cache to demonstrate bounded memory independently of total rows/bytes; do not allocate the test's whole input as a `Vec`.

### CLI and integration

- Clap/help tests for the new namespace, formats/full alias, zero batch size, local/environment options, and `-`.
- End-to-end redb export/import round trip with no server running; exercise plain JSONL and tar, content sniffing, and re-import upserts.
- Verify stdout contains only valid payload bytes and overwrite refusal preserves an existing output.
- Reopen the destination and verify entities, indexes, references, and file reads.
- Cover logfs/shared-log storage if the environment provides the existing test facilities, while documenting its intrinsic RAM use.

Run checks and tests through Nix when available. The repository exposes `.#base`, which avoids unnecessary UI dependencies for these Rust packages:

```sh
nix develop .#base --command cargo check --quiet --message-format=short -p semantic_db_core -p semantic_db_kv -p semantic_db_redb -p semantic_db_log -p semantic_app -p semantic_cli
nix develop .#base --command cargo test --quiet --message-format=short -p semantic_db_core -p semantic_db_kv -p semantic_db_redb
nix develop .#base --command cargo test --quiet --message-format=short -p semantic_app --features storage-redb,storage-logfs
nix develop .#base --command cargo test --quiet --message-format=short -p semantic_cli
nix develop .#base --command cargo fmt --all
nix develop .#base --command cargo check --quiet --message-format=short -p semantic_db_core -p semantic_db_kv -p semantic_db_redb -p semantic_db_log -p semantic_app -p semantic_cli
```

Adjust focused test filters while developing, then run the relevant complete package suites. Run broader workspace checking only if the additive trait APIs or feature matrix need it. Preserve unrelated changes; the planner observed an existing untracked `docs/plans/2026-09-12-js-embed/` directory.

## Implementation sequence and completion criteria

1. Add write settings and validation plumbing; verify default behavior and FK-only bypass.
2. Expose lazy entity scanning; prove backpressure/cancellation and snapshot behavior.
3. Establish the bounded point-upsert import path and resolve indexed-relationship support before claiming a memory guarantee.
4. Implement typed JSONL DTO, streaming import/export, counters, and error context.
5. Implement disk-backed blob inventory and tar streaming/staging with digest verification and exact-locator restoration.
6. Add local CLI wiring and concise format/usage documentation in `crates/cli/README.md` and an appropriate developer document.
7. Run focused/full relevant tests, formatting, and checks.
8. Have the requested fresh Astra reviewer inspect correctness, format round trips, hidden materialization, FK isolation, archive handling, and local CLI behavior. Fix review findings and rerun affected verification.

The implementation is complete only when JSONL and tar round trips work, only referenced blobs are present, FK suppression is scoped, batch size works, CLI paths are local, and the relevant streaming tests—including indexed relationship imports—pass. Explicitly report any remaining schema portability, backend-memory, or bounded-execution limitation; do not describe an unsupported case as a fully handled requirement.
