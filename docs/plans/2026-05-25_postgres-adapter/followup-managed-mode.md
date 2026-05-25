# Postgres Adapter — Managed Mode (Phase 2)

This document contains the implementation details for **Managed mode** —
where the semantic system owns the Postgres schema and translates DDL to
Postgres DDL. This is **Phase 2**, deferred from the initial read-only
Discovery mode implementation.

When Phase 2 begins, merge relevant sections back into
[`plan.md`](plan.md).

---

## 1. Managed Mode Init Flow

When `PostgresMode::Managed` is passed to `PostgresBackend::new()`:

1. Get a client from the pool.
2. Try to load stored catalog from `_semantic_meta` table:
   - If exists → deserialize and wrap in `SharedCatalog`.
   - If not → start with `fresh_catalog_with_core_schema()`, create
     `_semantic_meta` and `_semantic_migrations` tables.
3. Apply any new core schema migrations and persist.
4. Return `PostgresBackend`.

### `_semantic_meta` table

```sql
CREATE TABLE IF NOT EXISTS _semantic_meta (
    key   TEXT PRIMARY KEY,
    value JSONB NOT NULL
);
```

Keys:
- `"catalog"` → full `CatalogStorageSnapshot` serialized as JSON.
- `"version"` → `1` (schema version for future migration).

### `_semantic_migrations` table

```sql
CREATE TABLE IF NOT EXISTS _semantic_migrations (
    package  TEXT NOT NULL,
    module   TEXT NOT NULL,
    name     TEXT NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (package, module, name)
);
```

### Functions (in `catalog.rs`)

- `load_catalog(client: &Client) -> Result<Option<Catalog>>`
- `save_catalog(client: &Client, catalog: &Catalog) -> Result<()>`
- `record_migration(client: &Client, applied: &AppliedMigration) -> Result<()>`
- `mark_migrations_applied(client: &Client, catalog: &Catalog) -> Result<()>`

---

## 2. DDL Operations (`ddl.rs`)

Translate each `DdlOperation` to Postgres DDL.

| DdlOperation | Postgres DDL |
|---|---|
| `UpsertAttribute` | No DDL; just register in catalog |
| `DeleteAttribute` | No DDL; just de-register from catalog |
| `UpsertClass { class }` | `CREATE TABLE IF NOT EXISTS {schema}."{table}" (...)` |
| `DeleteClass { id }` | `DROP TABLE IF EXISTS {schema}."{table}" CASCADE` |
| `UpsertCollection` | `CREATE TABLE IF NOT EXISTS ...` with core fields |
| `DeleteCollection` | `DROP TABLE IF EXISTS ... CASCADE` |
| `UpsertIndex` | `CREATE [UNIQUE] INDEX IF NOT EXISTS ... ON {table}({column})` |
| `DeleteIndex` | `DROP INDEX IF EXISTS ...` |

### UpsertClass DDL

```sql
CREATE TABLE IF NOT EXISTS {schema}."{table_name}" (
    "postgres:id"   TEXT PRIMARY KEY,
    "postgres:type" TEXT NOT NULL,
    "parent"        TEXT,
    -- one column per non-computed attribute:
    "postgres:{table}:{attr}"  {pg_type} {NOT NULL} {DEFAULT},
    ...
);
```

**Important for `postgres:id` in managed mode:** Unlike discovery mode
where `postgres:id` is purely computed (no physical column), in managed mode
the `postgres:id` column IS stored in the Postgres table. The insert code
computes the ID client-side and stores it. The attribute is still marked
`computed` in the catalog, so normalization rejects direct writes, but the
backend code bypasses this by inserting the pre-computed value directly.

### Catalog Synchronization

After each DDL batch, call `save_catalog(client, &catalog)` to persist
the updated catalog to `_semantic_meta`.

---

## 3. In-Memory `postgres:id` Handling in Managed Mode

In managed mode, `postgres:id` serves dual roles:
- It is a **stored Postgres column** (`"postgres:id" TEXT PRIMARY KEY`).
- It is a **computed attribute** in the `Catalog` (marked `computed: Some(...)`).

**Insert flow:**
1. Normalization rejects writes to `postgres:id` (it's computed).
2. The backend computes the synthetic ID from the PK value(s) client-side.
3. The backend inserts the pre-computed value into the `postgres:id` column,
   bypassing the computed-attribute check.

**This bypass is only safe because the backend controls both the catalog and
the insert logic.** In discovery mode this construct is unnecessary and wrong
(no stored column), which is why discovery mode keeps `postgres:id` purely
computed.

---

## 4. CRUD Operations in Managed Mode

### `insert(collection, id, object)`

1. Get catalog snapshot (read lock, in-memory).
2. Look up the collection by name.
3. Extract `type` from object → look up class.
   - If no `type` → `Err(DbError::InvalidQuery("..."))`.
4. Parse `(schema, table) = parse_collection_name(&collection)`.
5. Look up PK columns from the class constraints.
6. Extract the native PK value(s) from the object.
7. Build INSERT SQL with parameterized values — columns = `postgres:id`,
   `postgres:type`, `parent`, plus one per non-computed attribute present.
   Computed attributes are excluded.
8. Execute `client.execute(query, &params)`.
9. On unique violation (code `23505`) → `DbError::InvalidQuery`.

### `get(collection, id)`

Same as discovery mode (parse ID → query by PK → inject computed attrs).

### `delete(collection, id)`

```sql
DELETE FROM {schema}."{table}" WHERE "postgres:id" = $1
```

(Since `postgres:id` is a stored column in managed mode, this works directly.)

### `query(query)`

All query types (SELECT, INSERT, UPDATE, DELETE) work. For INSERT/UPDATE/
DELETE, generate SQL with `RETURNING *` to return the modified entities.

### `execute_batch(batch)`

Execute all operations within a single Postgres transaction:

```rust
async fn execute_batch(&self, batch: Batch) -> Result<BatchOutcome, DbError> {
    let client = self.pool.get().await.map_err(pool_to_db_err)?;
    let tx = client.transaction().await.map_err(pg_to_db_err)?;
    for operation in batch.operations {
        match operation {
            BatchOperation::Insert { collection, id, object } => {
                self.execute_insert(&tx, &collection, &id, &object).await?;
            }
            BatchOperation::Update { collection, id, object } => {
                self.execute_update(&tx, &collection, &id, &object).await?;
            }
            BatchOperation::Delete { collection, id } => {
                self.execute_delete(&tx, &collection, &id).await?;
            }
            BatchOperation::Ddl(batch) => {
                self.execute_ddl_with_conn(&tx, batch).await?;
            }
        }
    }
    tx.commit().await.map_err(pg_to_db_err)?;
    Ok(BatchOutcome { .. })
}
```

### `explain(query)` / `plan(query)`

Properly implement these by:
- Inspecting the collection's registered indexes in the catalog.
- Translating the query filter to determine which index (if any) can
  satisfy it (see `Catalog::find_equality_index()`).
- Returning a `QueryExplain` / `QueryPlan` describing the chosen access path.

The discovery-mode stub (`// TODO: implement query planning`) should be
replaced with real logic.

---

## 5. Package & Relationship Support

### `upsert_package(package)`

Follow the pattern from `KvDb::upsert_package` (see `crates/db_kv/src/db.rs`):

1. Validate migrations via `validate_package_migrations(&package)`.
2. Normalize package via `normalize_package_definition(&package)`.
3. For each migration, apply DDL operations via `apply_migration_ddl_batch`.
4. For each `MigrationOperation::Insert` / `Update` / `Delete`, execute
   CRUD operations against Postgres.
5. Record applied migrations in `_semantic_migrations` table.
6. Persist the catalog state via `save_catalog()`.

### `upsert_relationship(relationship)` / `delete_relationship(id)`

- `RelationMode::Embedded`: handled at the catalog level (no extra DDL).
- `RelationMode::External`: create/drop a join table:

```sql
CREATE TABLE IF NOT EXISTS "_semantic_rel_{rel_id}" (
    "from" TEXT NOT NULL REFERENCES "{source_table}"("postgres:id"),
    "to"   TEXT NOT NULL,
    PRIMARY KEY ("from", "to")
);
```

---

## 6. Additional Modules Required

When implementing Phase 2, these files are needed. The Phase 1 crate has
`lib.rs`, `backend.rs`, `discovery.rs`, `query.rs`, `sql.rs`. Add:

```
crates/db_postgres/src/
├── catalog.rs       — _semantic_meta / _semantic_migrations persistence
├── ddl.rs           — DdlOperation → Postgres DDL translation
```

---

## 7. Managed Mode Tests

### Setup

```rust
async fn setup_managed(pool: &Pool) {
    let client = pool.get().await.unwrap();
    let _ = client
        .execute("DROP TABLE IF EXISTS _semantic_meta, _semantic_migrations CASCADE", &[])
        .await;
}
```

### Test 1: Standard test suite

```rust
#[tokio::test]
async fn test_suite_managed() {
    let pool = get_pool();
    setup_managed(&pool).await;
    let backend = PostgresBackend::new(pool, PostgresMode::Managed);
    let db = Db::new(backend);
    semantic_db_test::suite::test_db(&db).await;
}
```

### Test 2: DDL roundtrip

```rust
#[tokio::test]
async fn test_ddl_roundtrip() {
    let pool = get_pool();
    setup_managed(&pool).await;
    let backend = PostgresBackend::new(pool.clone(), PostgresMode::Managed);
    let db = Db::new(backend);

    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute { /* ... */ })
            .with_op(DdlOperation::UpsertClass { /* ... */ })
            .with_op(DdlOperation::UpsertCollection { /* ... */ })
    ).await.unwrap();

    let client = pool.get().await.unwrap();
    let rows = client.query(
        "SELECT table_name FROM information_schema.tables WHERE table_name = $1",
        &[&"my_table"]
    ).await.unwrap();
    assert_eq!(rows.len(), 1, "DDL should have created the Postgres table");

    // Insert and read back
    let mut obj = Object::new();
    obj.insert("id", Value::String("my_table-1".into()));
    obj.insert("type", Value::String("postgres:public:my_table".into()));
    db.insert("postgres:public:my_table", "my_table-1", obj).await.unwrap();

    let entity = db.get("postgres:public:my_table", "my_table-1").await.unwrap().unwrap();
    assert_eq!(entity.id, "my_table-1");
}
```

### Test 3: Class-less entity rejection

```rust
#[tokio::test]
async fn test_rejects_classless_entities() {
    let pool = get_pool();
    setup_managed(&pool).await;
    let backend = PostgresBackend::new(pool, PostgresMode::Managed);
    let db = Db::new(backend);

    let mut obj = Object::new();
    obj.insert("id", Value::String("no-class".into()));
    let err = db.insert("entities", "no-class", obj).await.unwrap_err();
    assert!(err.to_string().contains("class"), "expected class rejection, got: {err}");
}
```

---

## 8. Schema for Managed Mode Tables

Managed mode needs a configurable Postgres schema. Options:
- Default to `public`
- Allow configuration via `PostgresConfig`
- Could set per-table schema based on package/module naming

---

## 9. Transition Plan from Phase 1 to Phase 2

The `PostgresBackend` struct and `PostgresMode` enum are designed to
accommodate both modes without breaking changes:

1. Add `catalog.rs` and `ddl.rs` modules.
2. Extend `PostgresBackend::new()` to handle `PostgresMode::Managed`.
3. The `write guard` method becomes a real guard (only guards discovery mode,
   allows writes in managed mode).
4. The `parse_collection_name` helper and type mapping from Phase 1 are
   reused unchanged.
5. The CRUD operations get a new inner method (`execute_insert`, etc.) that
   discovery mode's `insert()` error is replaced by.
6. Replace the `execute_batch` stub with a real implementation that wraps
   all operations in a single Postgres transaction.
7. Replace the `explain()` and `plan()` stubs with real implementations
   using the catalog's index metadata.

No existing Phase 1 code needs modification beyond removing the generic
error-on-all-writes approach.
