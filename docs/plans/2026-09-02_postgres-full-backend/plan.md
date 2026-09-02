# PostgreSQL full-backend plan

## Goal

Make `semantic_db_postgres` a complete backend while supporting two deliberately
different physical layouts:

- **Semantic layout**: the full `Backend` contract, backed by generic entity and
  relation-edge tables containing lossless tagged `jsonb` documents.
- **Relational layout**: one strict concrete class per PostgreSQL table. Managed
  mode creates tables from classes; read-only mode discovers existing tables and
  maps them to classes. Classless, untyped, and polymorphic collections are
  rejected instead of silently falling back to document storage.

Physical layout and schema ownership are independent concepts. Preserve the
existing `PostgresBackend::new(pool, PostgresMode)` API: `Managed` maps to a
managed semantic layout and `Discovery` maps to read-only relational layout.

## Public configuration

Add `PostgresLayout`, `PostgresSchemaOwnership`, and a non-exhaustive
`PostgresBackendOptions` with builders for:

- `semantic_managed()` (the recommended default, schema `_semantic`)
- `semantic_read_only(schema)`
- `relational_managed(schema)`
- `relational_discovery(schemas)`
- metadata schema, `DbConfig`, transaction retry count, identity policy, and
  legacy discovery-ID compatibility

Add `PostgresBackend::new_with_options(pool, options)`. Keep `PostgresMode`,
`PostgresConfig`, `create_pool()`, and existing construction calls compatible.
Reject invalid combinations during construction.

## Crate structure

Split responsibilities into focused modules as implementation warrants:

```text
backend.rs                 orchestration and Backend implementation
config.rs                  layouts, ownership, options, compatibility mapping
error.rs                   SQLSTATE/pool/codec error conversion
codec.rs                   tagged JSON and native PostgreSQL value conversion
catalog_store.rs           bootstrap, snapshots, revisions, mapping metadata
transaction.rs             serialized writes and serializable retries
storage/{mod,semantic,relational}.rs
query/{mod,datasource,stats,pushdown,output}.rs
ddl/{mod,semantic,relational}.rs
package.rs
relationships.rs
discovery/{mod,catalog,pg_catalog,types,identity}.rs
sql.rs
```

Extract backend-neutral helpers from `db_kv` into `db_core` only when needed,
without changing public behavior or core types. Likely candidates are output
formatting, insert-source materialization, mutation-limit validation,
relationship-edge derivation, and access-path extraction.

## Managed metadata and concurrency

Create a quoted metadata schema (default `_semantic`) and versioned tables:

- `catalog_state(singleton, revision, format_version, layout, snapshot,
  updated_at)`
- `collection_map(collection_lid, collection_name, class identity, physical
  schema/table, managed, mapping)`
- `field_map(collection_lid, field_id, canonical field, physical column/type,
  writable, mapping)`

The `CatalogStorageSnapshot` serialized with `facet_json` is authoritative for
packages and applied migrations. Store its own format version.

Bootstrap and every catalog mutation must:

1. Acquire a transaction-scoped advisory lock derived from the metadata schema.
2. Lock `catalog_state` with `FOR UPDATE`.
3. Create a fresh core catalog or load the latest snapshot.
4. Apply idempotent core migrations and validate the stored layout.
5. Apply physical changes and persist the incremented snapshot atomically.
6. Commit before replacing the in-memory `SharedCatalog`.

Use an in-process write mutex plus the PostgreSQL row/advisory locks for
cross-process correctness. Check the database catalog revision before planning
or mutation and reload stale local state. Use serializable transactions for
multi-row writes, packages, and batches; retry SQLSTATE `40001` and `40P01` up
to the configured limit. Entity-only writes do not change catalog revision.

## Lossless semantic codec

Ordinary JSON cannot preserve every `Value`. Store a versioned tagged document:

```json
{"format":1,"value":{"t":"object","v":{"score":{"t":"u64","v":"18446744073709551615"}}}}
```

Give every variant an explicit tag. Encode wide integers as decimal strings,
floats as exact IEEE bit strings, bytes as unpadded base64url, UUID/IP as
canonical strings, temporal/duration values as exact stable components, maps as
ordered key/value pairs, and variants with their type/name/value. Preserve void
versus null, numeric widths, signedness, non-string map keys, NaN/infinity, and
negative zero. All decoding is fallible; no panic or silent null fallback.

Add exhaustive round-trip and malformed-input tests for every `Value` variant.

## Semantic layout

Create:

```sql
entities(
  collection_lid bigint,
  entity_id text,
  object_type text,
  parent_id text,
  document jsonb,
  row_revision bigint,
  primary key(collection_lid, entity_id)
)

relation_edges(
  relation_lid bigint,
  source_collection_lid bigint,
  source_id text,
  target_collection_lid bigint,
  target_id text,
  edge_collection_lid bigint null,
  edge_entity_id text null,
  primary key(relation_lid, source_collection_lid, source_id,
              target_collection_lid, target_id)
)
```

Add B-tree indexes for collection/type/parent and both relation traversal
directions. Add `GIN(document jsonb_ops)` as the general document index. Do not
claim that GIN accelerates every predicate.

Materialize catalog `IndexSchema`s as typed physical columns with names derived
only from persisted local IDs. Writes populate them from canonical values.
Use B-tree indexes for equality/range/order, partial unique indexes scoped by
collection, and `tsvector` plus GIN for full text. Index creation adds the
column, backfills by decoding in bounded batches, creates the index, and saves
the catalog in one transactional DDL operation. Reject index types without a
stable PostgreSQL representation.

### CRUD and batches

Match `KvDb` semantics exactly:

- canonicalize aliases and protect internal collections
- evaluate defaults once per transaction/batch
- call the shared write preparation/validation path
- validate IDs, refs, types, uniqueness, and computed fields
- atomically update entity data, materialized index columns, and relation edges
- support every current `BatchOperation`, sequential visibility, limits,
  returning data, complete touched datasets, and exact statistics
- make late failures roll back the whole batch

Implement `get`, upsert, delete, scans, equality/multi-lookups, and delta
application behind a layout-neutral internal storage interface.

### Query execution

Implement a PostgreSQL `AsyncPhysicalDataSource`. Parse SQL/PRQL using the
PostgreSQL dialect, canonicalize the AST, optimize with `Optimizer::core()`, and
execute through the shared physical executor over canonical objects. This is
the correctness path for joins, subqueries, aggregation, distinct/group/having,
ordering, nested refs, semantic functions, projections, aliases, `FieldFormat`,
computed fields, and the `all` collection.

Push down only operations with proven equivalent PostgreSQL semantics:
metadata/native equality and ranges, null checks, fully supported Boolean
expressions, literal lists, safe ordering/limit/offset, and declared index
lookups. Evaluate residual predicates before returning rows. Unsupported
expressions fall back to core execution.

Provide an immutable stats snapshot using `pg_class`, `pg_stats`, `pg_index`,
and semantic mapping data. Make `plan()` and `explain()` parse the supplied
query and reflect the actual optimized plan. Remove textual SQL collection-name
rewriting. `query_to_sql()` must either be truly layout-aware or return a clear
error when physical mapping is required.

### DDL, packages, and relationships

Apply every current `DdlOperation` to a cloned catalog, validate the final
catalog, compute the physical diff, backfill as necessary, and atomically save
the snapshot. Cover attributes, type definitions, record types, classes,
collections, indexes, relationships, and auto-index configuration.

Port current `KvDb::upsert_package` behavior rather than the obsolete PostgreSQL
follow-up: validate and normalize packages, compare applied migrations under
`DbConfig`, execute interleaved schema/data operations in order, and commit
physical schema, data, packages, migrations, and catalog together. Concurrent
registration must execute a migration once.

Maintain direct embedded/external relationship edges transactionally after
entity, catalog, and package changes. Keep direct edges authoritative; resolve
transitive traversal with a cycle-safe recursive CTE or the exact core fallback.

## Relational layout

Implementation direction: managed relational mode reuses the semantic/KV engine
as its correctness and query-execution source, and maintains the native
one-class-per-table representation as a projection in the same serializable
PostgreSQL transaction. Consequently every committed native table is consistent
and reopen-safe, while direct out-of-band writes to projected tables are not
supported. This avoids duplicating the planner, validation, package, batch, and
relationship semantics.

Managed relational mode requires strict registered-schema collections with
exactly one concrete class, an explicit matching object type, and a deterministic
collection/class binding. Validate the final DDL batch so class and collection
operations may occur in either order. Reject unsupported classless, untyped, or
polymorphic shapes with precise errors; do not use a hidden document fallback.

Use stable physical names based on local IDs (`semantic_c_<collection_lid>`,
`semantic_f_<field_id>`) and mapping metadata. Tables contain `_semantic_id`,
`_semantic_type`, `_semantic_parent`, and flattened stored attributes. Computed
attributes are not stored. Map lossless scalar types to native PostgreSQL types;
use tagged `jsonb` for complex/named values without lossless native mappings.
Evaluate defaults in application code. Apply safe `ALTER TABLE` diffs and reject
lossy type migrations. Create native declared indexes.

Decode rows into canonical semantic objects and use the same core planner and
executor. The layout has full query/mutation behavior for its supported strict
shape but intentionally rejects shapes outside its invariant.

## Relational discovery

Replace fragile `information_schema` joins with OID-based `pg_catalog`
introspection. Discover ordered columns, relation kind, type OIDs, PKs, unique
constraints/indexes, ordinary indexes/access methods, foreign keys,
generated/identity fields, domains, enums, arrays, and native type metadata.

Use schema-qualified identities:

- class/collection: `postgres:{schema}:{table}`
- attribute: `postgres:{schema}:{table}:{column}`

Only add the old schema-less attribute form as an alias when globally
unambiguous. Represent only single-field core indexes; retain composite metadata
for native planning without falsely marking each component unique. Convert FKs
to deterministic relationships.

Require a stable PK by default, with an explicit option to accept a unique,
non-null key. Fail construction with the complete no-identity table list. Use a
versioned base64url tagged-PK tuple entity ID (`pg1.` prefix), supporting
composite keys and arbitrary delimiters. Allow opt-in legacy reads. Bind decoded
PK values using their native types.

Generate explicit, type-aware projections and use `try_get`; never use a generic
text decode against non-text PostgreSQL values. Support native numeric,
floating, Boolean, UUID, bytea, JSON/B, date/time/timestamp, inet, array,
enum/domain cases or produce explicit configured errors. Add atomic
`refresh_catalog()` for read-only relational discovery. All writes remain
rejected.

## Error handling

Centralize conversions:

- `23505`, `23503`, `23502`, `23514` -> semantic validation/query errors
- `40001`, `40P01` -> `TransactionConflict`
- invalid snapshots/codecs -> serialization/deserialization
- pool/connect/timeout/cancel -> storage
- invalid layout/options -> precise invalid-query/configuration errors

Never expose credentials, interpolate unchecked identifiers, panic during row
conversion, or silently turn failures into null.

## Implementation order

1. Configuration API and isolated temporary-schema integration harness.
2. Tagged codec, identifier utilities, and SQLSTATE mapping.
3. Metadata bootstrap, snapshots, revisioning, and concurrent reopen.
4. Semantic entities CRUD, validation, transactions, and all batches.
5. Semantic physical data source and full core query execution.
6. Semantic DDL, materialized indexes, plan, and explain.
7. Packages and migrations.
8. Relationship edges and traversal.
9. Managed strict relational layout.
10. OID-based relational discovery, native codec, stable IDs, and refresh.
11. Conservative pushdown, stats, bounded operations, and documentation.

Keep every stage compiling and tested before proceeding.

## Acceptance tests

- Semantic managed mode passes the complete `semantic_db_test::suite::test_db`.
- Reopen preserves catalog, entities, indexes, relationships, packages, and
  applied migrations.
- Concurrent backend instances observe catalog changes and cannot apply the same
  migration twice.
- Batch, DDL/backfill, and package failures roll back all state.
- Every tagged value round-trips and malformed data never panics.
- Managed relational CRUD/query/DDL/package/relationship tests pass for strict
  classful collections and invalid shapes are explicitly rejected.
- Discovery covers composite PKs, schema collisions, constraint-name collisions,
  no-PK policy, indexes/FKs, generated fields, quoted identifiers, all supported
  native types, versioned/legacy IDs, refresh, and read-only enforcement.
- `plan()`/`explain()` use the supplied query; no managed-mode Backend method is
  a stub.
- No token-based SQL rewriting or unchecked identifier interpolation remains.

Final validation, through the Nix devshell when available:

```bash
cargo test --quiet --message-format=short -p semantic_db_postgres
cargo test --quiet --message-format=short -p semantic_db_test
cargo check --quiet --message-format=short
cargo fmt --check
```
