# Step 11: App, server and CLI integration

Wave W5. Depends on: 10. Requires owner confirmation of plan decisions 1 and 2
(core app seams, interface client reconnect). Agent: opus. Runs in parallel with step 14.

Read: research/semantic-app-ui-layers.md §1, §2, §3, §7 (Recipes A-C, E), §8, §9;
research/semantic-data-layer.md §3.4 (package installation scoping), §8;
steps/06 (API), steps/07, steps/10; `crates/app/src/{command,scope,context,config,capabilities}.rs`,
`crates/app/src/jobs/*`, `crates/app/src/task_comments.rs`, `crates/ui/src/backend.rs`,
`crates/cli/src/cmd/api/jobs.rs`, `crates/server/src/main.rs`, `crates/cli/src/cmd/server.rs`.

## Goal

Make the agent subsystem available through the existing app, so that:

* every domain command from step 06 works over HTTP, websocket and the embedded
  client;
* streams work;
* lifecycle and shutdown are correct in server, CLI and standalone desktop modes;
* the security gate (K8) is enforced.

## 1. Core app seams (decision 1). Keep them minimal and generic.

* `AppService` trait in `semantic_app`:
  `fn name(&self) -> &str; fn shutdown(&self) -> BoxFuture<'_, ()>;`.
  `SemanticAppBuilder::with_service(Arc<dyn AppService>)` stores it, and
  `SemanticApp::shutdown()` awaits the services' shutdown after jobs and plugins, in
  reverse registration order.
* Scope service hook in `ScopeManager`: a registry of
  `ScopeServiceFactory` (`fn open(&self, scope: &ScopeKey, db: Arc<dyn SemanticDb>) -> BoxFuture<Result<Arc<dyn ScopeService>, AppError>>`).
  Services are resolved lazily via `ctx.scope_service::<T>(scope)` (typed lookup by
  `TypeId`/name). An attached scope service **pins the DB** against idle retirement,
  like jobs. `close_scope_with_jobs` and `shutdown_jobs` also shut down scope
  services. Do not refactor jobs and plugins onto it in this step; leave a doc note
  for that follow-up.
* A single shared builder function `semantic_app::standard_builder(config, storage)`
  used by both `storage::open_app` and `crates/ui/src/backend.rs`, so new
  registrations happen in one place. Move code; do not change behaviour.
* Standalone desktop: call `app.shutdown()` on window close or exit. Find the Dioxus
  desktop exit hook; if none, use a drop guard in `main`. Add a test or manual
  verification note.

## 2. Agent wiring (`crates/app/src/agents/`)

* Config:
  * `AppConfig { agents_enabled: bool (default false), agents: AgentsAppConfig }`,
    where `AgentsAppConfig` holds:
    * `workspace_roots: Vec<PathBuf>` (default `[$HOME]`);
    * `include_fake_driver: bool`;
    * `max_concurrent_sessions`;
    * `mcp_enabled: bool` (true).
  * Env: `SEMANTIC_AGENTS`, `SEMANTIC_AGENTS_ROOTS` (path list separator),
    `SEMANTIC_AGENTS_FAKE`, `SEMANTIC_AGENTS_MAX_SESSIONS`, `SEMANTIC_AGENTS_MCP`.
  * Keep `AppConfig: Eq`.
* `AgentDb` impl for `Arc<dyn SemanticDb>` (a thin adapter).
* `AgentsAppService`: owns `Arc<AgentRuntime>`, registered via `with_service` when
  enabled. The bridge command is `std::env::current_exe()` plus
  `["agent-mcp-bridge"]` (see §4).
* Scope service factory: creates a `ScopeOrchestrator` with a `DbAgentStore` over the
  scope DB and runs recovery on open.
* `RuntimeHooks` implementations:
  * `SemanticDataHost` over the scope's `SemanticDb`: `catalog`, `query` (parse with
    `parse_sql` and reject anything but SELECT; force a limit),
    `get`, `search` (full-text query), `upsert`/`delete` validated through the
    catalog, and `TasksHost` via `semantic_base::tasks` service functions when the
    tasks package is installed.
* Package registration: add `semantic_agent_domain::package()` to the lazily applied
  default-DB packages when agents are enabled (same mechanism as tasks). Commands
  against explicitly opened scopes without the package return `agents_disabled` with
  "install the semantic.agents package in this scope". Do not auto-migrate unrelated
  scopes.
* Capabilities: add `agents: bool` to `semantic.app.capabilities`. It is true when the
  commands are registered **and** the scope catalog contains `semantic:agents:thread`.
  Additive field, default false.

## 3. Commands

* Implement every unary command from step 06 as `RpcCommand<AppRequestContext>` in
  `crates/app/src/agents/commands.rs` using a local `command!` macro (jobs pattern).
* Implement the three stream commands with `RpcStreamCommand`. They adapt
  `ScopeOrchestrator::watch_*` streams to `TypedStream`; `cancel` ends the stream.
* Security gate: a shared guard that every agent command calls first:
  * agents enabled, else `agents_disabled`;
  * principal kind `System`, else `forbidden` with the message "agent commands
    require a trusted local principal". Today `NoAuthPrincipalResolver` returns
    System, so local usage works; any future auth must opt in explicitly.
* Errors: map `OrchestratorError` to `RpcError` codes from `semantic_agent_domain::error`.

## 4. CLI and binaries

* `semantic api agents <threads list|get|create|send|interrupt|respond|events [--follow]|providers list|refresh|workspaces list|create>`
  in `crates/cli/src/cmd/api/agents.rs`, using the typed command specs. `--follow`
  uses `invoke_stream::<WatchThread>` and prints JSON lines. Add help-test assertions.
* Hidden subcommand `semantic agent-mcp-bridge` calls `semantic_agent_mcp::run_stdio_bridge()`.
  The server binary (`crates/server/src/main.rs`) and the UI binary
  (`crates/ui/src/main.rs`, standalone feature) also handle a first argument of
  `agent-mcp-bridge`, so `current_exe()` works in every mode.
* Server flags: `--agents`, plus the env above, in **both** `crates/server/src/main.rs`
  and `crates/cli/src/cmd/server.rs` (duplicate parsing exists).

## 5. Interface client reconnect (decision 2)

In `semantic_rpc::interface::client::InterfaceClient`, when the cached session is
closed, a **new** `invoke` creates a fresh session. In-flight operations still fail
and are never replayed (keep the documented semantics). Add a test: the server
closes the websocket, the next `invoke_stream` succeeds on a new connection.

## Tests

* App-level: build an app with `with_default_scope` (redb temp dir) and agents
  enabled with the fake driver. Exercise:
  * `providers.list` (fake auto-detected);
  * `workspaces.create` in a temp dir inside allowed roots; outside roots is rejected;
  * `threads.create` with an initial message;
  * `threads.watch` over `app.invoke` streaming until the run completes;
  * `threads.get` after an app restart returns the transcript from the fake
    driver's history files, with runs aligned;
  * `requests.respond` round trip;
  * the capabilities flag;
  * the disabled flag rejects;
  * a non-System principal rejects.
* Server-level: a real listener plus `HttpRpcClient::invoke_stream::<WatchThread>`
  (reuse the `typed_invoke_stream_covers_each_stream_shape` fixture style); reconnect
  after a server-side close.
* Lifecycle: `app.shutdown()` closes sessions (the fake session observes
  `SessionClosed(HostShutdown)`) and stops the MCP server.
* CLI help tests; `agent-mcp-bridge` subcommand smoke test (bridge against a test
  MCP server).

## Acceptance

* Server, CLI and standalone builds all compile (`cargo check -p semantic_server -p semantic_cli -p semantic_ui --features standalone`).
* Tests green.
* Commits:
  1. "Add generic app service and scope service seams";
  2. "Reconnect closed interface client sessions for new calls";
  3. "Wire agent orchestrator into app, server and CLI".
