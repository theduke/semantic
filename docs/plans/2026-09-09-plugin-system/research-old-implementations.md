# Prior importer and plugin-adjacent implementations

Research baseline: `../semantic-new` at `20ee396` and `../semantic-original` at
`3056094b`, inspected 2026-09-09. “Plugin” is used carefully below: neither tree
contains a general-purpose installable plugin runtime. `semantic-new` contains a
statically linked source/importer architecture plus an executable adapter;
`semantic-original` contains the newer schema interface model and a transportable
RPC substrate, while explicitly documenting that its plugin runtime is absent.

## Executive summary

The most reusable old design is the separation between URL probing, semantic
preview/fetching, and persistence. `semantic-new` models that separation in a
`Source` trait, routes requests through a registry/manager, and demonstrates an
out-of-process adapter through the `semanticrawl` executable. It is a useful
behavioral prototype, but not a plugin system: registration is in-memory, there is
no manifest, installation, version negotiation, capability policy, durable plugin
state, lifecycle protocol, isolation, or unload/update path.

`semantic-original` does not implement importers. Its important contribution is a
typed schema boundary inspired by the WebAssembly Component Model—contracts,
interfaces, typed functions, schema imports, and handles—and a small JSON RPC
crate with HTTP and WebSocket transports. Those two pieces are not connected.
The RPC protocol is suitable as a conceptual starting point for host plugins, but
needs handshake/versioning, discovery, cancellation, deadlines, backpressure,
stdio framing, lifecycle messages, and interface-based dispatch before it is a
safe plugin protocol.

No Wasmer or WEBC dependency or integration was found in either tree. Any Wasmer
backend, registry/package installation, package persistence, and `WebcPackageFile`
model are new work rather than migrations.

## 1. `semantic-new`: source/import pipeline

### 1.1 Architecture and contracts

The core application abstraction is `Source` in
`../semantic-new/crates/app/src/source/source.rs:9`. It is an async, object-safe
Rust trait (`Send + Sync + Debug`) with:

- identity and presentation: `id()`, `schema() -> DataSource`, and
  `healthcheck()`;
- a required cheap/static eligibility check, `can_try_url()`;
- non-persisting `fetch_url() -> Option<Object>`;
- richer non-persisting `fetch_url_webview() -> Option<WebView>`;
- persisting `import_url(url, app) -> Option<Vec<Id>>`.

The `Option` convention is significant: `Ok(None)` means “unsupported or no
data,” while `Err` means an attempted handler failed. Successful import returns
only the IDs of stored objects, not a structured receipt, warnings, provenance,
partial-success detail, or blob/object accounting.

The public semantic command layer mirrors these operations as
`CmdFetchUrl`, `CmdFetchUrlWebView`, and `CmdImportUrl` in
`../semantic-new/crates/base/src/api.rs:97`, `:116`, and `:135`. Command handlers
delegate directly to the source manager in
`../semantic-new/crates/app/src/command.rs:161-178`. Source metadata is exposed
as `DataSource`/`DataSourceWithStatus` and `CmdListDataSources` in
`../semantic-new/crates/base/src/source.rs:12-60`. This makes fetching/importing
available through the same application command endpoint as other operations, but
it does not expose a portable importer interface or describe method capabilities.

There is a second, closely related trait in the standalone crawler workspace:
`../semantic-new/semanticrawl/crates/core/src/source.rs:9-29`. Its `Source`
methods are `can_handle_url`, `fetch_semantic_view`, `fetch_semantic`, and
`import`, with a `Ctx` rather than full `App`. This duplication creates an
application boundary but also contract drift (`can_try_url` vs `can_handle_url`,
different method names/context and missing health/schema methods).

### 1.2 Discovery, registration, and routing

Application sources live in `SourceManager`, an `Arc<State>` containing
`RwLock<HashMap<String, SourceState>>`
(`../semantic-new/crates/app/src/source/manager.rs:17-37`). Registration inserts
an `Arc<dyn Source>` under its ID and silently replaces any prior entry
(`:47-56`). `SourceState` has a `priority`, but registration always sets it to
zero and exposes no priority API.

For fetch, preview, and import, the manager clones the current sources, sorts by
priority, applies the static predicate, and calls candidates sequentially
(`manager.rs:193-304`). The first `Some` wins. It records errors and continues;
if no source succeeds, aggregated errors are returned, otherwise accumulated
errors are discarded when a later source succeeds. Equal priorities inherit
`HashMap` iteration order, so winner selection is nondeterministic when multiple
sources accept a URL.

The crawler has another `RwLock<HashMap>` manager
(`../semantic-new/semanticrawl/crates/core/src/manager.rs:10-112`) and a process-
global `LazyLock<Mutex<HashMap<...>>>` `SourceRegistry`
(`core/src/source.rs:30-47`). The global registry also silently replaces duplicate
IDs and returns unordered values. `Semanticrawl::new_with_default_registry`
copies that snapshot; later global registrations do not appear in existing
instances. In practice built-ins are explicitly returned by
`../semantic-new/semanticrawl/crates/semanticrawl/src/lib.rs:4-7` and installed by
CLI option construction (`semanticrawl/crates/cli/src/opts.rs:23-25`), so the
static global registry is largely redundant.

There is no durable plugin/source record, enable/disable state, dependency graph,
configuration schema, installation provenance, or per-database selection.
Registration and lifecycle are startup-only and process-local.

### 1.3 Out-of-process execution behavior

`SemanticrawlExecutorSource` is the closest predecessor to a host plugin
(`../semantic-new/crates/app/src/source/semanticrawl.rs:19-217`). Server startup
optionally finds and registers it in
`../semantic-new/crates/cli/src/cmd/server.rs:120-140`.

For every operation it launches a fresh `semanticrawl` process with
`std::process::Command::output()`; commands are `fetch ...`, `fetch --webview
...`, or `import ...` and always request JSON (`semanticrawl.rs:73-161`). Stdout
is the single response document. A nonzero exit attempts to decode a typed
`SemanticError` from stdout, falling back to stderr text. The blocking launch is
wrapped in `tokio::task::spawn_blocking` (`:196-216`). The API URL/token are
configured on the executor so the child performs imports by calling back into the
server.

This is command-per-process stdio capture, not persistent duplex stdio RPC:

- no request IDs, multiplexing, handshake, protocol/interface version, or method
  discovery;
- no incremental result/events or bidirectional host calls over the process
  channel;
- no timeout, cancellation, output-size bound, resource budget, sandbox, restart
  policy, or supervised health state;
- stderr is buffered until exit and diagnostics are only surfaced on failure;
- auth token delivery is indirect CLI configuration and should not be copied into
  a new protocol without a capability/secret model;
- `healthcheck()` searches for *any* functional default-named binary rather than
  checking the configured `self.path`, so it can report a false healthy/unhealthy
  result (`semanticrawl.rs:178-189`).

The strength is operational simplicity and process isolation from crashes. A new
stdio transport can retain process supervision/isolation, while replacing
per-call spawning with framed, versioned RPC over a long-lived child.

### 1.4 Import persistence and data flow

Crawler `Ctx` centralizes HTTP, temporary files, CAPTCHA support, options, and an
optional semantic API client (`../semantic-new/semanticrawl/crates/core/src/context.rs:13-164`).
Its blob helpers stream remote, in-memory, or local-file data to the semantic API
(`context.rs:385-443`). This is a valuable capability-boundary idea: importer
logic need not know the database/blob backend.

On the application side, `App::file_create_stream` stores content through the
blob backend, deduplicates by blob hash, extracts/merges file metadata, analyzes
media, then commits a `File` object referencing the blob
(`../semantic-new/crates/app/src/app.rs:273-396`). `File` and typed
`Image`/`Video`/`Audio` wrappers are declared in
`../semantic-new/crates/base/src/file.rs:16-378`. Replacement modes exist in the
API but are explicitly not implemented in the persistence path.

Important limitations:

- importers receive broad power: in-process sources get the whole `App`; crawler
  sources get a general `SharedClient`, HTTP client, temp paths, and optional
  CAPTCHA solver. There is no least-privilege capability object;
- object import is not a declared atomic transaction. The output IDs cannot prove
  what was created, updated, or left behind after partial failure;
- no import job, progress, resumability, idempotency key, cancellation, dry-run,
  provenance record, diagnostics collection, or cleanup contract;
- remote import is inverted: the child gets server credentials and independently
  writes via API, so the host cannot validate/stage the complete mutation set
  before commit;
- `AttrImportedAt` and `AttrCanImport` are only schema attributes
  (`../semantic-new/crates/base/src/import.rs:3-10`), not lifecycle guarantees;
- the only included crawler source, feeds, implements preview/fetch conversion but
  leaves `import()` as `TODO` returning `None`
  (`semanticrawl/crates/sources/feeds/src/lib.rs:69-130`). Thus the architecture is
  more complete than its concrete importer coverage.

### 1.5 What to preserve

- Keep cheap deterministic “can handle?” matching separate from expensive
  execution, but represent match strength/reason rather than a boolean.
- Keep preview/read and mutating import distinct, and use the same typed semantic
  definitions across Rust, host, WebSocket, and Wasm implementations.
- Preserve first-class source metadata and health reporting, but make ordering and
  conflict resolution deterministic.
- Expose host-owned blob/object capabilities to plugins instead of backend handles
  or reusable all-powerful API credentials.
- Preserve streaming blob ingestion and content-addressed deduplication.
- Treat “unsupported,” “successful empty result,” “partial success,” and
  “execution failure” as different typed outcomes.

## 2. `semantic-original`: interface model and RPC substrate

### 2.1 Import/plugin status

No importer trait, manager, command, plugin manifest, plugin registry, Wasmer, or
WEBC integration exists in this tree. `../semantic-original/docs/schema.md:378-389`
explicitly marks schema-import resolution, handles/runtime wiring, and the plugin
system as unimplemented. Its UI gap analysis also says there is no importer route
in this implementation (`docs/plans/2026-08-22-ui-feature-gaps/result.md:109-123`).
Accordingly, it should be treated as the schema/RPC foundation for the requested
new design, not as a behavioral importer implementation.

### 2.2 Interface system available for reuse

The schema is deliberately component-model-shaped:

- `InterfaceType` is a list of named `InterfaceMethod`s, each carrying a
  `FunctionType` (`crates/data/src/schema/behavior/interface_type.rs:0-4` and
  `interface_method.rs:0-5`).
- `Contract` groups named types, functions, attributes, classes, and interfaces
  into an export boundary (`crates/data/src/schema/contract/contract.rs:2-15`).
- `ContractInterface` adds name/metadata to an interface
  (`contract_interface.rs:0-6`).
- `HandleType` describes own/borrow resource handles associated with an interface
  (`crates/data/src/schema/handle/handle_type.rs:0-5`).
- `SchemaImport` names a location, optional version, namespace, and optionality,
  although resolution is absent (`crates/data/src/schema/core/schema_import.rs:0-7`).
- `ExtensionType` reserves namespaced custom payload types, also without runtime
  support (`crates/data/src/schema/core/extension_type.rs:2-8`).

This is the right conceptual layer for an importer contract: plugin kinds should
be deployment/runtime adapters implementing the same declared interfaces, not
separate importer traits with subtly different behavior. However, the schema
currently describes interfaces; it does not bind an implementation instance,
negotiate versions, invoke a method, manage handles, or validate a component.

### 2.3 RPC architecture and transport behavior

The `semantic_rpc` crate uses a minimal request/response protocol
(`../semantic-original/crates/rpc/src/protocol.rs:5-43`): a `u64` request ID,
string command name, typed semantic `Value` payload, and either typed `Value` or
structured `RpcError`. `RpcCommandSpec` associates a static name with typed
payload/output/error and a `FunctionType`; `RpcCommand` supplies the async call;
`CommandAdapter` performs typed decode/encode
(`crates/rpc/src/command.rs:10-89`). `RpcRegistry` stores commands in a
`BTreeMap`, rejects duplicate names, and dispatches unknown commands as protocol
errors (`registry.rs:6-47`). The deterministic map and duplicate rejection are
stronger than the old source registries.

The native WebSocket client maintains a pending map keyed by request ID, serializes
requests as JSON text, and resolves concurrent calls from a background reader
(`transport/ws_client_native.rs:18-138`). The server accepts JSON text or binary
frames and spawns every valid request concurrently, routing responses through a
single writer (`server/axum_ws.rs:32-94`). HTTP and browser/native WebSocket client
variants are feature-gated in `crates/rpc/src/transport/mod.rs:0-15` and crate
features in `crates/rpc/Cargo.toml`.

Reusable strengths:

- transport-independent typed command adapters around semantic `Value`;
- concurrent calls and out-of-order responses correlated by IDs;
- structured application errors separated from client protocol/transport errors;
- duplicate registration rejection and a dynamic client abstraction;
- feature-gated transports, a useful precedent for optional Wasmer integration.

Limitations relevant to plugin hosting:

- protocol has no magic/version, hello/handshake, plugin identity, interface
  inventory, implementation version, feature negotiation, or compatibility
  checks;
- command names and one payload value are flat RPC conventions, not dispatch
  through schema `ContractInterface` identities/methods;
- no notifications, streaming RPC, host callbacks, cancellation, deadlines,
  request context, trace IDs, concurrency/window negotiation, or backpressure;
- request IDs are process-global client counters and are not protected against
  peer misuse/reuse; unknown response IDs are silently ignored;
- WebSocket sends have no call timeout, and pending calls live until a response or
  socket failure; server requests are unbounded detached tasks;
- frame/message size, in-flight count, rate, auth, origin, liveness, and graceful
  shutdown are outside this crate; file upload/download is HTTP-specific rather
  than a general stream/capability protocol;
- JSON preserves semantic type tags but is costly for large data; blobs must remain
  out-of-band/streamed rather than embedded in `Value`;
- there is no stdio transport. Newline-delimited JSON would be fragile if plugin
  logging shares stdout; use explicit length framing and reserve stderr for logs;
- browser and native clients have different thread bounds, which matters if one
  uniform runtime abstraction is expected.

## 3. Behavioral incompatibilities and migration risks

1. **Trait calls versus interface calls.** `semantic-new` uses Rust `dyn Source`
   and passes `&App`/`&Ctx`; the target must use the package interface system.
   Existing sources require adapters or rewrites and cannot be ABI-loaded as-is.
2. **Duplicate source contracts.** App and crawler `Source` traits differ. Choosing
   either as canonical would preserve accidental naming/context differences. Model
   importer operations once in interfaces and generate/adapt Rust bindings.
3. **Return semantics.** `Option<Vec<Id>>` conflates unsupported/no-data and lacks
   commit/partial-success metadata. Changing it can affect CLI/UI assumptions and
   needs an explicit compatibility adapter.
4. **Selection order.** Old equal-priority routing is nondeterministic. A new stable
   selection policy may intentionally choose a different importer for overlapping
   URLs; surface and test this migration change.
5. **Mutation authority.** Old external imports call back into the full semantic
   API using a token. Moving to scoped host capabilities will break plugins that
   query or write arbitrary data, but retaining that model defeats isolation and
   transactional oversight.
6. **Value/schema generation.** `semantic-new` command schemas/macros and
   `semantic-original`'s newer `facet`-based `semantic_data` types are different
   generations. Wire compatibility must be demonstrated; matching Rust-looking
   types or JSON shapes is insufficient.
7. **RPC/interface mismatch.** RPC `RpcCommandSpec::signature()` uses a function
   signature but does not identify a contract/interface version. Flat old command
   names need a namespaced interface-method mapping and negotiation layer.
8. **Lifecycle assumptions.** The old executable is stateless per invocation; a
   persistent stdio or WebSocket plugin introduces state, crash recovery, stale
   configuration, reconnection, draining, and upgrade concerns not previously
   observable.
9. **Concurrency.** Old source selection is sequential; old WebSocket RPC dispatch
   is unconstrained concurrent. Neither policy is suitable as a universal default.
   Define per-plugin and per-interface limits and host-wide scheduling.
10. **Persistence/atomicity.** Existing imports can perform multiple remote writes
    and blob uploads without a single durable import transaction. A staged/host-
    committed model changes failure behavior and requires orphan cleanup/migration
    rules.
11. **File specialization.** Existing `File` captures ordinary blob-backed files;
    it has no immutable package digest, WEBC manifest/runtime metadata, registry
    provenance, signature/trust status, or compiled-artifact cache linkage.
    Extending it for `WebcPackageFile` must preserve generic file/blob behavior
    while adding package-specific invariants.
12. **Optional Wasmer.** Since neither predecessor has the dependency, making it a
    default dependency would increase build/runtime footprint and platform risk.
    Follow the RPC crate precedent: isolate Wasmer/WEBC behind Cargo features and a
    runtime adapter so the core interface/import system works without it.

## 4. Design constraints derived from the predecessors

- Define plugin functionality as versioned package-system interfaces; make Rust,
  stdio host, WebSocket host, and Wasmer/WEBC four implementations of one runtime
  invocation contract.
- Separate durable plugin/package records from ephemeral runtime instances and
  health. Persist enablement, configuration, provenance, digest, trust decision,
  granted capabilities, and compatible interface versions.
- Give importers scoped host capabilities (HTTP policy, blob streaming, query,
  staged mutations, secrets by opaque reference), never `&App`, raw database
  access, or a general reusable bearer token.
- Use a typed match/probe result and deterministic ranking/tie-breaking. Cache only
  explicitly cacheable probe results and include plugin/config/version in keys.
- Model imports as jobs with cancellation, deadlines, progress/events,
  idempotency, structured diagnostics, provenance, and a host-controlled commit
  boundary. Make partial success explicit.
- Keep bulk bytes outside ordinary RPC values. Support bounded streaming or opaque
  blob handles consistently across all runtimes.
- Add a common handshake carrying protocol version, plugin/package identity,
  exported/imported interfaces, implementation versions, limits, and requested
  capabilities. Reject incompatibility before activation.
- Supervise processes/connections/instances with readiness, liveness, draining,
  restart/backoff, resource/concurrency budgets, and observability. Preserve stderr
  as stdio-plugin diagnostics and use length-delimited stdout/stdin frames.
- Make every runtime optional at the adapter layer. In particular, core data types,
  interfaces, manifests, persistence, and importer orchestration must compile and
  operate when the Wasmer feature is disabled.

## 5. Concrete reusable map

| Concern | Prior location | Reuse direction |
|---|---|---|
| Import operation split | `semantic-new/crates/app/src/source/source.rs` | Preserve probe/fetch/preview/import separation as interfaces |
| Routing | `semantic-new/crates/app/src/source/manager.rs` | Replace unordered first-wins behavior with stable scored selection |
| External process adapter | `semantic-new/crates/app/src/source/semanticrawl.rs` | Preserve isolation; replace command-per-call with supervised framed RPC |
| Import execution context | `semantic-new/semanticrawl/crates/core/src/context.rs` | Narrow into explicit host capabilities |
| Blob persistence | `semantic-new/crates/app/src/app.rs:273-396` | Preserve streaming/dedup; add staging, receipts, cleanup |
| File schema | `semantic-new/crates/base/src/file.rs` | Base for package-file semantics, subject to current package model |
| Typed interfaces/contracts | `semantic-original/crates/data/src/schema/**` | Canonical functionality declaration and binding source |
| Dynamic typed RPC | `semantic-original/crates/rpc/src/{command,registry,protocol}.rs` | Extend with interface-aware handshake/lifecycle |
| WebSocket multiplexing | `semantic-original/crates/rpc/src/transport/ws_client_native.rs` | Reuse request correlation with bounds/timeouts/cancellation |
| Server concurrency | `semantic-original/crates/rpc/src/server/axum_ws.rs` | Add admission control, context, shutdown, auth/capability policy |

The central lesson is to preserve the old importer *semantics* and RPC *mechanics*,
not their concrete trait boundaries. The new package interface system should be
the source of truth; runtime-specific code should only discover, activate, invoke,
and supervise implementations of those interfaces.
