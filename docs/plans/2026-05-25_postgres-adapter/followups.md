# Postgres Adapter — Followups (Deferred Items)

Items deferred from the initial read-only discovery implementation.
Address these as needed after Phase 1 is stable.

---

## 1. `SqlDialectKind::PostgreSql` for SQL Parsing

**What:** Override `Backend::sql_dialect()` to return
`SqlDialectKind::PostgreSql` instead of the default `Generic`.

**Why deferred:** The default `Generic` works correctly for SELECT queries
(the only supported operation in Phase 1). The `PostgreSql` dialect in
sqlparser mainly affects DDL parsing (CREATE TABLE syntax variants, data
type keywords) and edge cases in identifier resolution. None of these matter
for read-only SELECT operations.

**When to address:** When Phase 2 (managed mode) begins, or if a query
parsing bug is traced back to dialect differences.

---

## 2. Composite PK Ambiguity

**What:** The entity ID format for composite PKs uses `::` as a separator
between PK values (e.g., `composite_test-1::2`). If a PK value contains `::`,
parsing produces incorrect results.

**Why deferred:** Extremely unlikely in practice. The initial implementation
documents this limitation and rejects such IDs with a clear error.

**Potential fix:** Use a self-delimiting encoding like length-prefixed values
(e.g., `composite_test-3:1::2:abc` → pk1="1::2", pk2="abc"). This would be a
breaking change to the ID format, so decide early if needed.

---

## 3. Connection Pool Configuration

**What:** The initial implementation hard-codes pool size in tests and
provides a minimal `PostgresConfig`. A production-ready setup should expose:
- Pool size (min/max)
- Connection timeout
- Statement timeout
- Health check interval
- TLS configuration
- Application name

**Why deferred:** The `deadpool-postgres` defaults are reasonable for
development. Production tuning is site-specific.

---

## 4. `explain()` / `plan()` Real Implementation

**What:** Currently stubbed with an error + `// TODO`. A real implementation
would inspect the catalog's index metadata to determine the access path
(FullScan vs IndexLookup).

**Why deferred:** Only matters for the managed-mode query planner. In
discovery mode, all queries go to Postgres's own optimizer anyway.

---

## 5. `ALL_COLLECTION_ALIAS` ("all") Support

**What:** The `"all"` collection alias lets queries span multiple
collections via `UNION ALL`.

**Not needed in discovery mode.** The naming schema is fixed — collection
names are always `"postgres:{schema}:{table}"` and users know their table
names directly. The "all" alias is primarily a managed-mode feature for the
default `"entities"` collection. Can be implemented if a use case arises.

---

## 6. View and Materialized View Support

**What:** The `information_schema` query filters to `BASE TABLE` only.
Views (`VIEW`) and materialized views are not discovered.

**Potential addition:** Add a configuration option to include views. Views
have no PK, so `get()` by ID would not work — only SELECT queries.
