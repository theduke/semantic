# S3: local fragment round-trip

Question: can LocalSource obtain canonical raw rows through query_data?

Evidence:

- `crates/app/src/db.rs:97,231`: the async AST API delegates into the local
  database's query-data implementation.
- `crates/db_core/src/canonical.rs:949,992`: canonical entity-relative
  qualified field paths are retained by repeated canonicalization. The
  guarantee applies to leaf fragments, rather than binding-wrapped paths.
- `crates/db_core/src/plan/logical.rs:110`: an empty projection creates no
  Project node, so full entities (including id/type) pass through. An empty
  path wildcard also copies the full object (`query.rs:2049`), but is not
  needed for LocalSource.
- `crates/db_core/src/output.rs:16`: `FieldFormat::Qualified` immediately
  returns the same object; no keys or values are changed.
- `crates/db_core/src/embedded/db/reader.rs:153,191`: local selects
  canonicalize, optimize, execute, inject computed attributes, and format
  output in that order.
- `crates/db_core/src/plan/execute.rs:2751`: computed attributes are inserted
  only when their qualified key is absent, so injecting twice is harmless.

Answer and recommendation: construct local fragment queries with empty
projection and Qualified field format. Bind fragment parameters through
query_data, retaining canonical entity-relative predicates. The resulting
rows can participate directly in host joins, sorting, and residual filters.
Computed attributes can be injected again by federation without overwriting
local values. No local executor path or core type needs to change.
