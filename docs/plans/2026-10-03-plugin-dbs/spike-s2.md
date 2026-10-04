# S2: overlay catalog without index paths

Question: how can federation reuse lowering while suppressing local indexes?

Evidence:

- `crates/db_core/src/catalog/catalog.rs:34,707`: `Catalog` is cloneable;
  `upsert_collection` uses its private id allocator for a fresh local id.
  Mutating a clone does not touch persisted state.
- `crates/db_core/src/catalog/catalog.rs:739`: every new collection receives
  builtin id and type indexes; disabling automatic indexing alone is
  insufficient.
- `crates/db_core/src/catalog/catalog.rs:266,1080`: public `indexes()` and
  `delete_index(collection, name)` provide an existing way to remove every
  index from the clone, including its collection-index lookup entries.
- `crates/db_core/src/plan/optimizer.rs:1716`: scan lowering invokes index,
  range, and full-text access planners using the query context catalog.
  Passing absent statistics alone does not remove catalog index paths.
- `crates/db_core/src/plan/optimizer.rs:1475`: index joins additionally need
  statistics reporting a usable equality index and estimated row counts.
- `crates/db_core/src/plan/execute.rs:2751` and
  `crates/db_core/src/output.rs:6`: computed injection and formatting use
  only schema definitions in the supplied catalog.

Answer and recommendation: apply exposed-schema DDL and create virtual
collections in a cloned catalog, then collect `(collection, index name)`
from `indexes()` and call `delete_index` for each. Retain definitions and
collection ids, and construct `QueryContext` with that overlay. Normal
lowering remains reusable with no core type or optimizer change. The cost is
host scan/join execution; LocalSource still queries the original catalog
through the local engine and retains its own indexes. Phase 5 may add only
explicitly negotiated synthetic indexes after this removal.
