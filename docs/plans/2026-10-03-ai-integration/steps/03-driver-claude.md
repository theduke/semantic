# Step 03: Claude Code driver (`semantic_agent_drivers::claude`)

Wave W3. Depends on: 01, 02. Agent: opus. Runs in parallel with steps 04, 05, 07, 09 and 12.
Touch only `crates/agent_drivers/src/claude/**`, its fixtures, and one registration
line in `lib.rs`.

Read: research/agent-protocols.md §2 (all of it), §7.2 mapping table, §7.4;
research/t3code-providers.md §4.1 (runtime-mode mapping), §5.3 (Claude), §12.5-12.6
(quirks, background work); steps/01 and steps/02.

Reference implementation for behaviour (do not port line by line):
t3code `apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts` and the SDK
types in `node_modules/.pnpm/@anthropic-ai+claude-agent-sdk@*/node_modules/@anthropic-ai/claude-agent-sdk/sdk.d.ts`.
Use the `.d.ts` as the authoritative wire schema.

## Goal

A native driver for the `claude` CLI's stream-json control protocol. It does not use
the ACP adapter or the Node SDK.

## Module layout

```
claude/
  mod.rs          ClaudeDriver (kind "claude_code"), ClaudeInstance, config record ClaudeConfig
                  { binary_path, config_dir (CLAUDE_CONFIG_DIR), setting_sources, extra_args, include_partial_messages: true }
  wire.rs         serde types: SdkMessage (tagged by "type"/"subtype") with #[serde(other)]/Unknown(Value) fallbacks,
                  ControlRequest/ControlResponse envelopes, PermissionResult, PermissionUpdate, content blocks, stream events
  codec.rs        decode line -> WireIn; encode WireOut -> line
  launch.rs       build argv/env from SessionSpec (see below)
  mapper.rs       pure ClaudeMapper: (state, WireIn) -> Vec<SessionEvent> + Vec<HostAction> (e.g. auto-answer)
  control.rs      outgoing control_request correlation (request_id), incoming control_request routing (can_use_tool, hook_callback,
                  mcp_message -> error unsupported, elicitation)
  policy.rs       AccessMode/InteractionMode -> permissionMode + flags; option descriptors (effort, thinking) per model
  probe.rs        `claude --version`, `claude auth status` (JSON) -> ProviderStatus; models from a small bundled table
                  refreshed from the initialize response when a session runs (cache in instance)
  quirks.rs       named quirks: tool names (Agent/Task, TodoWrite, AskUserQuestion, ExitPlanMode), updatedInput echo,
                  init-per-turn, cumulative modelUsage, result-after-interrupt
  session.rs      ClaudeSession: AgentSession impl on top of the shell from step 02
tests/fixtures/claude/*.ndjson   recorded or synthesized transcripts (tag with CLI version in file header comment line)
```

## Launch (normative)

Always use:

```
claude --output-format stream-json --verbose --input-format stream-json --include-partial-messages
       --session-id <uuid> | --resume <native id>       (we own the session id for new sessions; UUID v4)
       --model <model> [--effort <level>]               (from ModelSelection options)
       --permission-mode <mode>                         (see policy)
       [--allow-dangerously-skip-permissions]           (only for FullAccess)
       --mcp-config <json>                              (our MCP server(s), http transport with Authorization header, tool timeout)
       [--strict-mcp-config --setting-sources ...]      (hermetic mode)
       --append-system-prompt <instructions>            (InstructionBlocks joined, stable order by key)
       --add-dir <extra_dirs>...
```

Env: `CLAUDE_CONFIG_DIR` (instance `home_dir`), and an entrypoint/client-app env so
telemetry identifies the host (see research §2.1).

Immediately after spawn, send `control_request{subtype: initialize}` (hooks: none for
v1; set `supportedDialogKinds: []`). Its response provides the models list
(refresh the instance model cache), account and commands. Emit
`SessionReady{info}` from the init response combined with the first `system/init`.

## Mapping (normative summary; details in research §2.2-2.6)

| Claude wire | SessionEvent |
|---|---|
| user message sent by us (`start_turn`) | allocate `TurnId`; emit `TurnStarted` and `ItemStarted(UserMessage)` + `ItemCompleted` |
| `stream_event` text_delta / thinking_delta | `ItemStarted(AgentMessage/Reasoning)` at `content_block_start`, `ItemDelta(Text/Reasoning)` |
| `stream_event` tool_use block + `input_json_delta` | buffer input; `ItemStarted(ToolCall/Command/FileChange)` at `content_block_stop` once input is parseable |
| `assistant` message (complete) | `ItemCompleted` for each block, authoritative text; tool_use input classification (below) |
| `user` message with `tool_result` | `ItemCompleted` for the matching tool item: output, error status |
| `TodoWrite` tool_use | `PlanUpdated` + `ItemCompleted(TodoList)` (no ToolCall item) |
| `Agent`/`Task` tool_use + `system/task_*` | `Subagent` item (parent for frames with `parent_tool_use_id`); `task_notification` sets the result |
| `can_use_tool` (generic tool) | `RequestOpened(Approval{subject from tool name/input, options})`; respond -> `PermissionResult` with `updatedInput` echoed |
| `can_use_tool` `AskUserQuestion` | `RequestOpened(Questions)`; respond -> allow with `updatedInput.answers` keyed by question text |
| `can_use_tool` `ExitPlanMode` | `ItemCompleted(PlanProposal{markdown})` + `RequestOpened(PlanApproval)`; approve -> allow (mode switch), reject -> deny with feedback |
| `control_cancel_request` | `RequestClosed(Cancelled)` |
| `result` | `UsageUpdated(Turn)` from `usage`, cost from `total_cost_usd` delta vs previous cumulative (basis estimated); `TurnEnded` with outcome from subtype/`is_error`/`terminal_reason` and failure class from `assistant.error` values |
| `rate_limit_event` | `RateLimitsUpdated` |
| `system/status` compacting, `compact_boundary` | `Compaction` item |
| `system/init` permissionMode change | `ConfigChanged` |
| unknown types and subtypes | ignored (debug log once per type) |

Tool classification for `ToolCall` vs specialised items:

* `Bash` becomes `CommandExecution` (command and description from input; output from
  tool_result).
* `Edit`, `MultiEdit`, `Write` and `NotebookEdit` become `FileChange`. Build the diff
  from `old_string`/`new_string` when present; otherwise leave `diff` empty with
  `kind: update`.
* `Read`, `Glob` and `Grep` become `ToolCall` with categories Read and Search.
* `WebFetch`/`WebSearch` become `WebSearch` or `ToolCall(Fetch)`.
* `mcp__<server>__<tool>` becomes `ToolCall{server}` with category Mcp.

Approval options for generic tools:

| Option | Decision | Wire behaviour |
|---|---|---|
| Allow once | AllowOnce | allow |
| Allow for session | AllowForSession | allow + `updatedPermissions` from `permission_suggestions` with destination `session` |
| Always | AllowAlways | only if suggestions exist; destination `localSettings`; label it clearly as persisted in project settings |
| Deny | Deny | deny with message |
| Deny and stop | DenyAndInterrupt | deny with `interrupt: true` |

## Policy mapping

| AccessMode | permissionMode | Notes |
|---|---|---|
| Supervised | `default` | |
| AcceptEdits | `acceptEdits` | |
| Auto | `auto` | if the CLI reports support (init capabilities/version); else clamp to `acceptEdits` |
| FullAccess | `bypassPermissions` | plus `--allow-dangerously-skip-permissions` |

`InteractionMode::Plan` sets `plan`; return to the access mode after plan approval
or rejection via `set_permission_mode`. `Enforcement::Native`. Changing
`update_config` access/model uses the `set_permission_mode`/`set_model` control
requests (`ConfigSwitch::Live`).

Capabilities:

* steering `Native`: send a `user` message with `priority: "now"` during a turn; emit
  a `UserMessage{intent: steer}` item;
* `interrupt` via the control request;
* `resume` via `--resume`;
* fork `Latest` via `--resume --fork-session`; `FromTurn` is out of scope;
* rollback false (v1);
* questions, plan proposals, todo lists and subagents true;
* MCP http true;
* images true (base64 image content blocks from `AttachmentRef.local_path` read by the
  driver; size cap 10 MiB);
* instructions `SystemPrompt`.

## History (plan K12)

Claude persists sessions as JSONL at
`$CLAUDE_CONFIG_DIR/projects/<encoded cwd>/<session id>.jsonl` (research §2.1, §2.7).
The cwd encoding replaces every non-alphanumeric character with `-`; long names are
truncated and hashed. Verify against the installed CLI. Since CLI 2.1.223 a lookup by
session id across project dirs also works: implement a fallback scan of
`projects/*/<session id>.jsonl`.

* `history.rs` holds a tolerant reader. Each line is an SDK-message-like record
  (`user`, `assistant`, tool results, `system` records, summaries). Map them through
  the **same mapper code** as the live stream, in a "replay" mode without
  `stream_event` deltas, so item ids and classification match the live session.
* Turn boundaries are human user messages (not tool results). Use the message
  `uuid` as the native turn ref (strong). Sidechain/subagent records (`isSidechain`,
  `parent_tool_use_id`) are nested under their Subagent item.
* Pagination reads the file from the end in chunks and stops after `max_turns`
  completed turns. The cursor is a byte offset plus turn ordinal. Files can be large.
* Unknown record types are skipped with a counted warning in `HistoryPage.warnings`.
  This file format is **not a stable contract**: fixtures are tagged with the CLI
  version.
* `Capabilities.history = Cheap`.

## Interrupt and close

* `interrupt` sends the `interrupt` control request. Its response is an ack; the
  turn ends when `result` arrives. Pending `can_use_tool` requests are answered `deny`
  and emitted `RequestClosed(Cancelled)`.
* `close` closes stdin, then follows the `ManagedProcess::shutdown` escalation.
* Background tasks: the turn ends at `result`. Later `task_*` events for that turn
  update the subagent item (the orchestrator accepts late item updates for completed
  attempts; see step 07). Document this.

## Tests

* Fixture replay (testkit) for these scenarios:
  * a simple answer with streaming;
  * Bash with approval (allow once, allow for session with `updatedPermissions`);
  * deny-and-interrupt;
  * AskUserQuestion;
  * ExitPlanMode approve and reject;
  * TodoWrite;
  * Task subagent with nested frames;
  * interrupt mid-stream;
  * an error result (rate limit, auth);
  * MCP tool call;
  * an unknown message type.
  Fixtures can be synthesized from the `.d.ts` shapes. Each fixture file's first line
  is a comment record with the CLI version or `synthetic`.
* Mapper unit tests for tool classification, cost delta computation and failure classification.
* History reader:
  * session JSONL fixtures covering a plain conversation, tools, a subagent
    sidechain, compaction summary records and an unknown record type;
  * paging from the end; cwd encoding (including the long-path hash case);
  * a live replay fixture and its history file produce the same item ids.
  * The live smoke test also reads the session's history file after the turn.
* Launch argv tests for each AccessMode/InteractionMode/hermetic/resume combination.
* Probe parsing of `claude auth status` JSON samples (logged in, logged out, API key).
* Live smoke test (ignored unless `SEMANTIC_AGENT_LIVE_TESTS=1` and `claude` is on
  PATH): one turn "reply with OK", assert `TurnEnded(Completed)` and a usage report.
  Record its transcript into `tests/fixtures/claude/live-*.ndjson` when blessing.

## Acceptance

* The driver passes all fixture and unit tests. The live test was run once locally if
  `claude` is available (report the result; skip if not).
* One commit: "Add Claude Code stream-json driver".
