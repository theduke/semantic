# S1: predicate shapes per leaf

Question: which field paths can a virtual scan evaluate directly?

Evidence:

- `crates/db_core/src/canonical.rs:949`: canonicalization removes the base
  binding and resolves its top-level field through the catalog. Join-binding
  paths are returned unchanged; `b.title` can therefore remain unqualified.
- `crates/db_core/src/canonical.rs:992`: plain names resolve through global
  attribute aliases (or a class-named collection). A `type` predicate does
  not supply class-specific alias resolution in a polymorphic collection;
  ambiguous names fail and require qualified attribute ids.
- `crates/db_core/src/plan/optimizer.rs:280`: single-side predicates move
  below compatible joins; cross-side predicates remain above the join.
- `crates/db_core/src/plan/optimizer.rs:359`: direct leaf pushdown strips
  the source binding recursively. Thus `b.z = 1` becomes `z = 1`, but an
  already qualified `b.ns:z` becomes `ns:z`.
- `crates/db_core/src/plan/execute.rs:59,773`: filtered scans evaluate
  predicates against raw entity batches. Binding wrappers belong to joins,
  rather than scans.

Answer and recommendation: preserve raw entity rows. Strip only the current
leaf binding from field paths, reject cross-binding scan candidates, then
canonicalize that leaf predicate against its own overlay collection using
the existing canonicalization entry point. Apply the resulting qualified
entity-relative expression to both negotiation and residual evaluation.
Do not assume whole-query canonicalization has qualified join-side fields.
Keep cross-source join predicates on the host. No core change is required.
