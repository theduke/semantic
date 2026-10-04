# Step 10: Orchestrator features (providers, VCS, MCP, delegation, handoff)

Wave W4. Depends on: 07, 08, 09 (and 03-05 for real-driver registration). Agent: opus.
Runs in parallel with step 13.

Read: plan.md, steps/07 (extension points: Effect::Hook, SessionDecorator,
InstructionSource), steps/08, steps/09 (McpHost), research/t3code-orchestration.md
§10 (switching, handoff), §11 (delegation and mailbox), §12 (checkpoints and rollback),
§13 (worktrees, launch flow); research/t3code-mcp.md §6.

## Goal

Complete the orchestrator on top of the core from step 07, using its extension
points. Keep each feature in its own module, so the core actor stays small:

```
crates/agent_orchestrator/src/
  providers/   ProviderService: instance configs (store), auto-detected defaults, probe cache + watch, instance (re)creation
  workspace/   WorkspaceService: CRUD + validation (allow-list, git detection); worktree preparation for threads
  checkpoint/  CheckpointHooks: baseline/after-run capture, finalize stage, diff queries, revert execution
  mcp/         McpIntegration: owns McpServer, SessionDecorator (adds McpServerSpec + token), InstructionSource, McpHost impl
  delegation/  DelegationService: child threads, completion mailbox, task status
  handoff/     HandoffRenderer: deterministic, budgeted transcript rendering for fresh sessions
  title.rs     first-message title heuristic
```

## 1. Providers

* `ProviderService` per scope (configs are persisted in the scope DB). Instances live in
  `AgentRuntime` keyed by `(scope, instance_id)`, rebuilt when the config changes.
* Auto-detection on first use when no instance configs exist: create disabled-by-default
  configs for each built-in driver whose binary is found:
  * `claude_code` if `claude` is found;
  * `codex` if `codex` is found;
  * ACP presets (`gemini`, `opencode`, `cursor`, `copilot`) if their binaries are
    found;
  * `fake` only if `include_fake_driver`.
  Set `enabled = true` for found binaries. The user can disable them.
* Probe cache: probing results (`ProviderStatus`) are kept in memory with a TTL
  (5 min), refreshed on demand (`providers.refresh`) and on config change. Probing
  never runs concurrently for the same instance. Publish changes on
  `providers.watch`.
* Model validation on thread create and send: the model must exist in the instance's
  catalog, unless the catalog is empty or unknown, or the model is a custom model
  string. Options must match the descriptors.

## 2. Workspaces and worktrees

* `create`:
  * canonicalize the path;
  * check `is_within_roots(workspace_roots)`;
  * detect git (`vcs = git|none`);
  * the title defaults to the directory name.
* Thread creation with `worktree: Some(WorktreeRequest{branch, base_ref, start_from_remote})`:
  1. The thread is created immediately and its first run gets status `starting`, with
     the phase "preparing workspace" shown as a `Notice` item plus a
     `SessionStateChanged{starting}`.
  2. The worktree is created via a `Hook` effect.
  3. On success the thread's `cwd`/`worktree` are updated (`ThreadConfigChanged`) and
     the attempt starts.
  4. On failure the run fails with a clear message and the worktree is cleaned up.
* Thread delete does **not** remove worktrees automatically in v1. It does so only
  when the `threads.delete` payload sets `remove_worktree: true` (defined in step
  06). Dirty worktrees are kept rather than force-removed. The thread delete still
  succeeds, and a warning naming the kept worktree path is logged.
* `workspaces.branches` is served via `Repo::branches` (step 08).

## 3. Checkpoints

The following applies only when the workspace is git:

* Before each attempt's first `start_turn` of a run, call
  `capture_if_missing(ns = thread_id, ordinal = run.ordinal - 1)` to create the
  baseline.
* After `TurnEnded`, the run enters `finalizing`. The hook captures the checkpoint
  `ordinal = run.ordinal`, persists the `checkpoint` row and `CheckpointCaptured` with
  the numstat file list, and then completes the run.
  * Failures are logged as a Notice and the run still completes, with
    `checkpoint_after: None`. Never block the queue on checkpoint failure.
  * Interrupted runs also capture, because the result is the rollback point. Failed
    runs capture too: unlike t3code, we keep diffs for failed runs, because users want
    to see what a failed run changed. Document this choice.
* `threads.diff`:
  * `run_after(N)` vs `run_before(N)` gives the per-run diff;
  * `thread_start` vs `working_tree` gives the overall diff;
  * results are returned as `DiffDto`, with the patch size capped.
* `threads.revert{to_run, restore_files}`:
  1. Validate that no run is active and that the target checkpoint exists.
  2. `restore_files` requires an isolated workspace: the thread has its own worktree,
     or no other non-archived thread uses the same cwd. Otherwise return
     `invalid_state` with an explanation.
  3. Conversation: if `Capabilities::rollback`, call the driver rollback (extend
     `AgentSession` only if needed; prefer resuming the fork point via
     `fork: FromTurn`). Otherwise mark `native = None` so the next attempt starts a
     fresh session with a handoff (section 6).
  4. Emit `RunsReverted`. Later runs get status `reverted` and their checkpoints are
     marked `stale` (refs deleted).

## 4. MCP integration

* `AgentRuntime` starts one `McpServer` lazily on first session open (config
  `mcp_enabled`, default true when agents are enabled).
* `SessionDecorator`: on `OpenSession`, issue or reuse a token for `(scope, thread,
  instance)` with the thread's capability set (from `thread.mcp_capabilities`, or
  defaults per steps/09). Add an `McpServerSpec { name: "semantic", url, bearer,
  stdio_bridge: runtime.bridge_command, tool_timeout_ms: 3_900_000 }`. Revoke on
  session close; touch on every attempt start.
* `InstructionSource`: delegates to `semantic_agent_mcp::instructions`.
* `McpHost` implementation (`mcp/host.rs`) maps tool calls to orchestrator commands:
  * `ask_user` opens a `Questions` request on the caller's thread with
    `ResponseMode::Live` and an origin marker `mcp`. It awaits resolution via a
    oneshot registry keyed by request key, with a timeout. On timeout the request
    becomes `expired` and the tool returns `timed_out`.
  * `thread_*` tools resolve targets **only within the caller's workspace**.
    `thread_read` uses the same snapshot composition as the UI (provider history plus
    live overlay, K12). It reports `history_unavailable` clearly when the target's
    provider has no readable history.
  * `semantic()` returns the `SemanticDataHost` supplied by the embedder through
    `RuntimeHooks::semantic_data_host(scope)` (the app implements it in step 11).
    `None` disables the semantic toolkits.
  * The escalation rule (child access ≤ parent access, plan ≤ default) is enforced
    here too.

## 5. Delegation and completion mailbox

* `delegate_task` creates a child thread:
  * `relationship = delegated` and `parent = caller` (`semantic:parent`);
  * same workspace and cwd (shares the checkout; document this; worktree option
    later);
  * origin `delegation`;
  * the derived id is `derive_id("thread", [caller_session, "delegate", client_request_id])`
    for idempotency.
  The first run's content is the task prompt only. Persist the delegation metadata on
  the **child thread row**, in its `delegation` record (defined in step 06):
  `{parent_thread, parent_run, task_id, mode: async|wait, delivery: pending|delivered|acknowledged|disposed, generation}`.
  While the parent's actor is live, emit a live `Subagent` item on the parent timeline
  (key derived from the child thread id) to show progress. It is not persisted (K4).
  After a restart the parent shows its delegated children through the inbox nesting
  and `thread_list`.
* Completion delivery happens when the child run reaches a terminal state:
  1. Update the parent's live subagent item, if the parent actor is live: status, and
     summary = the child's final agent message truncated to 4 KB, taken from the child's
     live transcript or its provider history.
  2. If the task was `async` and not yet acknowledged, deliver a mailbox message to the
     parent: a `Send` with origin `delegation` and content
     `<delegated_task_result task="…" status="…">summary</delegated_task_result>`,
     using mode `steer` if the parent has an active run whose capabilities support
     native steering, else `queue`. If the parent is idle, it starts a new run (wake).
  3. Delivery state is persisted in the child thread's `delegation` record. The run id
     of the mailbox message is derived from `(task, generation)` as its
     `client_request_id`, so a retried delivery is idempotent. After a restart, pending
     deliveries are re-offered by the recovery pass (step 07 recovery gets a hook for
     this).
* `task_status` with a terminal result acknowledges. `task_cancel` interrupts the
  child and disposes the delivery.
* Delegated threads appear in the inbox under their parent (UI step 14 groups them).

## 6. Context handoff

`HandoffRenderer::render(history: &HistoryPage(s), budget) -> InstructionBlock` builds a
deterministic transcript from the **provider history of the previous native session**
(K12; read via the history cache, paging back as far as the budget needs). It
includes user messages, final agent messages, command summaries (command plus exit
code), file change summaries (paths and stats) and notices. If the old session's
history is unavailable, no transcript can be rendered. The handoff then contains only
a short note with the run summaries from the DB (`input_summary`, `preview`, status),
and the UI shows a Notice that prior context could not be carried over. It works newest-first
until the budget is reached (default 16k tokens ≈ 64 KB; a simple chars/4 estimate),
then reverses into chronological order. It includes a coverage line ("Earlier
history omitted; use thread_read to fetch more"), only when the Threads capability is
attached. Used when:

* resume fails (step 07 retry path, now with context);
* the instance changes mid-thread (`threads.update` with a new instance; allowed only
  between runs). Emit a `Notice` item "Switched from X to Y; prior conversation
  provided as context";
* after a revert without native rollback.

Delivery is via the `TurnInput.context` of the next attempt. Record `handoff_turns` on
the thread (the number of old-session turns covered; the attribute is defined in step
06) so a retry does not double-deliver. If this step needs any other schema change, add a **new
migration** (`002_*`) with frozen definitions. Never edit `001_init` once step 06 has
been committed.

## 7. Title

* Title: on the first message, the title is the first non-empty line, stripped of
  markdown, truncated to 80 chars on a word boundary, unless the user set one.
  `set_thread_title` (MCP) and `threads.update` override it.

## 8. Register real drivers

`AgentRuntime::builder().with_builtin_drivers(opts)` registers claude, codex and acp
(feature-gated) plus fake (opt-in).

## Tests

* Providers:
  * auto-detection with a fake PATH dir containing stub executables (shell scripts
    printing versions);
  * probe cache TTL and refresh;
  * config change rebuilds the instance.
* Workspace:
  * allow-list rejection;
  * git detection;
  * a worktree thread flow with the fake driver in a temp repo: cwd is the worktree;
  * a failure path that cleans up.
* Checkpoints with the fake driver writing files (`/write` keyword or the script
  `write_file` step from step 02):
  * per-run diff contains exactly that run's changes;
  * the thread diff;
  * revert with `restore_files` restores content and marks later runs reverted;
  * revert refused on a shared cwd.
* MCP:
  * a real `McpServer` plus an rmcp client acting as the "agent" (the fake driver can't
    call MCP, so the test calls tools directly with the issued token);
  * `ask_user` round trip through `requests.respond`;
  * `delegate_task` async creates a child thread, the child completes, and the parent
    receives a mailbox run;
  * `task_status` acknowledges, so no duplicate wake;
  * a pending delivery survives an orchestrator restart and is delivered exactly once;
  * escalation denied;
  * a cross-workspace `thread_read` is denied.
* Handoff:
  * renderer budget and ordering table tests;
  * resume failure with the fake driver configured to reject resume: the next attempt
    receives the handoff context, rendered from the old session's fake history file,
    exactly once;
  * old history unavailable (fake `history: false`): the handoff contains only the
    run-summary note, and a Notice is emitted.
* If a `002_*` migration was added: package validation, and an upgrade test from a
  `001`-only DB.

## Acceptance

* Tests green. Each feature is in its own module, and the actor core has only
  extension-point wiring changes.
* Commits, one per sub-group:
  1. "Add provider and workspace services to agent orchestrator";
  2. "Add checkpoints, diffs and revert to agent orchestrator";
  3. "Integrate MCP server, delegation and context handoff into agent orchestrator".
