# AI integration: coding-agent orchestration for Semantic

Date: 2026-10-03. Status: plan, ready for implementation by subagents.

Goal: run existing coding-agent CLIs (Claude Code, Codex, any ACP agent such as
Gemini CLI, OpenCode, Cursor, Copilot) from Semantic. Manage them through a UI
that has a thread inbox, a chat timeline, approvals and questions, plans, diffs
and reverts. Persist orchestration metadata (workspaces, providers, threads, runs,
checkpoints, links) in the Semantic DB, and give agents a custom MCP server for
richer interaction patterns. Functionally this is similar to t3code, but built as
clean Rust layers on top of Semantic.

**Transcripts are not stored in the Semantic DB (for now).** Agent messages, items
and requests are fully modelled, streamed live and rendered, but the conversation
history comes from each agent's own session storage (Claude session JSONL files,
Codex threads, ACP `session/load`) through a driver history API (K12). Storing
transcripts in the DB is a planned later addition. The model is designed so it can
be added without reshaping anything (see "Later").

The multi-server orchestration (one UI controlling agents on several machines) is
**not** part of this plan. The data model and API are designed so it can be
added later without migrations that rewrite existing data. See "Future: multi-host"
below.

## Index

| Step | Doc | Crate(s) | Wave |
|---|---|---|---|
| 00 | [Scaffold workspace](steps/00-scaffold.md) | all new crates (skeletons), workspace deps | W0 |
| 01 | [Agent core model and traits](steps/01-agent-core.md) | `semantic_agent` | W1 |
| 02 | [Driver runtime, process supervisor, test kit](steps/02-driver-runtime.md) | `semantic_agent_drivers` | W2 |
| 03 | [Claude Code driver](steps/03-driver-claude.md) | `semantic_agent_drivers::claude` | W3 |
| 04 | [Codex driver](steps/04-driver-codex.md) | `semantic_agent_drivers::codex` | W3 |
| 05 | [ACP driver](steps/05-driver-acp.md) | `semantic_agent_drivers::acp` | W3 |
| 06 | [Domain schema and API contract](steps/06-domain-schema.md) | `semantic_agent_domain` | W2 |
| 07 | [Orchestrator core](steps/07-orchestrator-core.md) | `semantic_agent_orchestrator` | W3 |
| 08 | [Workspace and VCS (git checkpoints, worktrees)](steps/08-vcs.md) | `semantic_agent_vcs` | W2 |
| 09 | [MCP server](steps/09-mcp-server.md) | `semantic_agent_mcp` | W3 |
| 10 | [Orchestrator features: providers, VCS, MCP, delegation, handoff](steps/10-orchestrator-features.md) | `semantic_agent_orchestrator` | W4 |
| 11 | [App, server and CLI integration](steps/11-app-integration.md) | `semantic_app`, `semantic_cli`, `semantic_server` | W5 |
| 12 | [AI UI foundation](steps/12-ai-ui-foundation.md) | `semantic_ai_ui` | W3 |
| 13 | [AI UI: thread view, timeline, composer, requests](steps/13-ai-ui-thread.md) | `semantic_ai_ui` | W4 |
| 14 | [AI UI: inbox, new thread, providers, review panel](steps/14-ai-ui-workspace.md) | `semantic_ai_ui` | W5 |
| 15 | [Semantic UI integration, end-to-end tests, docs](steps/15-integration-e2e.md) | `semantic_ui`, docs | W6 |

Research inputs (read the ones referenced by your step, not all):

| File | Topic |
|---|---|
| [research/t3code-providers.md](research/t3code-providers.md) | t3code provider abstraction, canonical item/request taxonomy, capability matrix, pitfalls |
| [research/t3code-orchestration.md](research/t3code-orchestration.md) | t3code orchestration: event log, run lifecycle, recovery, checkpoints, worktrees, delegation |
| [research/t3code-mcp.md](research/t3code-mcp.md) | t3code MCP server, credentials, tool catalogue, instruction injection, rmcp |
| [research/t3code-ui.md](research/t3code-ui.md) | t3code UI features, timeline folding, composer, approvals UX, feature priorities |
| [research/agent-protocols.md](research/agent-protocols.md) | Claude stream-json control protocol, Codex app-server, ACP, Rust crates |
| [research/semantic-data-layer.md](research/semantic-data-layer.md) | how to define packages, migrations, classes, write path and live updates in Semantic |
| [research/semantic-app-ui-layers.md](research/semantic-app-ui-layers.md) | app/RPC/server/CLI/UI integration recipes and gaps |

## Goals and non-goals

Goals:

1. **Very strong foundation**: protocol-neutral agent model, explicit state machines,
   one canonical event vocabulary, and threads that survive restarts (DB metadata
   plus provider-owned transcripts).
2. **Clean boundaries**: each crate has one responsibility and depends only on
   lower layers. Protocol quirks stay inside drivers. Orchestration policy is pure,
   capability-driven code. The UI is reusable outside the Semantic app.
3. **High quality data model**: semantic types (`SemanticType`/`IntoValue`/
   `FromValue` derives) are the single source of truth for all model, persisted and
   RPC types. A forward-migration discipline applies to the new `semantic.agents`
   package.
4. **Extensible**: new drivers, MCP tools, UI panels and later multi-host
   orchestration can be added without reshaping the core.

Non-goals for this plan (designed for, but not built; see "Later" in each step):
multi-host orchestration, embedded preview browser and device automation, interactive
PTY terminals, PR watching and git hosting integration, scheduled tasks, fan-out to
multiple models, rich inline composer chips, mobile, LLM-generated titles and commit
messages, OpenCode HTTP driver, ACP v2.

## Architecture

```
                     ┌──────────────────────────────────────────────────────────┐
 UI (Dioxus)         │ semantic_ui  ── mounts ──►  semantic_ai_ui                │
                     │   (routes, nav)              chat/ (generic AI chat)       │
                     │                              agents/ (inbox, thread, review)│
                     └───────────────┬───────────────────────────┬──────────────┘
                                     │ RpcClient (unary + stream)│ types
 Contract (wasm-safe)                ▼                           ▼
                     semantic_agent_domain  (package semantic.agents, classes, DTOs,
                     │                       command specs, ThreadEvent, pure reducer)
                     semantic_agent        (agent model, events, capabilities,
                                            traits, transcript reducer, diff model)
 ─────────────────────────────────────────────────────────────────────────────────
 Host (native)
                     semantic_app ── commands ──► semantic_agent_orchestrator
                       (RPC commands, config,        (AgentRuntime, ScopeOrchestrator,
                        lifecycle, AgentDb impl)       thread actors, store, live bus,
                                                       recovery, providers, delegation)
                                                  │            │             │
                                                  ▼            ▼             ▼
                                  semantic_agent_drivers  semantic_agent_vcs  semantic_agent_mcp
                                  (process supervisor,    (git checkpoints,   (rmcp server,
                                   Claude, Codex, ACP,     diffs, worktrees,   tokens, toolkits,
                                   fake driver, testkit)   path safety)        stdio bridge)
                                                  │
                                                  ▼
                                   child processes: claude, codex app-server, ACP agents
```

### Crate map

| Crate | Path | Target | Depends on | Responsibility |
|---|---|---|---|---|
| `semantic_agent` | `crates/agent` | native + wasm | `semantic_data`, `futures` | Protocol-neutral agent abstraction. Holds the model types (items, requests, usage, failures, policies, capabilities, model catalog), the `SessionEvent` vocabulary, the history types, the `AgentDriver`/`AgentInstance`/`AgentSession` traits, the pure `Transcript` reducer, a unified diff model and parser, and id derivation helpers. No I/O. |
| `semantic_agent_drivers` | `crates/agent_drivers` | native | `semantic_agent`, tokio, serde | Concrete drivers. Covers process supervision (`ManagedProcess`), NDJSON and JSON-RPC framing, stderr capture, binary discovery, and the Claude Code, Codex app-server, ACP and fake drivers. Each driver also implements **history reading** from the agent's own session storage. A `testkit` feature adds scripted transports and fixture replay. No DB dependency, so it is reusable outside Semantic. |
| `semantic_agent_vcs` | `crates/agent_vcs` | native | `semantic_agent`, tokio | Workspace filesystem and git. Covers path containment, git checkpoints (hidden refs, temp index), diffs, restores and worktrees. No DB dependency. |
| `semantic_agent_domain` | `crates/agent_domain` | native + wasm | `semantic_agent`, `semantic_data`, `semantic_rpc_core`, `semantic_rpc` (default features) | The Semantic-specific orchestration contract. Holds the persisted package `semantic.agents` (metadata classes, collections, indexes, migrations), DTOs, unary and stream command specs, the `ThreadEvent` live-stream vocabulary (a wire type, not persisted), the pure `ThreadView` reducer and status derivation. |
| `semantic_agent_orchestrator` | `crates/agent_orchestrator` | native | domain, agent, drivers, vcs, mcp | The runtime. `AgentRuntime` is app-wide: drivers, instances, MCP server, process ownership. `ScopeOrchestrator` is per scope: metadata store, thread actors, live bus, recovery, history cache. Also holds the pure `decide`/`evolve` state machine, the in-memory live log and snapshot composition (provider history plus live overlay), provider config and probing, delegation and mailbox, and context handoff. Defines the `AgentDb` seam. |
| `semantic_agent_mcp` | `crates/agent_mcp` | native | `semantic_agent`, `semantic_agent_domain`, rmcp, axum | The MCP server agents connect to. Covers per-session bearer tokens, capability-scoped toolkits, instruction text, the stdio bridge for ACP agents and schema lint tests. It talks to the orchestrator only through its own `McpHost` trait. |
| `semantic_ai_ui` | `crates/ai_ui` | native + wasm | `semantic_agent`, `semantic_agent_domain`, `semantic_ui_core`, dxcomp, dxeditor | A reusable Dioxus AI UI. `chat/` holds generic chat components over `semantic_agent` types, usable by any project with an AI chat. `agents/` holds the coding-agent workspace (inbox, thread pane, composer, requests, review panel, provider settings) behind an `AgentsSource` trait with an RPC implementation. `widgets/` holds streaming markdown, the diff view and ANSI text. |
| `semantic_app` (existing) | `crates/app` | native | + orchestrator | Implements `AgentDb` for `SemanticDb`. Adds config, RPC command implementations, capability flag, lifecycle wiring and the security gate. |
| `semantic_ui` (existing) | `crates/ui` | native + wasm | + `semantic_ai_ui` | Routes `/agents` and `/agents/:thread`, nav entry, capability gate, desktop platform hooks. |
| `semantic_cli` (existing) | `crates/cli` | native | | `semantic api agents ...` subcommands and the hidden `agent-mcp-bridge` subcommand. |

Dependency rules (enforced in review):

* `semantic_agent` and `semantic_agent_domain` must compile for `wasm32-unknown-unknown`.
  They must not depend on tokio, process, fs or network crates.
* Drivers, VCS and MCP must not depend on `semantic_agent_domain`'s persistence code
  or on `semantic_app`. MCP may use domain DTOs.
* Nothing below `semantic_app` depends on `semantic_app`. The orchestrator talks to
  the DB only through its `AgentDb` seam (precedent: `LabelStore`, `JobStore`).
* `semantic_ai_ui` must not depend on `semantic_ui` or its `Route` enum. Navigation
  goes through callbacks (precedent: `UiCatalog::entity_navigation`).

## Core concepts and glossary

| Term | Meaning | Lives in |
|---|---|---|
| **Driver** | Implementation for one agent protocol family: `claude_code`, `codex`, `acp`, `fake`. Open slug type `DriverKind`. Unknown kinds must round-trip. | agent (trait), drivers (impl) |
| **Provider instance** | A user-configured use of a driver: binary path, env, home dir, ACP agent command, display name. `InstanceId` is a slug. Two instances of the same driver share no mutable state, so several accounts work. | agent (trait), domain (persisted config) |
| **Session** | One live connection to an agent: a process, or a thread on a shared Codex process. Ephemeral. | agent (trait) |
| **Native session** | The provider's own resumable conversation id, kept as a `NativeRef` with a strength (`strong`/`weak`/`none`). Treated as evidence, never as identity. | agent |
| **Workspace** | A directory (usually a git repo) on the agent host where agents work. | domain |
| **Thread** | The user-visible conversation, with its own app id. It survives sessions, restarts and resume failures. | domain |
| **Run** | One counted user request in a thread (ordinal N), from submission to terminal state. | domain |
| **Attempt** | One provider turn executing a run. Steer-by-restart, retry and recovery create new attempts. Provider events are fenced by attempt id. | domain |
| **Item** | A timeline record: user or agent message, reasoning, command, file change, tool call, plan, todo list, subagent, error, notice. Not persisted in v1; it comes from the live session or from provider history. | agent (model) |
| **Request** | A pending interaction that needs the user: an approval, structured questions, plan approval or MCP elicitation. Has a `ResponseMode` of `Live` (callback in a live process) or `Message` (answered by a follow-up message). In memory only; it dies with the session. | agent (model) |
| **History** | The transcript of a native session as stored by the agent itself, read through `AgentInstance::read_history` and normalized to `Item`s. | agent (trait), drivers (impl) |
| **Checkpoint** | A git snapshot of the workspace taken before and after each run, used for per-run diffs and revert. | vcs (mechanism), domain (record) |
| **Host** | The machine that runs agents. v1 has exactly one: `local`. | domain (attribute reserved) |

## Key design decisions

These decisions apply to every step. Steps reference them by number.

**K1. One canonical vocabulary per layer, with pure reducers.**

* Drivers emit `semantic_agent::SessionEvent`. The vocabulary is narrow and
  session-scoped, uses session-local ids and carries no timestamps. It is built
  around the Codex "item lifecycle" (started, delta, updated, completed) plus
  requests, plans, usage and turn terminal.
* The orchestrator wraps these in `semantic_agent_domain::ThreadEvent` for the live
  stream. A thread event has a stream cursor (epoch plus sequence), a timestamp,
  attempt fencing, and orchestration-level events (run queued/started/finished,
  request resolved by the user, checkpoint captured, config changed). Thread events
  are **not persisted** (K4).
* The same pure reducers run everywhere. `semantic_agent::Transcript` folds session
  events into items. `semantic_agent_domain::ThreadView` folds thread events into
  the full thread state the UI renders. The orchestrator uses the same evolve
  logic for its in-memory state. This avoids t3code's four overlapping
  vocabularies and its 86k lines of adapters.

**K2. Drivers are thin and split into three layers:**

1. a pure codec (`fn decode(line) -> NativeMsg`, `fn encode(cmd) -> line`);
2. a pure mapper `fn(&mut MapperState, NativeMsg) -> Vec<SessionEvent>`, unit-tested
   with recorded fixtures;
3. a small async shell that owns the process and channels.

All protocol quirks live in a named `quirks` module per driver.

**K3. Capabilities, not provider names.** Orchestration and UI behaviour derive
from `Capabilities`, a small struct of enums and sets (not t3code's 60 booleans).
Degradation policies are explicit and pure. Example: steering degrades
`Native → InterruptRestart → Queue`.

**K4. Persistence model: metadata in the DB, transcripts with the provider.**

* The DB stores **orchestration metadata only**:
  * workspaces, provider instance configs, threads (config, status, attention,
    native session ref, flags);
  * runs: ordinal, status, origin, timings, usage, failure, attempts with native turn
    refs, checkpoint refs. A run's input text is stored **only while the run is
    queued**, because it has not reached the agent's storage yet; it is cleared when
    dispatched;
  * checkpoints and thread links.
* The DB does **not** store messages, items, requests or stream events. The
  transcript's system of record is the agent's own session storage (K12).
* A single per-thread actor is the only writer for its thread's rows. Writes happen
  only at state transitions: run queued, started or ended, config change, checkpoint.
  That is a few writes per run, so no coalescing or write-path tuning is needed. There
  are no read-modify-write races and no interactive transactions (research
  semantic-data-layer §7.2).

**K5. Schema evolution.** Persisted lifecycle fields with stable value sets are
proper enums (thread status, attention, run status). Usage, failure and attempts are
records. Driver-specific instance config is the only open (`Any`) attribute;
verify in step 06 that `Any` is storable, else use `Json`. The agent model types
(items, requests, events) are not persisted, so they can evolve without migrations.
If transcript persistence is added later, open unions are stored as a `kind: String`
plus an `Any` payload, to avoid frozen multi-hundred-line variant snapshots in every
migration.

**K6. Live updates use snapshot plus cursor, served from memory.**

* Each active thread actor keeps an in-memory **live log**. It holds every
  `ThreadEvent` of the current actor incarnation, bounded: 20k events or 32 MB,
  dropping from the front. Events are tagged with a cursor `{epoch, seq}`, where
  `epoch` is random per actor incarnation.
* `threads.get` returns a snapshot. It is composed from provider history (K12) plus
  the live overlay, together with the current cursor.
* `threads.watch {cursor}` works like this:
  * if the epoch matches and the seq is still in the live log, it replays from the
    log and then tails;
  * otherwise it emits `Reset` and the client reloads the snapshot.
* Text deltas flow on the stream with offsets, so applying them is idempotent.
  Mutations stay unary commands. Reconnecting is resubscribing with the last cursor.
* This is independent of the DB backend and works in standalone desktop mode.

**K7. Restart semantics are explicit.** Process loss is never "resumed". On scope
open, a recovery pass moves every persisted state that is only valid with a live
process to a terminal state:

* active runs become `interrupted` (reason `host_restart`, with a failure note on the
  run);
* queued runs are held until the user resumes them.

Pending requests and in-flight items were never persisted; they are gone with the
process. The transcript itself survives in the provider's storage. Provider resume is
a new attempt that uses the stored native ref. If resume fails, a new native session
is started, with a deterministic context handoff rendered from the old session's
provider history when that is readable.

**K8. Security.**

* Agents are **disabled by default**: `AppConfig::agents_enabled`, env
  `SEMANTIC_AGENTS=1`.
* Agent commands are rejected unless the principal is `System`. This is the
  loopback/no-auth default today; revisit when auth exists.
* Workspace roots must be inside a configured allow-list (default: the user's home
  directory).
* Child env is built from an allow-list plus instance env.
* The MCP server binds to `127.0.0.1` on its own listener (never the public server
  port) and is protected by per-session bearer tokens stored only as SHA-256 hashes.
* Approvals fail closed. "Always allow" means "for this session", unless the driver
  supports provider-persisted rules and the user explicitly picks that option.
* Agent-provided strings are rendered as untrusted text (ANSI stripped or parsed,
  no raw HTML).

**K9. Persisted ids.**

* Random ids are UUID v4 with a type prefix: `thread-<uuid>`, `run-<uuid>`.
* Derived ids are `prefix-<hex(sha256(parts))[..32]>`, using the existing `sha2`
  workspace dep. They are used for idempotent commands (runs from
  `client_request_id`) and delegated child threads.
* UI item keys (not persisted) are derived from the native item id where strong.
  Otherwise they come from `(native turn ref or turn ordinal, item ordinal)`, so
  live and history renderings of the same item get the same key whenever the
  provider allows it.
* Clients send `client_request_id` on mutating commands, so retries replay instead
  of duplicating.

**K10. Injectable clock and id source** (`Clock`, `IdSource` traits in
`semantic_agent`) wherever timestamps or random ids are produced. Tests are
deterministic.

**K11. Data-model-first layering.** Derives and traits come from `semantic_data`.
RPC only consumes them, and domain DTOs are records with plain field names. Class
instances (persisted entities) use qualified attribute keys, following the
`project-qualified-attribute-keys` memory and `crates/data/src/jobs.rs`. Serde is
used only for native wire protocols inside drivers and the MCP crate. Model types
do not get serde derives.

**K12. Provider-owned transcripts (history).**

* `AgentInstance::read_history(native_ref, cwd, query)` returns the normalized
  `Item`s of a native session, paged from the end.
* Capability `Capabilities.history`:
  * `Cheap`: a local file read (Claude session JSONL, fake driver);
  * `Process`: needs the agent process (Codex `thread/read`/`thread/turns/list`, ACP
    `session/load` replay capture);
  * `None`.
* **Snapshot composition rule**, implemented once in the orchestrator:
  1. When a session opens, the actor records the history boundary: the number of
     turns already in history at that time, or the last native turn ref.
  2. While the actor lives, the snapshot is the history up to the boundary plus the
     in-memory live transcript of the current session.
  3. After the actor is evicted, the snapshot comes from history only.
  4. Runs from the DB are aligned with history turns by native turn ref where
     strong, otherwise by order. This attaches run metadata such as checkpoints,
     usage and status to the right turn.
* The history cache in the orchestrator is in memory, LRU-bounded, and invalidated
  on run end.
* Consequences, accepted for now:
  * message full-text search covers titles only;
  * deleting the provider's session files loses the transcript (the UI says so);
  * conversations continued outside Semantic (for example in the CLI with
    `--resume`) show up naturally in the history.
* History parsing of agent storage formats is not a stable contract. Parsers are
  tolerant and fixture-tested with versions tagged. On a parse failure the UI shows
  "history unavailable", and the thread still works.

## Data model overview

Details are in steps 01 and 06. This is the shape every step relies on.

```
semantic_agent (records and variants, not persisted directly)
  SessionSpec { cwd, extra_dirs, model: ModelSelection, access: AccessMode, interaction: InteractionMode,
                instructions: Vec<InstructionBlock>, mcp_servers: Vec<McpServerSpec>, resume: Option<NativeRef>,
                env: Vec<EnvVar>, hermetic: bool }
  SessionEvent = SessionReady | ConfigChanged | TurnStarted | ItemStarted | ItemDelta | ItemUpdated | ItemCompleted
               | RequestOpened | RequestClosed | PlanUpdated | UsageUpdated | RateLimitsUpdated
               | TurnEnded{outcome, disposition} | Notice | SessionClosed
  Item { id, turn, parent, status, body: ItemBody }   ItemBody = UserMessage | AgentMessage | Reasoning
       | CommandExecution | FileChange | ToolCall | WebSearch | Subagent | PlanProposal | TodoList
       | Compaction | Error | Notice
  Request { id, turn, item, response_mode, body: RequestBody }  RequestBody = Approval | Questions | PlanApproval | Elicitation
  Capabilities, ProviderStatus, ModelInfo, OptionDescriptor, Usage, Failure, AccessMode, InteractionMode

  HistoryQuery { before: Option<HistoryCursor>, max_turns }  HistoryPage { items: Vec<Item>, turns: Vec<HistoryTurn>, before: Option<HistoryCursor> }

semantic.agents package (persisted metadata only; prefix semantic:agents:)
  collection semantic_agents        : workspace, provider_instance, thread, run, checkpoint, thread_link

  workspace        { title, root_path, host, vcs: none|git, default_instance, default_model, default_access, archived }
  provider_instance{ title, instance_id, driver, host, enabled, binary_path, home_dir, launch_args, env, driver_config (Any) }
  thread           { title, workspace, host, cwd, worktree (record?), instance, model (record), access, interaction,
                     status, attention, parent (semantic:parent), relationship (fork|delegated|subagent), subject (Ref any),
                     native (record?), next_run_ordinal, active_run, queue_held, handoff_turns, pinned_at, settled_at,
                     archived, last_visited_at, last_activity_at, preview (short text of the last agent answer), mcp_capabilities,
                     created_at, updated_at }
  run              { thread, ordinal, status, origin, queued_input (record?; only while queued), client_request_id, queue_position,
                     attempts (list of records incl. native turn ref), usage, failure, checkpoint_before, checkpoint_after,
                     queued_at, started_at, completed_at }
  checkpoint       { thread, run, ordinal, ref_name, commit, files (list of records), status }
  thread_link      { thread, kind (entity|url|pull_request|task), target, label }

Not persisted (in-memory / provider-owned): items, requests, ThreadEvents, the live log.
```

## Decisions needing owner confirmation before execution

The plan assumes the recommended option. If you disagree, change the step docs
before dispatching wave W5 (items 1-2) or W0 (items 3-5).

1. **App lifecycle seam (core change, additive).** `semantic_app` has no generic
   service slot or shutdown hook, and the standalone desktop UI never calls
   `shutdown()`.
   *Recommended:* add a small generic `AppService` extension
   (`SemanticAppBuilder::with_service(Arc<dyn AppService>)`, called on
   `SemanticApp::shutdown()`). Also add a scope-service hook in `ScopeManager` that
   pins a scope's DB while a service is attached; it would also replace the jobs and
   plugins special-casing later. Factor the two app construction sites into one
   shared builder function, and make the standalone UI call `shutdown()` on exit.
   Step 11 implements this.
   *Alternative:* embedder-only registration with no core change (prototype quality).
2. **Interface client reconnect.** `InterfaceClient` never replaces a closed
   websocket session.
   *Recommended:* add an explicit "reset session on closed" behaviour to
   `InterfaceClient`. It is opt-in and new operations only; in-flight operations are
   never replayed, which keeps the existing documented semantics. Step 11 implements
   it. The UI already resubscribes with cursors (step 12).
   *Alternative:* the UI rebuilds the `RpcClient` on stream failure.
3. **Schema derive.** `#[derive(Class)]` does not generate `ClassType`/`AttributeType`.
   *Recommended for this plan:* write class definitions by hand, with
   codec/schema consistency tests (jobs precedent). A `ClassSchema` derive in
   `crates/data`/`crates/macros` is a separate, worthwhile core proposal; this plan
   does not depend on it.
4. **New third-party dependencies.**
   * `rmcp` 3.x for the MCP server.
   * `agent-client-protocol-schema` 1.x for ACP v1 types only. Our own JSON-RPC pump
     replaces the callback-style `agent-client-protocol` runtime crate.
   * `axum` is already in use.
   * Pin exact versions in step 00.
5. **Crate naming.** `semantic_agent`, `semantic_agent_drivers`,
   `semantic_agent_vcs`, `semantic_agent_domain`, `semantic_agent_orchestrator`,
   `semantic_agent_mcp`, `semantic_ai_ui`, all under `crates/`.

## Execution model (subagents)

* Implementation agents use `model: "opus"`, matching the user's standing
  preference. Research agents use sonnet. Reviews use opus.
* Waves run in dependency order. Steps within a wave run in parallel in the
  **shared working tree**. Step 00 already creates every crate skeleton and adds
  all workspace dependencies. Parallel agents therefore rarely touch `Cargo.toml`
  or `Cargo.lock`. An agent that must add a dependency adds it to its own crate's
  `Cargo.toml` and commits the resulting `Cargo.lock` hunk with its step.
* **One commit per step** (a step may make one extra commit for a clearly separate
  sub-group). Agents stage only their own paths, never `git add -A`. The tree
  contains unrelated uncommitted UI work (`crates/dxgraph`, `crates/ui`,
  `crates/ui_core`, `docs/plans/2026-10-03-graph-views`) that must not be touched or
  committed.
* **No stubs or TODO placeholders.** Each step delivers its full scope with tests.
  Interfaces needed from a later step are already defined by an earlier step (that
  is the point of the wave order). If a step finds that an upstream interface must
  change, it makes the minimal change in that crate, notes it in the commit message
  and reports it.
* Every step finishes with the following, all through the Nix devshell:
  * `nix develop -c cargo check --quiet --message-format=short`;
  * the step's tests: `nix develop -c cargo test --quiet --message-format=short -p <crate>`;
  * `cargo fmt`;
  * for wasm-safe crates: `nix develop -c cargo check --target wasm32-unknown-unknown -p <crate>`.
* After each wave the coordinator runs one opus review agent over the wave's
  commits. It checks layering rules, AGENTS.md rules (`Result<T, E>` without
  aliases, migrations), test coverage and cross-step consistency. Fixes go into a
  follow-up commit before the next wave starts.
* Precondition for W6 (step 15): the user's in-progress graph-views changes in
  `crates/ui/src/components/shell.rs` and `crates/ui/src/views/*` must be committed
  first, because step 15 edits the shell and route files.

Wave plan:

| Wave | Steps (parallel) | Needs |
|---|---|---|
| W0 | 00 | owner confirmation of decisions 3-5 |
| W1 | 01 | 00 |
| W2 | 02, 06, 08 | 01 |
| W3 | 03, 04, 05, 07, 09, 12 | 02 (03-05, 07), 06 (07, 09, 12), 08 (07 uses the interface) |
| W4 | 10, 13 | 07, 08, 09 (10); 12 (13) |
| W5 | 11, 14 | 10 (11); 13 (14); owner confirmation of decisions 1-2 |
| W6 | 15 | all |

Subagent brief template (the coordinator fills in `<step>`):

> You implement step `<step>` of `docs/plans/2026-10-03-ai-integration/plan.md` in
> `/home/theduke/dev/github.com/theduke/semantic`.
>
> 1. Read `AGENTS.md`, `plan.md` (all of it), and `steps/<step>.md`. Read only the
>    research files and source files the step lists, plus what you need to navigate
>    the code. Keep your context lean.
> 2. Follow the key decisions K1-K12.
> 3. Implement the full scope with tests. No stubs.
> 4. Validate inside the Nix devshell with check, the step's tests, `cargo fmt` and
>    wasm checks where required.
> 5. Commit once, staging only your paths. Unrelated dirty UI files exist; do not
>    touch them. Check `git status` before and after committing.
> 6. Report: what was built, any deviations from the step doc and why, any upstream
>    interface changes, and follow-ups.

## Cross-cutting conventions

* Errors: a `thiserror` enum per crate and module boundary. `Result<T, E>` is always
  spelled out (no aliases). No `anyhow` in library crates. Error messages that reach
  agents (MCP) or users are actionable.
* Async:
  * `semantic_agent` traits return `futures::future::BoxFuture<'_, Result<T, E>>`
    and `BoxStream`, so the crate stays runtime-agnostic and wasm-safe.
  * Native crates use tokio.
  * UI code never uses tokio (wasm); it uses `dioxus_sdk_time` and `futures`.
* Logging: `tracing`. Never log prompts, message text, tokens or env values at `info`
  or above. Redact secrets in stderr tails (see step 02).
* Bounded everything: channels, stderr ring, per-line size, raw frame logs, MCP
  results (about 20 KB per text result) and history pages.
* Naming: RPC commands are `semantic.agents.<area>.<verb>`. Class ids are
  `semantic:agents:<class>` and attribute ids `semantic:agents:<class>:<attr>`.
  Shared attributes (`semantic:title`, `semantic:parent`, `semantic:created_at`,
  `semantic:updated_at`) are reused.
* Tests:
  * pure logic is table-tested;
  * drivers are fixture-replay tested (recorded NDJSON transcripts tagged with CLI
    version);
  * the orchestrator gets integration tests on the in-memory `semantic_db_kv`
    backend with the fake driver;
  * the app gets RPC tests including streaming;
  * the UI gets SSR tests and mock-source loader tests;
  * Playwright acceptance runs against a server with the fake driver;
  * real-CLI smoke tests are opt-in (`SEMANTIC_AGENT_LIVE_TESTS=1`) and never run in
    default `cargo test`.

## Future: multi-host

Not implemented now. These choices keep it cheap later:

* `host` attributes on workspace, provider instance and thread (always `local` in v1).
* All ids are global (UUID), with no host-relative numbering.
* The orchestrator is reached only through domain commands and streams. A remote
  host can expose the same command surface, and a coordinator can route by `host`.
* The MCP server and process ownership live in `AgentRuntime`, which is per host.

## Later (explicitly out of scope, listed so foundations accommodate them)

* **Transcript persistence in the Semantic DB.** The model is ready for it: items,
  requests and `ThreadEvent` are already semantic types with idempotent,
  offset-based delta semantics.
  * Add an append-only event collection with a unique `(thread, seq)` index and
    deterministic ids, plus an item projection collection.
  * Commit coalesced deltas (about 200 ms or 4 KB) in the same `Batch` as the
    metadata.
  * Persist open unions as `kind` + `Any` (K5), with large outputs in the filestore.
  * Snapshot composition then prefers DB history over provider history.
  * Design notes: research/semantic-data-layer.md §10.
  * This also enables message full-text search and history that survives provider
    data deletion.

* Interactive PTY terminal panel (portable-pty plus xterm.js interop).
* Preview browser automation tools (broker pattern from research/t3code-mcp.md §5).
* PR linking with host APIs, PR watch wakes, git actions (commit, push, PR).
* Scheduled tasks. The data model has `run.origin = schedule` reserved.
* Native and portable thread fork, and merge-back. The model has `relationship = fork`
  and `parent`.
* LLM-based title, commit and PR text generation.
* OpenCode HTTP driver, ACP v2 path, ACP registry installation.
* A `ClassSchema` derive to generate class definitions from Rust types.
* Generic DB change-feed stream (would also give jobs and tasks live updates).
