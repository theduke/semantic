# Initial Graph Query Support Plan

Date: 2026-08-30

Status: proposed

Scope: read-only graph pattern queries over the existing typed relationships, initially for the KV and redb backends

## 1. Executive decision

Implement a deliberately bounded subset of **SQL/PGQ** (`ISO/IEC 9075-16:2023`) inside the existing SQL query format. Expose the current catalog and entity data as one read-only virtual property graph and support `GRAPH_TABLE(... MATCH ... COLUMNS ...)` queries. Do not introduce a separate Cypher, GQL, Gremlin, or SPARQL endpoint in the initial release. Keep the graph AST and execution operators language-neutral; Cypher is the strongest practical dedicated-language candidate for a later second front end, while ISO GQL is the preferred long-term dedicated standards target.

SQL/PGQ is the best fit because this project already has SQL parsing, a row-oriented result model, relational logical/physical plans, and typed relationships backed by ordinary entity records. SQL/PGQ was specifically standardized for property-graph pattern matching over data that remains in tables; `GRAPH_TABLE` produces a table that composes with normal SQL. PostgreSQL describes the same architecture as a read-only property-graph view over regular relations using the same planning and execution infrastructure ([PostgreSQL property graphs](https://www.postgresql.org/docs/19/ddl-property-graphs.html), [graph queries](https://www.postgresql.org/docs/19/queries-graph.html)). The standard is published as [ISO/IEC 9075-16:2023](https://www.iso.org/standard/79473.html).

The target flow is:

```text
SQL/PGQ text
    |
    v
SQL shell parser + focused GRAPH_TABLE parser
    |
    v
typed GraphQuery AST --catalog resolution--> canonical graph pattern
    |
    v
graph logical operators --cost/semantic rules--> physical graph operators
    |
    v
batched vertex lookup / edge expansion through AsyncPhysicalDataSource
    |
    v
existing Object row stream, projection, sort, aggregate, distinct, limit
```

No storage migration is part of this plan. In particular, do not add collections, relationship fields, graph objects, serialized formats, or mandatory indexes. KV/redb execution will use `RelationType`, the underlying entity collections, existing equality indexes, and the already-maintained internal `__semantic.relationship_edges` collection. Query-lifetime adjacency maps are allowed because they are execution state, not persisted state. This is a virtual graph over a looser entity model, not a claim that the persisted catalog already contains a complete SQL/PGQ property-graph declaration: collection membership, optional class membership, relationship type, and endpoint collection identity must be resolved as separate dimensions.

## 2. Why SQL/PGQ

### 2.1 Alternatives considered

| Language | Strengths | Mismatch with this repository | Decision |
|---|---|---|---|
| SQL/PGQ | ISO SQL standard; property-graph patterns return a table; combines with SQL predicates, aggregation, ordering, and joins; designed for graph views over existing relations | Rust `sqlparser` 0.61 has no `GRAPH_TABLE` AST, so a focused parser is needed initially | **Choose** |
| GQL | Standalone ISO property-graph language with optional labels, typed/property-bearing edges, and a broad long-term feature model ([ISO/IEC 39075:2024](https://www.iso.org/standard/76120.html), [GQL project](https://gql.net/)) | Requires a second statement, expression, transaction, DDL/DML, result, and tooling surface; no mature storage-neutral Rust parser is currently available | Preferred dedicated standards target, but not the first surface |
| Cypher/openCypher | Concise declarative property-graph syntax; optional labels fit mixed typed/untyped entities; public grammar and TCK are maintained in the [official openCypher repository](https://github.com/opencypher/openCypher) | Adds another text format and Cypher-specific expression/path semantics alongside SQL; openCypher compatibility is not the same as ISO GQL conformance | **Best practical dedicated alternative**; a later front end can lower to the same graph IR |
| Gremlin | Expressive traversal/data-flow language and broad TinkerPop ecosystem ([TinkerPop reference](https://tinkerpop.apache.org/docs/current/reference/)) | Its traverser, step, barrier, side-effect, and path semantics require a traversal virtual machine rather than merely another declarative parser; this is a poor fit for the current relational optimizer | Future compatibility surface only, not an initial database language |
| SPARQL | Mature W3C standard and excellent Rust parsing support | Its native model is RDF datasets/terms, not a labeled property graph ([SPARQL 1.1](https://www.w3.org/TR/sparql11-overview/)); class mapping to `rdf:type` is possible, but edge identity/properties require an RDF reification/RDF-star policy | Add only if an explicit RDF interoperability product is designed |
| PGQL | Mature property-graph query syntax close to SQL/PGQ | Vendor-led and now largely superseded for this use case by the ISO SQL/PGQ surface | Reject |

### 2.2 Product and implementation fit

SQL/PGQ preserves the existing user contract:

- `TextQueryFormat::Sql` remains the format; CLI, RPC, and `Db::query` do not need a new language selector.
- `GRAPH_TABLE` produces ordinary rows, so `QueryResult::Select(Vec<Object>)` remains sufficient.
- Existing scalar `Expr`, projection, aggregation, sorting, distinct, limit, and streaming execution can be reused after pattern matching.
- A graph pattern can be optimized as a declarative join/expansion plan rather than executed as per-row application callbacks.
- The read-only first release avoids graph DDL and mutation semantics while delivering the main usability improvement: pattern matching and traversal.

SQL/PGQ does not eliminate the storage-mapping work. Standard property-graph declarations normally identify vertex tables, edge tables, keys, and endpoint references; Semantic's `RelationType` does not always carry equivalent endpoint collection metadata. The initial `semantic` graph is therefore an implementation-defined, catalog-derived graph view with an explicitly documented label and endpoint policy. The same ambiguity would exist behind Cypher, GQL, or Gremlin, so changing syntax would not solve it.

The syntax should follow the standard shape, for example:

```sql
SELECT child_id, ancestor_id
FROM GRAPH_TABLE (
  semantic
  MATCH (child IS "collection:entities")-[IS "relation:semantic:parent"]->{1,4}(ancestor IS "collection:entities")
  WHERE child.type = 'semantic:document'
  COLUMNS (
    child.id AS child_id,
    ancestor.id AS ancestor_id
  )
) AS matches
ORDER BY child_id, ancestor_id
```

`semantic` is the initial built-in virtual graph name. It is a catalog view, not a persisted graph object.

### 2.3 Rust parser ecosystem and dependency decision

Parser quality changes the implementation risk, but it does not outweigh language/engine fit:

| Surface | Credible Rust implementation | Dependency assessment |
|---|---|---|
| SQL/PGQ | [`squawk-syntax`](https://docs.rs/squawk-syntax/latest/squawk_syntax/) parses PostgreSQL's property-graph/`GRAPH_TABLE` syntax into an error-recovering lossless CST; [Grafeo's adapters](https://grafeo.dev/architecture/crates/) also contain SQL/PGQ, GQL, Cypher, Gremlin, and SPARQL parsers | Squawk is the first spike candidate, not an automatic choice: it is PostgreSQL-oriented, sparsely documented, and still needs lowering into Semantic's graph AST. Grafeo is Apache-2.0 and useful as a reference, but its published parser layer is part of a full graph-engine workspace and currently requires Rust 1.91.1, above this workspace's Rust 1.90 MSRV. |
| ISO GQL | [`selene-db-gql` 1.4](https://docs.rs/selene-db-gql/latest/selene_gql/) exposes a typed, spanned AST and parser | Do not adopt it. The crate bundles analyzer, planner, optimizer, runtime, and Selene graph/storage dependencies; it requires Rust 1.95. More importantly, its own [release status](https://github.com/jscott3201/selene-db#release-status) declares 1.x end-of-life and the active 2.0 line alpha/path-dependency oriented. It remains a valuable implementation reference, not a stable parser dependency. |
| Cypher/openCypher | [`decypher`](https://docs.rs/decypher/latest/decypher/) has a parser-only, error-resilient Rowan CST, typed/spanned AST, printer, optional HIR, and MIT/Apache-2.0/EUPL licensing. [`opencypher`](https://docs.rs/opencypher/latest/opencypher/) is lighter and exposes a broad typed/spanned AST and visitor. [`cypher-parser`](https://docs.rs/cypher-parser/latest/cypher_parser/) offers a practical read-only subset plus a generic executor. | `decypher` is the best dedicated-language spike candidate, but it is pre-1.0, explicitly marks its AST/completeness unstable, and currently requires Rust 1.94. `opencypher` 0.1.4 fits Rust 1.74 and looks strong on paper, but its declared source repository currently returns 404, so provenance/maintenance must be resolved before adoption. `cypher-parser` fits Rust 1.89 but deliberately implements a subset and couples parsing to its executor API. Pin any experiment exactly; lower into a Semantic-owned IR. |
| Gremlin | [`gremlin-rs`](https://docs.rs/gremlin-rs/latest/gremlin_rs/) is generated from TinkerPop's official [`Gremlin.g4`](https://github.com/apache/tinkerpop/blob/master/gremlin-language/src/main/antlr4/Gremlin.g4) and is Apache-2.0 | It returns an ANTLR parse tree rather than an ergonomic stable AST, is version 0.1.0, and its repository has only two commits. Parsing is the easy part; faithfully lowering Gremlin traverser semantics is the larger cost. |
| SPARQL | Oxigraph's standalone [`spargebra`](https://docs.rs/spargebra/latest/spargebra/) parser is MIT/Apache-2.0, supports SPARQL 1.1 Query/Update plus optional 1.2 work, and emits SPARQL algebra; Oxigraph documents it as a [reusable building block](https://github.com/oxigraph/oxigraph#readme) | This is the most mature reusable parser surveyed, but adopting it would commit Semantic to an RDF virtual-dataset mapping. Good crate, wrong initial data model. |

Retain SQL/PGQ as the first surface because it composes graph matches with the engine's existing SQL and row operators. If a dedicated endpoint is requested, choose Cypher before Gremlin or SPARQL for near-term implementability, while designing its accepted subset to converge toward GQL concepts. Do not expose both SQL/PGQ and Cypher until one canonical graph IR and one differential semantic suite prove that equivalent patterns return equivalent rows.

## 3. Current repository architecture

### 3.1 Query surface and flow

- `crates/data/src/query/mod.rs` is the public neutral query contract. `Query` currently contains row-oriented select/insert/update/delete variants, while `TextQueryFormat` contains `Sql` and `Prql`.
- `crates/db_core/src/query.rs` converts public queries into the core query types. `SelectQuery`, `JoinQuery`, `Expr`, and `QueryResult` are the principal current contracts. `Expr::RelationExists` already models direct and transitive reachability.
- `crates/db_core/src/query/sql/enabled.rs` parses SQL with `sqlparser` and recognizes `has_relation(...)` and `has_relation_path(...)`. Its parser/printer already centralizes the supported SQL subset.
- `crates/db_core/src/plan/logical.rs::build_logical_plan` builds `LogicalPlan`; `crates/db_core/src/plan/optimizer.rs::Optimizer` applies rewrite/lowering passes; `crates/db_core/src/plan/execute.rs::execute_physical_plan_stream` executes `PhysicalPlan` as batches of `QueryObjectAccess` values.
- `crates/db_core/src/plan/execute.rs::AsyncPhysicalDataSource` currently offers scans, filtered scans, and equality-index lookup. Joins are hash or nested-loop joins and already bind source rows under nested object names.
- `crates/db_kv/src/db.rs::KvDb::select` canonicalizes, collects collection statistics, optimizes, and executes through `KvPhysicalDataSource`.
- `crates/db_postgres/src/query.rs::QueryCompiler` follows a different path: it prints the semantic select back to SQL and submits it to PostgreSQL. `PostgresBackend` is read-only discovery mode, and discovery currently registers tables/classes/index metadata but not foreign keys as `RelationType` values.
- `crates/db_core/src/federation.rs` resolves logical sources across backends and uses the shared physical executor.

### 3.2 Collections, classes, and mixed entities today

- A collection is the physical/catalog scan boundary and contributes row identity, but it is not a class. `CollectionKind::{Untyped, Schema, Polymorphic}` controls normalization/schema behavior; it does not establish one class for every row.
- `type` is an ordinary canonical object field. In schema-driven collections a row may resolve to a registered class or record type, contain an unknown type in permissive mode, or omit `type` entirely. Tests explicitly allow typeless rows even under registered-schema handling. One collection can therefore mix several classes, records, unknown types, and untyped entities.
- `ClassType` has one optional `inherits` base and zero or more `extends` mixins. Validation walks both relationships transitively for field collection and ref-target compatibility. A graph class label must use the same closure rather than treating only the concrete `type` string as membership.
- Collections always have equality indexes for canonical `id` and `type`, which can seed collection- and class-constrained vertex scans without changing storage.

### 3.3 Typed relationships today

- `crates/data/src/schema/relation/relation_type.rs::RelationType` defines an id, name, `source_collection`, `RelationMode`, and `RelationIndexingMode`. Its `source_collection` means the collection containing the rows from which edge occurrences are read. For an embedded relation this is also the source-vertex collection; for an external relation it is the edge-entity collection and says nothing about either endpoint's collection.
- `RelationMode::Embedded` identifies a ref-typed attribute on a vertex row. `RelationMode::External` identifies relationship entities whose class/collection aliases resolve canonical `from` and `to` fields containing bare endpoint ids. `RelationType` itself has no endpoint collection ids, endpoint key references, external-row predicate, or edge class id.
- `crates/db_core/src/catalog/catalog.rs::Catalog::upsert_relationship` validates the relationship against the collection and attribute schema. `RelationshipSchema` stores the assigned local id plus the original `RelationType`.
- Reference fields are already useful to the relational optimizer: `RefPathJoinLiftPass` in `crates/db_core/src/plan/optimizer.rs` turns nested ref paths into joins.
- `crates/db_kv/src/db.rs` maintains `__semantic.relationship_edges` with `relation`, bare `source`, bare `target`, `depth`, `relation_source`, and `relation_target`. The collection already has equality indexes on both composite endpoint keys, but neither key contains endpoint collection identity or an external edge-row id.
- `KvDb::compute_relationship_edges` emits direct rows for all registered relationships. When `RelationIndexingMode::Enabled`, it also computes shortest transitive reachability rows. `try_relation_lookup_ids`, `lookup_edge_endpoint_ids`, and `relationship_exists` use those rows to accelerate `RelationExists` filters.

### 3.4 Important limitations to address

1. `RelationExists` is a boolean predicate, not a row-producing traversal. It cannot bind intermediate/end vertices or compose several edge patterns naturally.
2. Relationship predicates are deliberately kept as backend filtered scans by `choose_scan_source`; the generic executor cannot evaluate them itself.
3. The optimized KV fast path recognizes only the exact shape where one endpoint is the current row id and the other is a constant. Other shapes can scan every row and perform repeated relationship lookups.
4. The materialized transitive rows collapse parallel edges and retain only shortest depth. They are valid as a reachability accelerator, but cannot by themselves implement SQL/PGQ bag semantics, edge identity, or actual path values.
5. External relationship properties live on the original relation entity, not in `__semantic.relationship_edges`.
6. Relationship endpoints are persisted as bare ids. A graph execution key must include collection identity in memory, but the stored edge cannot provide it. An adjacent collection label can supply it; a class label generally cannot because classes are not bound to one collection.
7. External edge entities can share a collection with ordinary vertex entities and with other relationship types. The current external cache builder scans every row having resolvable `from`/`to` fields for every `RelationType`; it does not first enforce `semantic:relation:relation = RelationType.id` or `type = RelationType.id`. That behavior is not a safe row discriminator for graph bag/edge identity semantics.
8. Rebuilding all relationship closure rows on mutations is outside this query-language task. The new executor must not make that write behavior worse or make correctness depend on transitive materialization.

## 4. Initial semantic profile

### 4.1 Virtual property graph mapping

Expose one graph named `semantic`:

- **Vertex occurrence:** one graph-visible entity row in a non-internal collection, subject to the external-edge-row policy below. Its execution identity is `VertexKey { collection: LocalCollectionId, id: String }`; the composite identity prevents collisions between equal ids in different collections. Vertex identity never derives from `type`.
- **Vertex properties:** the canonical object fields exposed by `CollectionSchema`, including computed fields after the existing materialization rules. Missing fields remain missing/null according to the existing expression semantics; mixed rows are not coerced to a common class shape.
- **Collection labels:** every vertex has exactly one reserved label `collection:<canonical collection name>`. This is graph-view metadata, not a persisted `type` and not a claim of class membership. Reserved prefixes and quoted identifiers keep collection/class/relation namespaces collision-free.
- **Optional class labels:** if the row's `type` resolves unambiguously to a registered class, add `class:<concrete class id>` and every transitive `inherits`/`extends` class label. A missing `type`, unknown permissive type, or record type contributes no class label, but the entity remains a vertex with its collection label. Class labels may span many collections and use each collection's existing `type` index as a union of indexed scans.
- **Unlabeled patterns:** an isolated `(n)` may scan the union of graph-visible vertex collections, subject to work limits. A class-only label also spans collections. An endpoint adjacent to an edge still needs a collection label unless the relationship schema can prove exactly one endpoint collection; class membership alone does not identify storage identity.
- **Edge type:** one catalog `RelationType`, exposed as reserved label `relation:<canonical relation id>`. A unique name alias may be accepted, but ambiguity is an error. This is independent of the external edge entity's concrete class.
- **Embedded edge occurrence:** the source entity plus its non-null ref attribute. It has a deterministic query-time edge key derived from relationship local id, source `VertexKey`, and target id.
- **External edge occurrence:** one relationship entity in `RelationType.source_collection` selected by the approved discriminator. Its edge identity is `EdgeKey { collection, id }`, so parallel rows between equal endpoints remain distinct; its non-system fields are edge properties. The edge entity collection is not an endpoint collection.
- **Endpoint resolution:** the source/target stored values are bare ids. The adjacent `collection:*` label supplies the missing collection and produces an exact `VertexKey`; a missing vertex is dangling and does not match. If an endpoint collection is not fixed statically, v1 rejects the pattern during canonicalization rather than globally searching by id, fanning out to every equal id, or silently choosing one. Do not assume ids are globally unique.
- **Direction:** preserve the declared source-to-target direction; support outgoing and incoming pattern arrows. Undirected patterns can lower to a union of both directions with duplicate control matching SQL/PGQ semantics.

This mapping is computed from the catalog at query compilation time. No `CREATE PROPERTY GRAPH` statement or persisted view is required.

The reserved labels are an implementation-defined profile, for example `(n IS "collection:entities")`, `(n IS "class:semantic:document")`, and `[e IS "relation:semantic:parent"]`. They make the storage/class distinction visible without inventing persisted labels. The user-facing documentation must also explain that `n.type = 'X'` tests the concrete stored property, whereas `n IS "class:X"` includes instances of classes that inherit or extend `X`.

### 4.2 Phase-0 semantic design gates

Resolve these before parser or executor implementation; none may be hidden behind a permissive match:

1. **External relation-row discriminator.** Existing code and fixtures exhibit two candidate conventions: canonical `semantic:relation:relation = RelationType.id`, and a concrete `type`/class id equal to the relation id. `RelationType` does not declare which is authoritative, and the current cache builder accepts any row with `from`/`to`. Audit existing persisted data and choose/document a compatibility rule (recommended direction: explicit relation field first, legacy exact type only under a catalog-validated relation class, conflict is an error). Add two relation types and ordinary vertices in one collection as a mandatory fixture. Until this is resolved, external graph edges and external cache rows cannot be considered type-safe.
2. **External edge entity as vertex.** Decide whether a row selected as an external edge is excluded from the vertex set or is intentionally exposed twice, once as a vertex and once as an edge. Property-graph element identity normally distinguishes nodes and edges; silently exposing it both ways would create surprising self-navigation. Recommended v1 policy is exclusion from the default vertex set, with a future explicit system-collection/edge-entity view if users need to query relationship records as nodes.
3. **Endpoint collection authority.** Neither `RelationType` nor a stored endpoint names a collection. Ref target classes may narrow valid classes but do not identify one collection. V1 therefore requires adjacent `collection:*` labels unless a unique collection follows from catalog metadata. A future choice among global-id uniqueness, per-relation endpoint collection metadata, or runtime unique resolution is a storage/catalog evolution and is outside this no-model-change plan.
4. **Class label closure.** Confirm that graph label membership follows the same transitive `inherits` plus `extends` closure already used by validation/ref compatibility. If `extends` is not intended as substitutability, stop and define separate graph semantics before exposing inherited labels.

### 4.3 Supported v1 syntax

Support read-only queries of this form:

```text
SELECT ...
FROM GRAPH_TABLE (
  semantic
  MATCH <one path pattern>
  [WHERE <existing scalar expression subset>]
  COLUMNS (<existing scalar expression> AS <name> [, ...])
) [AS alias]
[WHERE/GROUP BY/HAVING/ORDER BY/LIMIT/OFFSET handled by the existing SQL layer]
```

The graph pattern subset is:

- named or anonymous vertex patterns; isolated seeds may be unlabeled or class-labeled, while edge-adjacent endpoints require collection labels under the endpoint rule above;
- typed directed edges with a single relationship id;
- fixed multi-hop paths;
- node-property predicates in the graph `WHERE` clause;
- anonymous edges, plus named edge variables only for identity and direct external-edge properties when the backend can materialize the original edge occurrence;
- bounded quantified edges `{m,n}` with constant non-negative bounds and a configured maximum; and
- ordinary scalar graph columns, not graph-valued results.

### 4.4 Explicit v1 non-goals

- Graph DDL or graph mutation (`INSERT`, `MERGE`, `DELETE`, etc.).
- More than one `GRAPH_TABLE` or mixing graph and regular base sources in the first increment.
- Multiple comma-separated path patterns, `OPTIONAL MATCH`, non-linear patterns, alternation/type expressions, or quantified subpatterns.
- Unbounded quantifiers, shortest-path selectors, path variables, group variables, or returning arrays of traversed elements.
- User-selectable walk/trail/acyclic modes. V1 bounded expansion uses trail semantics: an edge occurrence cannot repeat within one matched path.
- Cross-backend or cross-database paths.
- Silent fallback to `has_relation_path` endpoint-set semantics when SQL/PGQ requires one row per matched path.

Unsupported syntax must fail during parsing or canonicalization with an actionable error, never return an approximate result.

## 5. Public and core query representation

### 5.1 Add an explicit graph AST

Add parallel public types in `crates/data/src/query/mod.rs` rather than overloading `SelectQuery.joins` or encoding graph state in collection strings:

```text
GraphSelectQuery
  graph_name
  path: GraphPathPattern
  graph_predicate
  graph_columns
  outer relational projection/filter/group/order/limit

GraphPathPattern
  vertices: ordered VertexPattern values
  edges: ordered EdgePattern values

VertexPattern
  binding
  labels
  inline_predicate

EdgePattern
  optional binding
  relationship_type
  direction
  min_hops/max_hops
```

Add `Query::GraphSelect(GraphSelectQuery)` and the matching conversions in `crates/db_core/src/query.rs`. Keep `QueryResult::Select` unchanged. Update exhaustive matches in `Backend`, KV, federation, RPC serialization, CLI plan formatting, and tests. This is an additive query capability; existing `SelectQuery` behavior must not change.

Do not lower directly to `Expr::RelationExists` in the parser. Keeping a typed graph AST preserves variable bindings, direction, multiplicity, quantifier semantics, source locations, and future path features.

### 5.2 Canonical graph representation

Add `crates/db_core/src/query/graph.rs` with resolved forms used only after catalog validation:

- graph name resolved to the built-in virtual graph;
- collection labels resolved to `LocalCollectionId` and canonical names, and class labels to class ids plus transitive closure;
- relationship ids resolved to `LocalRelationId`/`RelationshipSchema`;
- property fields canonicalized through each bound collection;
- every reference annotated with its binding and expected element kind;
- source spans retained for duplicate bindings, unknown labels, bad directions, invalid bounds, and type errors.

Add graph-aware canonicalization alongside `canonicalize_select_query` in `crates/db_core/src/canonical.rs`. Resolve reserved collection/class/relation labels separately, compute class closure, and derive each vertex binding's candidate collection set. Validate all bindings before planning. In particular, reject an edge type whose endpoint collections cannot be fixed/reconciled with the adjacent vertices; a class label is a row predicate, not an endpoint collection declaration.

## 6. SQL parser and printer

`sqlparser = 0.61.0` in `crates/db_core/Cargo.toml` does not currently expose SQL/PGQ/`GRAPH_TABLE` AST nodes. Avoid a fork of its full SQL grammar.

Before committing to a custom parser, run a time-boxed parser-adoption spike:

1. Pin the current `squawk-syntax` version in an isolated spike crate and parse the v1 fixture corpus, including the outer `SELECT`, `GRAPH_TABLE`, label expressions, quantified paths, graph `WHERE`, and `COLUMNS`.
2. Measure whether its typed CST provides stable accessors for every required node and whether PostgreSQL-specific grammar differs from the intended SQL/PGQ profile. Implement one throwaway lowering into the Semantic-owned `GraphSelectQuery`; do not leak Squawk nodes into public/core types.
3. In parallel, parse equivalent dedicated queries with an exact pinned `decypher` version (using a temporary newer toolchain because its current MSRV exceeds this workspace). This validates that the graph IR is genuinely language-neutral and gives a costed fallback if a dedicated endpoint becomes more important than SQL composition.
4. Record compile-time/dependency size, license/provenance, maintenance activity, diagnostic spans/recovery, fuzz behavior, grammar coverage, and parse-print-parse results. `opencypher` may enter the comparison only after its dead repository link/provenance is resolved. Grafeo/Selene may be read as licensed references, not copied or adopted as coupled engine dependencies.
5. Choose Squawk only if lowering is materially smaller and the dependency can meet this workspace's Rust 1.90 MSRV. Otherwise implement the focused parser below. Exit the spike with fixtures and a short ADR; do not let it delay the language-neutral graph IR.

Add `crates/db_core/src/query/sql/pgq.rs` with this narrow integration:

1. Lexically locate one top-level `FROM GRAPH_TABLE (` occurrence while respecting quoted strings, quoted identifiers, comments, and nesting.
2. Extract the balanced `GRAPH_TABLE` body and replace the factor with a synthetic table carrying the declared `COLUMNS` names.
3. Parse the outer SQL with the existing `sqlparser` bridge.
4. Parse graph name, `MATCH`, patterns, optional graph `WHERE`, and `COLUMNS` with a small recursive-descent parser over tokens.
5. Reuse the existing SQL expression conversion by parsing scalar fragments in controlled wrapper selects; do not implement a second expression language.
6. Combine the outer select AST and graph AST into `GraphSelectQuery`.

Before coding, create parser fixtures for comments, quoted relationship ids containing `:`, nested function calls, parentheses in string literals, aliases, and malformed/unbalanced input. If a maintained `sqlparser` release gains sufficient SQL/PGQ support during implementation, prefer upgrading and deleting the extractor after checking the existing SQL suite.

Extend `query_to_sql` to print the supported canonical SQL/PGQ subset. Add parse-print-parse tests. Continue reporting SQL/PGQ under `TextQueryFormat::Sql`; no CLI `QueryFormat` change is needed.

## 7. Logical and physical plans

### 7.1 Logical operators

Add graph nodes to `crates/db_core/src/plan/logical.rs`:

```text
GraphVertexScan { binding, collection, predicate }
GraphExpand {
  input,
  from_binding,
  edge_binding,
  to_binding,
  relationship,
  direction,
  min_hops,
  max_hops,
  target_collection,
  target_predicate
}
```

The builder selects an initial vertex and chains expansions. Rows use the existing nested binding shape already produced by joins, so qualified `binding.field` expressions continue to work with `ObjectAccess` and existing projection/sort/aggregate operators.

Teach every recursive logical utility about the new variants: source resolution, rewrite traversal, binding discovery, federation rejection/pushdown, explain formatting, and equivalence tests. Avoid a catch-all arm that could hide missed graph nodes.

### 7.2 Physical operators

Add to `crates/db_core/src/plan/physical.rs`:

```text
PhysicalGraphVertexScan
PhysicalGraphExpand { strategy, ... }

GraphExpandStrategy =
  DirectAdjacency
  | BoundedTrail
  | ReachabilitySemiJoin
```

`ReachabilitySemiJoin` is legal only when the graph subexpression asks existence/reachability and neither edge identity, intermediate elements, depth multiplicity, nor path multiplicity is observable. It may use the existing transitive closure. It must never replace a normal SQL/PGQ path match merely because endpoints are the only projected columns unless `DISTINCT` and all other semantics prove the rewrite safe.

### 7.3 Optimizer rules

Add graph-specific passes in `crates/db_core/src/plan/optimizer.rs`:

1. **Predicate pushdown:** split graph `WHERE` conjuncts by referenced binding and push single-vertex predicates into the corresponding vertex scan.
2. **Seed selection:** prefer a vertex constrained by id/equality index, then a selective vertex predicate, then the lower estimated collection cardinality. Reverse the path and direction when the end is cheaper to seed.
3. **Fixed expansion:** select `DirectAdjacency` for one hop.
4. **Bounded expansion:** select `BoundedTrail` and push target predicates into each terminal materialization.
5. **Reachability rewrite:** select `ReachabilitySemiJoin` only under the observability rule above and only when the relation has a usable closure index.
6. **Anonymous-edge pruning:** omit edge object materialization if no expression references the edge binding, while retaining correct edge multiplicity.
7. **Early limit:** push a safe result limit/work bound into expansion only when ordering, aggregation, and distinctness do not make it semantically visible.

Extend `StatsProvider` with defaulted graph statistics (direct edge count, estimated out/in degree, closure availability). Unknown statistics must produce conservative costs. KV can derive counts from the existing internal edge collection; no statistics persistence is required.

## 8. Execution data-source API

Extend `AsyncPhysicalDataSource` in `crates/db_core/src/plan/execute.rs` with default methods that return a clear unsupported error, so existing third-party implementations continue to compile with defined behavior:

```text
scan_graph_vertices(request) -> record-batch stream
expand_graph_edges(request, frontier: vertex-key batch) -> graph-edge-batch stream
lookup_graph_vertices(request, keys: vertex-key batch) -> record-batch stream
graph_capabilities() -> GraphCapabilities
```

Core request/result types should include:

- `VertexKey { collection_id, id }`;
- `GraphEdgeRecord { edge_key, relationship_id, source, target, optional_object }`;
- direction and exact relationship ids;
- requested edge observability (none, identity, full object);
- target collection constraint; and
- execution/work limits.

Keep graph expansion batched. For each input batch, deduplicate frontier keys for I/O, fetch adjacency once, then fan results back to the input bindings. Do not call `relationship_exists` once per input row. Bind output vertices/edges into the existing dynamic object row and pass the stream to existing filter/project/aggregate/sort/limit nodes.

For `BoundedTrail`, maintain per-partial-path edge identity sets, not just visited vertices: SQL graph trail semantics allow revisiting a vertex but not the same edge occurrence. Expand breadth-first by depth, emit matches for every depth in `[min,max]`, preserve distinct edge occurrences/parallel-edge multiplicity, and apply inline/terminal predicates as soon as their bindings exist.

Add resource controls to `ExecutionOptions` with documented defaults:

- maximum quantified depth;
- maximum expanded edge occurrences;
- maximum live partial paths;
- maximum query-lifetime adjacency-cache bytes; and
- cancellation checks between batches/depths.

Exceeding a bound returns a graph-resource error containing the pattern and consumed limit; it must not silently truncate results.

## 9. KV and redb implementation without storage changes

Implement the new data-source methods in `KvPhysicalDataSource`; redb inherits them because `RedbKvEngine` implements `KvEngine`.

### 9.1 Vertex access

- Resolve the labeled collection to `CollectionSchema`.
- Use `EntityStore::get_entity` for bound `VertexKey` values.
- Use current filtered scan/index lookup paths for unbound seeds and pushed equality predicates.
- Construct the in-memory composite `VertexKey`; never persist it or change entity ids.

### 9.2 Embedded relationships

- Forward expansion from bound sources reads the canonical ref attribute directly from the already materialized source row.
- Reverse expansion and unbound relation scans may use depth-1 rows from `__semantic.relationship_edges` through the existing `relation_target`/`relation_source` indexes only after the source collection is supplied by `RelationType` and the target collection is fixed by the adjacent vertex pattern.
- Generate a deterministic edge identity from relationship local id, source `VertexKey`, and resolved target `VertexKey`. Since an embedded relationship has at most one target per source attribute, this preserves occurrence identity.

### 9.3 External relationships

- The `RelationType.source_collection` contains the real edge occurrences and may also contain vertices/other edge types. Apply the Phase-0 approved relation-row discriminator before reading canonical `from`/`to`; never treat every row with endpoint-looking fields as this relation type.
- If suitable ordinary equality indexes exist, use them for a bound endpoint.
- Otherwise scan that relationship collection at most once per query and build bounded forward/reverse adjacency maps keyed by endpoint id. This preserves parallel edges, edge ids, and edge properties without changing persisted data.
- Key cached adjacency by `(relationship local id, endpoint collection, endpoint id)` after endpoint collections have been fixed; materialize the full original edge object only when a column/predicate references it.

### 9.4 Existing relationship-edge cache

Use `__semantic.relationship_edges` as an optional accelerator:

- depth `1` rows are direct adjacency for embedded reverse/unbound access and cheap endpoint pruning;
- depth `>1` rows for `RelationIndexingMode::Enabled` support legal reachability semi-joins and lower-bound/upper-bound prechecks;
- do not use closure rows to enumerate actual paths or external edge occurrences;
- verify `depth` and relationship id on every lookup, as existing `relationship_exists` does;
- because cache rows lack endpoint collection and external edge-row identity, use them only after endpoint collections are fixed and never to distinguish parallel external edges; and
- do not trust/reuse external cache rows produced under an unresolved or mismatched external row-discrimination rule.

No correctness path may require a new persisted index. Document that indexing external `from`/`to` fields improves repeated graph queries, using the existing index DDL, but keep the query-lifetime scan/hash fallback.

## 10. Backend capability and rollout matrix

### KV memory and redb

These are the first supported backends. Advertise SQL/PGQ only through a new `BackendCapabilities`/`GraphCapabilities` value; do not overload `supported_text_query_formats`, because plain SQL support does not imply SQL/PGQ support.

### PostgreSQL discovery backend

Do not pass SQL/PGQ blindly through `QueryCompiler`: the discovered catalog currently has no property-graph object and does not register foreign keys as semantic relationships. Initially return an explicit capability error before execution.

A later backend increment should:

1. discover foreign keys and register read-only `RelationType` metadata;
2. lower fixed graph expansions to parameterized joins and bounded paths to recursive CTEs, or use native SQL/PGQ only after server capability detection and an equivalent graph mapping exist; and
3. retain the no-storage-change rule by avoiding `CREATE PROPERTY GRAPH` in discovery mode.

### Federation

Initially accept a graph query only when every vertex and relationship resolves to one graph-capable backend. Reject cross-backend paths during source resolution with the involved bindings named. Later, partition fixed path fragments and exchange endpoint batches only after cost and identity semantics are specified.

## 11. Error model, explain, and observability

Add structured graph error categories under the existing `DbError::InvalidQuery`/core error boundary initially, with stable messages for:

- unsupported SQL/PGQ construct;
- unknown/ambiguous graph, vertex label, relationship id, or binding;
- endpoint collection mismatch;
- invalid or excessive quantifier bounds;
- graph capability missing on a backend;
- edge property requested from a non-materializable occurrence; and
- execution work limit exceeded.

Extend `QueryExplain`/`QueryPlan` additively to report graph access rather than calling it `FullScan`. Explain output should include seed binding, collection, relationship, direction, hop bounds, chosen strategy, whether the edge object is materialized, estimated rows/cost, closure/index use, and resource caps. Update CLI JSON plan formatting and RPC conversion accordingly.

Add tracing spans/counters around graph parse, canonicalization, seed selection, adjacency calls, cache hits, expanded edges, partial paths, output matches, and rejected work. Do not log entity properties or complete query literals by default.

## 12. Implementation phases

### Phase 0: semantic fixtures and capability boundary

- Resolve the four semantic design gates and write an ADR-style module document for collection/class labels, external row discrimination, edge-entity vertex visibility, endpoint authority, and the v1 subset.
- Add backend graph capability types with default unsupported behavior.
- Create shared embedded/external relationship fixtures, including typeless/unknown/concrete/inherited/extended classes mixed in one collection, relation rows and vertices coexisting in one collection, two external relation types sharing a collection, duplicate ids across collections, dangling endpoints, and parallel external edges.
- Add negative fixtures for every explicit non-goal.

Exit: mapping decisions are executable as tests; external rows cannot leak across relation types; ambiguous endpoints fail explicitly; no backend falsely advertises support.

### Phase 1: AST, parser, printer, canonicalization

- Add graph query types and `Query::GraphSelect` conversions.
- Run the Squawk/decypher parser-adoption spike and record the dependency decision; then adopt Squawk or implement the focused `GRAPH_TABLE` parser and SQL shell integration.
- Implement printer round trips and source-located errors.
- Resolve reserved graph/collection/class/relation labels, class closure, candidate collections, bindings, and properties through `Catalog`.

Exit: supported queries parse, print, and canonicalize; unsupported forms fail deterministically; existing SQL/PRQL tests are unchanged.

### Phase 2: fixed-path planning and execution

- Add vertex-scan/direct-expand logical and physical nodes.
- Extend recursive optimizer/executor utilities and explain output.
- Add batched graph data-source methods.
- Implement KV/redb vertex access and correct embedded/external one-hop expansion.
- Compose two or more fixed hops and existing relational projection/filter/order/aggregate/limit.

Exit: fixed paths preserve direction, bindings, parallel-edge multiplicity, and property filters without per-input-row scans.

### Phase 3: bounded quantified paths and safe acceleration

- Implement bounded trail enumeration with resource controls.
- Add optional closure semi-join under the observability proof rule.
- Add query-lifetime external adjacency caching and graph statistics.
- Add seed reversal and predicate pushdown cost rules.

Exit: bounded paths match a naive reference evaluator on randomized graphs, respect limits, and show bounded I/O in benchmarks.

### Phase 4: integration and guarded release

- Extend shared backend tests, RPC/query serialization, CLI explain output, UI query surfaces, and documentation.
- Enable for memory KV/redb behind a temporary feature/config gate.
- Run compatibility, fuzz, and performance gates; then enable by default for those backends.
- Leave PostgreSQL/federation capability-gated until their separate exit criteria pass.

## 13. Verification plan

### 13.1 Parser and semantic tests

- SQL/PGQ examples with quoted canonical ids, comments, aliases, and nested scalar expressions.
- Parse-print-parse equality for every supported construct.
- Unknown/duplicate bindings, wrong element kinds, endpoint mismatches, invalid bounds, and all non-goals.
- Fuzz the focused parser with arbitrary tokens and balanced/unbalanced delimiters; it must not panic.

### 13.2 Planner tests

- Anchors choose indexed/selective endpoints and reverse direction when cheaper.
- Predicates attach to the correct binding.
- Anonymous edges avoid object materialization.
- Closure semi-join appears only in semantically safe existence plans.
- All new logical nodes survive rewrite traversal and produce informative explain output.

### 13.3 Execution and shared backend tests

- Embedded and external forward/reverse fixed paths.
- Mixed typed/untyped/unknown-type vertices, inherited/extended class labels, class labels spanning collections, and unlabeled isolated scans.
- Multi-hop paths, cycles, self-loops, null refs, dangling endpoints, duplicate ids across collections, and parallel external edges.
- External rows are discriminated correctly when vertices and several relation types share one collection; conflicting legacy/explicit discriminators error.
- Edge properties for external relationships and deterministic identity for embedded edges.
- Bound ranges including zero hops, multiple emitted depths, and trail uniqueness.
- Projection, `WHERE`, `DISTINCT`, aggregation, ordering, limit, offset, and computed attributes.
- Result equivalence between streaming batch sizes.
- Memory KV and redb persistence/reopen behavior; graph support must require no migration.
- PostgreSQL and cross-source federation return explicit capability errors.

Build a small naive graph-pattern evaluator in tests and property-test random graphs against optimized fixed/bounded execution. Import applicable syntax/semantic scenarios from public SQL/PGQ examples; do not claim full standards conformance for the subset.

### 13.4 Performance gates

Benchmark chains, stars, trees, sparse random graphs, dense graphs, selective anchored queries, reverse queries, and external-edge collections. Record:

- storage calls and rows read;
- expanded edge occurrences;
- time to first batch and total latency;
- peak partial paths and adjacency-cache bytes; and
- comparison with equivalent existing SQL joins/`has_relation_path` predicates.

Acceptance gates:

- anchored one-hop expansion performs a bounded number of index/entity operations per input batch, not one collection scan per row;
- an external relation without endpoint indexes is scanned at most once per query;
- memory remains within configured work/cache limits;
- existing non-graph query benchmarks show no material regression; and
- no persisted bytes or catalog snapshot shape change merely by enabling/querying SQL/PGQ.

After implementation, obey the repository validation sequence through the Nix devshell when available:

```bash
cargo test --quiet --message-format=short
cargo check --quiet --message-format=short
cargo fmt --all -- --check
```

Run focused crate tests during development, then the workspace commands before release.

## 14. Documentation and compatibility

- Add a graph-query chapter showing the virtual mapping, supported grammar, direct and bounded examples, limits, and backend matrix.
- Document that SQL/PGQ is a subset under the SQL text format and that current `has_relation`/`has_relation_path` functions remain supported.
- Mark those functions as lower-level reachability predicates, not aliases for full path-match semantics.
- Document edge identity/property differences between embedded and external relationships.
- Include `EXPLAIN` examples showing direct adjacency, bounded trail, and reachability semi-join.
- Do not claim ISO SQL/PGQ conformance until an explicit conformance profile and external test suite pass.

## 15. Risks and mitigations

| Risk | Mitigation |
|---|---|
| A partial parser is mistaken for full SQL/PGQ | Publish an exact feature matrix; reject unsupported syntax early; do not use a conformance label |
| Closure rows produce incorrect path multiplicity | Restrict closure to proven existence/semi-join rewrites; enumerate direct edge occurrences for matches |
| Bare endpoint ids collide across collections | Use composite query-time `VertexKey`; require/resolve endpoint collection labels |
| External relationship rows leak across types | Resolve the row-discriminator design gate first; filter before endpoint extraction; do not trust old external cache rows blindly |
| Collection and class labels are conflated | Use reserved `collection:`, `class:`, and `relation:` namespaces; treat class labels as row predicates and collection labels as identity constraints |
| External edge entities unexpectedly appear as both nodes and edges | Make vertex visibility an explicit Phase-0 decision; default recommendation is to exclude selected edge rows from the vertex set |
| External edges cause repeated scans | Batch expansions and build one bounded query-lifetime adjacency map when indexes are absent |
| Variable paths explode | Require bounded quantifiers and enforce depth, expansion, partial-path, memory, and cancellation limits |
| New enum variants break consumers | Update exhaustive matches in one phase and retain all existing variants/behavior; add serialization compatibility fixtures |
| PostgreSQL appears to support graph SQL but has no graph mapping | Advertise a separate capability and fail before compilation until FK/relationship discovery exists |
| Optimizer rewrites change bag semantics | Encode observability/preconditions in each rule and test parallel edges, cycles, and `DISTINCT` explicitly |

## 16. Definition of done

Initial graph query support is complete when:

1. The documented SQL/PGQ subset parses and round-trips under `TextQueryFormat::Sql`.
2. Catalog resolution keeps collection identity separate from optional concrete/inherited/extended class labels, maps typed relationships with the approved external row discriminator, and never guesses ambiguous endpoint collections.
3. Memory KV and redb execute fixed and bounded patterns with correct binding, direction, trail, property, and bag semantics.
4. Expansion is batched and uses existing storage/index data or a bounded query-time cache; no persisted data model or migration changes.
5. Explain output names graph operators and access strategies.
6. Unsupported backends and syntax fail explicitly.
7. Randomized differential tests, parser fuzzing, shared backend tests, resource-limit tests, and performance gates pass.
8. Existing SQL, PRQL, mutation, relationship predicate, RPC, and storage compatibility suites remain green.
