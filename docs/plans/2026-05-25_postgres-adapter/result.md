# Postgres Adapter — Implementation Result

## Feature Description

A new crate `crates/db_postgres` (`semantic_db_postgres`) that implements the
[`Backend` trait](crates/db_core/src/backend.rs) from `semantic_db_core` using
a real PostgreSQL database via `tokio-postgres` + `deadpool-postgres`.

**Phase 1 — Read-Only Discovery mode.** The adapter scans an existing Postgres
database via `information_schema`, builds an in-memory `Catalog`, and serves
SELECT queries. All write operations (INSERT, UPDATE, DELETE, DDL, packages,
relationships) return `DbError::InvalidQuery`.

A **Managed mode** variant is declared in the `PostgresMode` enum but returns
a clear error at construction time — implementation is deferred to a follow-up.

---

## File Map

```
crates/db_postgres/
├── Cargo.toml
├── src/
│   ├── lib.rs           — Module declarations + re-exports
│   ├── backend.rs       — PostgresBackend, PostgresMode, PostgresConfig, Backend trait impl
│   ├── discovery.rs     — discover_catalog(), information_schema queries, catalog construction
│   ├── query.rs         — QueryCompiler: execute_select(), execute_get(), extract_pk_columns()
│   └── sql.rs           — quote_ident(), pg_type_to_semantic(), row_to_object(), parse helpers
└── tests/
    └── integration.rs   — 5 integration tests (single PK, composite PK, DDL/INSERT rejection, explain/plan)
```

---

## Implementation Overview

### sql.rs — SQL Utilities

Functions for quoting identifiers, mapping Postgres types to semantic types,
converting rows to semantic objects, and parsing collection names / synthetic
entity IDs.

### discovery.rs — Schema Discovery

`discover_catalog()` queries `information_schema.tables`, `.columns`, and
`.table_constraints` for each schema, then builds a fully populated
`Catalog` in memory:

- Registers the global `"id"` (synthetic) and `"postgres:id"` (raw column)
  attributes
- For each table: registers column attributes, builds a `ClassType` with all
  column attributes + computed synthetic `id` attribute + PK metadata constraint
- Registers the collection with `CollectionKind::Schema` /
  `IntegrityMode::StrictRegisteredSchema`
- Registers indexes for PK and unique constraint columns

### query.rs — Query Execution

`QueryCompiler` wraps a connection pool and provides:

- **`execute_select()`** — Compiles a `SelectQuery` to SQL via
  `semantic_db_core::sql::query_to_sql()`, executes against Postgres, maps
  rows to semantic Objects, injects `type` and computed attributes via
  `inject_computed_attributes()`.
- **`execute_get()`** — Parses the synthetic entity ID back into PK values,
  builds a parameterized SQL query, executes, maps the single row, and injects
  computed attributes.
- **`extract_pk_columns()`** — Reads PK metadata from a class constraint
  (stored as `ClassConstraint::MultiFieldExpr` with `description = "__pg_pk"`).

### backend.rs — Backend Trait Implementation

`PostgresBackend` implements the full `Backend` trait:

- **Read operations** — `catalog()`, `get()`, `query()` (SELECT only) — work
  as described above.
- **Write operations** — `insert()`, `delete()`, `create_collection()`,
  `execute_ddl()`, `upsert_package()`, `upsert_relationship()`,
  `delete_relationship()`, `execute_batch()` — all return
  `Err(DbError::InvalidQuery("...read-only..."))`.
- **`explain()`** — Always returns `AccessPath::FullScan` (no Postgres
  optimizer insight).
- **`plan()`** — Stubbed, returns error.

---

## Key Design Decisions

| Area | Decision |
|---|---|
| **Synthetic `id`** | Computed as `stringify("{table_name}-{pk_value}")`. Composite PK uses `::` separator. |
| **PK metadata** | Stored as `ClassConstraint::MultiFieldExpr` with `description = "__pg_pk"`, JSON-encoded column list. |
| **`type` injection** | Postgres tables don't store a `type` column, so the adapter injects `"type" = collection_name` on every result row before computed attribute evaluation. |
| **SQL generation** | Uses `semantic_db_core::sql::query_to_sql()` (generic SQL dialect) — produces valid Postgres syntax for basic SELECTs. |
| **Parameterized queries** | `get()` uses `$N` bindings. SELECT uses plain SQL from `query_to_sql`. |
| **Connection pooling** | `deadpool-postgres` with configurable `max_pool_size`. |
| **Write rejection** | Single `WRITE_ERROR` const reused across all write methods. |

---

## Verification

- ✅ `cargo check` — compiles with zero new warnings
- ✅ `cargo check --tests -p semantic_db_postgres` — test code compiles
- ✅ `cargo fmt` — passes
- ✅ File structure matches the plan exactly (5 source modules + 1 test file)
- ✅ 5 integration tests covering: single PK, composite PK, DDL rejection,
  INSERT rejection, explain/plan stubs

Integration tests require a live Postgres instance and `POSTGRES_URI` env var:

```bash
POSTGRES_URI="postgres://user:pass@host:5432/db" cargo test -p semantic_db_postgres
```
