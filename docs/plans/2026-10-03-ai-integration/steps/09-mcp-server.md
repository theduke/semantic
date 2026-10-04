# Step 09: MCP server (`semantic_agent_mcp`)

Wave W3. Depends on: 01, 06. Agent: opus. Runs in parallel with steps 03, 04, 05, 07 and 12.

Read: research/t3code-mcp.md (all of it; §2, §4.4-4.9, §6-7, §9-10 are essential),
research/agent-protocols.md §2.8, §3.9, §4.4 (how each protocol receives MCP servers),
plan.md K8.

## Goal

An MCP server that every agent session connects to under the server name
**`semantic`**. It enables interaction patterns the agent CLIs do not provide
uniformly:

* structured questions to the user;
* delegation to sub-agents on any provider, with completions pushed back to the parent;
* cross-thread reads and messages;
* thread metadata and links;
* **access to the Semantic database** (query, read, and gated writes; integration with
  `semantic.tasks` when installed).

This crate owns the transport, credentials, toolkits, schemas and instruction text.
It talks to the rest of the system only through the `McpHost` trait, which the
orchestrator implements in step 10.

## Structure

```
crates/agent_mcp/src/
  lib.rs
  server.rs        McpServer: binds 127.0.0.1:0 (configurable), axum router with bearer middleware -> rmcp StreamableHttpService (stateful sessions),
                   endpoint() -> url, shutdown(); 202 normalization for empty 200 responses
  token.rs         TokenRegistry: issue(scope) -> (token, McpSessionConfig), resolve(token) (sha256 lookup), touch(thread), revoke(session|thread),
                   prune(idle > 24h); injectable clock
  scope.rs         McpScope { scope_id, thread_id, workspace_id, instance_id, session_id, capabilities: CapabilitySet, access: AccessMode, interaction }
                   McpCapability { ThreadMeta, Interaction, Delegation, Threads, SemanticRead, SemanticWrite, Tasks }
  host.rs          McpHost trait (async, object-safe via BoxFuture)
  tools/
    mod.rs         ToolRouter composition per capability; list_tools filtered by scope
    thread.rs      thread_info, set_thread_title, link_resource, list_links
    interaction.rs ask_user, notify_user
    delegation.rs  delegate_task, task_status, task_cancel
    threads.rs     thread_list, thread_read, thread_send, thread_wait
    semantic.rs    semantic_catalog, semantic_query, semantic_get, semantic_search, semantic_upsert (write), semantic_delete (write)
    tasks.rs       task_list, task_get, task_update_status, task_comment (only when tasks package available)
  schema_lint.rs   test helper: walk every tool input schema (top-level object, no anyOf[object,array], no empty-struct pitfall, no recursive $ref)
  instructions.rs  instruction blocks per capability set (only describe tools that are attached)
  bridge.rs        stdio bridge: run_stdio_bridge(url, token) (reads JSON-RPC lines on stdin, POSTs to the HTTP endpoint with bearer + mcp-session-id
                   replay, writes responses/notifications to stdout); lean deps (reqwest rustls, no rmcp server)
  results.rs       helpers: text + structuredContent (object) duplication, size capping (20 KB) with `omitted` metadata, error results (is_error)
```

## McpHost trait

```rust
pub trait McpHost: Send + Sync + 'static {
    fn thread_info(&self, scope: &McpScope) -> BoxFuture<'_, Result<ThreadInfoDto, ToolFailure>>;
    fn set_title(&self, scope: &McpScope, title: String) -> BoxFuture<'_, Result<(), ToolFailure>>;
    fn add_link(&self, scope: &McpScope, link: LinkInput) -> BoxFuture<'_, Result<ThreadLinkSummary, ToolFailure>>;
    fn list_links(&self, scope: &McpScope) -> BoxFuture<'_, Result<Vec<ThreadLinkSummary>, ToolFailure>>;
    /// Opens a Questions request on the caller's thread and waits for the answer (bounded by timeout).
    fn ask_user(&self, scope: &McpScope, questions: Vec<Question>, timeout: Duration) -> BoxFuture<'_, Result<AskOutcome, ToolFailure>>;
    fn notify_user(&self, scope: &McpScope, level: NoticeLevel, message: String) -> BoxFuture<'_, Result<(), ToolFailure>>;
    fn delegate(&self, scope: &McpScope, req: DelegateRequest) -> BoxFuture<'_, Result<TaskStatusDto, ToolFailure>>;
    fn task_status(&self, scope: &McpScope, task: TaskRef, acknowledge: bool) -> BoxFuture<'_, Result<TaskStatusDto, ToolFailure>>;
    fn task_cancel(&self, scope: &McpScope, task: TaskRef, reason: Option<String>) -> BoxFuture<'_, Result<TaskStatusDto, ToolFailure>>;
    fn list_threads(&self, scope: &McpScope, q: ThreadListQuery) -> BoxFuture<'_, Result<ThreadListDto, ToolFailure>>;
    fn read_thread(&self, scope: &McpScope, q: ThreadReadQuery) -> BoxFuture<'_, Result<ThreadReadDto, ToolFailure>>;
    fn send_to_thread(&self, scope: &McpScope, req: ThreadSendRequest) -> BoxFuture<'_, Result<SendOutcome, ToolFailure>>;
    fn wait_thread(&self, scope: &McpScope, thread: ThreadId, timeout: Duration) -> BoxFuture<'_, Result<ThreadWaitDto, ToolFailure>>;
    fn semantic(&self, scope: &McpScope) -> Option<&dyn SemanticDataHost>;  // None => semantic tools not offered
}
pub trait SemanticDataHost: Send + Sync {
    fn catalog(&self, scope: &McpScope, filter: Option<String>) -> BoxFuture<'_, Result<CatalogDto, ToolFailure>>;
    fn query(&self, scope: &McpScope, sql: String, limit: u32) -> BoxFuture<'_, Result<QueryDto, ToolFailure>>;     // read-only enforced by host (parse + reject DML/DDL)
    fn get(&self, scope: &McpScope, collection: Option<String>, id: String) -> BoxFuture<'_, Result<Option<Value>, ToolFailure>>;
    fn search(&self, scope: &McpScope, text: String, class: Option<String>, limit: u32) -> BoxFuture<'_, Result<QueryDto, ToolFailure>>;
    fn upsert(&self, scope: &McpScope, req: UpsertEntityRequest) -> BoxFuture<'_, Result<EntityRefDto, ToolFailure>>;   // SemanticWrite
    fn delete(&self, scope: &McpScope, collection: Option<String>, id: String) -> BoxFuture<'_, Result<(), ToolFailure>>;
    fn tasks(&self) -> Option<&dyn TasksHost>;
}
pub struct ToolFailure { pub code: &'static str, pub message: String /* agent-actionable */ }
```

DTOs for tool inputs and outputs live in this crate with `serde` + `schemars` derives,
because rmcp requires them. They are wire types for the MCP protocol, the same
rationale as driver wire types. Map to and from domain types at the host boundary in
step 10. Do not add serde to the domain or core crates.

## Tool catalogue (v1)

Names are short; agents see `mcp__semantic__<name>`. Every mutating tool takes an
optional `client_request_id` for idempotent retries.

| Tool | Cap | Annot. | Behaviour |
|---|---|---|---|
| `thread_info` | ThreadMeta | RO | Caller thread: id, title, workspace root, cwd, worktree, provider, model, access, interaction, links. |
| `set_thread_title` | ThreadMeta | | Rename the caller thread (≤ 120 chars). |
| `link_resource` | ThreadMeta | I | Link a URL, PR URL, Semantic entity (`collection?`, `id`) or task to the thread. Shown in the UI. |
| `list_links` | ThreadMeta | RO | |
| `ask_user` | Interaction | | 1-4 structured questions (header, prompt, 2-6 options, multi_select, allow_custom). Blocks until answered or `timeout_secs` (default 600, max 3600). Returns answers or `{timed_out: true}`. Description: use for decisions that genuinely need the user, not for confirmation of routine steps. Works on every provider, including those without native question support. |
| `notify_user` | Interaction | | Posts a notice into the thread timeline (and a desktop notification if the user enabled it). Rate-limited (max 1 per 30 s). |
| `delegate_task` | Delegation | D OW | Spawns a child thread (relationship delegated) on a chosen or inherited provider and model, with only the given prompt. Mode `async` (default) or `wait` (bounded, 10 min default, max 60 min). The child's access/interaction mode can never exceed the caller's (escalation check here and in the host). Result: task id, child thread id, status, summary when finished. Description: async completions wake this thread automatically; end your turn instead of polling. |
| `task_status` | Delegation | | Status and result. Reading a terminal result acknowledges delivery. |
| `task_cancel` | Delegation | D | |
| `thread_list` | Threads | RO | Threads in the caller's workspace only (filter by status, title; cursor paging). |
| `thread_read` | Threads | RO | Messages and final answers of another thread in the same workspace, from the composed transcript (provider history plus live, K12). Paged by history cursor; long items via char offset; returns `history_unavailable` when the provider has none. |
| `thread_send` | Threads | D OW | Send a message to a thread in the same workspace (mode auto/queue/steer); the target's modes are not escalated. |
| `thread_wait` | Threads | RO | Wait for a thread to become idle (bounded). |
| `semantic_catalog` | SemanticRead | RO | List classes and attributes (optionally filtered) so the agent can write queries. |
| `semantic_query` | SemanticRead | RO | Read-only SQL (`SELECT … FORMAT PLAIN`), `limit` ≤ 200, result capped at 20 KB with `omitted`. |
| `semantic_get` | SemanticRead | RO | Entity by id. |
| `semantic_search` | SemanticRead | RO | Full-text search over the full-text indexes. |
| `semantic_upsert` | SemanticWrite | D | Create or update an entity of a registered class; validated by the catalog. |
| `semantic_delete` | SemanticWrite | D | |
| `task_list`/`task_get`/`task_update_status`/`task_comment` | Tasks | mixed | Only listed when `semantic.tasks` is installed in the scope. |

Default capability set per thread: `ThreadMeta, Interaction, Delegation, Threads,
SemanticRead`. `SemanticWrite` and `Tasks` are opt-in per workspace or thread (UI in
step 14). `Supervised` threads never get `SemanticWrite`.

## Credentials and transport (normative)

* `TokenRegistry::issue(scope)` mints 32 random bytes (base64url) and stores only the
  SHA-256 mapped to `McpScope` plus `last_alive`. It returns `McpSessionConfig { url,
  bearer_token: Secret, capabilities }`.
* Liveness: refreshed by MCP traffic and `touch(thread)`, which the orchestrator
  calls per turn. Pruned after 24 h idle. Revoked eagerly on session close.
* Tokens are reused across re-attach of the same `(thread, instance)` while valid and
  while capabilities are unchanged (long-lived Codex processes cache them).
* The axum middleware validates `Authorization: Bearer`. Missing or invalid gives
  `401 {"error":"invalid_mcp_credential"}` plus a warn log (without the token). It
  inserts `McpScope` into request extensions. Tool handlers read the scope from rmcp's
  request context (`http::request::Parts` extensions). Verify the accessor in the pinned
  rmcp version. Fallback: the per-session handler factory binds the scope.
* Identity is never taken from tool arguments.
* `list_tools` is filtered by scope capabilities, and `call_tool` re-checks them. A
  missing capability returns an `is_error` result with an actionable message.
* Bind address: `127.0.0.1` with port 0 (random) by default. The endpoint is
  announced as `http://127.0.0.1:<port>/mcp`.
* Stdio bridge for ACP agents: `run_stdio_bridge()` is called by host binaries when
  started as `<exe> agent-mcp-bridge` (step 11 wires the CLI/UI binaries). URL and
  token come from env (`SEMANTIC_MCP_URL`, `SEMANTIC_MCP_TOKEN`), never argv.
  `McpServer::stdio_bridge_spec(exe_path)` builds the `CommandSpec` that drivers
  receive in `McpServerSpec.stdio_bridge`.

## Instructions (`instructions.rs`)

`fn instruction_blocks(caps: &CapabilitySet, ctx: &InstructionContext) -> Vec<InstructionBlock>`
returns keyed blocks:

* `semantic_mcp_overview`: the server name, that tools may appear prefixed, and that
  lazily attached servers deserve one bounded direct attempt;
* `semantic_delegation`: when to use `delegate_task` vs native subagents; end the
  turn instead of polling; a new `delegate_task` per review round with a distinct
  `client_request_id`;
* `semantic_interaction`: use `ask_user` for genuine decisions;
* `semantic_data`: what the Semantic DB is, and to use `semantic_catalog` before
  querying.

Only blocks for attached capabilities are emitted. The text is concise; detailed
guidance lives in tool descriptions.

## Tests

* Schema lint over every tool (`schema_lint.rs`).
* An end-to-end test with an rmcp client (dev-dep) over real HTTP against a
  `FakeHost`:
  * initialize, then list_tools, filtered by capability;
  * call each tool family;
  * missing capability gives an `is_error` result;
  * invalid token gives 401;
  * a revoked token gives 401;
  * session DELETE.
* Token registry: issue, resolve, touch, prune with a test clock, revoke by thread,
  reuse rules.
* Result capping (`omitted` list), structured content always an object, image-free
  for now.
* Stdio bridge: spawn the bridge in-process against a test server, run an
  initialize/list/call round trip over pipes, and check that the session id is
  replayed.
* Escalation check helper table (child access ≤ parent).

## Acceptance

* Tests green. No dependency on the orchestrator crate.
* One commit: "Add semantic MCP server with scoped credentials and toolkits".
