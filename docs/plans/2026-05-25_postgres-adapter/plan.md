# Postgres Adapter — Implementation Plan (v2: Read-Only Discovery)

## 1. Overview

Create crate `crates/db_postgres` (`semantic_db_postgres`) that implements the
[`Backend` trait](crates/db_core/src/backend.rs) from `semantic_db_core` using
a real PostgreSQL database via `tokio-postgres` + `deadpool-postgres`.

**Scope: Read-only Discovery mode only.** The adapter scans an existing
Postgres database via `information_schema`, builds an in-memory `Catalog`,
and serves SELECT queries. All write operations (INSERT, UPDATE, DELETE, DDL,
packages, relationships) return `DbError::InvalidQuery`.

The code architecture prepares for a later **Managed mode** (semantic owns the
schema, translates DDL to Postgres DDL) — the `PostgresMode` enum includes the
variant, but no Managed-mode implementation is provided in this phase. Managed
mode details live in [`followup-managed-mode.md`](followup-managed-mode.md).

| Code Resource | What to learn from it |
|---|---|
| [`crates/db_core/src/backend.rs`](crates/db_core/src/backend.rs) | `Backend` trait — all methods to implement |
| [`crates/db_core/src/catalog/catalog.rs`](crates/db_core/src/catalog/catalog.rs) | `Catalog` struct, `CatalogBatchOperation`, upsert/delete methods |
| [`crates/db_core/src/catalog/schema.rs`](crates/db_core/src/catalog/schema.rs) | `CollectionSchema`, `ClassSchema`, `AttributeSchema`, etc. |
| [`crates/db_core/src/plan/execute.rs`](crates/db_core/src/plan/execute.rs) | `evaluate_computed_expr`, `inject_computed_attributes` |
| [`crates/data/src/schema/class/class_type.rs`](crates/data/src/schema/class/class_type.rs) | `ClassType` struct (id, name, inherits, extends, attributes) |
| [`crates/data/src/schema/class/class_attribute.rs`](crates/data/src/schema/class/class_attribute.rs) | `ClassAttribute` struct (attribute ref, required, computed, constraints) |
| [`crates/data/src/expr/mod.rs`](crates/data/src/expr/mod.rs) | `Expr` enum, `BinaryExpr`, `CallExpr`, `FieldAccessExpr`, `LiteralExpr` |
| [`crates/data/src/expr/refs/ref_expr.rs`](crates/data/src/expr/refs/ref_expr.rs) | `RefExpr::Identifier("self")` for computed attribute field access |
| [`crates/db_kv/src/backend.rs`](crates/db_kv/src/backend.rs) | Pattern: how `KvBackend` implements `Backend` with `RwLock<Catalog>` |
| [`crates/db_test/src/suite/mod.rs`](crates/db_test/src/suite/mod.rs) | The full test suite that `test_db()` runs (phase 2) |

---

## 2. Architecture

### 2.1 New Crate: `crates/db_postgres`

**Dependencies:**

```toml
[package]
name = "semantic_db_postgres"
version.workspace = true
edition.workspace = true

[dependencies]
semantic_db_core = { path = "../db_core" }
semantic_data = { workspace = true }
tokio-postgres = { version = "0.7", features = ["with-uuid-1", "with-serde_json-1", "with-chrono-0_4"] }
deadpool-postgres = "0.14"
async-trait = "0.1"
bytes = { workspace = true }
ordered-float = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
uuid = { workspace = true, features = ["serde"] }
tracing = { workspace = true }
tokio = { workspace = true, features = ["sync"] }

[dev-dependencies]
semantic_db_test = { path = "../db_test" }
```

`tokio-postgres` features:
- `with-uuid-1` — convert `uuid::Uuid` to/from Postgres `uuid` type.
- `with-serde_json-1` — convert `serde_json::Value` to/from Postgres `jsonb`.
- `with-chrono-0_4` — convert timestamps (optional, fallback to text if easier).

### 2.2 Two Operating Modes (Managed is Future Work)

```rust
#[derive(Debug, Clone)]
pub enum PostgresMode {
    /// Discover existing tables from `information_schema`.
    /// **No tables are ever created or modified.** The catalog is in-memory only.
    Discovery {
        schemas: Vec<String>,
    },
    /// **Not yet implemented.** Placeholder for the phase-2 managed mode
    /// where semantic owns the schema via DDL → Postgres DDL translation.
    Managed,
}
```

**Phase 1 — Discovery (this plan):**
- Scans `information_schema` on init to build an in-memory `Catalog`.
- No tables are ever created or modified.
- DDL mutations (`execute_ddl`, `upsert_package`, `create_collection`, etc.)
  return `Err(DbError::InvalidQuery(…))`.
- INSERT, UPDATE, DELETE return errors.
- SELECT queries work — compile semantic `Query` → parameterized SQL →
  execute via `tokio-postgres` → map rows → inject computed attributes.

**Phase 2 — Managed (see [`followup-managed-mode.md`](followup-managed-mode.md)):**
- Schema defined via `DdlBatch` / package migrations translated to Postgres DDL.
- Internal `_semantic_meta` / `_semantic_migrations` tables for catalog
  persistence and migration history.
- Full read-write support.

### 2.3 Core Built-in Fields

Every Postgres-backed collection carries these built-in fields at the semantic
level (in the `Catalog`).

| Semantic Field | Attribute ID | Type | Discovery Mode |
|---|---|---|---|
| `id` | `"id"` | `String` | **Computed** via `stringify(table_name + "-" + pk_value)` — no physical Postgres column |
| `type` | `"type"` | `String` | **Inferred** — set to the class ID during discovery, injected at read time |
| `parent` | `"parent"` | `Ref("id")` | **Not used in discovery mode** — no hierarchy support |
| `postgres:id` | `"postgres:id"` | column-dependent | **Stored column value** — the raw Postgres `id` column value (if present), available alongside the computed `"id"` |
| `postgres:{table}:{col}` | `"postgres:{table}:{col}"` | column-dependent | **Stored column value** — each Postgres column mapped to a prefixed attribute |

The canonical computed ID attribute is `"id"` (matching `PRIMARY_ID_FIELD`).
It is evaluated by `inject_computed_attributes()` and is never stored in
Postgres. This means:
- `get()` must parse the computed ID to extract PK values and query by PK columns.
- SELECT queries work normally — `inject_computed_attributes()` fills in the
  `id` field on every result row.

The `"postgres:id"` attribute (registered in the catalog) provides access to
the raw Postgres `id` column value when a table has a column named `id`.
It is populated by `row_to_object()` alongside the prefixed
`"postgres:{table}:id"` key.

`inject_computed_attributes()` reads the `"type"` field (`OBJECT_TYPE_FIELD`)
to look up the class. Postgres tables have no `type` column, so the adapter
injects `"type"` at read time with the collection name (which equals the
class ID).

### 2.4 Collection Properties

- Every collection uses `CollectionKind::Schema` with
  `IntegrityMode::StrictRegisteredSchema`.
- The synthetic `id` attribute (canonical `PRIMARY_ID_FIELD`) is marked
  `computed: Some(Expr::Call(…))`.
- The raw-column `"postgres:id"` attribute is registered in the catalog and
  aliased to `"postgres:{table}:id"` in the collection via field aliases.
- Each discovered table gets **one class** that maps all its columns as
  attributes. The class's `type` value is derived from the table's identity
  (e.g. `"postgres:public:users"`).

---

## 3. Step-by-step Implementation

### Step 1: Scaffold the Crate

```
crates/db_postgres/
├── Cargo.toml
└── src/
    ├── lib.rs           — Re-exports: PostgresBackend, PostgresConfig, PostgresMode
    ├── backend.rs       — PostgresBackend struct, Backend trait impl, init
    ├── discovery.rs     — discover_catalog(), information_schema queries
    ├── query.rs         — Query → SQL compilation, execution, row mapping
    └── sql.rs           — Quote ident, pg_type_to_semantic(), row_to_object()
```

The `catalog.rs` and `ddl.rs` modules from the original plan are **not
created in this phase** — they are Managed-mode only and deferred.

Workspace registration: no change needed — the existing `members = ["crates/*"]`
glob covers the new crate.

### Step 2: `PostgresConfig` and `PostgresBackend` Skeleton

```rust
use async_trait::async_trait;
use deadpool_postgres::Pool;
use semantic_db_core::{
    Backend, Batch, BatchOutcome, DbError, DdlBatch, DdlOutcome, DeleteQuery,
    EntityRecord, MutationStats, PackageRegistrationOutcome, Query,
    QueryExplain, QueryPlan, QueryResult, SqlDialectKind, TextQueryFormat, TextQueryInput,
    UpdateQuery,
    catalog::{CollectionKind, LocalCollectionId, SharedCatalog},
};

#[derive(Debug, Clone)]
pub enum PostgresMode {
    Discovery { schemas: Vec<String> },
    Managed,
}

pub struct PostgresConfig {
    /// Active Postgres mode.
    pub mode: PostgresMode,
}

pub struct PostgresBackend {
    pool: Pool,
    catalog: SharedCatalog,
    mode: PostgresMode,
}
```

**`new()` — init flow (discovery mode only):**

1. Acquire a client from the pool.
2. Call `discovery::discover_catalog(&client, &schemas)` to build a `Catalog`
   entirely from `information_schema`.
3. The catalog lives **only in memory** — nothing is persisted.
4. No internal tables are ever created, queried, or written to.
5. Wrap in `SharedCatalog` (no outer `Arc<RwLock<…>>` —
   `SharedCatalog` already has its own `Arc<RwLock<CatalogState>>`).
6. Return `PostgresBackend`.

If `PostgresMode::Managed` is passed, `new()` returns
`Err(DbError::InvalidQuery("managed mode not yet implemented"))`.

All subsequent `catalog()` calls use `self.catalog.catalog_arc()` which
returns `Arc<Catalog>` with zero Postgres I/O.

**DDL / mutation guard:**

```rust
fn guard_write(&self) -> Result<(), DbError> {
    Err(DbError::InvalidQuery(
        "this operation is not available in read-only discovery mode".into(),
    ))
}
```

Called at the top of every method that mutates state.

### Step 3: In-Memory Mapping Architecture

All schema data lives in the in-memory `Catalog` struct (see
`crates/db_core/src/catalog/catalog.rs` — all fields are `IdMap` and
`FnvHashMap`). Discovery mode populates it once at init and never persists it.
Every `catalog()` call returns an `Arc<Catalog>` with zero Postgres I/O.

| Query-time need | How it's satisfied (all in-memory, zero I/O) |
|---|---|
| Collection name → actual Postgres table | `CollectionSchema.name` = `"postgres:{schema}:{table}"`. Helper `parse_collection_name()` extracts `(schema, table)`. |
| Field name → Postgres column | `CollectionSchema.field_types[key]` maps canonical field IDs to semantic `Type`. The Postgres column name **is** the canonical field ID. |
| PK columns → synthetic ID construction | Stored as `ClassConstraint::MultiFieldExpr` with `description = "__pg_pk"` on the class. At runtime, deserialized from the constraint. |
| Field aliases (plain/underscore) | `CollectionSchema.field_aliases` — in-memory `FnvHashMap`. |
| Class for a given `type` value | Each table gets exactly one class. The class is discoverable via `Catalog.class_ids(type)`. |
| Attribute definition | `Catalog.attribute_by_id(id)` → `&AttributeSchema`. In-memory hash lookup. |
| Index on a field | `Catalog.find_equality_index(collection_lid, field)` → `Option<&IndexSchema>`. In-memory. |

#### PK Metadata Tracking

PK column info is needed at runtime for synthetic ID construction and `get()`
lookups. Store PK column names as a `ClassConstraint::MultiFieldExpr`:

```rust
ClassConstraint::MultiFieldExpr {
    expr: Expr::Literal(LiteralExpr {
        value: LiteralValue::String(
            serde_json::to_string(&pk_columns).unwrap()
        ),
    }),
    description: Some("__pg_pk".into()),
}
```

At runtime, look up `class.constraints` for the one with
`description == "__pg_pk"` and deserialize the PK column names.

#### Collection Name → Schema/Table Parsing

```rust
/// Parses a collection name like "postgres:public:users" into (schema, table).
pub fn parse_collection_name(name: &str) -> Result<(&str, &str), DbError> {
    let parts: Vec<&str> = name.splitn(3, ':').collect();
    if parts.len() < 3 || parts[0] != "postgres" {
        return Err(DbError::InvalidQuery(format!(
            "invalid postgres collection name: '{}'", name
        )));
    }
    Ok((parts[1], parts[2]))
}
```

#### Synthetic ID ID Parsing (`get()`)

The synthetic ID format is `{table_name}-{pk_values}`.

- **Single PK:** The format is `{table_name}-{pk_value}`. Since we know the
  table name (from the collection), we strip `{table_name}-` and the rest is
  the PK value. This works even if the PK value contains `-`, because we know
  the prefix length.

- **Composite PK:** The format is `{table_name}-{pk1}::{pk2}::...`. The `::`
  separator between PK values is chosen because it's extremely unlikely in
  column values. **Limitation:** if a PK value contains `::`, parsing fails.
  This limitation is documented and acceptable for v1.

```rust
fn parse_entity_id(
    id: &str,
    schema: &str,
    table_name: &str,
    pk_column_count: usize,
) -> Result<Vec<String>, DbError> {
    let prefix = format!("{}-", table_name);
    let remainder = id.strip_prefix(&prefix).ok_or_else(|| {
        DbError::EntityNotFound {
            collection: format!("postgres:{}:{}", schema, table_name),
            id: id.to_string(),
        }
    })?;
    if pk_column_count == 1 {
        return Ok(vec![remainder.to_string()]);
    }
    let parts: Vec<&str> = remainder.split("::").collect();
    if parts.len() != pk_column_count {
        return Err(DbError::InvalidQuery(format!(
            "expected {pk_column_count} PK values in id '{id}', got {}",
            parts.len()
        )));
    }
    Ok(parts.into_iter().map(|s| s.to_string()).collect())
}
```

### Step 4: Schema Discovery (`discovery.rs`)

#### DiscoveredTable Intermediary

```rust
struct DiscoveredColumn {
    name: String,
    data_type: String,
    is_nullable: bool,
    column_default: Option<String>,
    character_maximum_length: Option<i32>,
    numeric_precision: Option<i32>,
    numeric_scale: Option<i32>,
}

struct DiscoveredTable {
    schema: String,
    name: String,
    table_type: String,
    columns: Vec<DiscoveredColumn>,
    pk_columns: Vec<String>,
    unique_constraints: Vec<Vec<String>>,
}
```

**Signature:**

```rust
pub async fn discover_catalog(
    client: &Client,
    schemas: &[String],
) -> Result<Catalog, DbError>;
```

**Step-by-step logic:**

1. Query `information_schema.tables`:

```sql
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_schema = ANY($1)
  AND table_type = 'BASE TABLE'
  AND table_name NOT LIKE '\_semantic\_%';
```

2. For each table, query `information_schema.columns`:

```sql
SELECT column_name, data_type, is_nullable, column_default,
       character_maximum_length, numeric_precision, numeric_scale
FROM information_schema.columns
WHERE table_schema = $1 AND table_name = $2
ORDER BY ordinal_position;
```

3. For each table, query primary key columns:

```sql
SELECT kcu.column_name
FROM information_schema.table_constraints tc
JOIN information_schema.key_column_usage kcu
  ON tc.constraint_catalog = kcu.constraint_catalog
 AND tc.constraint_schema  = kcu.constraint_schema
 AND tc.constraint_name    = kcu.constraint_name
WHERE tc.table_schema = $1
  AND tc.table_name   = $2
  AND tc.constraint_type = 'PRIMARY KEY'
ORDER BY kcu.ordinal_position;
```

4. For each table, query unique constraints (for index registration):

```sql
SELECT kcu.column_name
FROM information_schema.table_constraints tc
JOIN information_schema.key_column_usage kcu
  ON tc.constraint_catalog = kcu.constraint_catalog
 AND tc.constraint_schema  = kcu.constraint_schema
 AND tc.constraint_name    = kcu.constraint_name
WHERE tc.table_schema = $1
  AND tc.table_name   = $2
  AND tc.constraint_type = 'UNIQUE'
ORDER BY kcu.ordinal_position;
```

5. For each column, map Postgres type to semantic type using
   `sql::pg_type_to_semantic()` (see Step 7).

6. Build a `Catalog` via catalog operations (in-memory only):

```
// Global setup (once):
  0a. Register attribute "postgres:id" with Type::Any in the catalog

for each table:
  1. Register each column as an attribute: "postgres:{table}:{col_name}"
  2. Build a ClassType with all column attributes
  3. Add computed attribute for synthetic "id"
  4. Add ClassConstraint with PK column metadata
  5. Register the class (class id = "postgres:{schema}:{table}")
  6. Register the collection (name = "postgres:{schema}:{table}",
     CollectionKind::Schema, IntegrityMode::StrictRegisteredSchema)
     a. Add field alias "postgres:id" -> "postgres:{table}:id" if
        the table has an id column
     b. Add "postgres:id" to field_types with the id column's type
  7. Register indexes for PK and unique constraints
```

7. Return the populated `Catalog`.

**Synthetic ID computed attribute construction:**

```rust
fn build_synthetic_id_attr(
    table_name: &str,
    pk_columns: &[String],
) -> ClassAttribute {
    // Single PK: stringify(tablename + "-" + self."postgres:tablename:pk_col")
    // Composite PK: stringify(tablename + "-" + self."postgres:tablename:a" + "::" + self."postgres:tablename:b")

    let mut concat = Expr::Literal(LiteralExpr {
        value: LiteralValue::String(format!("{}-", table_name)),
    });

    for (i, pk_col) in pk_columns.iter().enumerate() {
        if i > 0 {
            concat = Expr::Binary(BinaryExpr {
                op: BinaryOperator::Concat,
                left: Box::new(concat),
                right: Box::new(Expr::Literal(LiteralExpr {
                    value: LiteralValue::String("::".into()),
                })),
            });
        }
        let pk_field = Expr::FieldAccess(FieldAccessExpr {
            target: Box::new(Expr::Ref(RefExpr::Identifier("self".into()))),
            field: format!("postgres:{}:{}", table_name, pk_col),
        });
        concat = Expr::Binary(BinaryExpr {
            op: BinaryOperator::Concat,
            left: Box::new(concat),
            right: Box::new(pk_field),
        });
    }

    ClassAttribute {
        attribute: AttributeRef { id: PRIMARY_ID_FIELD.to_string() },
        required: true,
        computed: Some(Expr::Call(CallExpr {
            callee: Callee::Name(vec!["stringify".into()]),
            args: vec![CallArg::Positional(concat)],
        })),
        constraints: vec![],
        meta: Meta::default(),
    }
}
```

### Step 5: CRUD Operations

All operations use parameterized `$N` bindings. Table/column names come from
the catalog and are `quote_ident()`-escaped.

#### `insert(collection, id, object)` — **Rejected**

Return `Err(DbError::InvalidQuery("read-only mode"))`. Discovery mode never
writes to Postgres.

#### `get(collection, id)` — **Works**

1. Get catalog snapshot (read lock, in-memory).
2. Look up the collection by name.
3. Parse `(schema, table) = parse_collection_name(&collection)`.
4. Find class for the collection → extract PK column names from constraints.
5. Parse the entity ID via `parse_entity_id(id, schema, table_name, pk_count)` to get
   the PK value(s).
6. Build a WHERE clause matching the PK columns:

```sql
SELECT {all_columns} FROM {schema}."{table}"
WHERE "{pk_col_1}" = $1
  AND "{pk_col_2}" = $2
  ...
```

7. Execute, map the row to an `Object`.
8. Inject the `"type"` field into the object with the class ID
   (the collection name). This is required because Postgres tables don't
   have a `type` column — `inject_computed_attributes()` reads `type`
   to look up the class and evaluate computed attributes.
9. Run `inject_computed_attributes(&catalog, &mut object)` to populate
   the synthetic `id` (no I/O — pure expression evaluation).
10. Return `EntityRecord { id, collection, object }`.

If no row matches → return `Ok(None)` (entity not found).

#### `delete(collection, id)` — **Rejected**

Return `Err(DbError::InvalidQuery("read-only mode"))`.

#### `query(query)` — **SELECT works; INSERT/UPDATE/DELETE rejected**

The `Backend::query()` method receives a `TextQueryInput` and must return a
`QueryResult`. Approach:

```rust
async fn query(&self, input: TextQueryInput) -> Result<QueryResult, DbError> {
    let query = match input {
        TextQueryInput::Ast(q) => q,
        TextQueryInput::Text { format, query } => {
            self.parse_text_query(format, &query).await?
        }
    };

    match query {
        Query::Select(select) => {
            let collection_name = select.collection_or_default().to_string();
            let (schema, table_name) = parse_collection_name(&collection_name)?;
            let sql = self.query_to_sql(&Query::Select(select.clone()))?;
            let client = self.pool.get().await.map_err(...)?;
            let rows = client.query(&sql, &[]).await.map_err(...)?;
            let mut objects: Vec<Object> = rows.iter()
                .map(|row| sql::row_to_object(row, table_name))
                .collect::<Result<Vec<_>, _>>()?;
            let catalog = self.catalog.catalog_arc();
            for obj in &mut objects {
                // Inject "type" so inject_computed_attributes can find the class.
                // Postgres tables don't store type; in discovery mode each table
                // maps to exactly one class whose ID is the collection name.
                obj.insert(
                    "type".to_string(),
                    Value::String(collection_name.clone()),
                );
                inject_computed_attributes(&catalog, obj)
                    .map_err(|e| DbError::InvalidQuery(e.to_string()))?;
            }
            Ok(QueryResult::Select(objects))
        }
        Query::Insert(_) | Query::Update(_) | Query::Delete(_) => Err(DbError::InvalidQuery(
            "INSERT/UPDATE/DELETE queries are not available in read-only discovery mode".into(),
        )),
        Query::Ddl(_) => Err(DbError::InvalidQuery(
            "DDL queries are not available in read-only discovery mode".into(),
        )),
    }
}
```

**Why this works:** The default `Backend::query_to_sql()` calls
`semantic_db_core::sql::query_to_sql()` which generates SQL in
`SqlDialectKind::Generic`. For SELECT queries this produces valid Postgres
syntax (identifier quoting uses double quotes which is correct for Postgres).
The `sql_dialect()` method is left at its default (`SqlDialectKind::Generic`)
for now — switching to `PostgreSql` for SQL parsing is tracked in
[`followups.md`](followups.md).

#### `update_where(...)` / `delete_where(...)` — **Rejected**

Both return `Err(DbError::InvalidQuery("read-only mode"))`.

#### `execute_batch(...)` — **Rejected**

Returns `Err(DbError::InvalidQuery("read-only mode"))`.

**On the managed-mode TODO:** `execute_batch` must wrap all operations in a
single Postgres transaction for atomicity.

### Step 6: Error Handling

Map `tokio-postgres` and `deadpool-postgres` errors:

```rust
fn pg_to_db_err(e: tokio_postgres::Error) -> DbError {
    if let Some(code) = e.code() {
        match code.code() {
            "42P01" => DbError::InvalidQuery(format!("table not found: {}", e)),
            _ => DbError::Storage(e.to_string()),
        }
    } else {
        DbError::Storage(e.to_string())
    }
}

fn pool_to_db_err(e: deadpool_postgres::PoolError) -> DbError {
    DbError::Storage(format!("connection pool error: {}", e))
}
```

### Step 7: SQL Dialect & Type Mapping (`sql.rs`)

#### Identifier Quoting

```rust
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('\"', "\"\""))
}
```

#### PostgreSQL → Semantic Type Mapping

```rust
pub fn pg_type_to_semantic(
    data_type: &str,
    is_nullable: bool,
    numeric_scale: Option<i32>,
) -> Type {
    let base = match data_type {
        "integer" | "int" | "int4"     => Number(NumberType::Int(IntWidth::I32)),
        "bigint" | "int8"              => Number(NumberType::Int(IntWidth::I64)),
        "smallint" | "int2"            => Number(NumberType::Int(IntWidth::I16)),
        "serial" | "serial4"           => Number(NumberType::Int(IntWidth::I32)),
        "bigserial" | "serial8"        => Number(NumberType::Int(IntWidth::I64)),
        "real" | "float4"              => Number(NumberType::Float(FloatWidth::F32)),
        "double precision" | "float8"  => Number(NumberType::Float(FloatWidth::F64)),
        "numeric" | "decimal" if numeric_scale == Some(0)
                                         => Number(NumberType::Int(IntWidth::I64)),
        "numeric" | "decimal"          => Number(NumberType::Float(FloatWidth::F64)),
        "boolean" | "bool"             => Bool(BoolType),
        "text" | "varchar" | "char"    => String(StringType { format: None, normalization: None }),
        "uuid"                         => Uuid,
        "json" | "jsonb"               => Json,
        "bytea"                        => Bytes(BytesType { encoding: BytesEncoding::Base64 }),
        "date"                         => Temporal(TemporalType::Date),
        "time" | "time without time zone"
                                       => Temporal(TemporalType::Time),
        "timestamp" | "timestamp without time zone"
                                       => Temporal(TemporalType::Timestamp(TimestampType { time_zone: None })),
        "timestamptz" | "timestamp with time zone"
                                       => Temporal(TemporalType::Timestamp(TimestampType { time_zone: Some("UTC".into()) })),
        "inet"                         => IpAddr(IpAddrType { version: None }),
        _                              => Any(AnyType),
    };
    if is_nullable {
        Type { kind: Optional(OptionalType { inner: Box::new(base) }), constraints: vec![], annotations: vec![] }
    } else {
        Type { kind: base, constraints: vec![], annotations: vec![] }
    }
}
```

#### Row → Object Conversion

```rust
pub fn row_to_object(row: &Row, table_name: &str) -> Result<Object, DbError> {
    let mut obj = Object::new();
    for (i, column) in row.columns().iter().enumerate() {
        let name = column.name();
        let value = match column.type_().name() {
            "text" | "varchar" | "char" => {
                let s: Option<&str> = row.get(i);
                s.map(Value::String).unwrap_or(Value::Null)
            }
            "int4" => {
                let v: Option<i32> = row.get(i);
                v.map(|v| Value::I32(v)).unwrap_or(Value::Null)
            }
            "int8" => {
                let v: Option<i64> = row.get(i);
                v.map(Value::I64).unwrap_or(Value::Null)
            }
            "float8" => {
                let v: Option<f64> = row.get(i);
                v.map(|v| Value::F64(ordered_float::OrderedFloat(v))).unwrap_or(Value::Null)
            }
            "bool" => {
                let v: Option<bool> = row.get(i);
                v.map(Value::Bool).unwrap_or(Value::Null)
            }
            "uuid" => {
                let v: Option<uuid::Uuid> = row.get(i);
                v.map(Value::Uuid).unwrap_or(Value::Null)
            }
            "jsonb" | "json" => {
                let v: Option<serde_json::Value> = row.get(i);
                v.map(json_value_to_semantic).unwrap_or(Value::Null)
            }
            "bytea" => {
                let v: Option<Vec<u8>> = row.get(i);
                v.map(|b| Value::Bytes(bytes::Bytes::from(b))).unwrap_or(Value::Null)
            }
            _ => {
                let s: Option<&str> = row.get(i);
                s.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null)
            }
        };
        let key = format!("postgres:{}:{}", table_name, name);
        obj.insert(key, value.clone());
        // Additionally store the raw id column value at "postgres:id"
        if name == "id" {
            obj.insert("postgres:id".to_string(), value);
        }
    }
    Ok(obj)
}
```

### Step 8: Computed Attribute Injection

After fetching rows, run `inject_computed_attributes()` from
`crates/db_core/src/plan/execute.rs` to populate computed fields like the
synthetic `id`:

- Reads the `type` field to find the class.
- For each computed attribute in the class whose field is not already present
  in the object, evaluates the expression via `evaluate_computed_expr()`.
- The synthetic ID expression references `self."postgres:{table}:{pk_col}"`
  and concatenates it with the table name prefix.

**⚠️ Critical:** `inject_computed_attributes()` reads `OBJECT_TYPE_FIELD =
"type"` from the object to look up the class. Postgres tables have no `type`
column, so the adapter must inject **`"type"`** (not `"postgres:type"`)
**before** calling `inject_computed_attributes()`. The class ID equals the
collection name (`"postgres:{schema}:{table}"`) since each discovered table
maps to exactly one class. This applies in both `query()` and `get()`.

**Performance:** Runs purely on the in-memory `Object` and `Catalog` — no
Postgres I/O.

### Step 9: Testing

Test file at `crates/db_postgres/tests/integration.rs`.

#### Helper Setup

```rust
use deadpool_postgres::{Manager, Pool};
use tokio_postgres::{Config, NoTls};
use semantic_db_postgres::{PostgresBackend, PostgresMode};

fn get_pool() -> Pool {
    let uri = std::env::var("POSTGRES_URI")
        .expect("POSTGRES_URI env var required for postgres tests");
    let config: Config = uri.parse().expect("invalid POSTGRES_URI");
    Manager::new(config, NoTls).create_pool(4)
}
```

#### Test Cases

**Test 1: Discovery — single PK table**

```rust
#[tokio::test]
async fn test_discovery_single_pk() {
    let pool = get_pool();
    let client = pool.get().await.unwrap();

    let _ = client.execute("DROP TABLE IF EXISTS test_items CASCADE", &[]).await;
    client.execute(
        "CREATE TABLE test_items (id SERIAL PRIMARY KEY, name TEXT NOT NULL, score INTEGER)",
        &[]
    ).await.unwrap();
    client.execute(
        "INSERT INTO test_items (name, score) VALUES ($1, $2), ($3, $4)",
        &[&"hello", &42i32, &"world", &99i32]
    ).await.unwrap();

    let backend = PostgresBackend::new(pool,
        PostgresMode::Discovery { schemas: vec!["public".into()] });
    let db = Db::new(backend);

    // SELECT all
    let results = db.select(
        semantic_db_core::SelectQuery::new()
            .with_collection("postgres:public:test_items")
    ).await.unwrap();
    assert_eq!(results.len(), 2);

    // Computed id present
    for obj in &results {
        let id = obj.get("id").and_then(|v| v.as_str()).unwrap();
        assert!(id.starts_with("test_items-"), "id should start with table name");
    }

    // GET by id
    let first_id = results[0].get("id").and_then(|v| v.as_str()).unwrap().to_string();
    let entity = db.get("postgres:public:test_items", &first_id).await.unwrap();
    assert!(entity.is_some(), "get() should find entity by synthetic id");
}
```

**Test 2: Discovery — composite PK**

```rust
#[tokio::test]
async fn test_discovery_composite_pk() {
    let pool = get_pool();
    let client = pool.get().await.unwrap();
    let _ = client.execute("DROP TABLE IF EXISTS composite_test CASCADE", &[]).await;
    client.execute(
        "CREATE TABLE composite_test (a INT, b INT, val TEXT, PRIMARY KEY (a, b))",
        &[]
    ).await.unwrap();
    client.execute(
        "INSERT INTO composite_test (a, b, val) VALUES (1, 2, 'data')",
        &[]
    ).await.unwrap();

    let backend = PostgresBackend::new(pool,
        PostgresMode::Discovery { schemas: vec!["public".into()] });
    let db = Db::new(backend);

    let results = db.select(
        semantic_db_core::SelectQuery::new()
            .with_collection("postgres:public:composite_test")
    ).await.unwrap();
    assert_eq!(results.len(), 1);

    let id = results[0].get("id").and_then(|v| v.as_str()).unwrap();
    assert_eq!(id, "composite_test-1::2");

    // get() by synthetic ID
    let entity = db.get("postgres:public:composite_test", id).await.unwrap();
    assert!(entity.is_some());
}
```

**Test 3: DDL forbidden in discovery**

```rust
#[tokio::test]
async fn test_ddl_forbidden_in_discovery() {
    let pool = get_pool();
    let backend = PostgresBackend::new(pool,
        PostgresMode::Discovery { schemas: vec!["public".into()] });
    let db = Db::new(backend);

    let err = db.create_collection("test", CollectionKind::Schema).await.unwrap_err();
    assert!(err.to_string().contains("read-only") || err.to_string().contains("forbidden"),
        "expected write rejection, got: {err}");
}
```

**Test 4: INSERT rejected in discovery**

```rust
#[tokio::test]
async fn test_insert_rejected_in_discovery() {
    let pool = get_pool();
    let client = pool.get().await.unwrap();
    let _ = client.execute("DROP TABLE IF EXISTS test_items CASCADE", &[]).await;
    client.execute(
        "CREATE TABLE test_items (id SERIAL PRIMARY KEY, name TEXT)",
        &[]
    ).await.unwrap();
    client.execute("INSERT INTO test_items (name) VALUES ('seed')", &[]).await.unwrap();

    let backend = PostgresBackend::new(pool,
        PostgresMode::Discovery { schemas: vec!["public".into()] });
    let db = Db::new(backend);

    let mut obj = Object::new();
    obj.insert("name".to_string(), Value::String("should-fail".into()));
    let err = db.insert("postgres:public:test_items", "ignored", obj).await.unwrap_err();
    assert!(err.to_string().contains("read-only"), "expected read-only rejection, got: {err}");
}
```

**Test 5: `explain()` and `plan()` — stubbed**

```rust
#[tokio::test]
async fn test_explain_and_plan_stub() {
    let pool = get_pool();
    let client = pool.get().await.unwrap();
    let _ = client.execute("DROP TABLE IF EXISTS test_items CASCADE", &[]).await;
    client.execute(
        "CREATE TABLE test_items (id SERIAL PRIMARY KEY, name TEXT)",
        &[]
    ).await.unwrap();

    let backend = PostgresBackend::new(pool,
        PostgresMode::Discovery { schemas: vec!["public".into()] });
    let db = Db::new(backend);

    // explain should return FullScan
    let explain = db.explain(
        semantic_db_core::SelectQuery::new()
            .with_collection("postgres:public:test_items")
    ).await;
    assert!(explain.is_ok(), "explain() should return FullScan: {:?}", explain);
    if let Ok(qe) = explain {
        assert_eq!(qe.access_path, semantic_db_core::AccessPath::FullScan);
    }

    // plan() remains stubbed
    let plan = db.plan(
        semantic_db_core::SelectQuery::new()
            .with_collection("postgres:public:test_items")
    ).await;
    assert!(plan.is_err(), "plan() is stubbed and should return an error");
}
```

**Why stub `plan()`?** Real query planning requires index cost models
targeted to the specific backend. For discovery mode, `plan()` is stubbed
with `Err(DbError::InvalidQuery("not yet implemented"))`.

**`explain()` returns `FullScan`.** Even though Postgres's own optimizer
handles all query execution, the `Backend` trait requires `explain()`. In
discovery mode it always reports `AccessPath::FullScan` since the adapter
has no insight into Postgres's internal index selection:

#### Test Execution

Tests fail fast if `POSTGRES_URI` is not set — `get_pool()` panics with a
clear message. This is intentional.

---

## 4. Acceptance Criteria

### Build & Compilation

1. `cargo build` succeeds including the new `semantic_db_postgres` crate.
2. `cargo check --quiet --message-format=short` passes with no warnings.
3. `cargo fmt` passes on all new/changed files.

### Discovery Mode (Read-Only)

4. With `POSTGRES_URI` set, `cargo test -p semantic_db_postgres` passes:
   - `test_discovery_single_pk` — discovers a table with a serial PK, queries
     it, verifies computed `id` is present.
   - `test_discovery_composite_pk` — discovers a table with a composite PK;
     synthetic ID uses `::` separator between PK values.
   - `test_ddl_forbidden_in_discovery` — `create_collection` returns an error
     mentioning "read-only" or "forbidden".
   - `test_insert_rejected_in_discovery` — `insert` returns an error.
   - `test_explain_and_plan_stub` — `plan()` returns an error (stubbed).

### SELECT Query Support

5. `SELECT * FROM postgres:public:table` works — all columns returned.
6. Filtering works: `SELECT ... WHERE col = value`.
7. Projection works: `SELECT col1, col2 FROM ...`.
8. ORDER BY and LIMIT/OFFSET work.
9. Computed `id` is injected on all read results via
   `inject_computed_attributes()`.

### Read-Only Guarantees

10. `insert()` returns an error.
11. `delete()` returns an error.
12. `update_where()` returns an error.
13. `delete_where()` returns an error.
14. `create_collection()` returns an error.
15. `execute_ddl()` returns an error.
16. `upsert_package()` returns an error.
17. `upsert_relationship()` / `delete_relationship()` return errors.
18. `execute_batch()` returns an error.
19. `explain()` returns `QueryExplain` with `AccessPath::FullScan`.
20. `plan()` returns an error (stubbed until managed mode).
21. No internal tables (`_semantic_meta`, `_semantic_migrations`) are created.

### Integration

22. The `PostgresBackend` integrates with the `Db` wrapper.
23. `catalog()` returns the in-memory catalog with zero Postgres I/O on
    subsequent calls.
24. `get()` correctly resolves the synthetic ID to PK values and queries
    the Postgres table.

---

## 5. File Map

```
crates/db_postgres/
├── Cargo.toml
└── src/
    ├── lib.rs           — Re-exports: PostgresBackend, PostgresConfig, PostgresMode
    ├── backend.rs       — PostgresBackend struct, Backend trait impl, init
    ├── discovery.rs     — discover_catalog(), information_schema queries
    ├── query.rs         — Query → SQL compilation, execution, row mapping
    └── sql.rs           — Quote ident, pg_type_to_semantic(), row_to_object()
```

---

## 6. Risks and Mitigations

| Risk | Mitigation |
|---|---|
| `tokio-postgres` lacks compile-time query checking | Runtime parameterized queries; use `.prepare()` for cached plans on hot paths |
| Type mapping incompleteness | Start with common types, extend iteratively as needed |
| Composite PK parsing ambiguity with `::` | `::` is extremely rare in column values. Acceptable for v1. Document limitation. |
| Connection pool exhaustion | `deadpool` handles lifecycle; provide configuration via `PostgresConfig` |
| ORDER BY / LIMIT with computed attributes | Computed attributes evaluated after fetch — cannot participate in server-side ORDER BY. Same limitation as KV backend |
| SQL injection via table/column names | Table/column names come from the catalog and are `quote_ident()`-escaped. All values use parameterized `$N` bindings |
| Discovery mode accidentally creating tables | All write paths return errors. Init path never creates tables |
| Managed mode causes confusion | `PostgresMode::Managed` returns clear error at construction time: "not yet implemented" |
| Generic SQL dialect may miss Postgres-specific features | Acceptable for v1 — basic SELECT syntax is portable. Postgres-specific SQL parsing tracked in [`followups.md`](followups.md) |
