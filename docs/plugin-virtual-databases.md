# Plugin virtual databases

A virtual database (VDB) exposes one read-only, polymorphic entity collection
from a plugin. SQL, query ASTs and collection-addressed reads can combine its
entities with local collections and other VDBs. Plugins implement scans; the host
handles residual filters, joins, subqueries, aggregates, distinct results and
output formatting.

## Addressing and discovery

An activation with one `VirtualDatabase` export uses its activation id as the
collection name, for example `fx`. Multiple VDB exports use
`<activation_id>.<export>`, such as `github.issues`. Export names are not tables:
each export remains a polymorphic collection, and `WHERE type = 'github:Issue'`
selects one class within it.

```sql
SELECT * FROM fx WHERE type = 'json_dir:Document' ORDER BY id;
SELECT l.id, v.id FROM local_rows l JOIN fx v ON l.id = v.id;
SELECT a.id FROM fx a JOIN other_plugin b ON a.id = b.id;
```

The public AST sets `SelectQuery.collection = Some("fx".into())`; the
collection-addressed API uses `get("fx", id)`. Ordinary local collections and
registered local class names retain their existing SQL precedence, including
class-shaped JOIN names. A VDB never shadows a local collection. Conflicting or
invalid exports appear unavailable with a reason.

`semantic.vdb.list` reports names, plugin ids, exports, generations, availability,
schema revisions and class ids. `semantic.vdb.schema { name }` returns the
explicit exposed schema. `semantic.vdb.explain { query, params }` reports the
federated leaves and their negotiated support. These commands accept an optional
`scope_id`; the persisted catalog remains available through
`semantic.db.catalog` and does not contain runtime VDB definitions.

INSERT, UPDATE, DELETE, batches and DDL targeting a VDB are rejected as read-only.
Local-only requests follow the existing local database path and do not activate
plugins. VDBs are resolved lazily when a request references a nonlocal collection.
The `all` alias covers local data, and PRQL requests retain their local behavior.
UNION is not supported by the query AST/SQL implementation.

## Interface and entities

The interface is `semantic.vdb/v1/VirtualDatabase`, with asynchronous methods
throwing `VdbError { code, message }`:

| Method | Parameters | Result |
| --- | --- | --- |
| `describe` | none | `DatabaseDescriptor` |
| `negotiate` | `ScanRequest` | `ScanPlan` |
| `scan` | request, `AcceptedScan`, parameter bindings | entity stream ending with `ScanSummary` |

An entity is an `Object` with a non-empty string `id`. Ids must be unique within
one scan. Its optional `type` must name a class explicitly exposed in the
descriptor; absent `type` is accepted only when `allow_untyped` is true. Typed
entities use qualified attribute ids, for example `json_dir:content`, rather than
class aliases or JOIN-prefixed keys. Attributes must belong to the class or an
inherited/extension class.

The host checks structure, class membership, supplied value types, required
attributes and value constraints with the existing database validators. It does
not look up foreign-key targets in local storage or fill missing defaults into
remote entities. Invalid rows and duplicate ids fail the query; they are not
silently skipped. The host injects computed attributes after execution.

Raw interface streams emit `StreamEvent::Item(Value::Object(entity))` and then
`StreamEvent::End(Some(ScanSummary { rows }.into_value()))`. A successful stream
needs exactly that terminal summary with a matching row count. The Rust adapter
constructs it automatically and preserves plugin error codes/messages. Dropping
the stream cancels the scan's child token without cancelling the whole plugin
generation. External operations should observe the supplied token, for example
with `tokio::select!` around network or filesystem reads.

## Declaring the runtime schema

`DatabaseDescriptor.schema` is a `DatabaseSchema` containing the existing
`TypeDef`, `AttributeType`, `ClassType` and `RelationType` definitions. It is the
complete database surface, independent of the plugin's configuration types,
interface records and implementation helpers. Those plugin-layer definitions
must not be added merely because the plugin uses them internally.

Every reference must resolve within the exposed schema or the local scope
catalog. Reuse a local attribute by reference, such as `semantic:title`;
do not redeclare its definition. Redeclaring any local definition id is a
conflict. Two VDBs used by one query may share equal definitions, but conflicting
definitions fail planning. Relationships in v1 must use the effective virtual
collection name as `source_collection`, including an export suffix when needed.

The host lowers `schema.to_ddl_batch()` to upsert-only DDL, creates the virtual
collection shell, and applies the existing pure DDL validation to an overlay.
Missing references, invalid computed definitions and id conflicts make that VDB
unavailable. None of this installs a package, records a migration, or changes the
persisted scope catalog. Local entities therefore cannot use runtime VDB classes.

`schema_revision` is an opaque string. Change it whenever the exposed schema
changes; changing rows alone does not require a new revision. The synchronous
`VirtualDatabase::schema_revision()` must match the descriptor that `describe`
would currently return. The adapter copies that revision into every accepted
scan plan.

Queries and schema requests prepare only their referenced virtual collections;
list prepares all exports concurrently. Descriptions are shared across
concurrent snapshots and retained through local catalog changes, which only
revalidate the cached schema. Transient invocation failures have a five-second
retry backoff. Permanent describe/descriptor-codec failures remain cached until
generation replacement. Overlay validation failures are rechecked when the
local catalog changes; explicit revision invalidation refreshes descriptions.
If negotiation reports
a revision different from the query's snapshot, the app invalidates that
**observed old revision**, refreshes `describe`, and retries the query once. An
old concurrent invalidation cannot evict a newer revision or a refresh already
in progress. A second mismatch fails with `schema_changed: <collection>`.
Disabling or replacing a plugin drops the corresponding cached generation.

## Negotiating scans honestly

`ScanRequest.filters` contains AND-ed, entity-relative, canonical `Expr`
conjuncts. Literals are already bound. A parameter remains unbound only when its
name appears in `request.parameters`; its value arrives in the `scan` bindings.
The request also carries ordering, optional limit, offset, a projection hint and
an optional `fetch_hint`.

Return `ScanPlan::Accepted { plan }` or `ScanPlan::Rejected { reason }`. An
accepted plan has one `FilterSupport` entry per requested filter, in the same
order:

| Support | Plugin promise | Host action |
| --- | --- | --- |
| `Exact` | Every failing entity is excluded. | Trust this conjunct. |
| `Inexact` | Returns a superset of matching entities. | Recheck the conjunct. |
| `Unsupported` | Ignores the conjunct. | Apply the conjunct. |

`ordered_prefix` counts guaranteed leading ORDER BY terms. A pushed limit
requires a requested limit, every filter exact, and all ordering guaranteed.
Applied offset likewise requires exact filtering and full ordering. An inexact
filter followed by a plugin-side limit can discard valid rows and is forbidden.
Call `AcceptedScan::validate(request)` when implementing negotiation; the adapter
and host also enforce these rules.

For example, this helper negotiates exact id equalities and leaves every other
operation to the host:

```rust
fn negotiate_id_only(
    request: &semantic_vdb::ScanRequest,
    revision: String,
) -> Result<semantic_vdb::ScanPlan, semantic_vdb::VdbError> {
    let filters = semantic_vdb::classify_simple(&request.filters, |field, op| {
        *field == semantic_data::value::FieldPath::from_fields(["id"])
            && op == semantic_data::query::BinaryOp::Eq
    });
    let plan = semantic_vdb::AcceptedScan {
        filters,
        ordered_prefix: 0,
        limit_applied: false,
        offset_applied: false,
        estimated_rows: None,
        token: None,
        schema_revision: revision,
    };
    plan.validate(request)?;
    Ok(semantic_vdb::ScanPlan::Accepted { plan })
}
```

This promise requires `scan` to apply every supported equality, including
contradictory or repeated equalities. `classify_simple` also recognizes literal
IN lists when the callback accepts `BinaryOp::In`. `simple_eq_value` is useful
for routing, but extracting the first equality alone is insufficient to enforce
all exact conjuncts.

Projection is a hint: returning complete entities is safe. `fetch_hint` is a
hint, not a result cap. Tokens are opaque values returned by negotiation and
echoed in the accepted scan plan. Order/limit candidates are offered only on
eligible single-input query paths; joins and aggregates keep those operations
on the host. A source that rejects an unbounded scan can reject a joined query;
do not assume the host always supplies an id filter.

For an inner equality join, the host can offer a canonical `key IN :__keys`
filter with `parameters = ["__keys"]`. Its scan bindings contain a list of
concrete, distinct, non-null keys. `Exact` membership means returning every
matching entity for every supplied key; interpret the list as membership,
including lists with multiple keys. An opaque accepted-plan token must be echoed
from that negotiation's plan into its scans.

The host offers a bind lookup only when the ordinary scan is rejected or estimates
more than 10,000 rows. One exact accepted membership plan supports groups of
64 outer rows; that plan and its token are reused for every key list. A rejected
or non-exact membership offer retains an accepted ordinary scan; malformed
plans and revision changes still fail. Eligible leaves report `batch_size = 64`
in explain; ordinary leaves leave that field absent or null. Embedded and outer joins retain their existing bulk
lookup path. A group can have large inner fanout; the bound applies to outer
rows and their keys, not total query memory.

## Minimal Rust plugin

This plugin emits one untyped entity and uses default negotiation, which marks
all filters unsupported and leaves ordering/pagination to the host:

```rust
use async_trait::async_trait;
use semantic_data::{Object, Value};
use semantic_db_core::catalog::Catalog;
use semantic_plugin::{Plugin, PluginManifest};
use semantic_vdb::{
    AcceptedScan, CancellationToken, DatabaseDescriptor, DatabaseSchema,
    EntityStream, ScanRequest, VdbError, VirtualDatabase, VirtualDatabasePlugin,
    implementation_descriptor,
};

#[derive(Clone)]
struct GreetingVdb;

#[async_trait]
impl VirtualDatabase for GreetingVdb {
    async fn describe(&self) -> Result<DatabaseDescriptor, VdbError> {
        Ok(DatabaseDescriptor {
            title: "Greetings".into(),
            description: None,
            schema: DatabaseSchema::default(),
            schema_revision: self.schema_revision(),
            allow_untyped: true,
        })
    }

    fn schema_revision(&self) -> String { "1".into() }

    fn scan(
        &self,
        _request: ScanRequest,
        _plan: AcceptedScan,
        _bindings: Object,
        cancellation: CancellationToken,
    ) -> EntityStream {
        Box::pin(async_stream::try_stream! {
            if cancellation.is_cancelled() {
                Err(VdbError { code: "cancelled".into(), message: "scan cancelled".into() })?;
            }
            let mut entity = Object::new();
            entity.insert("id", Value::String("greeting".into()));
            entity.insert("message", Value::String("Hello".into()));
            yield entity;
        })
    }
}

fn greeting_plugin(catalog: &Catalog) -> Result<impl Plugin, VdbError> {
    let manifest = PluginManifest {
        id: "example.greetings".into(),
        revision: "1".into(),
        title: "Greetings".into(),
        exports: vec![implementation_descriptor(catalog, "database")?],
        configuration_schema: None,
        source_bindings: Default::default(),
    };
    Ok(VirtualDatabasePlugin::new(manifest, |_context| async {
        Ok::<GreetingVdb, VdbError>(GreetingVdb)
    }))
}
```

Use `semantic_data`, `semantic_db_core`, `semantic_plugin`, `semantic_vdb`,
`async-trait` and `async-stream` dependencies for this example. Descriptor
resolution requires the `semantic.query` and `semantic.vdb` interface packages
in the catalog. A standalone constructor can register their `package()` values
in a temporary `Catalog`; that catalog is for interface fingerprint resolution,
not for persisting exposed VDB schema. Register the resulting Rust plugin with
the app builder's `register_plugin`, then configure its activation in the scope.

For a typed example with filesystem I/O, see
[json_dir_vdb.rs](../crates/vdb/examples/json_dir_vdb.rs). Its schema exposes
`json_dir:Document` and the required `json_dir:content` attribute. Each `.json`
file becomes one entity, its filename becomes the id, and its complete JSON
value becomes content. It supports exact id/type equalities and ordering by id,
honours limit/offset only when safe, and returns complete entities for projection
hints.

## Running the stdio example through the CLI

Build the CLI and example from the workspace root. When developing in a
worktree, set `CARGO_TARGET_DIR` to the main clone's target directory first.

```sh
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
nix develop -c cargo build -p semantic_cli -p semantic_vdb --example json_dir_vdb --bin semantic --quiet --message-format=short
mkdir -p /tmp/semantic-vdb-documents
printf '%s\n' '{"title":"Example document"}' > /tmp/semantic-vdb-documents/one.json
"$CARGO_TARGET_DIR/debug/examples/json_dir_vdb" /tmp/semantic-vdb-documents --activation fx > /tmp/fx.json
```

Start a server in another terminal, using a disposable directory for this demo:

```sh
"$CARGO_TARGET_DIR/debug/semantic" server --data-dir /tmp/semantic-vdb-server --port 8888
```

Configure the generated activation and query it:

```sh
"$CARGO_TARGET_DIR/debug/semantic" api plugin configure /tmp/fx.json
"$CARGO_TARGET_DIR/debug/semantic" api vdb list
"$CARGO_TARGET_DIR/debug/semantic" api vdb schema fx
"$CARGO_TARGET_DIR/debug/semantic" api vdb explain 'SELECT id FROM fx'
"$CARGO_TARGET_DIR/debug/semantic" api query 'SELECT * FROM fx ORDER BY id'
"$CARGO_TARGET_DIR/debug/semantic" api query "SELECT id FROM fx WHERE type = 'json_dir:Document'"
```

The default RPC endpoint is `http://127.0.0.1:8888/api/v1/rpc`. Set
`SEMANTIC_RPC_URL` for another server and `SEMANTIC_SCOPE` for another scope. The
generated activation includes the absolute example executable path, canonical
directory path, stdio arguments, export version and fingerprint. The server must
be able to execute that path and read the directory; its stdio provider needs no
Rust plugin registration. Stdout is reserved for framed protocol messages when
`--stdio` is active.

## Common mistakes

- Returning plain attribute aliases or alias-prefixed JOIN rows instead of
  canonical entities; prefix stripping is done by the host before negotiation.
- Declaring a local definition again instead of referring to its id, or relying
  on plugin configuration definitions to resolve exposed schema references.
- Changing exposed schema without changing the synchronous revision and the
  descriptor revision together.
- Applying limit/offset before residual filters or claiming more ordering than
  the scan actually guarantees.
- Treating `projection` or `fetch_hint` as permission to discard required entity
  fields or truncate a scan that promises no limit.
- Ignoring cancellation during expensive work, emitting duplicate ids, or
  implementing a raw stream without its terminal summary.
- Assuming virtual schema is installed locally, included in `all`, or usable for
  local writes. It remains runtime-only and read-only.
