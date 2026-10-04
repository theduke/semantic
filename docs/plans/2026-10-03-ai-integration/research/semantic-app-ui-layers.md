# Research: app, RPC, server, CLI and UI layers for a coding-agent subsystem

Date: 2026-10-03. Scope: how to plug a new long-running subsystem (agent
orchestration: run Claude Code / Codex CLIs as child processes, sessions /
threads / turns, event streaming, approvals, diffs) into the Semantic workspace,
and how to build its UI. Read-only research; no code was modified. Files under
`crates/dxgraph`, `crates/ui` and `crates/ui_core` have uncommitted edits in the
working tree; where a file is dirty (notably `crates/ui/src/components/shell.rs`)
I read the `HEAD` version.

Conventions used below: paths are relative to the repo root
(`/home/theduke/dev/github.com/theduke/semantic`). "Gap" marks something the
agent feature needs that does not exist yet.

---------------------------------------------------------------------------

## 0. Executive summary

1. **Layering to follow** (matches `docs/ARCHITECTURE.md`, and the memory notes
   "data-model-first layering" and "qualified attribute keys"):
   `crates/data` (types, derives, package/migration) -> new runtime crate
   (process management, no app dependency, like `crates/jobs`) -> `crates/app`
   (scope-bound adapters + RPC commands) -> `crates/rpc*` (consumes) ->
   `crates/server`/`crates/cli` (transport only) -> `crates/ui_core` (data
   access/hooks/reusable components) -> `crates/ui` (routes/pages).
2. **First-class crate wired into the app, not a plugin.** The plugin system
   (`crates/plugin`) is a per-scope, interface-contract mechanism whose only
   defined contracts are `semantic.import/v1` (and the proposed VDB). Its
   stdio provider speaks the Semantic interface protocol (Content-Length framed
   `InterfaceMessage` JSON), not the CLIs' own JSON-lines protocols. Agent
   *adapters* (Claude Code, Codex) are Rust code in the new crate behind an
   internal trait; exposing that trait later as a plugin interface is possible
   but not needed. See section 4.
3. **Streaming exists and is the right transport for live events**: typed
   server-streaming commands (`RpcStreamCommand`, `StreamOf<T, End>`) are
   served over an interface WebSocket (`/api/v1/interface/ws`) with credit-based
   backpressure, and `RpcClient::invoke_stream::<C>()` is the client entry
   point. **But it has never been used by the UI or by any production
   command**: only tests in `crates/server/src/lib.rs` and `crates/app/src/lib.rs`
   register stream commands. There is also **no reconnect** in the interface
   client (section 2.5).
4. **The DB change feed is not exposed** through `SemanticDb` / app / RPC
   (`db_core::change_feed` exists, `Backend::subscribe_changes` exists, the app
   trait does not surface it). Every existing live-ish UI polls (jobs page: 2 s
   loop). The agent subsystem should therefore own an in-memory event bus
   (broadcast) + DB persistence, and expose a streaming command that replays
   from a sequence cursor then tails.
5. **App has no generic extension slot** for services, startup hooks or
   shutdown hooks. `SemanticApp::shutdown()` only shuts down per-scope jobs and
   plugin runtimes. Two places build an app (`semantic_app::storage::open_app`
   and `crates/ui/src/backend.rs` for standalone desktop), and the standalone
   desktop UI never calls `app.shutdown()`. A new subsystem must add (a) a
   builder method + field in `SemanticAppInner`, (b) a shutdown step, (c) wiring
   in both construction sites. These are small additive changes but they touch
   core app types, so confirm with the owner before doing them (AGENTS.md:
   "never aggressively change core behaviour").
6. **UI gaps for an agent chat UI**: no diff viewer, no syntax highlighting, no
   terminal component, no resizable split-pane, no chat/timeline component,
   no auto-scroll/virtualised timeline helper beyond dxcomp `VirtualList`
   (a thin wrapper of the dioxus-primitives one). Markdown *rendering* without
   JS exists (`dxeditor::markdown::parse_markdown` + `dxeditor::DocumentView`).
   Reusable pieces that do exist: `dxcomp` (Dialog, AlertDialog, Sheet, Tabs,
   Collapsible, Textarea, Select/Combobox, Popover, Tooltip, ScrollArea,
   Skeleton, Progress, Badge, Item*, Sidebar*, Toast), `ui_core` `ToastProvider`,
   `InlineNotice`, `LoadingView`, `EmptyState`, `AsyncState`, and
   `ConfirmAction`/`ConfirmDangerDialog` in `crates/ui`.
7. **Testing**: DB-backed tests use `semantic_db_kv` / `semantic_db_redb` and
   `tokio::test`; UI tests are `VirtualDom` + `dioxus_ssr` unit tests with a
   hand-written `impl RpcClientDyn` mock; end-to-end browser checks are
   one-off Playwright `.cjs` scripts stored next to plan docs and run through
   `nix develop .#ui`.

---------------------------------------------------------------------------

## 1. `crates/app` — structure, startup, lifecycle, commands

### 1.1 Layout

`crates/app/src/lib.rs` (1552 lines, mostly tests) declares:

| Module | Role |
|---|---|
| `command.rs` (1409) | `SemanticApp`, `SemanticAppInner`, `SemanticAppBuilder`, all built-in `semantic.scope.*`, `semantic.db.*`, `semantic.file.analyze` commands and their payload types |
| `scope.rs` (1222) | `ScopeManager`: per-(principal, scope-id) DB entries, per-scope `ScopeJobs` and `AppScopePlugins`, idle retirement, shutdown ordering |
| `context.rs` | `AppRequestContext` (what every command receives) |
| `auth.rs`, `session.rs` | `Principal{id, kind: User/Service/System}`, `AppSession{id, current_scope}` |
| `db.rs` | `trait SemanticDb` (async DB facade; `impl SemanticDb for semantic_db_core::Db`), `DbProvider`, `DbOpenRequest` |
| `config.rs` | `AppConfig` (+ `from_env`) |
| `jobs/` | `DbJobStore` (JobStore over `SemanticDb`) + `semantic.jobs.*` commands |
| `plugins.rs`, `imports.rs`, `import_commands.rs`, `interface.rs` | plugin persistence, import host adapters, the `application` interface export |
| `capabilities.rs`, `command_introspection.rs` | `semantic.app.capabilities`, `semantic.command.list/get` |
| `file*.rs`, `media.rs`, `object_store.rs`, `storage/` | file service, blob stores, provider URIs (`redb:`, `logfs:`, `log:`) and `open_app` |
| `task_comments.rs`, `labels.rs` | app-side `impl TaskContext/CommentContext/LabelContext for AppRequestContext` |

Cargo features: `base` (default; pulls `semantic_base`), `plugin-stdio`,
`plugin-websocket`, `storage-redb`, `storage-logfs`.

### 1.2 Startup

The server path is `semantic_app::storage::open_app(app_config, storage)`
(`crates/app/src/storage/mod.rs:147`):

```rust
let builder = SemanticApp::builder().with_config(app_config);
#[cfg(feature = "storage-redb")]  let builder = builder.with_provider(RedbDbProvider);
#[cfg(feature = "storage-logfs")] let builder = builder.with_provider(LogFsDbProvider);
let app = builder
    .with_default_scope_request(scope_id.clone(), DbOpenRequest { uri, mode })
    .with_default_file_store(scope_id, blob_store)
    .register_builtin_commands()?
    .build()?;
app.scopes().resolve_scope(&Principal::system(), None, None).await?; // opens default scope
```

The standalone desktop UI builds its own app in
`crates/ui/src/backend.rs::build_embedded_handle_with_app_config_and_blob_store`
(default scope id `"local"`, redb only, `Principal::system()`, session
`"semantic-ui"`). **Two independent construction sites** — any new builder
call must be added to both (or factored into a shared function in `crates/app`).

`build()` (`command.rs`) does the heavy lifting: registers the import plugin and
the import job kind (`self.jobs_registry.register(ImportJobHandler::default())`),
registers `BasePackage` / `CommentsPackage` / `TasksPackage` (feature `base`,
toggled by `AppConfig.tasks_enabled/comments_enabled`), collects all package
schemas into `ScopeManager::with_packages(...)` (applied lazily to the default
DB before first use), builds the `RpcRegistry` -> interface descriptor, and
returns `SemanticApp { inner: Arc<SemanticAppInner> }`.

```rust
pub struct SemanticAppInner {
    registry: Arc<RpcRegistry<AppRequestContext, AppError>>,
    command_descriptor: ImplementationDescriptor,
    scopes: ScopeManager, object_stores: ObjectStoreManager, file_service: FileService,
    jobs_registry: JobsRegistry, jobs_config: JobsConfig,
    plugins: PluginRegistry,
    pub(crate) import_job: RegisteredJob<ImportJobHandler>,
}
```

Builder extension points that already exist (all on `SemanticAppBuilder`):
`register_command`, `register_stream_command`, `register_package(RuntimePackage)`,
`register_plugin`, `with_jobs(registry, config)`, `with_provider`,
`with_default_scope*`, `with_default_file_store*`, `with_config`, `with_tasks`,
`with_comments`, `with_idle_ttl`. There is **no** `with_service(...)`,
`on_start`, or `on_shutdown` hook.

### 1.3 Principals, sessions, request context, scopes

```rust
#[derive(Clone)]
pub struct AppRequestContext {
    pub app: SemanticApp,
    pub principal: Principal,
    pub session: Option<Arc<AppSession>>,   // None for plain HTTP unary calls
    pub request_scope: Option<DbScopeId>,   // header/query scope, or embedded scope
}
```

- `ctx.resolve_db(Option<DbScopeId>) -> Arc<dyn SemanticDb>` resolves in this
  order: explicit payload scope, `request_scope`, session current scope,
  default scope (`resolve_scope_id` mirrors it for non-DB resources). Command
  payloads therefore carry `scope_id: Option<String>` by convention (see
  `ScopeParams`, `TaskListPayload`, `IdPayload`).
- `ctx.jobs(scope)` -> `ScopeJobs`; `ctx.app.plugins(&principal, scope)` ->
  `AppScopePlugins`; `ctx.default_file_store(scope)`.
- Principals: `NoAuthPrincipalResolver` always returns `Principal::system()`
  (`crates/server/src/auth.rs`); `HeaderPrincipalResolver` is "test/integration
  scaffolding, not an authentication scheme". `PrincipalKind::System` is
  treated as privileged (e.g. `comment_is_privileged`). There is **no permission
  or authorization system**: any caller of any command is trusted. For an agent
  subsystem that spawns processes with the user's credentials this is a
  significant property: exposing the server on a non-loopback interface
  (`SEMANTIC_INTERFACE`) would give remote code execution to anyone who can
  reach it. Default bind is `127.0.0.1:8888`. Needs a deliberate policy
  (e.g. refuse agent commands unless principal is System/loopback, or an
  explicit config flag `agents_enabled`).
- Scope ownership: `ScopeKey{owner: PrincipalId, scope_id}`; visibility
  `Principal` (owner only) or `System` (visible to all). `semantic.scope.open`
  opens a DB by URI at runtime; `semantic.scope.use` sets the *session* scope.
  The UI passes `scope_id` in every payload instead (HTTP calls have no session).

### 1.4 How to add commands

Trait pair (`crates/rpc_core/src/command.rs`):

```rust
pub trait RpcCommandSpec {
    type Payload: SemanticType + IntoValue + FromValue + Send + 'static;
    type Output:  SemanticType + IntoValue + FromValue + Send + 'static;
    type Error: Send + 'static;
    const NAME: &'static str;
    fn definition(&self) -> CommandDef { ... derived from the types ... }
}
pub trait RpcCommand<Ctx>: RpcCommandSpec + Send + Sync + 'static {
    fn call<'a>(&'a self, ctx: &'a Ctx, payload: Self::Payload)
        -> Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send + 'a>>;
}
```

Payload/output types derive `SemanticType, IntoValue, FromValue` (derive macros
from `crates/macros`, re-exported by `semantic_data::value`). The command's
wire schema is derived from those types and surfaced by
`semantic.command.list/get`. Three equivalent registration routes:

1. **Directly in `crates/app`** (built-in): implement `RpcCommand<AppRequestContext>`
   and `registry.register(MyCmd)?` inside `SemanticAppBuilder::register_builtin_commands`
   or a module-level `pub(crate) fn register(registry: &mut RpcRegistry<AppRequestContext, AppError>)`
   called from it — this is how `jobs/commands.rs`, `import_commands.rs`,
   `capabilities.rs`, `db_maintenance_commands.rs` do it. They use a local
   `macro_rules! command` (see `crates/app/src/jobs/commands.rs`) to cut
   boilerplate:

   ```rust
   command!(List, "semantic.jobs.list", ListPayload => JobListPage, |ctx, payload| {
       Ok(ctx.jobs(scope_id.map(DbScopeId::new)).await?.list(query).await?)
   });
   ```
2. **Embedder-registered**: `SemanticApp::builder().register_command(C)` /
   `register_stream_command(C)` — no changes to `semantic_app` required. The
   command struct can capture `Arc<MyService>` itself (the only way to reach
   service state today, because the context carries none).
3. **`RuntimePackage` (schema + commands together)**:
   `register_package(pkg)` where `pkg: impl RuntimePackage<AppRequestContext, AppError>`
   (`crates/rpc_core/src/package.rs`: `fn schema() -> Package`, `fn commands() ->
   Vec<Box<dyn DynCommand<Ctx, E>>>`). The package schema is added to
   `ScopeManager.packages` and applied **lazily to the default DB** before first
   use (explicitly opened scopes keep existing schema initialisation).
   `semantic_base::TasksPackage` is the reference: the domain crate is generic
   over a `Ctx` trait (`TaskContext { type Store: LabelStore; async fn task_store(scope) }`),
   commands are declared via a macro in `crates/base/src/tasks/commands.rs`
   (`ListTasks`, `CreateTask`, ... are `pub` so the UI can call
   `rpc.invoke::<ListTasks>(payload)` with full typing), and `crates/app/src/task_comments.rs`
   implements the ctx trait for `AppRequestContext` using `resolve_db`.

Command names: `semantic.<area>.<verb>` (`semantic.jobs.list`,
`semantic.tasks.create`, `semantic.db.query`). Errors: `AppError` (thiserror enum
in `crates/app/src/error.rs`, `http_status()`, `From<AppError> for RpcError`);
domain crates use `RpcError::new(code, message)` / `AppError::Rpc(RpcError)`.

Feature gating for the UI: `semantic.app.capabilities` (`capabilities.rs`) returns
`{tasks, comments}` booleans computed as "command registered AND class present in
the scope catalog"; `crates/ui/src/views/tasks/mod.rs::use_tasks_available()` calls
it and hides the nav item. A new `agents` capability needs either a new field in
that `Output` struct (additive) or its own availability command.

### 1.5 Background services / long-running tasks

What the app hosts today:

| Mechanism | Where | Notes |
|---|---|---|
| Per-scope jobs coordinator (`ScopeJobs`) | `ScopeManager.state.jobs: BTreeMap<ScopeKey, Option<ScopeJobs>>`, created on demand by `resolve_jobs` | one `tokio::spawn(coordinator.run())` per scope; FIFO, `max_concurrent_jobs` (default 4) |
| Per-scope plugin runtime (`AppScopePlugins`) | `ScopeState.plugins` | owns plugin generations; `plugin_cancellation: CancellationToken` per scope entry |
| Ad-hoc `tokio::spawn` | `plugins.rs`, `imports.rs`, `command.rs:84` | used to make lifecycle steps cancel-safe against dropped RPC callers |

There are **no** periodic timers (`retire_idle_scopes` exists but nothing calls
it outside tests), no global service registry, no supervisor.

Lifecycle (`crates/app/src/scope.rs`, `command.rs`):

- `SemanticApp::shutdown()` -> `ScopeManager::shutdown_jobs()`: sets
  `jobs_closing = true` (new `resolve_jobs`/`resolve_plugins` fail with
  `JobsError::Closed`), snapshots running coordinators, awaits
  `ScopeJobs::shutdown()` for each in parallel, then also shuts down plugin
  runtimes (`plugins.runtime.shutdown()`), using `jobs_lifecycle` mutex to
  serialise against concurrent initialisations.
- `close_scope_with_jobs(principal, scope)` shuts down that scope's jobs/plugins
  before dropping the DB; `close_scope` refuses if a coordinator is attached;
  `retire_idle_scopes` skips scopes with jobs. "Attached coordinators pin their
  DB until explicit asynchronous close."
- Server: `SemanticServer::serve`/`serve_with_shutdown(listener, signal)` call
  `app.shutdown().await` after axum stops (`crates/server/src/router.rs:96-117`).
  CLI `semantic server` passes `tokio::signal::ctrl_c()`. CLI `db` subcommands
  call `context.app.shutdown()` too.
- Standalone desktop UI **never calls `shutdown()`** (grep: only server/cli
  call it). Child processes spawned by a new subsystem would be orphaned or
  killed uncleanly when the window closes unless the subsystem registers its
  own cleanup (`kill_on_drop(true)` plus a drop guard, or the UI main adds an
  explicit shutdown on window close/`Event::Exit`).

### 1.6 `crates/jobs` as the subsystem exemplar

How the jobs subsystem is split (this is the template to copy):

1. **Data model in `crates/data`** (`crates/data/src/jobs.rs`, 575 lines):
   `PACKAGE_NAME = "semantic.jobs"`, `COLLECTION = "semantic_jobs"`,
   `CLASS_ID = "semantic:jobs:job"`, the entity struct with
   `#[derive(facet::Facet, Class, ...)] #[semantic(id = "semantic:jobs:job")]
   pub struct JobRecord` where each field is
   `#[facet(rename = "semantic:jobs:job:<field>")]` (class instances are keyed by
   *qualified* attribute ids — records/RPC payload structs like `JobListQuery`
   derive `SemanticType, IntoValue, FromValue` and use plain names), command
   payload/output types, and `pub fn package() -> Package` building attributes,
   class and a first migration (`MigrationDdlOperation::UpsertAttribute /
   UpsertClass / UpsertCollection / UpsertIndex`). Per AGENTS.md: all core/base
   schema changes go through new forward migrations, never edit an applied one.
   (A smaller helper `semantic_base::domain_support::package(name, area,
   attributes, classes, migration)` is used by tasks/comments/labels.)
2. **Runtime crate** `crates/jobs` (`semantic_jobs`, no dependency on app or on
   any concrete DB): trait `JobStore` (async persistence port), trait
   `JobHandler { type Input; type Output; fn kind(); fn run(input, JobContext) }`,
   `JobsRegistry`/`RegisteredJob<H>`, `ScopeJobs::open(store, registry, config)`
   (spawns the coordinator), `submit(&registration, input, SubmitOptions) ->
   JobTicket` (with `watch::Receiver<JobRecord>` progress), `cancel`, groups
   (`create_group`, `invalidate_group`), `shutdown`, `subscribe_health`.
   Cancellation is cooperative via `JobContext::cancellation()`
   (`tokio_util::sync::CancellationToken`).
3. **App adapter** `crates/app/src/jobs/`: `DbJobStore(Arc<dyn SemanticDb>)
   impl JobStore` (verifies stored package == expected package; fails closed on
   incompatibility), commands, and `ScopeManager::resolve_jobs_key` which
   lazily creates the coordinator for a scope key under `jobs_lifecycle`.
4. **Registration**: handler kinds are registered into `JobsRegistry` inside
   `SemanticAppBuilder::build()` (`import_job` is a field on
   `SemanticAppInner`); custom registries can be injected with `with_jobs`.
   **Gap**: the job kind list is closed inside `build()`; an agent job kind
   needs a builder hook (e.g. `register_job(handler) -> RegisteredJob`) or it
   must use `with_jobs(registry, ..)` before `build()`.
5. **Semantics to be aware of**: "Inputs, outputs, ... are runtime-only; restart
   interrupts"; max 4 concurrent jobs per scope shared by all kinds (agent turns
   lasting minutes/hours would starve imports and vice versa — a separate
   concurrency domain is advisable); history threshold 1000 records with
   periodic cleanup of oldest terminal jobs (so job records are not suitable as
   a durable thread/turn record). A turn *could* be a job (gives cancel +
   status + listing UI at `/data/jobs` for free), but the durable thread/turn/
   event model should be its own collection.

### 1.7 Configuration

`AppConfig` (`config.rs`): `jobs: JobsConfig`, `data_dir`, `temp_dir`,
`auto_analyze_media`, `tasks_enabled`, `comments_enabled`; `from_env()` reads
`SEMANTIC_JOBS_CONCURRENCY`, `SEMANTIC_JOBS_HISTORY_THRESHOLD`,
`SEMANTIC_DATA_DIR`, `SEMANTIC_TEMP_DIR`, `SEMANTIC_AUTO_ANALYZE_MEDIA`,
`SEMANTIC_TASKS`, `SEMANTIC_COMMENTS` (bool parser accepts 1/true/yes/on).
`SemanticAppBuilder::with_config(config)` copies the relevant fields into the
builder (note `AppConfig` is `Clone + Eq`, so new fields must be `Eq`).
Server-only config is `ServerConfig` (`crates/server/src/config.rs`:
`SEMANTIC_INTERFACE`, `SEMANTIC_PORT`, `SEMANTIC_MAX_*`; paths `/api/v1/rpc`,
`/api/v1/file`; scope header `x-semantic-scope`). CLI flags mirror these in
`crates/cli/src/cmd/server.rs` *and* in `crates/server/src/main.rs` (duplicate
arg parsing — both must be updated for new flags). A new agent config
(`agents_enabled`, `agents_max_concurrent`, CLI binary paths, default
working directory root, env allow-list) naturally lives in `AppConfig`.

---------------------------------------------------------------------------

## 2. `crates/rpc_core` and `crates/rpc`

### 2.1 Crates

- `semantic_rpc_core` (portable, wasm-safe): `RpcRequest{id, command, payload: Value}`,
  `RpcResponse{id, result: RpcResult::{Ok(Value)|Err(RpcError)}}`, `RpcError{code,
  message, data}`, `RpcClientError::{Transport,Remote(code,msg),Decode,Protocol}`,
  `CommandDef{name, input, input_stream, output}`, `RpcCommandSpec/RpcCommand/
  DynCommand/CommandAdapter/CallError`, `RuntimePackage`, and the **interface
  protocol** (`interface_protocol.rs`): `InterfaceMessage::{Hello, Configure,
  Ready, Call, Return, CancelCall, StreamDemand, StreamItem, StreamEnd,
  StreamError, StreamCancel, Shutdown, ShutdownAck}`, `PROTOCOL_VERSION = 2`,
  `MAX_WINDOW = 64`, `CODEC = "typed-value-json"`, WS subprotocol
  `semantic.interface.v1`, `interface_ws_path("/api/v1/rpc") = "/api/v1/interface/ws"`.
- `semantic_rpc` (runtime): `RpcRegistry<Ctx, E>` (unary + stream handlers,
  `register`, `register_dyn`, `register_stream`, `call`, `invoke`, `commands()`),
  `stream_command` (typed streaming commands), `interface/` (`ConformingImplementation`,
  `ExportRouter`, `session::Session`, `registry::registry_implementation` which
  serves the whole registry as the `semantic.command` export, `client::InterfaceClient`),
  `plugin/{stdio,websocket}` (provider transports), `client` (`RpcClient`,
  `RpcClientDyn`), `transport/{http_client, http_client_wasm}`, `file` (file
  upload/download types), `server/axum`.
  Features: `client`, `client-http-native` (reqwest + tokio-tungstenite),
  `client-http-web` (gloo-net, web-sys), `interface-session`, `plugin-stdio`,
  `plugin-ws`, `server-axum`.

### 2.2 Unary protocol and client API

HTTP: `POST {rpc_path}` with `RpcRequest` JSON (typed-value JSON payload), or
`POST {rpc_path}/{command}` with the typed JSON payload as body
(`crates/server/src/router.rs`; the second is what the Playwright scripts use).

Client (`crates/rpc/src/client.rs`):

```rust
#[derive(Clone)] pub struct RpcClient { inner: Arc<dyn RpcClientDyn> /* Rc on wasm */ }
impl RpcClient {
    pub async fn invoke_value(&self, command: impl Into<String>, payload: Value) -> Result<Value, RpcClientError>;
    pub async fn invoke<C: RpcCommandSpec>(&self, payload: C::Payload) -> Result<C::Output, RpcClientError>;
    pub async fn invoke_stream<C: RpcStreamCommandSpec>(&self, payload: C::Payload,
        input: <C::Input as InputShape>::Live) -> Result<<C::Output as OutputShape>::Live, RpcClientError>;
    pub async fn invoke_interface(&self, call: ValidatedInvocation) -> Result<InvocationOutput, InvocationError>;
    pub async fn upload_file(..); pub fn file_url(..); pub async fn stream_file_from(..); pub async fn read_file_range(..);
}
```

`RpcClient: PartialEq` (pointer equality, so it can be a Dioxus prop / memo key).
`RpcClientDyn` implementations: `HttpRpcClient` (native via reqwest; wasm via
gloo-net; both create an `InterfaceClient` for streaming over WebSocket),
`EmbeddedRpcClient` (`crates/ui/src/backend.rs`; calls `SemanticApp::invoke`
directly with a fixed principal/session/scope, and routes `invoke_interface` for
`export == "semantic.command"` to `semantic_app::interface::command_implementation`),
and test mocks. On wasm the futures are `LocalBoxFuture` (not `Send`).
`invoke::<C>` needs `C: RpcCommandSpec` visible to the caller, so **request/
response types for the UI should live in `crates/data`** (or the domain crate
that both app and UI depend on, like `semantic_base::tasks`) not in `crates/app`
(UI only optionally depends on `semantic_app`).

### 2.3 Streaming commands (server -> client push)

Definition (`crates/rpc/src/stream_command.rs`):

```rust
pub trait RpcStreamCommandSpec {
    type Payload: SemanticType + IntoValue + FromValue + Send + 'static;
    type Input: InputShape;    // () or StreamOf<T, End>
    type Output: OutputShape;  // Single<T> or StreamOf<T, End>
    type Error: Send + 'static;
    const NAME: &'static str;
}
pub trait RpcStreamCommand<Ctx>: RpcStreamCommandSpec + Send + Sync + 'static {
    fn call<'a>(&'a self, ctx: &'a Ctx, payload: Self::Payload,
        input: <Self::Input as InputShape>::Live, cancel: CancellationToken)
        -> BoxFuture<'a, Result<<Self::Output as OutputShape>::Live, Self::Error>>;
}
```

Server side produces `TypedStream::from_events(stream)` of
`Result<TypedEvent::{Item(T)|End(End)}, InvocationError>` (example:
`StreamCount` in `crates/server/src/lib.rs` tests). A stream must end with an
explicit `End` (end type `()` carries no value on the wire); a missing terminal
event is a failure, never a clean EOF. Client side:

```rust
let mut events = client.invoke_stream::<WatchThread>(payload, ()).await?;  // TypedStream<Item, End>
while let Some(ev) = events.next().await { match ev? { TypedEvent::Item(e) => ..., TypedEvent::End(_) => break } }
```

Bidirectional is supported: `Input = StreamOf<UserInput>` gives the handler a
live `TypedStream` of client->server messages (e.g. prompts, approval
decisions) for the lifetime of the call. A unary client call to a streaming
command fails with `streaming_required`. All streaming commands also appear in
`semantic.command.list` (placeholder entry) with `input_stream` populated.

Flow control: consumer-granted credit. `Session` starts with window 1 and
doubles per grant up to `MAX_WINDOW = 64`; a producer may not emit without
credit, so a slow UI back-pressures the producer's `poll_next`. **Therefore the
agent event stream handler must not let the agent process block on a slow
websocket consumer**: decouple with an in-memory broadcast + DB persistence
(handler stream reads from a `broadcast::Receiver`/DB cursor, never the process
pipe directly). Dropping the client stream sends `StreamCancel`; dropping the
call sends `CancelCall` which cancels the `CancellationToken` passed to the
handler.

Transport: `GET /api/v1/interface/ws` (subprotocol `semantic.interface.v1`
required), handler in `crates/server/src/interface.rs`: resolves the principal
via `resolver.resolve_ws(headers)`, creates `AppSession("interface-N")`,
sets the session scope from `x-semantic-scope` header / `?scope=` query, builds
`ExportRouter{ "application" export (import Application interface),
"semantic.command" export (the full command registry) }` over **one websocket
per client**, text frames of JSON `InterfaceMessage`. Handshake exports use
`ExportMatch::RequiredSubset`. The websocket carries no explicit heartbeat
(Ping/Pong are ignored); proxies with idle timeouts could close it.

Embedded (standalone) mode needs no socket: `EmbeddedRpcClient::invoke_interface`
creates the `command_implementation` for the call and invokes it in-process,
returning the same `InvocationOutput::Stream`.

### 2.4 How the UI gets live updates today

- **Polling**: `crates/ui/src/views/jobs.rs` runs
  `use_resource(loop { rpc.invoke_value("semantic.jobs.list", ..); sleep(2s) })`.
  Tasks/comments reload by bumping a `revision` signal after mutations.
- **Local progress channels**: file upload progress via
  `FileUploadProgressSender` (mpsc inside the client).
- **Not used**: `invoke_stream` (no UI call site), DB change feed (not exposed),
  `JobTicket::subscribe` (in-process only).

### 2.5 Streaming gaps and risks (relevant to chat/event streaming)

1. No production streaming command exists; only test fixtures. Expect to shake
   out edge cases (the wasm `gloo-net` WebSocket path in
   `crates/rpc/src/interface/client.rs` is implemented but, as far as I could
   tell, never exercised by a UI).
2. **No reconnect**: `InterfaceClient::invoke` caches one `Session` in an
   `Arc<Mutex<Option<Session>>>` and only connects when it is `None`; a closed
   session is never replaced (comment in source: "A closed cached session fails.
   A fresh client is an explicit new connection; operations are never replayed or
   silently reconnected."). After a server restart/network drop all
   `invoke_stream` calls on that `RpcClient` fail until the client is rebuilt.
   The UI needs a resubscribe-with-cursor strategy plus either client-side
   reconnect support in `InterfaceClient` or a way to reset the cached session
   (`HttpRpcClient` clone shares it). Changing `InterfaceClient` is a core
   behaviour change: ask first.
3. Scope for streaming sessions comes from the websocket URL query/header at
   connect time (`HttpRpcClient` was built with a base RPC URL without scope), so
   streaming commands should keep taking `scope_id: Option<String>` in the
   payload like unary ones.
4. Only one top-level input stream and one output stream per call; no nested
   streams (profile `values-and-top-level-streams-v2`). Multiplexing "all
   threads" into one stream needs a tagged event enum.
5. Payload size: events travel as typed-value JSON text frames; large diffs/tool
   output should be chunked or stored as blobs (the File service) and referenced.
6. A unary HTTP request (`session: None`) has no session scope; streaming
   requests do. Handler code must use `ctx.resolve_db(scope)` consistently.

---------------------------------------------------------------------------

## 3. `crates/server`, embedded mode, `crates/cli`

### 3.1 Server (axum)

`SemanticServer::router()` (`crates/server/src/router.rs`) mounts:

| Route | Handler |
|---|---|
| `POST {rpc_path}` (`/api/v1/rpc`) | `rpc_http_handler` (RpcRequest/RpcResponse JSON) |
| `POST {rpc_path}/{command}` | `command_http_handler` (typed JSON body -> typed JSON) |
| `GET {rpc_path-sibling}/interface/ws` | `interface::handler` (WebSocket, streaming) |
| `POST /api/v1/file`, `GET/DELETE /api/v1/file/{id}` | file upload/download/delete (range requests) |
| fallback (feature `embed-ui`) | `ui::handler` serves the compiled web UI |

State: `ServerState{app, commands (server-level RpcRegistry<ServerState, AppError>
containing only `semantic_base::server::ConfigGet`), config, resolver}`;
`ServerState::call` first checks server-level commands then `app.call(ctx, ...)`.
Everything else is app commands — **a new subsystem needs no server code** if it
registers commands (unary and streaming) in the app registry; the router and
interface handler already expose the whole registry. Binary: `crates/server/src/main.rs`
(hand-rolled arg parsing) and `semantic server` in the CLI (clap) — both call
`open_app` then `serve_with_shutdown(listener, ctrl_c)`.
Dev proxy: root `Dioxus.toml` proxies `/api` to `http://127.0.0.1:8888/api` for
`dx serve --web`.

Auth/ACL: none (see 1.3). Request size limits: `max_rpc_request_size` 1 GiB,
upload limit 100 GiB.

### 3.2 Desktop / embedded mode

`semantic_ui` (`crates/ui/src/main.rs`) has three launch modes:

- `--rpc-url`/`SEMANTIC_RPC_URL` (default `http://127.0.0.1:8888/api/v1/rpc`):
  desktop shell over `HttpRpcClient` (native, supports streaming via WS).
- `--standalone` (feature `standalone`, implies `desktop`): `build_embedded_handle_*`
  creates a `SemanticApp` in-process with redb and fs blob store; `EmbeddedRpcClient`
  calls the app directly (no server, no sockets). Files are served through a
  `semantic-file://` wry custom protocol. The agent runtime would run **inside the
  UI process** in this mode (it can spawn CLIs directly).
- web (`wasm32`, `launch_web()`): `HttpRpcClient::new("/api/v1/rpc")` relative to
  the document; the agent runtime must run on the server.

Consequence: the same command surface must work for both an in-process app and a
remote server; anything that needs raw local access (open a folder picker, show a
file in the OS) must be an RPC command executed server-side or a desktop-only UI
affordance behind `cfg(feature = "desktop")`.

### 3.3 CLI

`crates/cli` (`semantic` binary). Structure:

```
cmd/mod.rs         Args { command: SubCmd }  ->  Api | Db | Fuse | Server   (clap derive)
cmd/api/mod.rs     api::SubCmd: Import, Plugin, Jobs, Query, ParseSql, Get, Delete, Catalog,
                   Package, File, Apply, Upload      (each a file with `Args` + `run`)
cmd/shared.rs      ApiClientArgs{--rpc-url (SEMANTIC_RPC_URL), --scope (SEMANTIC_SCOPE)},
                   OutputArgs{--pretty}, CollectionArgs, FileInputArgs
cmd/server.rs      `semantic server` -> open_app + serve_with_shutdown
cmd/db/, fuse.rs   direct local DB access / FUSE mount
lib.rs             CliError (thiserror) + run()
```

Template for a thin RPC wrapper: `crates/cli/src/cmd/api/jobs.rs` (≈70 lines):
a `clap::Subcommand` enum whose variants fill an `Object` payload, call
`args.client.insert_scope(&mut payload)`, then
`args.client.rpc_client().invoke_value(command, Value::Object(payload)).await?` and
`args.output.print(&result)`. Adding `semantic api agent <list|get|start|send|
stop|events>` = one new file + `SubCmd` variant + `match` arm in `api/mod.rs`
(+ add the name to the help-assertion test in `cmd/mod.rs` tests). A
`--follow` subcommand would use `rpc_client().invoke_stream::<WatchThread>()` and
print events as JSON lines. Daemon-style start of the agent runtime is simply
`semantic server` (no separate process).

---------------------------------------------------------------------------

## 4. `crates/plugin` — can the agent subsystem be a plugin?

Read `docs/plans/2026-09-09-plugin-system/design.md` (364 lines) and
`crates/plugin/src/lib.rs`.

What the plugin system is:

- Plugins implement **interfaces declared in packages** (`semantic.import/v1`
  `Source/Fetcher/Importer`; `docs/plans/2026-10-03-plugin-dbs/plan.md` proposes
  a VDB interface). Providers: **Rust** (`trait Plugin { manifest(); create(ctx)
  -> Arc<dyn InterfaceImplementation> }`, registered with
  `SemanticAppBuilder::register_plugin`), **host stdio** (`tokio::process`
  with Content-Length framed `InterfaceMessage` JSON on stdin/stdout — a
  *Semantic-protocol* child, not an arbitrary CLI) and **host WebSocket**.
- State is **per scope**: installation/activation/configuration entities in the
  scope DB (`semantic.plugin` collection), generations, health, jobs group per
  generation, bindings (`ScopePlugins::activate/bindings/shutdown`). Plugins
  are "trusted code"; "no permission system", no sandbox.
- Calls are request-scoped: `PluginBinding::invoke(call)` with values plus at
  most one input stream and one output stream. A plugin has **no way to call
  back into the host** (no DB/command access; "External plugins need no broad
  application credentials or DB callbacks"). The host does validation and
  persistence from the streamed proposals.

Fit analysis for an agent subsystem:

| Need | Plugin system | First-class crate |
|---|---|---|
| Spawn Claude Code/Codex CLIs, parse their own JSONL/ACP protocols | stdio provider only speaks Semantic `InterfaceMessage`; an adapter would have to be a Rust plugin anyway | native |
| Persist threads/turns/events, own their schema | plugin has no DB access; host writes on its behalf | direct via `SemanticDb`/ports |
| Per-thread long-lived bidirectional session (prompts, approvals) | possible with one input + one output stream per call, but the plugin generation is torn down on any plugin/package change ("cancel affected jobs/streams"), unsuitable for hours-long threads | owns lifecycle |
| Commands/UI contract | none defined for agents | define freely |
| Third-party extensibility (add new agent backends) | good future fit: `AgentProvider` interface with `run(config, input: Stream<ClientMsg>) -> Stream<AgentEvent>` | internal trait first |

**Recommendation**: build `crates/agents` (name TBD) as a first-class runtime
crate wired into `crates/app` exactly like `crates/jobs` + `crates/import`
(trait ports for storage; adapters in app). Keep provider adapters behind an
internal `trait AgentProvider` so a plugin-backed provider can be added later
without touching the orchestration core. Reuse `crates/jobs` only if the owner
wants turns in the generic jobs list (see 1.6 caveats); otherwise own the
task handles in the subsystem and register a `Drop`/shutdown that kills child
processes.

---------------------------------------------------------------------------

## 5. UI layers

### 5.1 Crates and targets

| Crate | Role | Notes |
|---|---|---|
| `crates/dxcomp` | prestyled primitives ported from `DioxusLabs/dioxus-components` (git dep `dioxus-primitives` pinned rev) + one stylesheet `assets/dxcomp.css` (5880 lines) rendered by `dxcomp::Stylesheet {}` | features `web`, `desktop`, ... |
| `crates/dxform` | headless form state (`FormRoot`, `FieldHandle`, validation, lists) | used by `ui_core::form` |
| `crates/dxeditor` | document/markdown editor: Rust model + JS engine bridge (`web/`, ProseMirror-style), `MarkdownEditor`, `PlainTextEditor`, `DocumentEditor`, `DocumentView` (pure-Rust read-only render), `parse_markdown` / `serialize_markdown` | features `markdown`, `web` |
| `crates/dxgraph` | content-agnostic graph canvas (SVG edges, HTML nodes) | dirty in working tree |
| `crates/ui_core` (`semantic_ui_core`) | reusable app-level UI: contexts (`provide_rpc_client`/`use_rpc_client`, `UiScopeContext`/`use_active_scope_id`, toasts), `UiCatalog` (schema-aware renderers, entity actions/navigation), generic components (`EntityCard`, `EntityList`, `ValueView`, `ClassView`, `DirectoryBrowser`, `EntityAutocomplete`, comments, labels, `MainContentEditor/View`), `form` (dynamic class/value forms), `graph` | features `markdown`, `web` |
| `crates/ui` (`semantic_ui`) | the application: `app.rs` (providers, `AppRoot`, launch fns), `views/` (routes), `components/` (shell, sidebar, page header, global search, action primitives), `backend.rs` (embedded app client, feature `standalone`), `navigation_guard.rs` | features `web`, `desktop`, `standalone`, `markdown` |

Targets: `dx serve --web --package semantic_ui --no-default-features --features
web`, desktop (`desktop`, wry webview), standalone desktop (`desktop,standalone`).
`Justfile build-release`: `dx build --web --release ...` then the CLI with
`embed-ui`. Dev shell `nix develop .#ui` (same as default). Web has no Tokio
runtime (`wasm32`): UI code must use `dioxus_sdk_time::sleep`, `spawn`,
futures — not `tokio::time` (the `tokio` dependency in `crates/ui` is
`cfg(not(wasm32))` only).

### 5.2 Boot, providers, routing, shell

`crates/ui/src/app.rs`: `launch_with_client*` store `AppRootProps{client,
initial_scope_id, file_api_prefix}` in a thread-local and `dioxus::launch(boot_app)`.
`AppRoot`:

```rust
provide_rpc_client(props.client);
provide_ui_scope_context(props.initial_scope_id);
use_navigation_guard_provider();
rsx! { dxcomp::Stylesheet {} dxgraph::Stylesheet {}
       document::Stylesheet { href: CORE_STYLES }   // assets/core_styles.css (7537 lines)
       UiCatalogProvider { render_settings, configure_catalog: configure_ui_catalog,
           ToastProvider { Router::<Route> {} } } }
```

`configure_ui_catalog` registers renderers, the entity href/open/link builders
and entity actions — this is the extension seam if agent entities (threads) should
get entity-page treatment.

Routes: `crates/ui/src/views/mod.rs`, one `#[derive(Routable)] enum Route` with
`#[layout(AppShell)]` (standard chrome) and `#[layout(PlayerShell)]` (immersive,
viewport-constrained). Examples: `/`, `/data/jobs`, `/import`, `/tasks`,
`/tasks/create?:parent`, `/tasks/:id`, `/graph?:root&:collection&:mode&:layout`,
`/browse?...`. State that must survive reload lives in route params/query (graph
page encodes ids with `b64:` to avoid delimiter issues; reuse that approach for
thread ids if they can contain reserved characters).

Shell (`crates/ui/src/components/shell.rs`, HEAD version):

- `AppFrame{variant: Standard|Immersive}` renders skip link, `AppFrameHeader`
  (sidebar toggle, `NewMenu` dropdown, `GlobalSearch`, mobile nav) and, for
  Standard, `aside.semantic-ui__sidebar` with `PrimaryNav` + `main#semantic-main-content`.
- `PrimaryNav` hard-codes `PrimaryNavLink{to, label, icon (dioxus_icons::lucide::*),
  active: nav_item_is_active(&route, NavItem::X)}` for Home, Browse, Tree, Graph,
  Tasks (shown only when `use_tasks_available() == Some(true)`), Labels, Player,
  Data (secondary). `nav_item_is_active` is a `matches!` over `(Route, NavItem)`
  pairs and has unit tests. Adding "Agents" = one `NavItem` variant, one
  `PrimaryNavLink`, route variants in `nav_item_is_active`, optionally a
  capability gate. `PlayerShell` (immersive `semantic-player-shell`) is the
  precedent for a full-height, non-document-scrolling layout — **a chat view
  (fixed composer, scrolling timeline) wants this variant** (or a new
  `AppFrameVariant`).
- `SidebarToggle` state is a `GlobalSignal` persisted in `localStorage` via
  `document::eval`.
- `NavigationGuardPrompt` / `use_navigation_guard_provider` blocks navigation
  when forms are dirty (`navigation_guard.rs`); useful for unsent composer drafts.

### 5.3 Existing views and the patterns they set

| View | File | Pattern |
|---|---|---|
| Tasks list/detail/create | `crates/ui/src/views/tasks/{mod,detail}.rs` | **The closest precedent for a feature area.** Route wrapper keyed by scope (`TasksPage` -> `TaskWorkspace { key: "{scope:?}", scope }` so scope switches remount state), capability probe, filter signals + `use_resource` that builds a typed payload and calls `rpc.invoke::<ListTasks>(..)`, `reload` signal for manual refresh, pagination via `limit` signal, `PageHeader`, empty/error/loading states, domain types from `semantic_base::tasks` shared with the server, CSS classes `semantic-task-*` in `core_styles.css`, browser-tested by `docs/plans/2026-09-30-task-system/*.cjs`. |
| Jobs | `views/jobs.rs` | poll loop in `use_resource`, plain `table`; no live push. |
| Import | `views/import.rs` | multi-step flow over interface/commands; progress. |
| Graph | `views/graph.rs` + `ui_core/src/graph/*` | thin route page in `ui` (URL-backed params, controls) over a reusable `EntityGraphView` in `ui_core`, async loading via a `GraphSource` trait object backed by RPC (mockable), pure model/explorer logic separately testable; CSS embedded in the component via `style { {include_str!("graph.css")} }`. |
| Browse/Collection/Entity/Query/Catalog/Tree/Labels/Upload/Record/Play | `views/*` | generic schema-driven pages using `UiCatalog` (class forms, renderers). |
| Comments | `ui_core/src/components/comments.rs` | `EntityComments{target}` -> `CommentsSession` with revision signal, `use_resource`, `CommentComposer` (uses `MainContentEditor`), `CommentTree`; embedded CSS `comments.css`. Closest thing to a "message list + composer", but it is not a streaming chat. |

Placement rule visible in the code: **routing, pages and anything depending on
`Route` live in `crates/ui`; reusable data-access, hooks and components that are
route-agnostic live in `crates/ui_core`** (it even avoids depending on the
router: entity links go through `UiCatalog::entity_navigation()` callbacks set by
the app). A new Agents area should follow it: `ui_core/src/agents/` (RPC
wrappers, event reducer / timeline state, components such as `Timeline`,
`Composer`, `ApprovalCard`, `ToolCallBlock`, `DiffView`) and
`ui/src/views/agents/` (routes `AgentsPage`, `AgentThreadPage { id }`, nav entry).

### 5.4 Data loading and reactivity conventions

- RPC client from context: `let rpc = use_rpc_client(); let scope = use_active_scope_id();`
  (`ui_core/src/context/`). `RpcClient` is `Clone + PartialEq`.
- Reads: `use_resource(move || { let rpc = rpc.clone(); let scope = ..; async move { rpc.invoke::<C>(..).await.map_err(|e| e.to_string()) } })`,
  re-run by reading signals inside the closure (including a `reload`/`revision`
  signal bumped after mutations). Render with `match &*res.read()` plus
  `res.state()` for "pending/refreshing". `AsyncState<T, E>` in
  `ui_core::components::feedback` models loading/ready/error for components.
- Writes: `spawn(async move { ... })` or `use_callback`, then bump `revision`,
  show a `Toast` (`use_toast_dispatcher()` -> `.show(Toast::error(msg))`).
- Scope: re-key page/state on `scope` (`key: "{scope:?}"`) so cached state is
  dropped when the active scope changes.
- Schema-aware catalog: `UiCatalogProvider` loads `semantic.db.catalog` through
  RPC and provides it as context (`use_ui_catalog()`); not needed for a bespoke
  agent UI.
- Streams (none yet): a `use_coroutine`/`use_future` that holds the
  `TypedStream`, reduces `TypedEvent::Item` into a `Signal<ThreadState>` and
  restarts with the last seen `seq` on error; dropping the future cancels the
  stream (`OwnedValueStream` drop -> `StreamCancel`). Keep the reducer a pure
  function in `ui_core` so it can be unit tested without Dioxus.
- Dioxus 0.7.9 (`workspace.dependencies`). `dioxus-sdk-time` for `sleep`.

### 5.5 Component inventory (what exists, what is missing)

**dxcomp** (`crates/dxcomp/src/components/*`, all wrapped with `dx-*` classes):
Accordion, AlertDialog(+Title/Description/Actions/Cancel/Action), AspectRatio,
Avatar/ImageAvatar, Badge, Button (`ButtonVariant::{Primary,Outline,Ghost,
Destructive,..}`, `ButtonSize`), Calendar/RangeCalendar, Card*, Checkbox,
Collapsible*, ColorPicker, Combobox (+Option/Empty), ContextMenu, DatePicker,
Dialog (+Close/Title/Description), DragAndDropList, DropdownMenu*, HoverCard,
Input, Item* (ItemGroup/Item/ItemMedia/ItemContent/ItemTitle/ItemDescription/
ItemActions/Header/Footer — list-row building blocks), Label, Menubar, Navbar,
Pagination, Popover*, Progress, RadioGroup, ScrollArea (7 lines, thin wrapper),
Select/SelectMulti, Separator, Sheet*, Sidebar* (full shadcn-like sidebar with
provider/mobile support; the app shell does **not** use it, it has its own),
Skeleton, Slider, Switch, Tabs*, Textarea, Toast (dxcomp) , Toggle/ToggleGroup,
Toolbar*, Tooltip*, **VirtualList** (wrapper around
`dioxus_primitives::virtual_list::VirtualList{count, buffer, estimate_size,
render_item}` — fixed/estimated heights; chat timelines with variable-height,
bottom-anchored content need more).
Theming: `dxcomp.css` uses `--dark/--light` switch variables driven by
`html[data-theme="dark|light"]` or `prefers-color-scheme`, and custom properties
`--primary-color-N`, `--secondary-color-N`, `--dx-*`. `core_styles.css` re-maps
`--dx-*` to `--semantic-color-*` tokens and defines a dark palette under
`:root[data-semantic-theme="dark"]` — **no Rust code sets that attribute** (grep:
only the CSS mentions it), so the app is effectively light-only unless the
attribute is set externally; dxcomp's own `prefers-color-scheme` handling may
therefore disagree with core_styles. Be careful if designing dark-themed
code/diff blocks.

**ui_core components**: `ToastProvider` + `Toast::{info,success,warning,error}`
with actions/timeouts, `InlineNotice` (variants + live region), `LoadingView`,
`LoadingSkeleton`, `RefreshingIndicator`, `EmptyState`, `ErrorView`,
`EntityCard/List/TableRow/Autocomplete/OpenButton/DeleteButton`, `ValueView`,
`ObjectView`, `ClassView`, `MediaView`/`MediaPlaybackView`, `ImageLightbox`,
`DirectoryBrowser`/`FileTreePicker` (a 3.6k-line file tree browser + picker —
reusable for choosing a workspace folder if the data lives in the semantic
filestore, not for OS folders), comments (`EntityComments`, `CommentComposer`),
labels editor, `MainContentEditor/View` (markdown/plain, entity-link aware),
dynamic forms (`DynamicClassForm`, `DynamicValueForm`).
**crates/ui components**: `PageHeader`, `FormPage/FormActions/UnsavedChangesPrompt`,
`ConfirmAction`, `ConfirmDangerDialog`, `IconButton`, `CopyableCode`
(copy-to-clipboard code box, 149 lines — closest thing to a code block),
`GlobalSearch`, `EntityExplorer` + `QueryEditor` + `Pagination`, `DropZone`,
`JobProgress`, `StructuredQueryBuilder`.
**dxeditor**: `MarkdownEditor` (JS-bridged contenteditable engine; heavy and
async-initialised, fine for the composer, wrong for rendering hundreds of
messages), `DocumentView{document: EditorDocument}` (static Rust render:
paragraphs, headings, lists, quotes, tables, code blocks, links, images;
`dxeditor::markdown::parse_markdown(&str) -> EditorDocument` with feature
`markdown`) — use this for assistant message bodies. `CodeBlockAttributes`
exists in `document_v2` but there is no highlighter.
**dxgraph**: pan/zoom canvas; potentially useful for agent/thread graphs, not for
chat.

**Missing for the agent UI (confirmed by grep for syntect/prism/highlight/xterm/
diff crates in all `Cargo.toml` and sources):**

1. Diff viewer (unified/split, per-file collapsible hunks, line numbers) — no
   diff crate (`similar`, `imara-diff`, `diffy`) in the workspace; also no unified
   diff *parser*. Needs a new component in `ui_core` (pure-Rust parse of unified
   diff text into hunks/lines + plain `pre`/`table` rendering) — easy to unit
   test; syntax colouring optional.
2. Syntax highlighting — none. Options: plain monospace first; later `syntect`
   (native only) or a tiny JS highlighter loaded via `document::eval`/asset
   (works in both web and desktop); wasm size matters.
3. Terminal/ANSI output component — none (no xterm.js, no ANSI parser). Minimal
   viable: ANSI-SGR parser -> `span` classes in a `pre`, append-only, capped;
   a full interactive terminal is a much larger item.
4. Chat/timeline primitives: message bubble layout, auto-scroll-to-bottom with
   "user scrolled up" detection, stream-append rendering without remounting,
   grouped tool-call blocks (Collapsible exists), thinking indicator, attachment
   chips, approval prompt cards (AlertDialog/Card exist).
5. Resizable split panes (only the player playlist has a bespoke
   `semantic-player__playlist-resizer`). Thread list + chat can be a CSS grid
   (list in a column/`Sheet` on mobile).
6. Composer: `dxcomp::Textarea` (auto-grow not provided), key handling for
   Enter/Shift+Enter, slash commands, `@file` mentions (the dxeditor suggestion
   system + `EntityAutocomplete` are possible bases), image paste (see
   `views/upload_clipboard.rs` for clipboard handling patterns).
7. Keyboard shortcuts framework — ad hoc `document::eval` listeners (see sidebar
   `Ctrl+\`), no central registry.

### 5.6 Styling approach

- One big app stylesheet `crates/ui/assets/core_styles.css` (BEM-ish
  `semantic-<block>__<elem>--<mod>` classes, design tokens
  `--semantic-color-*`, `--semantic-space-N`, `--semantic-radius-*`,
  `--semantic-font[-mono]`), loaded with `document::Stylesheet { href: asset!(..) }`.
  New area styles are appended there (task area: `semantic-task-*`), or shipped
  with a `ui_core` component as an `include_str!("x.css")` `style {}` element
  (graph, comments).
- dxcomp styles are global `.dx-*` rules from the single `dxcomp::Stylesheet`.
  Buttons: `dxcomp::Button { variant: ButtonVariant::Ghost, .. }` or raw
  `a/button` with `class: "dx-button"` and `data-style/data-size` attributes (as
  `PrimaryNavLink`).
- Accessibility conventions are strong (skip link, `aria-*`, `role="status"/"alert"`
  for loading/error, 44 px hit targets in `IconButton`); the browser scripts check
  for no horizontal overflow at 390 px. A chat timeline should use `role="log"`
  with `aria-live="polite"` and avoid re-announcing the whole list.
- Responsive breakpoints in CSS: 860/720/640/560/520/480 px (`.semantic-ui__layout`
  collapses the sidebar below 861 px).

### 5.7 Patterns a new "Agents" feature area should follow

1. Types shared by server and UI (thread/turn/event DTOs, command payloads)
   defined once in `crates/data` (or the new domain crate) with
   `SemanticType/IntoValue/FromValue` derives; commands declared as pub
   `RpcCommandSpec` structs so the UI uses `rpc.invoke::<ListThreads>(..)` /
   `rpc.invoke_stream::<WatchThread>(..)`.
2. Page wrappers keyed by scope; inner components take `scope: Option<String>`
   as a prop; thread detail keyed by `(scope, thread_id)`.
3. Capability-gated nav (`use_agents_available()` modelled on
   `use_tasks_available`).
4. Pure logic (event reducer, diff parser, ANSI parser, markdown splitting)
   in `ui_core` free functions with unit tests; thin Dioxus components over them;
   SSR tests for static rendering, mock `RpcClientDyn` for loaders.
5. URL-backed selection (`/agents/:thread_id`), `NavigationGuard` for unsent drafts.
6. Errors: toast for transient failures, `InlineNotice`/`role="alert"` for
   page-level; never `unwrap` RPC results.
7. Desktop vs web parity: nothing in the pages may assume Tokio on wasm or local
   file access; folder choice must be an RPC-validated server-side path string.

---------------------------------------------------------------------------

## 6. Testing conventions

- **`docs/testing.md`** covers DB-level suites only: seeded property /
  differential / concurrency / crash tests (`SEMANTIC_TEST_SEED`,
  `SEMANTIC_TEST_STEPS`, ...; failing tests print the seed). Run via
  `nix develop -c cargo test --quiet -p semantic_db_kv -p semantic_db_redb ...`.
  Project rule (AGENTS.md): `cargo check --quiet --message-format=short`,
  `cargo test --quiet --message-format=short`, then `cargo fmt`, all inside the
  Nix devshell.
- **`crates/db_test`**: `suite/` (backend conformance incl. `change_feed.rs`,
  `transactions.rs`, `batch_returning.rs`), `differential`, `concurrency`, `rng`.
  App-level tests use real redb in `tempfile::tempdir()`
  (`crates/app/tests/jobs.rs`: `Arc::new(Db::new(semantic_db_redb::open_backend(path, DbOpenMode::AutoCreate)))`,
  `#[tokio::test]`) or the `MockDb` implementing `SemanticDb` inside
  `crates/app/src/lib.rs` tests. `semantic_db_kv` provides an in-memory
  `Backend` for fast tests (dev-dependency of `ui`, `ui_core`, `base`).
- **App/RPC tests**: `SemanticApp::builder().with_default_scope(DbScopeId::new("default"), db)
  .register_package(..)/.register_command(..)/.register_stream_command(..).build()`
  then `app.invoke(ctx(&app, Principal::system()), request(name, payload))`
  (see `lib.rs` tests; the helper `ctx`/`request` live in that test module).
  Server tests drive the axum router with `tower::ServiceExt::oneshot` and, for
  streaming, a real listener + `HttpRpcClient::invoke_stream::<StreamCount>`
  (`crates/server/src/lib.rs` `typed_invoke_stream_covers_each_stream_shape`,
  fixture struct with `client`). Re-use that fixture style for agent streaming
  tests, with a fake `AgentProvider` that emits scripted events (no real CLI).
- **UI unit tests** (`#[test]` in the same file under `#[cfg(test)]`): build a
  `VirtualDom::new(app)` / `new_with_props`, `rebuild_in_place()`,
  `dioxus_ssr::render(&dom)` and `assert!(rendered.contains(..))`
  (`ui_core/src/graph/view.rs`, `dxgraph/tests/ssr.rs`); for loaders provide a
  hand-written `impl RpcClientDyn` capturing `(command, payload)` and answering
  through a `oneshot` (`crates/ui/src/views/labels/mod.rs` tests), call
  `dom.process_events(); dom.render_immediate_to_vec()` a few times (`flush`)
  and assert on captured payloads (scope switch remounts). `ui_core/tests/form.rs`
  shows runtime-only tests (`run_in_runtime`). Dev-deps: `dioxus-ssr`,
  `dioxus-html` (feature `serialize`).
- **Browser / desktop acceptance** (not in CI; artifacts kept beside plan docs):
  Playwright `.cjs` scripts, e.g. `docs/plans/2026-09-30-task-system/acceptance.cjs`
  and `docs/plans/2026-10-03-graph-views/browser.cjs`, using the Playwright module
  from `crates/dxeditor/web/node_modules/playwright` with system Chromium.
  Flow documented in `docs/plans/2026-09-30-task-system/browser-testing.md`:
  start an isolated server `nix develop .#ui -c bash -c 'SEMANTIC_DATA_DIR=/tmp/x
  SEMANTIC_PORT=8888 cargo run --quiet --package semantic_server'`, serve the web
  UI `dx serve --web --package semantic_ui --no-default-features --features web
  --port 8080`, seed fixtures through `POST /api/v1/rpc/<command>` with typed JSON
  (`{"object": {...}}`-style tagged values; `browser.cjs` has `rpc()/decode()`
  helpers), assert via role/label locators, check no console errors and no
  horizontal overflow at 390 px. Desktop (WebKitGTK) verification:
  `docs/plans/2026-10-03-graph-views/desktop.mjs` uses the WebKit inspector
  protocol plus `xdotool`. For agents, a fake provider binary / scripted provider
  must be selectable by config so these scripts can run deterministically.

---------------------------------------------------------------------------

## 7. Recipes

### Recipe A — new runtime crate + app wiring ("subsystem")

1. **Data model first** (`crates/data/src/agents.rs`, new `mod` in `lib.rs`):
   `PACKAGE_NAME = "semantic.agents"`, collections (e.g. `semantic_agents` for
   threads/turns, a separate high-volume `semantic_agent_events`), class ids
   `semantic:agents:thread|turn|event|approval`, entity structs with
   `#[derive(facet::Facet, Class)] #[semantic(id = "...")]` and
   `#[facet(rename = "semantic:agents:thread:title")]` style qualified field
   names, payload/output structs with `#[derive(SemanticType, IntoValue, FromValue)]`,
   `pub fn package() -> Package` with migration `001_initial` (copy
   `data/src/jobs.rs::package`). Event rows need an ordering key
   (`thread_id` + monotonically increasing `seq` u64) with an index on
   `(thread, seq)` so cursor reads are indexed range scans. Large payloads
   (full tool output, diffs) as File entities / blobs referenced by id.
   Schema changes later = new migration only.
2. **Runtime crate** `crates/agents` (`semantic_agents`): depends on
   `semantic_data` (+ tokio, tokio-util, futures, tracing, thiserror), **not** on
   `semantic_app`. Contents: `trait AgentStore` (persistence port: create/
   update thread, append events, load events after seq, record approval),
   `trait AgentProvider` (spawn/resume a session for a given config; returns
   `ProviderSession { events: BoxStream<ProviderEvent>, send(UserMessage), approve(..),
   interrupt(), shutdown() }`), concrete adapters (`ClaudeCodeProvider`,
   `CodexProvider`, `FakeProvider` for tests) built on `tokio::process::Command`
   with `kill_on_drop(true)`, line-framed stdout reader tasks, bounded channels,
   stderr capture; `Orchestrator` (thread registry, per-thread actor task owning
   the provider session, `tokio::sync::broadcast::Sender<ThreadEvent>` per
   thread, sequence allocation, persistence, cancellation via
   `CancellationToken`, `shutdown().await` that interrupts and kills all
   children and waits). Make everything generic over ports so it is testable
   with the fake store/provider (mirror `crates/jobs`' `JobStore` +
   `tests/runtime.rs`).
3. **App adapter** (`crates/app/src/agents/`): `DbAgentStore(Arc<dyn SemanticDb>)
   impl AgentStore` (copy `jobs/store.rs` compatibility checks: package
   equality, fail closed), resolution of one orchestrator per scope in
   `ScopeManager` (copy `resolve_jobs_key`: lock `jobs_lifecycle`, check
   `jobs_closing`, store `Arc<ScopeAgents>` in `ScopeState`, shut down in
   `close_scope_with_jobs` / `shutdown_jobs` / `finish_jobs_shutdown`) **or**,
   simpler, one app-level `Arc<Orchestrator>` that keys threads by
   `(scope, thread_id)` and receives a per-call `Arc<dyn AgentStore>` built from
   `ctx.resolve_db(scope)`. The simpler variant avoids touching `scope.rs`
   lifecycle code but then needs its own shutdown call from
   `SemanticApp::shutdown()`.
   Add a field to `SemanticAppInner` (+ `AppConfig` fields, `SemanticAppBuilder::
   with_agents(bool)`/`with_agent_providers(..)`), initialise in `build()`,
   extend `shutdown()` to `orchestrator.shutdown().await`, and add the package
   in `build()` next to `TasksPackage` (so schema is applied lazily to the default
   DB) — **these are edits to core app types: get owner sign-off first.** A
   non-invasive fallback for a prototype is an embedder crate that calls
   `SemanticApp::builder().register_package(..).register_command(..)
   .register_stream_command(..)` with commands capturing `Arc<Orchestrator>`,
   and calls `orchestrator.shutdown()` itself.
4. **Both construction sites**: `storage::open_app` and
   `ui/src/backend.rs::build_embedded_handle_*`. Also call shutdown on
   standalone-window exit.
5. **Capabilities**: add `agents` to `semantic.app.capabilities` (requires the
   package class in the scope catalog and the command registered) or a separate
   `semantic.agents.available`.
6. **Security gate**: config flag default **off** (`SEMANTIC_AGENTS=1`);
   workspace paths validated against an allow-list root; env passed to children
   built from an allow-list; do not log prompts at info level.

### Recipe B — commands

Unary (list/get/create/rename/archive threads, send message, interrupt, approve,
list events page, get diff, list providers/models):

```rust
// crates/data/src/agents.rs   (types shared with UI)
pub struct ListThreads; impl RpcCommandSpec for ListThreads {
    type Payload = ThreadListPayload; type Output = ThreadPage;
    type Error = RpcError; const NAME: &'static str = "semantic.agents.threads.list"; }
// crates/app/src/agents/commands.rs   (execution; macro like jobs/commands.rs)
impl RpcCommand<AppRequestContext> for ListThreads { fn call(..) { Box::pin(async move {
    let agents = ctx.agents(payload.scope_id.clone()).await?; agents.list_threads(payload).await }) } }
// registration
crate::agents::register_commands(&mut self.registry)?;   // inside register_builtin_commands
```

Streaming (live thread events):

```rust
pub struct WatchThread; impl RpcStreamCommandSpec for WatchThread {
    type Payload = WatchThreadPayload;       // { scope_id, thread_id, after_seq: Option<u64> }
    type Input = ();                         // or StreamOf<ClientMsg> for prompts/approvals
    type Output = StreamOf<ThreadEvent, ThreadWatchEnd>;
    type Error = AppError; const NAME: &'static str = "semantic.agents.threads.watch"; }
impl RpcStreamCommand<AppRequestContext> for WatchThread { fn call(..) -> BoxFuture<..> {
    // 1) subscribe to the thread broadcast BEFORE reading history (no gap)
    // 2) page persisted events with seq > after_seq from the store and emit them
    // 3) then tail the receiver, dropping duplicates (seq <= last emitted);
    //    on RecvError::Lagged re-read from the store at last seq
    // 4) End(..) when the thread is archived/deleted or `cancel` fires
    Ok(TypedStream::from_events(stream)) } }
```

Register with `registry.register_stream(WatchThread)?` (app) or
`builder.register_stream_command(..)`. Recommend send-side operations (prompt,
approval, interrupt) stay **unary commands** and the stream stays
server->client only: this keeps reconnection trivial (resubscribe with
`after_seq`), works over plain HTTP for the CLI, and avoids the one-input-stream
limitation. Event type design: one tagged enum
(`#[semantic(tag = "kind", rename_all = "snake_case")]`, see `QueryOutput` in
`command.rs` for the derive syntax) — `message_delta`, `message_complete`,
`tool_call_started/updated/completed`, `approval_requested/resolved`,
`file_change`/`diff`, `turn_started/completed/failed`, `session_status`,
`usage`. Coalesce deltas (e.g. 30-50 ms) before persisting to bound write volume;
persist final states, keep deltas in the broadcast only (or persist coalesced).

### Recipe C — server / CLI

- Server: nothing, if commands are in the app registry. Only change
  `main.rs`/`cmd/server.rs` if new CLI flags/env are needed (update **both**).
- CLI: `crates/cli/src/cmd/api/agent.rs` modelled on `jobs.rs` (clap
  `Subcommand` enum -> payload `Object` -> `invoke_value`); `--follow` via
  `rpc_client().invoke_stream::<WatchThread>(..)`, printing JSON lines; register
  in `api/mod.rs` (`mod agent;`, `SubCmd::Agent`, `run` arm) and extend the help
  assertions in `cmd/mod.rs` tests.

### Recipe D — UI feature area

1. `ui_core/src/agents/` (new module, `pub mod agents;` in `lib.rs`):
   `client.rs` (typed wrappers: `list_threads(&RpcClient, ..)`, `watch_thread`
   returning a `Stream`), `state.rs` (pure `ThreadView` + `apply(event)` reducer,
   ordered by `seq`, idempotent), `hooks.rs` (`use_thread_events(thread_id) ->
   ReadSignal<ThreadView>` using `use_future` + resubscribe with backoff using
   `dioxus_sdk_time::sleep`), components (`Timeline`, `MessageView` (uses
   `dxeditor::DocumentView` via `parse_markdown`), `ToolCallBlock` (dxcomp
   `Collapsible`), `ApprovalCard` (dxcomp `Card`/`Button`, `AlertDialog` for
   destructive approvals), `Composer` (dxcomp `Textarea` + `Button`, draft kept
   in a signal + `NavigationGuard`), `DiffView` (new), `AnsiOutput` (new)),
   `agents.css` embedded with `include_str!` (like graph/comments).
2. `ui/src/views/agents/{mod,thread}.rs`: `AgentsPage` (thread list +
   "New thread" with provider/model/workspace `dxcomp::Select`), `AgentThreadPage
   { id }`; routes added to `Route` (`#[route("/agents")]`, `#[route("/agents/:id")]`),
   `NavItem::Agents` + `PrimaryNavLink` (gated by `use_agents_available`) and
   `nav_item_is_active` arms; use an immersive layout variant for the thread
   page; add the `NewMenu` entry "New agent thread" if desired.
3. Add styles under `semantic-agent-*` in the component CSS; reuse tokens.
4. Tests: reducer unit tests; SSR tests of `Timeline` with canned events; mock
   `RpcClientDyn` loader tests (scope switch); Playwright `agents.cjs` against a
   server started with the fake provider.

### Recipe E — verification checklist (per AGENTS.md)

`nix develop -c cargo check --quiet --message-format=short`; per-crate tests
`cargo test --quiet --message-format=short -p semantic_agents -p semantic_app
-p semantic_server -p semantic_ui_core -p semantic_ui`; `cargo fmt`; for UI also
`dx build --web ...` (wasm compile catches `tokio`/`Send` mistakes) and a Playwright
pass.

---------------------------------------------------------------------------

## 8. Gaps and risks (prioritised)

| # | Gap / risk | Severity | Suggested handling |
|---|---|---|---|
| 1 | No generic app extension/lifecycle hook (service slot, shutdown hook); two app construction sites; standalone desktop never shuts down | High | Small additive builder API + shared "standard app builder" function in `crates/app`; ask owner (core change). Interim: embedder wiring + explicit shutdown in `ui` main |
| 2 | No security model: any caller can invoke any command; agent commands = remote code execution on the host. NoAuth resolver returns System | High | Default-off flag, loopback-only enforcement, path allow-list; document in config; do not weaken existing resolver semantics without approval |
| 3 | Streaming commands never used in production/UI; interface client has no reconnect; no WS heartbeat | High | Design stream as resumable (`after_seq`), UI resubscribe loop, add reconnect to `InterfaceClient` only with owner approval; integration test through `HttpRpcClient` (native) and a wasm smoke test in browser script |
| 4 | DB change feed not exposed through `SemanticDb`/RPC | Medium | Subsystem owns broadcast bus; optional follow-up: expose `subscribe_changes` as a stream command (would also fix jobs polling) |
| 5 | Jobs: 4 concurrent per scope shared; job history prunes at 1000; job kinds registered only inside `build()` | Medium | Do not make threads jobs; if turns are jobs, add a registration hook and a separate concurrency domain |
| 6 | No diff viewer / unified-diff parser / syntax highlighting / ANSI terminal / chat timeline / bottom-anchored virtual list / resizable panes | Medium (UX-visible) | Build in `ui_core` as pure, unit-tested logic + simple components first; syntax highlighting later |
| 7 | Markdown rendering for messages: `DocumentView` is static and complete enough but not streaming-optimised; `MarkdownEditor` is JS-bridged | Low/Med | Re-parse only the last growing block while streaming; finalize on `message_complete`; escape raw HTML (the codec has a `raw_html` component — verify it is rendered inertly) |
| 8 | Theme: dark mode attribute `data-semantic-theme` never set from Rust; dxcomp follows `prefers-color-scheme` | Low | Verify look in both; avoid hard-coded colours in new CSS |
| 9 | Web target = wasm: no tokio, no filesystem; folder pickers cannot be native | Low | Workspace selection via server-side validated path text field / recent list |
| 10 | CLI arg parsing duplicated (`server/src/main.rs` and `cli/src/cmd/server.rs`); `AppConfig: Eq` constraint | Low | Update both; keep new config fields `Eq` |
| 11 | Event volume: persisting every token delta as an entity is expensive (each is a DB write + index) | Medium | Coalesce, persist completed items; keep a bounded replay buffer in memory for in-flight deltas |
| 12 | Capabilities command has a fixed output struct (`tasks`, `comments`) | Low | Additive field `agents` or separate command |
| 13 | Idle scope retirement/`close_scope` semantics must account for a scope with running agents (DB must stay open) | Medium | Mirror jobs: "attached coordinators pin their DB"; refuse/await in `close_scope_with_jobs` |
| 14 | Child process hygiene on shutdown/crash: orphaned CLIs | Medium | `kill_on_drop`, process groups, on startup mark non-terminal threads/turns `interrupted` (as jobs does on reopen) |

---------------------------------------------------------------------------

## 9. Open questions for the owner

1. Permission to add a small generic service/shutdown hook to `SemanticAppBuilder`
   / `SemanticApp::shutdown()` (and a shared standard builder used by both
   `open_app` and the standalone UI), versus keeping the subsystem purely
   embedder-registered?
2. Should agent threads be allowed in non-`System` principal scopes at all, and is
   a hard "loopback only" rule for agent commands acceptable until real auth
   exists?
3. Reconnect: may `InterfaceClient` gain automatic session re-establishment (needed
   for robust live UI), or should the UI recreate the `HttpRpcClient` on stream
   failure?
4. Are turns allowed to appear in the generic Jobs list (adds a job kind and shares
   the concurrency limit), or is the agent subsystem its own concurrency domain?
5. Exposing the DB change feed as a general streaming command (benefits jobs/
   tasks/comments live updates too) — in scope for this effort or a separate task?
6. UI scope: is a lightweight custom diff/ANSI viewer acceptable for v1, with syntax
   highlighting deferred?

---------------------------------------------------------------------------

## Appendix: key file index

- App: `crates/app/src/{lib,command,context,scope,auth,session,config,capabilities,command_introspection,task_comments,labels}.rs`, `crates/app/src/jobs/{mod,commands,store}.rs`, `crates/app/src/storage/mod.rs`, `crates/app/src/interface.rs`
- Jobs: `crates/jobs/src/{lib,runtime,store}.rs`, `crates/data/src/jobs.rs`, `crates/app/tests/jobs.rs`, `docs/plans/2026-09-10-jobs-system/design.md`
- RPC: `crates/rpc_core/src/{command,protocol,error,package,interface,interface_protocol}.rs`, `crates/rpc/src/{client,registry,stream_command}.rs`, `crates/rpc/src/interface/{mod,client,registry,session}.rs`, `crates/rpc/src/transport/{http_client,http_client_wasm}.rs`
- Server: `crates/server/src/{router,interface,commands,auth,config,main}.rs`, stream test fixtures in `crates/server/src/lib.rs` (`StreamCount`, `StreamEcho`, `StreamSum`)
- CLI: `crates/cli/src/cmd/{mod,shared,server}.rs`, `crates/cli/src/cmd/api/{mod,jobs}.rs`
- Plugin: `crates/plugin/src/lib.rs`, `crates/rpc/src/plugin/{mod,stdio,websocket}.rs`, `docs/plans/2026-09-09-plugin-system/design.md`, `docs/plans/2026-10-03-plugin-dbs/plan.md`
- Domain-package exemplar: `crates/base/src/tasks/{mod,commands,service,model,migration_v1,schema}.rs`, `crates/base/src/domain_support.rs`
- UI: `crates/ui/src/{app,main,backend,navigation_guard}.rs`, `crates/ui/src/views/{mod,tasks/mod,jobs,graph}.rs`, `crates/ui/src/components/{shell,sidebar,page_header,action_primitives,copyable_code}.rs`, `crates/ui/assets/core_styles.css`
- UI core: `crates/ui_core/src/{lib,context/*,graph/*,components/{comments,main_content,toast,feedback,loading,empty,notice}.rs,ui_catalog/*}`
- Components: `crates/dxcomp/src/components/*`, `crates/dxcomp/assets/dxcomp.css`, `crates/dxeditor/src/{component,markdown,document_v2}.rs`
- Tests/docs: `docs/testing.md`, `docs/plans/2026-09-30-task-system/{browser-testing.md,acceptance.cjs}`, `docs/plans/2026-10-03-graph-views/{browser.cjs,desktop.mjs}`
