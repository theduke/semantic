# Step 05: ACP driver (`semantic_agent_drivers::acp`)

Wave W3. Depends on: 01, 02. Agent: opus. Runs in parallel with steps 03, 04, 07, 09 and 12.
Touch only `crates/agent_drivers/src/acp/**`, its fixtures, and one registration line
in `lib.rs`.

Read: research/agent-protocols.md §4 (all of it), §5.1-5.3, §7; research/t3code-providers.md
§5.4 (ACP flavors), §6.2 (ACP capability column); research/t3code-mcp.md §3 (ACP MCP
attachment: stdio bridge rationale); steps/01 and steps/02.
Reference: t3code `apps/server/src/provider/acp/*`, `packages/effect-acp`.

## Goal

One generic ACP v1 client driver that covers Gemini CLI, OpenCode (`opencode acp`),
Cursor (`cursor-agent acp`), Copilot, Grok and the long tail. Per-agent differences
live in a declarative **flavor** table, not in branches spread through the code.

## Design

* Types come from `agent-client-protocol-schema` (v1 module). Transport and
  correlation use our `JsonRpcPeer` (step 02, `jsonrpc_field: Required`). Do not use
  the callback-style `agent-client-protocol` runtime crate (decision 4 in plan.md).
* Kind `acp`. Each instance config identifies the agent:

```rust
pub struct AcpConfig {
    pub agent_id: String,            // "gemini", "opencode", "cursor", "copilot", ... (flavor lookup key; unknown => generic)
    pub command: CommandSpec,        // e.g. program "gemini", args ["--acp"]
    pub display_name: Option<String>,
}
```

  Ship built-in presets (`acp/presets.rs`) for gemini, opencode, cursor, copilot and
  qwen with their documented commands (research §4.10). Presets are only defaults; the
  user can edit the command.
* Initialize with `protocolVersion: 1`, `clientCapabilities: {}`. Do **not**
  advertise `fs`/`terminal`; the agent uses its own tools. Record
  `agentCapabilities`, `authMethods` and `agentInfo`.
* `session/new {cwd, mcpServers}` when starting. When resuming, use `session/resume`
  if `sessionCapabilities.resume` is present (no replay), else `session/load` (if
  `loadSession`). With `session/load`, the replayed updates that arrive before the
  response are **not** emitted as live events; they are history.
* History (plan K12):
  * `read_history` spawns a short-lived agent process (same command and env),
    `initialize`s, calls `session/load` with the native session id, captures the
    replayed `session/update` notifications through the same mapper in replay mode,
    and returns them as one `HistoryPage` (no paging support in ACP: return all
    turns, cursor `None`), then closes the process.
  * Requires `loadSession`; otherwise `history = None`.
  * Cache it: the orchestrator calls it rarely (snapshot after actor eviction).
  * Sessions currently open in a live process are not re-loaded: the orchestrator
    serves those from memory.
  * `Capabilities.history = Process`. Turn boundaries come from
    `user_message_chunk` runs.
* MCP: always pass our server as a **stdio** server using
  `McpServerSpec.stdio_bridge` (command plus env carrying URL and token; never put the
  token in argv). Additionally pass it as `http` if `mcpCapabilities.http`. Flavor
  can override this, e.g. "http only" or "stdio only".
* Prompt: `session/prompt` with content blocks (text, image if `promptCapabilities.image`,
  `resource_link` for `WorkspacePath`). Instruction blocks: ACP has no system channel,
  so wrap them on the first prompt (and whenever their content changes) as
  `<semantic_instructions>…</semantic_instructions>\n\n<user_request>…</user_request>`.
  Skip the wrapper when the prompt starts with `/` (slash command). Set
  `Capabilities.instructions = FirstMessage`.
* Turn end is the `session/prompt` **response** `stopReason`: `end_turn`→Completed,
  `cancelled`→Interrupted, `refusal`→Failed(Refusal), `max_tokens`/`max_turn_requests`→Failed(Provider).
  Errors map to Failed with the classified failure. `-32000` maps to Auth.

## Mapping

| ACP `session/update` | SessionEvent |
|---|---|
| `agent_message_chunk` | AgentMessage item (start on first chunk per message id or per turn if no id) + `ItemDelta(Text)` |
| `agent_thought_chunk` | Reasoning item + delta |
| `user_message_chunk` | ignored live; in history replay mode it becomes a UserMessage item and a turn boundary |
| `tool_call` | `ItemStarted` mapping by `kind`: execute→CommandExecution (rawInput.command when present), edit/delete/move→FileChange (diff content `oldText/newText` → unified diff via a small helper), read/search→ToolCall(Read/Search), fetch→ToolCall(Fetch), think→Reasoning-ish ToolCall(Think), other→ToolCall(Other) |
| `tool_call_update` | `ItemUpdated` (merge patch semantics: omitted fields unchanged) or `ItemCompleted` when status completed/failed |
| `plan` | `PlanUpdated` + TodoList item (whole list replaced) |
| `available_commands_update` | `ConfigChanged` (slash commands) |
| `current_mode_update`, `config_option_update` | `ConfigChanged` |
| `usage_update` | `UsageUpdated` |
| `session/request_permission` (agent→client request) | `RequestOpened(Approval)`. Options come from the agent's `options[]`, mapping `kind` allow_once/allow_always/reject_once/reject_always to decisions, keeping the native `optionId`. Respond with `{outcome: {outcome: "selected", optionId}}`. On cancel or turn end respond `{outcome: {outcome: "cancelled"}}` |
| `elicitation/create` | `Elicitation` request |
| flavor extension requests (e.g. `cursor/ask_question`, `cursor/create_plan`) | flavor hook maps to Questions/PlanApproval; unknown extension requests get a method-not-found error |

## Modes, models and access

* Prefer config options with `category: "mode"`/`"model"` (`session/set_config_option`).
  Fall back to `modes` + `session/set_mode` (Gemini) or the unstable model state.
* Flavor table maps our `AccessMode`/`InteractionMode` to the agent's mode ids, e.g.
  Gemini `default`/`auto_edit`/`yolo`, plan → `plan` or `architect` if offered.
  Unknown agents use a heuristic: match mode ids or names containing
  `plan`/`architect`, `yolo`/`full`/`bypass`, `edit`/`accept`. If no mode
  matches, use `clamp_access` to the safest available mode.
* Report `EffectivePolicy{enforcement: ClientBoundary}` unless the flavor declares
  native sandbox semantics.

Capabilities:

* steering `InterruptRestart` (v1 has no steer); interrupt true (`session/cancel`
  notification; then wait for the prompt response with a timeout of 10 s, escalating
  to closing the session);
* resume if `loadSession` or resume capability; fork none; rollback false;
* model and access switch `Live` if config options exist, else `Restart`;
* approvals: whatever kinds appear (declare command, file change, tool);
* questions only via flavor or elicitation; todo lists true; plan proposals per
  flavor;
* MCP stdio true, http per agent; images per `promptCapabilities`;
* identity: turn `None`, item `Strong` (toolCallId) or `Weak`.

## Auth

`probe()` runs the agent, sends `initialize` only, and inspects `authMethods`. If
methods exist, it attempts a disposable `session/new` only when the flavor declares
that this is free and side-effect-free; otherwise auth is `Unknown`. It never calls
`authenticate`. Terminal auth methods are surfaced as `AuthMethod{kind: terminal, command}`
for the UI to run in a terminal later.

## Tests

* Fixture replay:
  * Gemini-like: modes, permission request allow and reject, tool calls with diff
    content, plan updates;
  * OpenCode-like: config options, usage_update;
  * cancel mid-turn: the pending permission is answered `cancelled` and the turn ends
    Interrupted;
  * `-32000` on `session/new` gives an Auth failure;
  * an unknown `sessionUpdate` kind is ignored;
  * `session/load` replay on resume does not leak into live events;
  * `read_history` captures a replay fixture into the expected `HistoryPage`.
* Flavor heuristic table tests for mode mapping.
* Diff construction from oldText/newText (uses `semantic_agent::diff` types for
  verification).
* Live smoke tests (ignored by default) for each preset whose binary is on PATH.

## Acceptance

* Tests green. Presets documented in `acp/README.md`.
* One commit: "Add generic ACP v1 driver with agent flavors".
