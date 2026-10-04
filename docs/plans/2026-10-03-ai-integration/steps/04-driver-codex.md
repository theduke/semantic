# Step 04: Codex app-server driver (`semantic_agent_drivers::codex`)

Wave W3. Depends on: 01, 02. Agent: opus. Runs in parallel with steps 03, 05, 07, 09 and 12.
Touch only `crates/agent_drivers/src/codex/**`, its fixtures, and one registration
line in `lib.rs`.

Read: research/agent-protocols.md §3 (all of it), §7; research/t3code-providers.md
§4.1, §5.2; steps/01 and steps/02.
Reference: t3code `packages/effect-codex-app-server` (generated client) and
`Adapters/CodexAdapterV2.ts`.

## Goal

A native driver for `codex app-server` (JSON-RPC over stdio, with no `"jsonrpc"` member).

## Schema source

Generate the JSON Schema bundle from the installed binary:

```
codex app-server generate-json-schema --out <tmp>
```

Use it as the reference for the version you pin (0.159.x at research time). Then
hand-write serde types for the subset we use: initialize, thread/start, resume,
fork, turn/start, steer, interrupt, model/list, account/read, the item and turn
notifications, approval server requests and requestUserInput. Use lenient decoding:
`#[serde(other)]` on enums, and `Value` for unknown item types. Store the schema
version in `codex/SCHEMA_VERSION` and add a short `codex/README.md` with
regeneration instructions. Do not vendor the whole generated bundle.

## Module layout

```
codex/
  mod.rs        CodexDriver (kind "codex"), CodexInstance, CodexConfig { binary_path, codex_home (CODEX_HOME), extra_args, config_overrides: Vec<(String,String)> }
  wire.rs       serde types (subset), ThreadItem enum with Unknown fallback
  process.rs    ONE app-server process per instance, shared by all sessions of that instance (Codex multiplexes threads):
                lazy start, initialize { clientInfo, capabilities: { experimentalApi: true, optOutNotificationMethods: ["turn/diff/updated", ...noise] } },
                notification router by threadId -> per-session channel, restart on crash (sessions observe SessionClosed)
  mapper.rs     pure CodexMapper per thread: notifications/server requests -> SessionEvents
  policy.rs     AccessMode/Interaction -> approvalPolicy + sandboxPolicy + approvalsReviewer (always explicit) + collaborationMode
  probe.rs      `codex --version`; account/read via a short-lived initialize-only connection (never start a thread); model/list (paginate)
  quirks.rs     no jsonrpc member, numeric/string ids echo, cumulative token usage -> per turn delta, "cancel" vs "decline",
                per-turn overrides are sticky, schema-drift notes
  session.rs    CodexSession: thread/start or thread/resume on open; AgentSession impl
tests/fixtures/codex/*.ndjson
```

## Mapping summary

| Codex | SessionEvent |
|---|---|
| `thread/start` or `thread/resume` result | `SessionReady{native_session: thread id (Strong)}` |
| `turn/start` (ours) + `turn/started` | `TurnStarted{native: turn id}` + UserMessage item |
| `item/started` | `ItemStarted` mapping `ThreadItem` to `ItemBody`: agentMessage→AgentMessage, reasoning→Reasoning, commandExecution→CommandExecution, fileChange→FileChange (per change diff), mcpToolCall→ToolCall{server}, dynamicToolCall→ToolCall, webSearch→WebSearch, plan→PlanProposal, contextCompaction→Compaction, subAgentActivity/collabAgentToolCall→Subagent, other→ToolCall(Other) with the raw value |
| `item/agentMessage/delta`, reasoning deltas, `commandExecution/outputDelta`, `item/plan/delta` | `ItemDelta` on the matching channel |
| `item/completed` | `ItemCompleted` (authoritative; replaces accumulated text) |
| `turn/plan/updated` | `PlanUpdated` (+ TodoList item per turn, updated in place) |
| `thread/tokenUsage/updated` | `UsageUpdated` (Turn usage = `last`; context window from `modelContextWindow`) |
| `item/commandExecution/requestApproval` | `Approval{Command}` with options accept / acceptForSession / decline / cancel(=DenyAndInterrupt); execpolicy amendment offered as AllowAlways when proposed |
| `item/fileChange/requestApproval` | `Approval{FileChange}` |
| `item/permissions/requestApproval` | `Approval{Permission}` (grant the requested set or decline) |
| `item/tool/requestUserInput` | `Questions` (ResponseMode Live; answered by a JSON-RPC response) |
| `mcpServer/elicitation/request` | `Elicitation` |
| `serverRequest/resolved` | `RequestClosed(AnsweredElsewhere or Cancelled)` |
| `turn/completed` | `TurnEnded` with outcome from status; failure class from `codexErrorInfo` |
| `account/rateLimits/updated` | `RateLimitsUpdated` |

## Policy mapping (always send explicitly on every `turn/start`)

| AccessMode | approvalPolicy | sandboxPolicy | reviewer |
|---|---|---|---|
| Supervised | `untrusted` | `readOnly` | `user` |
| AcceptEdits | `on-request` | `workspaceWrite{writableRoots:[cwd, extra_dirs], networkAccess:false}` | `user` |
| Auto | `on-request` | `workspaceWrite` | `auto_review` |
| FullAccess | `never` | `dangerFullAccess` | `user` |

Values must match the pinned schema. Verify them against the generated schema;
`untrusted` vs `unlessTrusted` drift is documented in research §3.5.
`InteractionMode::Plan` maps to
`collaborationMode: {mode: "plan", settings: {model, reasoning_effort}}`.

Capabilities:

* steering `Native` (`turn/steer` with `expectedTurnId`); `interrupt`; `resume`;
* fork `FromTurn` (`thread/fork` with `lastTurnId`); rollback true (`thread/revert`);
* model and access switch `NextTurn`;
* approvals: command, file change, permission; questions true; plan proposals true;
  todo lists true; subagents true;
* MCP http true; images true (local image input items if the schema supports them,
  else false);
* instructions `DeveloperContext` (`developerInstructions` on thread/start for stable
  blocks; per-turn additional context where supported); enforcement `Native`;
* usage: tokens and context window, no cost.

History (plan K12): `read_history` uses the instance's shared app-server process,
which is started if needed. Prefer `thread/turns/list` + `thread/items/list` (paged)
when the pinned schema has them; otherwise use `thread/read`. Map `ThreadItem`s with
the same mapper code as the live stream, so item ids match the native item ids.
Never resume or subscribe the thread just to read it; use the read-only methods.
Capability `history = Process`. Document in `quirks.rs` that reading the history of a
thread that another Codex client is running returns the in-progress turn without an
outcome.

MCP: pass our server via `thread/start.config.mcp_servers.<name> = {url, http_headers}`.
If that is not accepted, use `-c` launch overrides on the shared process. The
per-thread config is preferred because the process is shared across sessions. Raise
the tool timeout config if available.

## Tests

* Fixture replay: simple turn, streaming, command approval (each decision), file
  change approval, requestUserInput, plan mode, interrupt, steer, failed turn
  (usageLimitExceeded → `UsageLimit` class), unknown item type, two sessions
  multiplexed on one process (routing by threadId).
* Token usage delta computation across resume.
* History: fixture of `thread/turns/list`/`thread/items/list` (or `thread/read`)
  responses mapped to a `HistoryPage`; paging; same item ids as the live fixture of
  the same conversation.
* Policy table tests.
* Process crash: both sessions receive `TurnEnded(Failed, Broken)` and `SessionClosed`.
* Live smoke (ignored unless `SEMANTIC_AGENT_LIVE_TESTS=1` and `codex` on PATH): one
  turn "reply with OK". Also run `generate-json-schema` and check that our pinned
  method names exist (prints a diff summary).

## Acceptance

* Tests green. The schema version is pinned and documented.
* One commit: "Add Codex app-server driver".
