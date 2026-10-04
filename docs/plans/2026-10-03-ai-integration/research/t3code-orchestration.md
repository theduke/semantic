# t3code: Orchestration, Persistence and Domain Model

Research notes on the orchestration layer of t3code (`/home/theduke/dev/github.com/pingdotgg/t3code`,
read-only). Scope: the "orchestration v2" engine in `apps/server/src/orchestration-v2`, the SQLite
persistence under `apps/server/src/persistence`, the supporting server modules (checkpointing, VCS/git,
terminal, scheduled tasks, pull requests, workspace, environment) and the matching schemas in
`packages/contracts`. Provider adapter internals and the orchestrator MCP server are only referenced
where they touch the domain model.

All paths below are relative to the t3code repo root unless noted. The stack is TypeScript on the
Effect library (services = `Context.Service`, schemas = `effect/Schema`), SQLite via
`effect/unstable/sql`. Line counts are given where size is itself a finding.

Contents

1. Architecture in one page
2. Identity and ids
3. Domain entities (fields and relationships)
4. Event log and event catalogue
5. Commands, dispatch pipeline, idempotency
6. Projections and read models, streaming
7. Effect outbox
8. Run lifecycle (queueing, steering, interrupt, finalization)
9. Restart and crash recovery
10. Provider switching, context handoff, forks
11. Subagents, delegation, notifications, PR links
12. Checkpointing, diff, rollback
13. Workspaces, worktrees, thread launch, cleanup
14. Terminals
15. Scheduled tasks and the scheduler
16. Thread housekeeping (settle, snooze, pin, title, usage limits)
17. Projects
18. "Multi-server" in t3code
19. Persistence details
20. Testing approach
21. Strengths, complexity hotspots, lessons for a Rust design
22. Key file index

---

## 1. Architecture in one page

```
 client (web/desktop/mobile)                     server (one "environment")
 ───────────────────────────                     ─────────────────────────────────────────────────────────────
  dispatchCommand(cmd) ──── RPC/WS ───────────►  Orchestrator.dispatch(cmd)
                                                   │ per-thread keyed lock (ThreadCommandExecutor)
                                                   │ 1. receipt lookup (CommandReceiptStore)  -> replay / reject
                                                   │ 2. dispatchOnce: read projection, CommandPolicy (capabilities),
                                                   │    plan = { events[], effects[], cancelUnsettledEffects? }
                                                   ▼
                                           EventSink.commitCommand  (ONE SQLite transaction)
                                                   │  reserve receipt
                                                   │  append events        -> orchestration_events (global sequence)
                                                   │  apply events         -> orchestration_v2_projection_* tables
                                                   │  enqueue effects      -> orchestration_v2_effect_outbox
                                                   │  finalize receipt (resultSequence)
                                                   │  cancel superseded effects
                                                   ▼ after commit
                                           PubSub publishCommitted(events) + outbox.notifyAvailable
        ▲                                          │                               │
        │ subscribeShell / subscribeThread         │                               ▼
        │ (snapshot @ seq N, then events > N)      │                      EffectWorker (4 workers, leases)
        └──────────────────────────────────────────┘                       provider-turn.start/interrupt/steer/...
                                                                           checkpoint.capture, provider-thread.rollback,
   ProviderAdapter session (Codex/Claude/...)  ◄────────── effect ─────────  terminal.cleanup, thread-title.generate ...
        │  normalized ProviderAdapterV2Event stream
        ▼
   RunExecutionService (forked fiber per run) -> ProviderEventIngestor -> EventSink.write / writeIfRunCurrent
        │                                              (domain events, NOT commands, bypass the Orchestrator)
        └── on turn.terminal: run -> "waiting", enqueue checkpoint.capture effect
            checkpoint.capture effect -> git checkpoint -> run -> "completed"
            run.updated(terminal) subscriber (Orchestrator.handleTerminalRun) -> start next queued run
```

Key properties:

* **Event-sourced with synchronous inline projections.** A single append-only `orchestration_events`
  table holds both *project* events and *thread* (v2) events under one global `sequence`. Projection
  tables are updated in the same transaction as the append. There is no async projector lag.
* **Events are mostly "entity upserted" snapshots**, not fine-grained domain facts: `run.updated`
  carries the full `Run`, `thread.metadata-updated` carries the full `AppThread`, etc. The reducer
  (`applyToProjection`) is "upsert by id". (See section 21 for the consequences.)
* **Commands are thread-scoped and serialized per thread**. Provider output is *not* a command: the
  ingestor writes domain events directly via the event sink, guarded by compare-and-set style
  `writeIfRunCurrent` / `writeIfProviderThreadOwner`.
* **Side effects go through a durable outbox** (leases, retry/backoff, cancellation, process-loss
  reconciliation), with an explicit split between *replay-safe* and *process-bound* effects.
* **Capabilities, not provider names, drive policy** (`CommandPolicy`, `ProviderSwitchService`,
  `ProviderSessionTransitionPolicy`).
* Project domain entities are on a single server ("environment"); there is no cross-server orchestration.

Size of the central files (a complexity signal): `Orchestrator.ts` 9,977 lines, `ProjectionStore.ts`
6,077, `RunExecutionService.ts` 1,467, `ProviderTurnStartService.ts` 1,267, `EventSink.ts` 891,
`EffectWorker.ts` 827, `ProviderRuntimeRecoveryService.ts` 804, `contracts/orchestrationV2.ts` 3,357.

---

## 2. Identity and ids

Principle (`docs/orchestration-v2/entity-ids-and-correlation.md`): *provider ids are evidence, app ids
are identity.* Every durable entity has an app-owned id; provider-native ids are stored as optional
`OrchestrationV2ProviderRef`:

```ts
ProviderRef { driver, nativeId: string|null, strength: "strong"|"weak"|"none",
              fingerprint?: string, ordinal?: number }
```

Id families (branded strings in `packages/contracts/src/baseSchemas.ts`, allocated by
`orchestration-v2/IdAllocator.ts`): `CommandId, EventId, RawEventId, ProjectId, ThreadId, MessageId,
RunId, RunAttemptId, NodeId, ProviderSessionId, ProviderThreadId, ProviderTurnId, RuntimeRequestId,
TurnItemId, CheckpointScopeId, CheckpointId, ContextHandoffId, ContextTransferId, PlanId,
ScheduledTaskId, EnvironmentId`.

`IdAllocator` has two modes:

* `allocate.*` (random via `effect/Random`, so tests can substitute a deterministic source).
* `derive.*` (deterministic from inputs): e.g. `delegatedTaskNode/Thread/Message/TurnItem({commandId})`,
  `providerSession({providerInstanceId})`, `threadFromProviderThread`, `turnItemFromProviderItem`,
  `providerTurn`. Deterministic derivation from a command id is how multi-entity commands stay
  idempotent on replay.

Correlation rules for weak providers: scope every native-id lookup (provider + session + thread),
fall back to scoped ordinals, then fingerprints; never match native ids globally; once an app id is
allocated for an entity it never changes.

Command ids carry meaning by convention (string prefixes), which is the idempotency mechanism for
server-originated commands: `command:system:start-queued:<runId>`,
`command:effect:checkpoint.capture:<runId>`, `command:restart-continuation:<runId>`,
`scheduled-task:<taskId>:<epochMs>:<trigger>`, `limit-resume:<threadId>:<runId>:<resetMs>:<reqId>`,
`limit-arm:...`, `command:runtime-reconcile:...`, `command:restart-prepare:<runId>`.

---

## 3. Domain entities

Contract definitions: `packages/contracts/src/orchestrationV2.ts` (+ `project.ts`, `terminal.ts`,
`scheduledTask.ts`, `threadPullRequest.ts`, `worktreeSetup.ts`, `environment.ts`).

### 3.1 Relationship overview

```
Project (event-sourced, own aggregate)
  └─ AppThread  (aggregate for all v2 events; lineage: fork | subagent -> parent AppThread)
       ├─ ConversationMessage*  (user | assistant | system)
       ├─ Run*  (ordinal, counted user-visible turn; status machine)
       │    ├─ RunAttempt*  (initial | steering_restart | retry | provider_recovery)
       │    └─ ExecutionNode tree  (root_turn > assistant_message|reasoning|tool_call|approval_request|
       │          user_input_request|plan|todo_list|subagent|hook|system)
       │          └─ Subagent (provider_native | app_owned)  ── childThreadId ─► child AppThread
       ├─ TurnItem*  (ordered timeline records: the thing the UI renders)
       ├─ PlanArtifact*  (proposed_plan | todo_list)
       ├─ RuntimeRequest*  (approvals / user input / auth refresh / dynamic tool call)
       ├─ ProviderSession*  (live-or-resumable runtime process)  ◄── shared by many ProviderThreads
       ├─ ProviderThread*   (provider-native conversation handle; several per AppThread)
       │    └─ ProviderTurn* (provider-native turn; linked to RunAttempt + root node)
       ├─ CheckpointScope* ── Checkpoint*  (git ref per scope+ordinal; root_run scope advances run count)
       ├─ ContextTransfer* (fork | provider_handoff | merge_back | subagent_spawn | subagent_result)
       │    └─ ContextHandoff* (materialized portable context payload)
       ├─ ThreadPullRequestLink* (+ snapshot, + optional watch)
       └─ (outside event log) Terminals, WorktreeSetupSnapshot (memory), attachments (files)
ScheduledTask (plain CRUD table) ── fires ─► thread launch (new thread) or message into existing thread
```

### 3.2 Project

Event-sourced under `aggregate_kind='project'`, events `project.created | project.meta-updated |
project.deleted` (`contracts/applicationEvent.ts`). Commands (`ProjectCommands.ts`, plain TS types,
not part of the v2 union): `project.create`, `project.meta.update`, `project.delete`. Receipts live in
the same receipt table keyed by project.

| Field | Type / meaning |
|---|---|
| id | `ProjectId` |
| title, workspaceRoot | name + absolute checkout path (a project = one workspace root) |
| repositoryIdentity | `{canonicalKey, locator{git-remote,name,url}, webUrl?, rootPath?, provider?, owner?, name?}`; correlates clones across environments but never routes work |
| defaultModelSelection | `ModelSelection | null` (`{instanceId, model, options...}`) |
| defaultThreadEnvMode | `"local" | "worktree" | null` (new-thread workspace default) |
| autoPull | bool, opt-in background pull |
| faviconPath, projectIcon | cosmetics |
| scripts | `ProjectScript[]`: `{id, name, command, icon, runOnWorktreeCreate, async?, previewUrl?, autoOpenPreview?}` |
| createdAt, updatedAt, deletedAt | soft delete |

Per-project settings overrides (`PROJECT_SCOPED_SERVER_SETTING_KEYS`: worktreeCleanup,
defaultModelSelection, defaultRuntimeMode, defaultThreadEnvMode, newWorktreesStartFromOrigin,
worktreeSubmodules, branchNaming, ...) are layered over environment settings (`settings.ts`), and an
optional repo file `t3.json` (`t3ProjectFile.ts`, `project/T3ProjectFileLoader.ts`) supplies in-repo
config (setup scripts, thread env mode).

### 3.3 AppThread (the user-visible conversation)

Aggregate root for all v2 events (`stream_id = threadId`).

| Field | Notes |
|---|---|
| id, projectId, title | |
| createdBy: `user|agent|system`; creationSource: `web|mobile|mcp|provider|server` | provenance of every thread/message/run |
| providerInstanceId, modelSelection | currently selected provider *instance* + model (+options); historical runs keep their own |
| runtimeMode, interactionMode | permission mode (e.g. approval-required / full-access) and chat vs plan interaction |
| branch, worktreePath | workspace binding (null worktree = project root) |
| activeProviderThreadId | provider-native thread currently backing the thread |
| lineage `{parentThreadId, relationshipToParent: fork|subagent|null, rootThreadId}`, forkedFrom `{run\|node\|provider_thread ...}` | |
| historyOrigin: `native|v1_import` | |
| linkedPullRequest (legacy), pullRequests[], branchPullRequest | see 11.4 |
| archivedAt, deletedAt, settledOverride (`settled|active|null`), settledAt, unsettledAt, snoozedUntil/At, pinnedAt, pinOrderKey, activeOrderKey (fractional index), autoSettleDisabledAt, lastVisitedAt | UX lifecycle (section 16) |
| limitRecovery `{requestId?, runId, resetAt, autoResume, snooze?}` | usage-limit auto resume |
| titleRegeneration `{requestId, startedAt}` | in-flight title job marker |
| rollbackRequestId, rollbackFailure | last checkpoint rollback outcome |

### 3.4 ConversationMessage

`{id, threadId, runId|null, nodeId|null, role: user|assistant|system, text, context?, attachments[],
streaming, createdAt, updatedAt, createdBy, creationSource, scheduledTaskId?, senderThreadId?,
notification?, delegatedCompletion?{parentRunId, generation, taskIds}}`.
`attachments` are `ChatAttachment` refs to files in the attachment store (not blobs in SQLite);
`context` is structured composer context (file refs, etc.).

### 3.5 Run (the counted turn)

| Field | Notes |
|---|---|
| id, threadId, ordinal (1-based, unique per thread) | |
| providerInstanceId, modelSelection, providerThreadId | each run pins the provider it used |
| userMessageId, rootNodeId, activeAttemptId | |
| status | `preparing | queued | starting | running | waiting | completed | interrupted | failed | cancelled | rolled_back` |
| queuePosition, queueHeld | app-owned queue; held after failures/restart until `queue.resume` |
| requestedAt, startedAt, completedAt | |
| checkpointId, contextHandoffId | |
| restartContinuationOfRunId, workStartedAt | wake/continuation runs |
| restartCancelledBackgroundWork[] | note delivered with the next run |
| sourcePlanRef `{threadId, planId}` | "implement this plan" |
| delegatedCompletion `{disposition: open|stopped|disposed, nextGeneration, delivery{generation,messageId,taskIds}}` | mailbox cohort for subagent results |

Status semantics in practice (differs slightly from the design doc):

* `preparing`: workspace (worktree/setup script) being provisioned; run exists but provider not started.
* `queued`: waiting behind an active run (app-owned queue, ordered by `queuePosition`).
* `starting`: committed, `provider-turn.start` effect pending/running.
* `running`: provider turn live.
* `waiting`: **provider turn finished but run not yet finalized** (checkpoint capture pending). The
  design doc's "waiting for user" meaning is carried instead by pending `RuntimeRequest`s.
* terminal: `completed | interrupted | failed | cancelled | rolled_back`.

### 3.6 RunAttempt

`{id, runId, attemptOrdinal, rootNodeId, providerInstanceId, providerThreadId, providerTurnId,
nativeThreadId?, reason: initial|steering_restart|retry|provider_recovery, status: pending|running|
completed|interrupted|failed|cancelled|superseded, startedAt, completedAt}`.
One run, many attempts; only the active attempt can complete the run. Steering by interrupt+restart
creates a new attempt under the same run and marks the old `superseded`.

### 3.7 ExecutionNode and Subagent

`ExecutionNode {id, threadId, runId|null, parentNodeId, rootNodeId, kind, status (idle|pending|running|
waiting|completed|interrupted|failed|cancelled|rolled_back), countsForRun, providerThreadId,
providerTurnId, nativeItemRef, runtimeRequestId, checkpointScopeId, startedAt, completedAt}`.
Kinds: `root_turn, assistant_message, reasoning, plan, todo_list, tool_call, approval_request,
user_input_request, subagent, hook, system`. Invariant: only the `root_turn` node completes the run.
`idle` = resumable but not keeping a turn alive (background work).

`Subagent {id(=NodeId), threadId, runId, parentNodeId, origin: provider_native|app_owned, createdBy,
driver, providerInstanceId, providerThreadId, childThreadId, nativeTaskRef, prompt, title, model,
completionWake: always|settled_only, completionDelivery{state: pending|claimed|acknowledged|delivered|
disposed, observedByRunId}, status, progress, result, startedAt, completedAt, updatedAt}`.
`provider_native` = spawned by the provider (Claude Agent tool, Codex/Cursor native); not
message-able. `app_owned` = created by the orchestrator (`delegated_task.request`, from the MCP
`delegate_task` tool), backed by a real child `AppThread` with `lineage.relationshipToParent="subagent"`.

### 3.8 ProviderSession, ProviderThread, ProviderTurn

* `ProviderSession {id, driver, providerInstanceId, status: starting|ready|running|waiting|stopped|
  error, cwd, model, capabilities, createdAt, updatedAt, lastError}`. Durable metadata for a
  live-or-recoverable runtime process; the in-memory handle may vanish (restart, idle reap, crash).
  A binding table `orchestration_v2_projection_provider_session_bindings(provider_session_id, thread_id)`
  records which app threads share a session.
* `ProviderThread {id, driver, providerInstanceId, providerSessionId|null, appThreadId|null,
  ownerNodeId|null, nativeThreadRef, nativeConversationHeadRef, status: not_loaded|idle|active|
  archived|closed|error, firstRunOrdinal, lastRunOrdinal, handoffIds[], forkedFrom{providerThreadId,
  providerTurnId?, checkpointId?}, pendingBackgroundTasks[], contextUsage, nativeMetadata, ...}`.
  `appThreadId` set -> first-class backing thread; `ownerNodeId` set -> nested under a node (subagent).
  An AppThread can have several (one per provider instance it has used, plus replacements).
* `ProviderTurn {id, providerThreadId, nodeId, runAttemptId|null, nativeTurnRef, ordinal, status,
  startedAt, completedAt, tokenUsage?, turnTokenUsage?}`.

`pendingBackgroundTasks` is a roster of provider-owned work that can outlive the root turn:
`{kind: subagent|command|monitor|background_task, taskId, description?, childThreadId?}`. Unknown
kinds decode through a fallback arm (`kindUnionWithFallback`) so older clients survive newer servers.

### 3.9 RuntimeRequest (approval / user-input request)

`{id, nodeId, providerTurnId, nativeRequestRef, kind: ProviderRequestKind | dynamic_tool_call |
user_input | auth_refresh, status: pending|resolved|expired|cancelled, responseCapability:
{type:"live", providerSessionId} | {type:"message"} | {type:"not_resumable", reason}, createdAt,
resolvedAt, decision?, answers?}`.
Approvals are therefore *three linked records*: a `RuntimeRequest`, an `approval_request` (or
`user_input_request`) ExecutionNode, and an `approval_request`/`user_input_request` TurnItem. After a
server restart pending requests are marked `expired` + `not_resumable` (provider callbacks are live-only
for most adapters; capability `approvals.approvalCallbacksAreLiveOnly`).
`ProviderApprovalDecision` and `ProviderUserInputAnswers` come from `providerRuntime.ts`;
`UserInputQuestion {id, header, question, options[{label,description,value?}], multiSelect?,
allowCustomAnswer?, required?}`.

### 3.10 TurnItem (the timeline) and PlanArtifact

TurnItems are the *rendered* record; the UI never merges messages/plans/checkpoints itself. Base
fields: `id, threadId, runId, nodeId, providerThreadId, providerTurnId, nativeItemRef, parentItemId,
ordinal, status (idle|pending|running|waiting|completed|failed|cancelled|interrupted), title,
startedAt, completedAt, updatedAt, toolSurface/toolIcon/toolSource`.
Variants (`type`): `user_message` (inputIntent: turn_start|queued_turn|steer|promoted_queued_to_steer),
`assistant_message`, `reasoning`, `proposed_plan`, `todo_list`, `user_input_request`, `file_change`
(diffStr/oldStr/newStr/additions/deletions/changes[]), `command_execution`, `file_search`,
`web_search`, `approval_request`, `checkpoint`, `run_interrupt_request`, `run_interrupt_result`,
`system_notice`, `error` (ProviderFailure + retry), `compaction`, `handoff`, `fork`, `thread_created`,
`subagent`, `dynamic_tool`, `notification`.

Ordering is stored in `orchestration_v2_turn_item_positions (thread_id, turn_item_id, ordinal)` with
`ordinal ≈ run.ordinal * 1_000_000 + n`, so items from different producers (provider, checkpoint,
server) interleave stably; `TurnItemPositionStore.normalize` rewrites ordinals at write time.
`visibleTurnItems` is a derived view for forked threads (`visibility: local|inherited|synthetic`) so a
fork shows inherited source items plus synthetic "forked from" markers without copying rows.

`PlanArtifact` = `proposed_plan {markdown}` | `todo_list {steps[{id,text,status,durationMs?}], explanation?}`
with `status: draft|active|completed|superseded`.

`ProviderFailure {class: usage_limit|provider_error|transport_error|permission_error|validation_error|
unknown, message (≤4096), code, retryable, resetAt?}` and `ProviderRetry {attempt,maxAttempts,
retryDelayMs}` are persisted with `error` items; credentials must be redacted by the producer.

### 3.11 Checkpoint scope / checkpoint

* `CheckpointScope {id, threadId, runId, nodeId, parentScopeId, providerThreadId, kind: root_run|
  subagent|tool|provider_thread|manual, ordinalWithinParent, advancesAppRunCount, cwd, createdAt}`.
* `Checkpoint {id, threadId, scopeId, runId, nodeId, parentCheckpointId, ordinalWithinScope,
  appRunOrdinal|null, ref (git ref), status: ready|missing|error|stale, files[{path,kind,additions,
  deletions}], capturedAt}`.
Only `root_run` scopes (advancesAppRunCount=true) are used in the implementation; nested scopes are
modelled but not yet exercised (see section 12).

### 3.12 ContextTransfer and ContextHandoff

`ContextTransfer {id, type: fork|provider_handoff|merge_back|subagent_spawn|subagent_result,
sourceThreadId, targetThreadId, sourcePoint{threadId, runId?, checkpointId?, turnItemId?,
providerThreadRef?, providerTurnRef?}, basePoint?, sourceProvider(InstanceId), targetProvider,
status: pending|resolved_native|resolved_portable|failed|consumed|superseded, resolution?:
native_fork{providerThreadRef} | portable_context{contextHandoffId} | delta_context | checkpoint_context,
createdBy, createdAt, consumedAt}`.

`ContextHandoff {id, transferId, threadId, targetRunId, fromProviderThreadIds[], toProviderThreadId,
coveredRunOrdinals{from,to}, strategy: delta_since_target_last_seen|fork_delta_summary|
full_thread_summary|checkpoint_summary|manual_context, status: pending|ready|failed|superseded,
summaryMessageId, summaryText, history{messages[], coverage, omittedItems, omittedItemIds},
delivery{nativeThreadId, status: pending|injected|inline, itemIds, omittedItemIds}, ...}`.

### 3.13 Notification (in-thread) and device awareness

`Notification {source: delegated_task{taskIds,childThreadId?} | subagent | command | monitor |
background_task, outcome: completed|failed|cancelled|updated|unknown, summary, detail?}` rides on a
`ConversationMessage` and a `notification` TurnItem. It is an *observed event record*; delivery to
the agent (wake / steer / queue) is a separate backend policy (section 11.3). Device push
("agent finished / needs approval") is a separate path: `AgentAwarenessRelay` projects threads to
`RelayAgentActivityState` and publishes it to the relay (section 18).

### 3.14 Terminal

Not in the event log. `TerminalSessionSnapshot {threadId, terminalId, cwd, worktreePath, status:
starting|running|exited|error, pid, history, exitCode, exitSignal, label, updatedAt, sequence}`;
`TerminalSummary` adds `hasRunningSubprocess`. Terminal ids are client-chosen (`term-N`), scoped by
thread. See section 14.

### 3.15 Worktree setup (ephemeral)

`WorktreeSetupSnapshot {threadId, phase: running|done|failed|cancelled, startedAt, endedAt, branch,
baseRef, worktreePath, setupScript{name,command,terminalId}, stages[{id: fetch|checkout|submodules|
setup-script|agent, status, percent, detail, tail[]}], error, sequence}`; memory only
(`project/WorktreeSetupTracker.ts`), also persisted as a thread activity item at start/end so
other devices can render the outcome.

### 3.16 ScheduledTask

`{id, title, prompt, enabled, schedule: interval{everyMs ≥ 60000} | fixed_time{timeOfDay "HH:MM",
weekdays?}, projectId, threadId|null, workspaceStrategy (root|existing_worktree|worktree{baseRef,...}),
modelSelection, runtimeMode, interactionMode, createdBy, creationSource, createdAt, updatedAt,
nextRunAt, lastRunAt, lastRunStatus: never|running|succeeded|failed, lastRunError, runCount}`.
Table `scheduled_tasks` (plain CRUD, not event-sourced). Section 15.

### 3.17 Thread pull request link

`ThreadPullRequestLink {key{host,repository,number}, url, source: manual|created|agent|stack|
stack-dismissed, snapshot?{state,title,headBranch,baseBranch,isDraft,checksState,reviewDecision,
mergeability,...syncedAt}, stack?{kind:native,layers[]}, watch?}` and
`ThreadPullRequestWatch {startedAt, headSha, failedChecks[], passed, remarksThrough, remarkIds[],
conflicting, wakes}` (what the agent was last told, so each change is reported once). Section 11.4.

### 3.18 Effect (outbox row)

`{id, commandId, threadId, request (union), status: pending|running|succeeded|failed|cancelled,
attemptCount, availableAt, leaseOwner, leaseExpiresAt, createdAt, updatedAt, completedAt, lastError}`.
Section 7.

---

## 4. Event log and event catalogue

### 4.1 Storage

One table `orchestration_events` (shared; extended by migration 055 with
`application_event_version`) with columns: `sequence INTEGER PK AUTOINCREMENT` (global order),
`event_id UNIQUE`, `aggregate_kind` (`project` | `thread`), `stream_id` (projectId/threadId),
`stream_version` (per-stream counter), `event_type`, `occurred_at`, `command_id`,
`causation_event_id`, `correlation_id`, `actor_kind` (`server|provider|user`), `payload_json`,
`metadata_json` (run/node/driver/provider-instance/raw-event ids, client origin), `application_event_version`.
Historic design: a separate `orchestration_v2_events` table (still defined in migration 055 and
back-filled into the shared table); `persistence/Migrations/OrchestrationV2/ApplicationEventSource.ts`
performs that fold. Indexes: by command, by (thread, sequence), (thread, type, sequence), run, node,
raw event, provider instance.

`OrchestrationV2StoredEvent = {sequence, commandId|null, event}`. Envelope fields common to all v2
events (`OrchestrationV2EventBase`): `id, threadId, runId?, nodeId?, driver?, providerInstanceId?,
rawEventId?, occurredAt`.

Raw provider frames are **not** persisted in SQLite (only `rawEventId` pointers); they go to rotating
diagnostic log files (replay fixtures are generated from them).

### 4.2 Event types (`OrchestrationV2DomainEvent`, 40 types, payload = full entity)

| Group | Event types | Payload |
|---|---|---|
| Thread | `thread.created`, `thread.archived`, `thread.unarchived`, `thread.deleted`, `thread.settled`, `thread.unsettled`, `thread.snoozed`, `thread.unsnoozed`, `thread.pinned`, `thread.unpinned`, `thread.pin-reordered`, `thread.active-reordered`, `thread.auto-settle-set`, `thread.visited`, `thread.marked-unread`, `thread.metadata-updated`, `thread.pull-request-synced`, `thread.runtime-mode-updated`, `thread.interaction-mode-updated`, `thread.model-selection-updated`, `thread.provider-switched` | full `AppThread` |
| Run | `run.created`, `run.updated`, `run.background-work-cancelled` (patch), `run-attempt.created/updated` | `Run` / `RunAttempt` |
| Graph | `node.updated`, `subagent.updated` | `ExecutionNode` / `Subagent` |
| Provider | `provider-session.attached/updated`, `provider-session.detached`, `provider-thread.updated`, `provider-turn.updated`, `runtime-request.updated` | entity |
| Content | `message.updated`, `turn-item.updated`, `plan.updated` | entity |
| Checkpoint | `checkpoint-scope.created`, `checkpoint.captured`, `checkpoint.rollback-requested` | entity / `{scopeId, checkpointId, requestedAt}` |
| Context | `context-handoff.updated`, `context-transfer.created`, `context-transfer.updated` | entity |

Project events (separate union): `project.created`, `project.meta-updated`, `project.deleted`.

### 4.3 Observations

* There are only a few *verbs* (created/updated). "What happened" must be inferred by diffing the
  payload with the previous state or by reading the command type (receipt). Terminal detection for
  runs is a subscriber on `run.updated` with a terminal status (`Orchestrator.handleTerminalRun`).
* Reducers do semantic merging for fields written by concurrent producers (`preserveRunRecordedFields`,
  `preserveCompletionDelivery`; finalizers deliberately *omit* `delegatedCompletion` from run
  snapshots "so a newer cohort write is not overwritten by a stale snapshot").
* Unknown event types decode to an `unknown-event` stream item on old clients, which still advance
  their cursor (forward compatibility).
* Event compaction exists (`ProjectionMaintenance.compactEventStore`) and snapshots are rebuilt from
  projections, so the log is not required for steady-state reads. `ORCHESTRATION_PROTOCOL_VERSION=2`
  is enforced on the WS handshake (HTTP 426 on mismatch).

---

## 5. Commands, dispatch pipeline, idempotency

### 5.1 Command catalogue (`OrchestrationV2Command`, sent by clients via `orchestration.dispatchCommand`)

| Area | Commands |
|---|---|
| Thread lifecycle | `thread.create` (optional `importedNativeThread`), `thread.archive`, `thread.unarchive`, `thread.delete`, `thread.metadata.update` (title, branch, worktreePath, `expectedWorktreePath`, `expectedEmpty`, `regenerateTitle`, `limitRecovery`, `linkedPullRequest`), `thread.title.regeneration.complete` |
| Thread UX | `thread.settle`, `thread.unsettle`, `thread.snooze`, `thread.unsnooze`, `thread.auto-settle.set`, `thread.pin`, `thread.unpin`, `thread.pin.reorder`, `thread.active.reorder`, `thread.visit`, `thread.mark-unread` |
| Thread config | `thread.runtime-mode.set`, `thread.interaction-mode.set`, `thread.model-selection.set`, `provider.switch` |
| Messaging / runs | `message.dispatch` (the big one), `run.interrupt` (`holdQueue?`), `queued-message.promote-to-steer`, `queue.resume`, `queued-run.reorder`, `queued-run.cancel`, `queued-run.edit`, `prepared-run.progress`, `prepared-run.release`, `prepared-run.fail` |
| Runtime requests | `runtime-request.respond` (decision / answers / attachments), `thread.user-input.dismiss` |
| Checkpoints | `checkpoint.rollback` (`restoreFiles?`) |
| Forks | `thread.fork` (`sourcePoint: latest_stable | run | checkpoint`), `thread.merge_back` |
| Delegation | `delegated_task.request`, `delegated_task.wake-policy`, `delegated_task.completion-delivery.acknowledge`, `delegated_task.completion-delivery.dispose`, `thread.created.record` |
| Provider sessions | `provider-session.detach` |
| Pull requests | `thread.pull-request.link`, `.unlink`, `.watch`, `thread.pull-request-link.sync`, `thread.pull-request.sync` |
| Notifications | `notification.delivery.accept` |

Server-internal commands (not in the client union, `OrchestrationV2InternalCommand`):
`thread.pull-request-watch.sync` (records watch state and wakes agent in one transaction),
`checkpoint.rollback.fail`, `thread.background-work.settle`, plus `thread.auto-settle` (in the client
union but documented as server-dispatched only; rejected if the thread changed since `snapshotAt`).

`message.dispatch` fields worth noting: `dispatchMode` (`defer_start | steer_active{targetRunId} |
restart_active{targetRunId} | queue_after_active | start_immediately`), `deliveryIntent`
(`auto|steer|restart`, resolved server-side under the thread lock), `titleSeed`, `modelSelection`
(per-message model override = potential provider switch), `sourcePlanRef`, continuation markers
(`restartContinuationOfRunId`, `usageLimitContinuationOfRunId`, `manualContinuationOfRunId`),
`notification`, `delegatedCompletion`, `scheduledTaskId`, `senderThreadId`.

Result of dispatch: `{sequence}` = last committed stored-event sequence for the command; the client
uses it as a reconnect cursor.

### 5.2 Dispatch pipeline (`Orchestrator.dispatchWithReceiptEffect`)

1. Serialize on `threadDispatch.withLock(commandThreadId(cmd))` (`KeyedSerialExecutor<ThreadId>`;
   shared with project deletion). One writer per thread; other threads proceed in parallel.
2. Lookup receipt by `commandId`:
   * `rejected` -> `OrchestratorCommandPreviouslyRejectedError` (a rejection is also idempotent);
   * `accepted` for a *different* thread -> `OrchestratorCommandIdConflictError`;
   * `accepted` for same thread -> return stored `{sequence, storedEvents}` without re-running side
     effects (some commands, e.g. `queue.resume`, additionally re-trigger queue start).
3. `dispatchOnce(command)`: a giant `switch` into ~35 `dispatchXxx` planners. Planners read a *command
   projection* (`getThreadRecords(threadId, [fields...], filter)` — column-selective, bounded reads),
   run `CommandPolicy` checks against provider capabilities, and append to two `Ref` lists
   (`events`, `effects`); `getProjectionWithPendingEvents` folds uncommitted events through
   `applyToProjection` so later planning steps see earlier ones. A plan with zero events is an error
   (except `thread.background-work.settle`, which records a receipt only).
4. On planner failure: `commitRejectedCommand` records a `rejected` receipt in its own transaction.
5. `EventSink.commitCommand` (single SQL transaction): `insertIfAbsent` receipt (resultSequence 0) ->
   `normalizeEvents` (turn-item positions) -> `eventStore.append` -> `applyStoredEvents` (projection
   upserts + `orchestration_v2_projection_metadata.last_sequence`) -> `effectOutbox.enqueue` ->
   `receipts.upsert(accepted, resultSequence)` -> `cancelUnsettled` effects (for interrupts etc).
   After commit: signal cancellations, `notifyAvailable`, `PubSub.publish(events)` (so subscribers
   never see uncommitted data; `commitThenPublish`).
6. Post-commit reactions inside the same dispatch: `queue.resume` -> `startNextQueuedRun`;
   `notification.delivery.accept` / `delegated_task.wake-policy` -> `offerDelegatedCompletionDeliveries`.

Receipts: table `orchestration_command_receipts` (shared; v2 columns `command_id PK, thread_id,
command_type, accepted_at, result_sequence, status accepted|rejected, error`). Project commands use
the same table keyed by project (`ProjectCommandReceiptV2`). Reusing a thread command id for a project
command (or vice-versa) is an error.

### 5.3 `CommandPolicy` (pure, capability-driven)

`CommandPolicyV2` (`CommandPolicy.ts`, 491 lines) takes the thread projection and
`OrchestrationV2ProviderCapabilities` and returns decisions or typed `unsupported` errors:

* `decideMessageDispatch` -> `start_run | steer_active{providerTurnId} | restart_active{interruptProviderTurnId}
  | queue_after_active{activeRunId} | switch_provider`.
* `resolveMessageDispatchIntent` maps client `deliveryIntent` against the *serialized* state: no
  active run -> start; `steer` -> steer_active; `restart` -> restart_active; `auto` while
  preparing/starting -> queue; else `supportsActiveSteering` ? steer : `supportsQueuedMessages` ?
  queue : `supportsSteeringByInterruptRestart` ? restart : queue.
* `decideSteeringExecution` (`active_steering | interrupt_restart`), `ensureInterrupt`,
  `ensureNativeFork`, `decideForkExecution` (`native_fork | portable_context`), `ensureRollback`
  (needs `canRollbackThread`, `providerCanRollbackConversation`, `providerRollbackReturnsSnapshot`),
  `ensureContextHandoff` (strategy-specific).

Provider capability object (`OrchestrationV2ProviderCapabilities`): `sessions, threads, turns,
streaming, tools, approvals, planning, subagents, context, checkpointing, identity, runtimePolicy`
(booleans plus a few enums, e.g. `terminalStatusQuality`, `identity.nativeThreadIds: strong|weak|none`,
`runtimePolicy.enforcement: native|client-boundary`).

---

## 6. Projections and read models

### 6.1 Reducer and tables

`ProjectionStore.ts` (`applyToProjection(projection, event)` is a pure reducer over
`OrchestrationV2ThreadProjection`; the SQL layer `apply` does equivalent per-row upserts and also has
an in-memory variant `layerMemory` for tests). Tables (migration 055 + follow-ups), each with
indexed scalar columns plus `payload_json`:

`orchestration_v2_projection_threads, _runs, _run_attempts, _nodes, _subagents, _provider_sessions,
_provider_session_bindings, _provider_threads, _provider_turns, _runtime_requests, _messages, _plans,
_turn_items, _checkpoint_scopes, _checkpoints, _context_handoffs, _context_transfers` plus
`orchestration_v2_turn_item_positions`, `orchestration_v2_projection_metadata(projection_name,
schema_version, last_sequence)`, `orchestration_v2_thread_launch_workflows(command_id PK, thread_id,
project_id, status, title, worktree_path, branch, setup_committed, thread_committed, message_committed,
last_error)` (resumable multi-step launch), `orchestration_v2_legacy_imports`, and the project
projection (`projection_projects`).

Pattern: hot filter/sort columns are real columns (status, ordinal, requested_at, parent ids,
provider instance); everything else is JSON in `payload_json`; sqlite `json_extract` is used in a few
guards (`writeIfRunCurrent` reads `activeAttemptId` from payload).

### 6.2 Read models exposed to clients

| Read model | Contents | RPC |
|---|---|---|
| `ThreadShell` (sidebar row) | thread meta + `latestRunId/Status`, `activeRunId`, `activityRunStatus`, `status` (`idle` or run status), `lastError(+class)`, `usageLimitResetAt`, `pendingRuntimeRequest` summary, `latestVisibleMessage`, `hasActionableProposedPlan`, `pendingBackgroundTasks[]`, `providerInstanceHistory[]`, `itemCount`, `visibleItemCount` | `subscribeShell`, `getArchivedShellSnapshot` |
| `ShellSnapshot` | `{schemaVersion, snapshotSequence, projects[], threads[], archivedThreads[]}` | `subscribeShell` |
| `ThreadProjection` (detail) | thread, runs, attempts, nodes, subagents, providerSessions/Threads/Turns, runtimeRequests, messages, plans, turnItems, checkpointScopes, checkpoints, contextHandoffs, contextTransfers, visibleTurnItems | `getThreadProjection`, `subscribeThread` |
| Bounded snapshot | full control-plane arrays + a recent window of timeline rows, `historyCursor`, `hasMoreHistory`, `payloadBudgetExceeded` | `subscribeThread({acceptBoundedSnapshot})`, `ThreadHistoryPage` |
| Diffs | `getTurnDiff`, `getFullThreadDiff` (turn-count ranges) | RPC |
| Search | `searchThreads` | RPC |
| `ThreadLaunch` | `launchThread` (create-thread+worktree+first message workflow) | RPC |

### 6.3 Streaming contract (snapshot + cursor)

* Thread stream items: `synchronized | snapshot{snapshotSequence, projection, historyCursor?...} |
  event{sequence, event} | unknown-event`.
* Client either takes a snapshot at sequence N and then streams `sequence > N`, or passes
  `afterSequence` to get a replay of missed events. Replay is bounded
  (`THREAD_RESUME_MAX_REPLAY_EVENTS=128`, 1 MiB encoded / 1 MiB raw payload); beyond that the server
  falls back to a fresh (possibly bounded) snapshot.
* Server-side protection: `LiveStreamBudget` (≤1000 items / 8 MiB retained per subscriber), a
  `ThreadLiveEventCoalescer` (50 ms window, ≤512 pending updates; streaming tool/text updates
  coalesce), `WireProjection` strips heavy payloads (diffs, tool output) from events; shell updates
  refetch the aggregate rather than ship transcript bodies; `afterSequence` shell resume avoids
  re-sending the whole list.
* Race-free replay-to-live via `OrchestrationEventStore.streamApplicationEvents` (subscribe to the
  PubSub before reading the backlog; dedupe by sequence).

### 6.4 Projection maintenance and recovery

* `ProjectionMaintenanceV2`: `verify` (compares `last_sequence`/schema version, reports unreadable /
  missing / unexpected threads), `rebuild` (replays all events), `compactEventStore`.
  `ORCHESTRATION_V2_PROJECTION_SCHEMA_VERSION = 2` bumps force rebuild.
* `getRecoveryThreadIds(kind)` with kinds `queued-runs | runtime | subagent-results |
  delegated-completions` — predicates over the projection (`needsRecovery`) used by startup
  recovery workers.

---

## 7. Effect outbox

Table `orchestration_v2_effect_outbox(effect_id PK, command_id, thread_id, effect_type, payload_json,
status CHECK pending|running|succeeded|failed|cancelled, attempt_count, available_at, lease_owner,
lease_expires_at, created_at, updated_at, completed_at, last_error)` with claim index
`(status, available_at, lease_expires_at, created_at)` and `(thread_id, status, effect_type)`.

Effect request union (`EffectOutbox.ts`):

| Effect | Payload | Executor | Class |
|---|---|---|---|
| `provider-turn.start` | runId | `ProviderTurnStartService.start` (ensure session+thread, deliver handoffs, `RunExecutionService.startRootRun`) | process-bound |
| `provider-turn.interrupt` | sessionId, providerThreadId, providerTurnId | `ProviderTurnControlService.interrupt`, then dispatches `thread.background-work.settle` | process-bound |
| `provider-turn.steer` | …, messageId | `ProviderTurnControlService.steer` (+ tolerant of retry) | process-bound |
| `provider-turn.restart` | …, interruptedAttemptId, runId, sessionTransition? (`replace`/`detach`) | compound interrupt + detach + start | process-bound |
| `runtime-request.respond` | sessionId, requestId, decision/answers | `RuntimeRequestService` | process-bound |
| `provider-runtime.continue` | sourceRunId | `RestartContinuation.continueRestartedRun` | replay-safe |
| `provider-session.detach` | sessionId, detail, `revokeMcpCredential?` | session manager | replay-safe |
| `provider-thread.rollback` | providerThreadId, checkpointId, scopeId, restoreFiles? | `CheckpointRollbackService` | replay-safe |
| `checkpoint.capture` | runId, scopeId | `RunFinalizationService.finalize` -> `CheckpointCaptureService` | replay-safe |
| `terminal.cleanup` | – | `ResourceCleanupService.cleanupTerminals` | replay-safe |
| `attachment.cleanup` | attachmentIds | delete files | replay-safe |
| `thread-title.generate` | initial{messageId} \| regenerate | `ThreadTitleRegenerationService` | replay-safe |

Mechanics (`EffectOutbox.ts`, `EffectWorker.ts`):

* **Claim** is a single `UPDATE ... WHERE effect_id = (SELECT ... ORDER BY available_at, created_at LIMIT 1)`
  setting `running`, `attempt_count+1`, lease owner/expiry (default lease 30 s). Optional exclusion of
  restart continuations.
* **Daemon**: 4 concurrent workers; each worker loops `runOnce`; when idle it sleeps until
  `min(next available_at, 30 s liveness poll)` or an in-process `notifyAvailable` wake-up (post-commit).
* **Retry**: exponential backoff `min(30 s, 100 ms * 2^(attempt-1))`, `maxAttempts=5`, then `failed`.
  `willRetry` is passed to the executor so a step can fail without terminally failing the run until
  the last attempt. Non-retryable classification for benign interrupt races ("not active", "already
  stopped").
* **Cancellation**: `cancelUnsettled(threadId, effectTypes, reason)` inside the command transaction
  (e.g. interrupt cancels queued `provider-turn.start/restart`); running executions race against a
  process-local `awaitCancellation` Deferred.
* **Lease-loss/settlement failures**: replay-safe effects are re-queued; process-bound effects are
  terminalized (never replayed after partial execution).
* **Process loss** (`reconcileAfterProcessLoss`, called by startup recovery): `pending|running`
  process-bound effects -> `cancelled`; `running` replay-safe -> `pending`. Rationale: a process-bound
  effect refers to an in-memory provider session that no longer exists.
* Effect ids are deterministic: `effect:<commandId>:provider-turn.start:<runId>`,
  `effect:checkpoint.capture:<runId>`, `effect:restart-continuation:<runId>`, so re-planning a
  command cannot double-enqueue.

---

## 8. Run lifecycle

### 8.1 State machine (implementation-accurate)

```
                    message.dispatch (workspace prep needed: defer_start)
                          │
                      preparing ──prepared-run.progress/release──► starting
                          │  └─prepared-run.fail──► failed
 message.dispatch ────────┴──────────────────────────────────────► starting ──(provider-turn.start effect ok)──► running
 (idle thread)                                                         │ start failure (last attempt)                │
 message.dispatch (active run, queue)──► queued ──startNextQueuedRun──┘                                            │
                                            │ queue.cancel -> cancelled                                            │ provider turn.terminal
                                            │ held (queueHeld) after failure/restart until queue.resume             ▼
                                                                                                    ┌────── completed? ──► waiting  (barrier:
 running ──runtime request pending (stays "running"; request.status=pending, node waiting)           │                     assistant streams flushed, subagents
 running ──run.interrupt (no live provider turn)──────────────────────────────────► interrupted     │                     terminalized, `checkpoint.capture`
 running ──run.interrupt + provider-turn.interrupt effect ─► provider turn.terminal(interrupted)     │                     effect enqueued in same tx)
 running ──turn.terminal failed/cancelled ───────────────────────────────────────► failed/cancelled │
 waiting ──checkpoint.capture ok ───────────────────────────────────────────────► completed         │
 completed|interrupted|failed|cancelled ──checkpoint.rollback (later ordinal)──► rolled_back        │
 any active ──server restart reconcile──► cancelled (+ optional restart continuation run)            │
```

Notes:

* `completed` is *only* written by `CheckpointCaptureService` (together with `checkpointId`, root node
  completion, and a `checkpoint` TurnItem). `interrupted|cancelled` runs also capture a checkpoint
  (it is the rollback point for the next message) but leave status alone. `failed` runs do not
  capture.
* A run in `waiting` that loses the server before capture is handled by the "waiting + cancelled
  background work" branch of recovery (section 9).
* Blocking statuses (`isBlockingRun`, queue start, deletion, recovery): `preparing|starting|running|waiting`.
  "Live for wake purposes" (`hasLiveRun`) excludes `waiting`, because a run parked at `waiting` is
  post-terminal drain and still needs a wake for delegated results.

### 8.2 Starting a run (`message.dispatch` planner, `ProviderTurnStartService`, `RunExecutionService`)

1. Planner (under thread lock) un-settles/un-snoozes the thread, seeds/regenerates the title (emits
   `thread-title.generate` effect) on first message, validates continuation markers, resolves the
   dispatch mode, and either:
   * emits `message.updated` (user message) + `run.created(starting|preparing)` + `run-attempt.created`
     + root `node.updated` + `checkpoint-scope.created` + provider session/thread placeholders +
     `turn-item.updated(user_message)`, and enqueues `provider-turn.start`; or
   * emits the same with `run.status="queued"` and a `queuePosition`; no effect yet; or
   * steers/restarts an active run (8.4).
2. `provider-turn.start` effect -> `ProviderTurnStartService.start`: read `TurnStartContext` from
   projections, resolve runtime policy (`RuntimePolicy`), open/resume `ProviderSession` through
   `ProviderSessionManager` (idle-timeout residency, pinned while background work pending),
   `ensureThread`/`resumeThread` on the adapter, run **context handoff delivery** if needed (10.3),
   emit `run.updated(running)` + attempt/node/provider-thread/session events in one `write`, then
   `RunExecutionService.startRootRun`.
3. `startRootRun` subscribes to the session's `events` stream **before** `startTurn`, forks a detached
   fiber that routes each `ProviderAdapterV2Event` through `routeProviderEvent`
   (owned thread/provider-thread/turn ids; inherited background items) and `ProviderEventIngestor`
   to domain events written with `EventSink.write*`. `/compact` text is special-cased to
   `compactThread`. If `startTurn` fails, a synthetic failed `turn.terminal` is written.
4. Adapter event vocabulary (`ProviderAdapterV2Event`): `app_thread.created, provider_session.updated,
   provider_thread.updated, provider_turn.updated, node.updated, subagent.updated, message.updated,
   turn_item.updated, runtime_request.updated, plan.updated, turn.terminal` (the last carries
   `status, failure?, retry?, threadDisposition: reusable|broken`).

### 8.3 Queueing

* A message while a run is blocking becomes `queued` (requires `supportsQueuedMessages`;
  otherwise rejected by policy). `queuePosition` provides user reordering (`queued-run.reorder`),
  edit (`queued-run.edit`, replaces text/attachments), cancel, and `queued-message.promote-to-steer`
  (turns a queued message into a steer of the active run; `inputIntent=promoted_queued_to_steer`).
* Delivery order (`QueuedRunOrder.queuedRunsInDeliveryOrder`): delegated-completion (automatic
  wake) messages first, then `queuePosition ?? ordinal`, then ordinal.
* Promotion (`Orchestrator.startNextQueuedRun`) triggers on every terminal `run.updated` event via a
  subscriber on the event stream (skipping `command:runtime-reconcile:*` commands), and on
  `queue.resume`. It refuses when: thread archived/deleted, any blocking run, any `queueHeld` queued
  run, usage-limit blocked, or the previous run failed with a non-validation provider failure on the
  same provider instance (then **all queued runs get `queueHeld=true`** and the user decides). A
  queued run for a different model selection performs a provider switch plan at promotion time
  (10.1). Startup does not auto-start queued runs; recovery holds them until explicit `queue.resume`
  (`resumeQueuedRuns` exists but queue recovery is intentionally manual).

### 8.4 Steering

Three user-level modes (`MessageDispatchMode`), resolved under the thread lock against live capabilities:

* **steer_active** — `provider-turn.steer` effect; same run/attempt/turn; user message is attached as a
  `user_message` item with `inputIntent=steer`. Falls back to `start_immediately` when the target run
  already completed or its root provider turn is completed ("steering too late").
* **restart_active** (interrupt-and-restart; for providers with `supportsSteeringByInterruptRestart`
  and no native steering) — new `RunAttempt(reason=steering_restart)` on the same run, old attempt
  `superseded`, `provider-turn.restart` effect (interrupt + optional session replace/detach + start);
  the superseded attempt's terminal event is suppressed (`shouldFinalizeRun=false`) so it cannot
  complete the run.
* **queue_after_active** — a new run (8.3).

Mailbox deliveries (delegated completions) are never interrupt-restarted; they steer if the active
session supports native steering, else queue.

### 8.5 Interrupt

`run.interrupt {runId, reason?, holdQueue?}`:

* planner emits a `run_interrupt_request` TurnItem and enqueues `provider-turn.interrupt`
  (cancelling unsettled `provider-turn.start|restart` effects of that thread); if the run has no
  active provider turn/session it terminalizes immediately (`interrupted`, attempt, node) and emits
  `run_interrupt_result`;
* the run only becomes `interrupted` when the provider emits `turn.terminal(interrupted)` — the
  interrupt RPC returning is *not* terminal (probe-derived requirement for Codex);
* `holdQueue` sets `queueHeld` on all queued runs so a stop doesn't immediately start the next message;
* after the interrupt effect returns, `thread.background-work.settle` marks leftover background items
  `interrupted`.

### 8.6 Finalization barrier (`RunExecutionService.writeFinalRunEvents`)

On root `turn.terminal`:

1. cascade-terminalize run-owned open subagents/nodes/items (so no forever-running cards);
2. close streaming assistant messages; flush plan artifacts;
3. write attempt terminal, provider thread status (`finalProviderThreadStatus` from
   `threadDisposition`: broken -> `error`), root node/run set to `waiting` (for `completed`) or the
   terminal status (`failed|interrupted|cancelled`), enqueue `checkpoint.capture` in the **same**
   transaction;
4. refresh derived workspace state (`RunFinalizationObserver`: workspace entries, VCS status, PR
   status for the thread branch, `PullRequestService.refreshAfterTurn`).
Background-capable items (`command_execution`, `dynamic_tool`, `subagent`) can outlive the root turn;
ingestion keeps running while any are non-terminal, and a provider thread's
`pendingBackgroundTasks` roster plus `derivePendingBackgroundWork` drive "Waiting" pills in the shell.

### 8.7 Failures and usage limits

* Provider failures become `error` TurnItems with a classified `ProviderFailure`.
* `usage_limit` + `resetAt`: shell exposes `usageLimitResetAt`; `thread.metadata.update{limitRecovery}`
  arms recovery; `UsageLimitRecoveryWorker` (periodic sweep) dispatches
  `message.dispatch{usageLimitContinuationOfRunId, "Continue where you left off."}` once `resetAt`
  passes (identity = threadId:runId:resetMs:requestId; heavy re-validation inside the planner; the
  thread may be snoozed until reset).
* Manual continue: `manualContinuationOfRunId` (only for latest interrupted or usage-limit-failed run).
* Failed run + same provider -> queue hold (8.3).

---

## 9. Restart and crash recovery

Component: `ProviderRuntimeRecoveryService` (`reconcile("startup"|"shutdown")`, `prepareForShutdown`,
`recover`), `RestartContinuation`, `RestartBackgroundNote`, outbox `reconcileAfterProcessLoss`.

Startup/shutdown reconciliation (per thread with `getRecoveryThreadIds("runtime")`), one
`commitCommand("provider-runtime.reconcile")` per thread:

1. Non-terminal runs (`preparing|starting|running|waiting`) -> `cancelled` (queue position cleared);
   attempts, nodes, subagents, provider turns, streaming messages, and non-terminal turn items under
   them -> `cancelled`/non-streaming. Queued runs get `queueHeld=true`.
2. Pending `RuntimeRequest`s -> `expired` (startup) / `cancelled` (shutdown) with
   `responseCapability = not_resumable{reason}`.
3. Background-capable items on already-settled runs and runless provider-native subagent root nodes
   -> `cancelled` (the dead process can never report them); linked subagent rows too.
4. Provider threads `active -> idle`, pending-background rosters cleared; sessions not
   stopped/error -> `stopped`.
5. Cancelled background work is recorded as `run.background-work-cancelled` (a dedicated patch
   event, not a run snapshot, to avoid regressing newer lifecycle writes) and delivered **once** to
   the next provider turn as a "note" (`restartCancelledBackgroundWork`).
6. Process-bound outbox effects of that thread are cancelled in the same transaction;
   replay-safe `running` effects are re-queued.
7. Optionally (setting `continueThreadsAfterServerUpdate`, per-project overridable) enqueue
   `provider-runtime.continue{sourceRunId}`. `restartContinuationRun` is a strict eligibility
   predicate (run was `running` with live strong native thread ref, provider instance unchanged,
   session not errored, a running provider turn existed; or a settled run that lost background
   work). The effect dispatches `message.dispatch("Continue where you left off." or the lost-work note,
   restartContinuationOfRunId)`; the planner re-validates (source run still the latest, thread not
   archived, instance unchanged) and otherwise records an accepted no-op.
8. `prepareForShutdown` snapshots continuation intent while providers are still live
   (`command:restart-prepare:<runId>`), because shutdown reconcile cancels the work.

Other recovery loops at Orchestrator construction: terminal app-owned subagents whose parent has not
been told (`subagent-results`), delegated completion deliveries (`delegated-completions`) and
start-queued (held). Scheduled tasks stuck in `running` are failed with
"Run was interrupted by a server restart." and re-aimed (section 15). Terminal PTYs die with the
process (history persisted separately). Legacy v1 import continues lazily.

Design lesson: provider ingestion is an **in-process fiber**, not a durable consumer, so "crash =
every active run is cancelled and optionally re-prompted", never "resume the stream".

---

## 10. Provider switching, context handoff, forks

### 10.1 Switch planning

`thread.model-selection.set`/`provider.switch`/`message.dispatch{modelSelection}` call
`ProviderSwitchService.plan` -> `{instanceChanged, modelChanged, targetProviderThreadId,
releaseProviderSessionIds, transition}` where `transition` comes from the pure
`decideProviderSessionTransition` (`ProviderSessionTransitionPolicy.ts`):

| Transition | When |
|---|---|
| `reuse` | no change affecting the session (interaction mode is turn-scoped) |
| `switch_model_in_session` | same continuation identity, adapter says `apply_on_next_turn` |
| `restart_and_resume` | instance/runtime-mode/workspace changed with the same driver+continuation key, or adapter says `restart_session` (resume the native thread in a new process) |
| `create_with_handoff` | different driver/continuation identity, or no current session, or adapter says so (new ProviderThread + ContextHandoff) |
| `reject` | target unavailable or adapter refuses |

The adapter contributes `planSelectionTransition(current, target, sessionCapabilities)` returning
`apply_on_next_turn | restart_session | create_with_handoff | reject`. Continuation identity is
`{driverKind, continuationKey}` so two instances of the same driver with the same key can share native
history (e.g. same CLI home).

### 10.2 Thread/provider-thread relationship

An AppThread keeps *all* provider threads it ever used. Example from the docs: runs 1-5 on Codex thread
C1; switch to Claude -> create L1 with `full_thread_summary` handoff H1 (runs 1-5); runs 6-8 on L1;
back to Codex -> resume C1 and deliver `delta_since_target_last_seen` H2 (runs 6-8) with the run-9
message. `firstRunOrdinal/lastRunOrdinal/handoffIds` per provider thread record native and handoff
coverage. Fallback to a fresh thread + full summary when resume fails, settings are incompatible, or
the user asks for clean context.

### 10.3 Handoff content and delivery

Important: the "summary" is a **deterministic transcript rendering**, not an LLM summary.

* `ContextHandoffService.prepareProviderHandoff` selects eligible turn items (user messages, assistant
  messages, command executions, file changes, checkpoints, prior handoffs) via `selectHistory`, bounded
  by a token budget (`ContextHandoffBudget`: default cap 16,000 tokens, 64 KB byte cap, configurable
  via `T3CODE_CONTEXT_HANDOFF_TOKEN_CAP`; budget accounts for target model window, native usage, and
  attachments) and renders with `renderHistory`. Each item is whitespace-collapsed and truncated to
  240 chars (`compactText`) in the fork-delta/legacy variants.
* A handoff stores `history.messages[]`, `coverage` text, omitted item ids, and the **delivery state**
  (`pending → injected | inline`, keyed by native thread id) so a retried start doesn't double-inject
  (`deliverContextHandoffs`: persist before and after injection; an ambiguous `pending` delivery forces
  a fresh native thread).
* Delivery paths: `session.injectHistory` (native history injection, where the adapter supports it)
  or inline preamble text prepended to the user message. If detail doesn't fit the budget, the coverage
  marker tells the agent to recover history via the MCP `t3_thread_read` tool.
* `CommandPolicy.ensureContextHandoff` guards by capabilities (`canConsumeHandoffSummaries`,
  `acceptsSyntheticUserContext`, `supportsDeltaHandoff`, `supportsFullThreadHandoff`).
* Rollback invalidates handoffs that cover rolled-back runs (design doc; marks `superseded`).

### 10.4 Fork and merge-back

* `thread.fork {sourceThreadId, targetThreadId, sourcePoint: latest_stable|run|checkpoint}`:
  `ThreadForkService.plan` creates the target AppThread (`lineage.relationshipToParent="fork"`,
  `forkedFrom`), a `ContextTransfer(type=fork, status=pending)`, and a `fork` TurnItem. No provider
  session or thread is created. Forkable source run statuses: completed, waiting, failed,
  interrupted, cancelled (not in-progress or rolled_back).
* First message on the fork resolves the transfer lazily: `decideForkExecution` -> `native_fork`
  (same provider instance, strong native source, `canForkThread`, `canForkFromTurn` if mid-thread,
  source run completed/waiting) else `portable_context` (ContextHandoff `full_thread_summary`).
  `visibleTurnItems` of the fork inherit source items up to the fork point.
* `thread.merge_back`: `ContextTransfer(type=merge_back, basePoint=S, sourcePoint=F)`; the next
  message in the source thread carries a `fork_delta_summary` handoff. Queued merge-back consumption
  is explicitly unimplemented (rejected when the thread has an active run).

### 10.5 Session residency

`ProviderSessionManager`: open sessions keyed by provider session id; idle release after a timeout
(`idle_timeout`), deferral capped by `maxIdlePinMs` while the adapter reports pending background work
(`hasPendingBackgroundWork[ForThread]`); `provider-session.detach` command/effect for explicit
release (thread archive/delete also revokes the thread's MCP credential). Recovery of an evicted
session is the normal path (`resumeThread` from the durable native ref).

---

## 11. Subagents, delegation, notifications, PR links

### 11.1 Provider-native subagents

Adapters emit `subagent.updated` + a `subagent` TurnItem + (for providers with thread ids)
`app_thread.created` for the child thread (`lineage.relationshipToParent="subagent"`,
`creationSource="provider"`). Child provider turns complete independently and never close the parent
run (invariant 3). Such threads are read-only (`OrchestratorSubagentThreadReadOnlyError`). Pending
approvals from subagents are still respondable if the response capability is live.

### 11.2 App-owned delegated tasks

`delegated_task.request {parentThreadId, parentRunId, parentNodeId, task, title?, modelSelection,
runtimeMode, interactionMode, completionWake?}`:

* validates the parent run is blocking and the node belongs to it;
* derives child ids deterministically from the command id (`delegatedTaskNode/Thread/Message/TurnItem`);
* emits: child `thread.created` (subagent lineage, `creationSource: mcp`), `Subagent(app_owned,
  status=running)`, a `subagent`-kind `ExecutionNode`, a `subagent` TurnItem on the parent, a
  `ContextTransfer(subagent_spawn)`, the child's first user message + run (via the normal dispatch
  machinery);
* on child terminal (`handleTerminalRun` -> `finalizeAppOwnedSubagent`, under the **parent** lock):
  updates the Subagent (status/result), writes a `ContextTransfer(subagent_result)`, and runs the
  delegated-completion mailbox.

### 11.3 Delegated-completion mailbox ("notifications to the agent")

State on `Run.delegatedCompletion` (cohort per parent run: `disposition open|stopped|disposed`,
`nextGeneration`, `delivery{generation, messageId, taskIds}`) and per task
`completionDelivery.state: pending → claimed → delivered | acknowledged | disposed`.

* `offerDelegatedCompletionDeliveries` picks terminal tasks whose wake policy allows it
  (`completionWake: always` or parent has no live run) and creates the next generation's message id.
* `ProviderContinuationService` worker drains `ProviderContinuationRequests` (offered by adapters,
  e.g. a Claude background-task wake, or by the orchestrator) and dispatches an internal
  `message.dispatch{dispatchMode: queue_after_active, createdBy: agent, creationSource: server,
  delegatedCompletion{parentRunId, generation, messageId}}` with retry/backoff (100 ms … 5 s).
* The planner re-reads the cohort under the lock and either steers the active session (if it
  supports native steering and every task is `always`), or queues; stale offers are dropped.
* `notification.delivery.accept` = provider accepted the mailbox delivery (distinct from the agent
  reading the result via the `task_status` tool, which acknowledges).
* Delivery is **at-least-once** ("provider acceptance and our receipt cannot commit atomically");
  stable message ids prevent duplicate timeline items (`isUndeliveredMailboxSteer`).
* Background work reports (`Notification.backgroundWorkNotification`) produce human summaries like
  `Subagent "x" finished`, `3 commands failed` with combined outcome rules.

### 11.4 Pull requests

* Linking: `thread.pull-request.link|unlink` (source manual|created|agent|stack), legacy
  `linkedPullRequest` kept for old clients (capability flags negotiate; see section 18).
* `PullRequestSyncReactor`: syncs link snapshots (state, checks, mergeability, stack layers) on events
  and every 15 min; looks for `gh pr merge|close` style shell commands in command items to resync.
  Dispatches `thread.pull-request-link.sync` / `thread.pull-request.sync` (with an `expected` guard
  block so racing branch/worktree changes reject the sync).
* `PullRequestWatchReactor`: agent tool `watch_pull_request` -> `thread.pull-request.watch`; a
  minute sweep reads each watched PR; when checks finish on the head commit, someone else comments, or
  the branch starts conflicting it dispatches `thread.pull-request-watch.sync` which **records the new
  watch state and enqueues a notification wake message atomically** (so each change is reported
  once). Limits: 15 consecutive read failures end the watch; comment-only wakes capped (bots cannot loop
  it); merged/closed PR ends the watch; settled/archived threads are skipped.
* PR providers are abstracted (`pullRequest/PullRequestProvider*`: GitHub, GitLab, Bitbucket, Azure
  DevOps, Forgejo) over `sourceControl/*` CLIs/APIs, with a read cache and rate-limit budgeting.

---

## 12. Checkpointing, diff, rollback

### 12.1 Mechanism (git hidden refs, isolated temp index)

`checkpointing/CheckpointStore.ts` delegates to the VCS driver's `checkpoints` capability
(`vcs/GitVcsDriver.ts`, only git implements it). Capture:

1. temp index file under the repo's common dir (`t3-checkpoint-index-<uuid>`, `GIT_INDEX_FILE`),
2. `read-tree HEAD` into it (sparse-checkout aware), `git add -A -- .` (retry excluding nested
   repositories without commits),
3. `write-tree` -> `commit-tree` (author "T3 Code") -> `update-ref refs/t3/orchestration-v2/checkpoints/<sha256(scopeId)[:32] base64url>/ordinal/<n> <commit>`,
   with `core.fsync=objects,reference` (unclean-restart safety).
The working tree and the user's real index are untouched. This snapshots tracked + untracked,
non-ignored files; ignored files are not captured.

Restore: `git restore --source <commit> --worktree --staged -- .` (only when the checkpoint tree has
files), `git clean -fd -- .`, `git reset --quiet -- .`; missing ref optionally falls back to `HEAD`.
Diff: `git diff --patch|--numstat -z <from>^{commit} <to>^{commit}` with output caps and optional
`--ignore-all-space`; `fallbackFromToHead`. Delete: `update-ref -d` (best effort). Per-`cwd` semaphore
(`withWorkspaceLock`) serializes capture/restore. A checkpoint for a non-git cwd is "not checkpointable"
(status `missing`).

### 12.2 Ordinal scheme

Ref per `(scopeId, ordinalWithinScope)`; for the root scope `ordinalWithinScope = run.ordinal` and
`appRunOrdinal = run.ordinal`. Ordinal 0 = thread-start baseline. Just before the provider turn starts,
`RunExecutionService.startRootRun` calls `captureBaseline(ordinal = run.ordinal - 1)`, which only
captures if that ref does not exist yet (for run N>1 the previous run's post-capture ref already
serves as the baseline; for run 1 this creates the ordinal-0 snapshot). A failed baseline capture is
logged and the turn starts anyway. At finalization `materializeBaselineCheckpoint` only builds the
`Checkpoint` record for an already existing baseline ref (`ready` vs `missing`). So turn diff N =
diff(ordinal N-1, ordinal N). `CheckpointDiffQuery.getTurnDiff({threadId, fromTurnCount, toTurnCount})` maps turn
counts to refs, verifies both projection status and ref existence, and calls `diffCheckpoints`.
`getFullThreadDiff` = from 0. Per-run `files[]` summaries (numstat) are stored on the Checkpoint and
emitted as the `checkpoint` TurnItem.

### 12.3 Capture flow

Triggered by the `checkpoint.capture` effect enqueued in the finalization transaction (section 8.6).
`CheckpointCaptureService.execute`: idempotent (a run that already has `checkpointId` returns; a
`rolled_back` run is skipped), requires run `waiting` (or stopped), then emits in **one** commit:
baseline checkpoints (if missing), `checkpoint.captured`, a `checkpoint` TurnItem, `run.updated`
(`completed`, `checkpointId`), root `node.updated(completed)`. Retries via outbox backoff; failure
leaves the run in `waiting` (visible, retriable).

### 12.4 Rollback

`checkpoint.rollback {scopeId, checkpointId, restoreFiles?}` (planner `dispatchCheckpointRollback`):
requires an active provider thread with session, capability `ensureRollback` (provider can roll back
conversation **and** return a snapshot), checkpoint `ready` and in scope, the target's provider turn
to belong to the active provider thread, and — when restoring files — an *isolated* workspace
(`CheckpointRestoreSafety.isCheckpointRestoreIsolated`: thread has its own worktree and no other
non-deleted thread/session/project root overlaps that path; checkpoints snapshot the whole checkout).
It sets `rollbackRequestId` on the thread, emits `checkpoint.rollback-requested`, and enqueues
`provider-thread.rollback`. Executor `CheckpointRollbackService.execute`:

1. re-validate; open the provider session;
2. build a `ProviderAdapterV2RollbackTarget` (`thread_start` for ordinal 0 or `provider_turn`);
3. `session.rollbackThread` (returns authoritative snapshot) when there are runs after the target;
4. `checkpoints.restore` (files) -> git restore/clean;
5. delete stale refs for later checkpoints;
6. commit events: provider-thread snapshot (lastRunOrdinal), later checkpoints -> `stale`, later runs
   and root nodes -> `rolled_back`.
Failure after all retries -> internal `checkpoint.rollback.fail` stored as `rollbackFailure`
(only the latest rollback request's failure is recorded). Rolled-back turns remain in history
(audit) but are excluded from later provider-turn counting.

Gap: nested `CheckpointScope`s (subagent/tool/manual) are in the model and docs but the live code
creates only root-run scopes.

---

## 13. Workspaces, worktrees, thread launch, cleanup

### 13.1 Workspace strategies (`OrchestrationV2ThreadLaunchWorkspaceStrategy`)

`root{branch?}` (project root; Scratch threads at root get a managed scratch folder,
`ManagedProjectFolders`), `existing_worktree{worktreePath, branch?}`,
`worktree{baseRef, branch?, startFromOrigin?}`. Thread env mode `local|worktree` default per
project/env/t3.json.

### 13.2 Launch workflow (`ThreadLaunchService`, RPC `orchestration.launchThread`)

Resumable, idempotent (keyed on `commandId`, rows in `orchestration_v2_thread_launch_workflows`):
`resolve-project -> read-receipt -> generate-metadata (title/branch name via TextGeneration) ->
provision-worktree -> run-setup-script -> create-thread -> update-thread -> dispatch-message ->
release-run | fail-run`.

* The thread and first run are created **immediately** with `run.status = "preparing"`
  (`message.dispatch{dispatchMode: defer_start}`); preparation runs in a background scope
  (`prepareInBackground`) reporting `prepared-run.progress{phase: worktree|setup}`, then
  `prepared-run.release` (-> `starting`, enqueue `provider-turn.start`) or `prepared-run.fail`
  (failure item + failed run). This lets the UI show a thread instantly.
* Worktree creation: temporary branch `t3code/<hash>` first (never blocks on naming), `git worktree
  add` through `GitWorkflowService.createWorktree` with progress parsing (`Updating files: N%`),
  submodule policy (`recursive|top-level|none`), optional `startFromOrigin` (fetch), then optional async
  rename to a generated semantic branch name. Setup script (project scripts with `runOnWorktreeCreate`
  or `t3.json`) runs in a terminal (`ProjectSetupScriptRunner`); `async` scripts don't block the agent
  start. Progress is tracked in `WorktreeSetupTracker` (stages fetch/checkout/submodules/setup-script/
  agent) and mirrored into a thread activity item.
* `withWorkspaceLease(cwd, effect)`: process-wide per-cwd semaphore coordinating checkout removal vs
  startup across threads.

### 13.3 Worktree and thread cleanup

* Thread delete (`ThreadDeletion.planThreadDeletion`, shared with project removal): `thread.deleted`,
  terminalizes active runs/requests/subagents (events), enqueues `provider-session.detach`
  (+ MCP credential revoke), `terminal.cleanup`, `attachment.cleanup`.
* Worktree removal is policy-driven by `storageCleanup.ts` (worker) with `WorktreeCleanupRules`
  `{worktreeAfterDays, worktreeOnMerge, worktreeOnDelete, worktreeUnchanged}` (env + per-project
  overrides): only for worktrees managed by the server, skipped when threads are active/queued,
  terminals running, or a workspace lease is held; uses `GitVcsDriver.removeWorktree`/`pruneWorktrees`.
  Capability flag `projectWorktreeCleanup`.
* `git` high-level flows (`git/GitManager.ts`, `GitWorkflowService.ts`): stacked actions
  (commit/push/PR), `preparePullRequestThread` (checkout a PR into a worktree thread), branch
  rename, remote resolution. `vcs/*` is the driver abstraction (`VcsDriver`, `VcsDriverRegistry`,
  `VcsStatusBroadcaster`) — git is the only implemented driver but checkpoints/review are routed through
  the driver interface.
* `review/ReviewService` provides diff preview/file contents for arbitrary workspace diffs
  (separate from checkpoint diffs), constrained to the workspace root.

---

## 14. Terminals

`terminal/Manager.ts` (`TerminalManager`) + `PtyAdapter` (`NodePtyAdapter`, node-pty) + `OutputProtocol`.

* Terminals are **thread-scoped** (`threadId`, `terminalId` client-chosen `term-N`), owned by the server
  process, shared by all attached clients (`terminal.open|attach|write|resize|clear|restart|close`).
* `attach` returns an event stream: `snapshot` then `output|exited|closed|error|cleared|restarted|
  activity` events with a `sequence`; a separate metadata stream (`snapshot|upsert|remove` of
  `TerminalSummary`) drives sidebar badges (label from foreground subprocess, `hasRunningSubprocess`).
* Retention: server history capped at 5,000 lines and 8 MiB per terminal (evict oldest, never split
  code points); incremental append; persisted to history files with coalesced writes; restore reads only
  a bounded tail; query/response terminal control traffic stripped from retained history so replays
  don't answer device queries.
* Env: per-provider-instance environment (`providerInstanceId`) is merged so a terminal can launch a
  provider CLI with the same home/credentials as the agent; `PATH` merging for managed binaries;
  process table integrated with `resourceTelemetry` for CPU/RAM per terminal.
* Used by: user shells, project script runs, worktree setup scripts (`terminalId` in
  `WorktreeSetupSnapshot`), `terminal.cleanup` effect on thread delete, storage cleanup (skip running).
* Not event-sourced; cwd validated with typed errors (`TerminalCwdNotFound/NotDirectory/Stat`).
* Renderer is client-side (libghostty-vt on web/Android via WASM/C ABI) — irrelevant for a Rust server
  except that server strips protocol noise from history.

---

## 15. Scheduled tasks and the scheduler

* `scheduling/Scheduler.ts`: one 5 s clock; sources `register(name, runDueWork)`; each source runs with a
  `withPermitsIfAvailable(1)` semaphore (never overlaps itself, no missed-tick backlog), failures logged.
  Registered sources (found in code): `scheduled-tasks` and `usage-limit-recovery`; other sweeps
  (PR watch, PR sync, settlement, storage cleanup) run their own loops.
* `scheduledTasks/ScheduledTaskService.ts` + `Schedule.ts`: CRUD (`scheduledTasks.list|upsert|setEnabled|
  delete|runNow`) over table `scheduled_tasks` (index on `(enabled, next_run_at)` where enabled).
  `nextRunAt(task, now)` computes from local wall-clock (`interval` or `fixed_time` with weekdays).
  Intervals have a 60 s write minimum (older sub-minute rows still readable).
* `runTask(task, trigger scheduled|manual)`: in-memory `activeRuns` set rejects overlap (manual overlap =
  error); re-reads the row (deleted/paused/postponed tasks must not fire); `markRunning`; builds
  deterministic ids `scheduled-task:<taskId>:<startMs>:<trigger>` (command) and
  `scheduled-task-message:...`; then:
  * `threadId == null` -> `ThreadLaunchService.launch({workspaceStrategy, initialMessage{scheduledTaskId}})`
    creating a **new thread per run**;
  * else `ThreadManagement.sendToThread({mode: "auto", scheduledTaskId})` into a fixed thread (steer /
    start / queue chosen by the normal policy).
  Result is recorded via `markCompleted` guarded by `startedAtIso` (a task deleted and recreated mid-run is
  not stamped); `next_run_at` recomputed from the *current* schedule; `run_count` always increments.
* Failure semantics: `releaseStuckRun` on any error between mark-running and mark-completed; startup
  sweep fails rows stuck in `running` ("interrupted by a server restart"), advancing `next_run_at`
  (dispatch may already have happened, so no refire).
* Missed runs: a due fixed-time run long past its slot (server off/asleep) is skipped and rescheduled,
  not fired late (`isMissedFixedTimeRun`).
* The dispatched user message carries `scheduledTaskId`, `createdBy/creationSource` of the task creator.
  Whether a run "succeeded" only means the dispatch was accepted, not that the agent finished.
* Tasks are also manageable by agents through the MCP tools and mobile settings (UI aggregates across
  environments client-side).

---

## 16. Thread housekeeping

* **Settlement** (`ThreadSettlementService`): `settledOverride (settled|active|null)` + `settledAt`;
  automatic settle (`thread.auto-settle` internal dispatch with `snapshotAt` optimistic guard) driven
  by rules: idle/age (days), linked PR merged/closed, no pending requests/queued turn start (2 min
  grace `QUEUED_TURN_START_GRACE_MS`), `autoSettleDisabledAt` opt-out. Sending a message un-settles and
  un-snoozes the thread. Settled threads are excluded from background sweeps (`unsettledOnly`).
* **Snooze**: `snoozedUntil`; wakes only when a user acts (or limit-recovery logic accounts for it).
* **Pin / order**: `pinnedAt`, fractional-index `pinOrderKey` / `activeOrderKey` (a drag writes one key to
  one thread; clients compute keys).
* **Visited/unread**: `lastVisitedAt` is monotonic max (replays and out-of-order deliveries can't move it
  back); visits don't bump `updatedAt`.
* **Title generation**: first message seeds `titleSeed`, `thread-title.generate{initial}` effect uses
  `TextGeneration` (a small model call via configured provider); `regenerate` is a user action;
  `titleRegeneration{requestId}` marks in-flight work; completion command
  `thread.title.regeneration.complete`.
* **Thread search**: `ThreadSearch` + RPC `searchThreads`.
* **Legacy import** (`orchestration-v2/legacy/LegacyV1ThreadImporter`): copy of v1 sqlite into
  `statev2.sqlite`; shells imported at startup, transcripts lazily; first continuation uses a
  32,000-char transcript handoff. Not relevant for a greenfield Rust port.

---

## 17. Projects

* Aggregate with its own tiny event set and `ProjectStore` projection (row per project with `deleted_at`).
* `ProjectService` (`project/`): add/remove/mutate, repository identity resolution
  (`RepositoryIdentityResolver` from git remotes, `canonicalKey` normalizing hosts such as Azure DevOps
  variants), favicon/icon resolution, scripts, clone tracking (`ProjectCloneTracker` rejects thread
  commands during clone), enrichment (`ProjectEnrichmentService`), importing external agent sessions
  (`AgentSessionScanner/Importer`: discovers Codex/Claude native sessions on disk and creates threads with
  `importedNativeThread` strong refs), t3.json loader.
* Project deletion: removes/terminalizes all its threads under per-thread locks using the same
  `planThreadDeletion`.
* Workspace layer (`workspace/`): path safety (`WorkspacePaths` — relative path resolution that rejects
  escape), entries listing/search index (`WorkspaceEntries`, `WorkspaceSearchIndex`), file reads/writes
  (`WorkspaceFileSystem`), `workspaceLease`.
* Environment (`environment/`): persistent `EnvironmentId` file (atomic publish, recovery file), label,
  platform descriptor, machine kind.

---

## 18. "Multi-server orchestration" in t3code

What it is **not**: a cross-server scheduler or distributed event log. Each server is an independent
*environment* that owns providers, files, terminals, git and all durable state
(`docs/internals/remote.md`, `overview.md`). A project and its threads belong to exactly one
environment; repository identity may correlate clones across environments but never routes work.

What exists:

* **Client-side multi-environment**: `packages/client-runtime` keeps one connection owner per
  environment (supervisor with jittered exponential backoff capped at 5 min, registry scoped by
  `EnvironmentId`, shell + thread subscriptions with replay cursors cached 5 min). `ScopedProjectRef
  {environmentId, projectId}` / `ScopedThreadRef` address entities across environments in the UI.
  Settings can fan out to "all environments" (bulk edit, not sync).
* **Reachability layers** (do not change execution): direct URL, Tailscale, SSH tunnel (desktop launches/
  forwards, owns remote server lifecycle only if it launched it), hosted web (pairing secret in URL
  fragment), and **T3 Connect** (Clerk identity + a relay Worker in `infra/relay` that brokers
  environment links, mints one-time DPoP-bound bootstrap credentials from the environment, and
  allocates Cloudflare tunnels; relay never proxies app traffic or sees session tokens).
* **Capability negotiation**: `ExecutionEnvironmentDescriptor {environmentId, label, platform,
  serverVersion, orchestrationProtocolVersion, capabilities{~40 optional booleans}}`. Clients,
  servers and mobile apps upgrade independently, so behaviour (e.g. PR multi-link) is negotiated per
  environment, not per client version. WS handshake enforces `orchestrationProtocol=2`.
* **Agent awareness relay** (`relay/AgentAwarenessRelay`): the server projects thread state
  (`projectThreadAwarenessV2`) into `RelayAgentActivityState {phase, headline, detail, deepLink, ...}`
  and publishes it signed (JWT with the environment key) to the relay for mobile push/live activities.
  Event filter `shouldPublishAgentAwarenessEvent`, drainable worker, catch-up publish on link.
* **Orchestrator MCP**: agents inside a thread call back into the *same* server (`delegate_task`,
  `t3_thread_*`, `create_threads`, `task_status`) — "orchestration" within one environment, authenticated
  by per-thread MCP credentials. Cross-environment delegation does not exist.

For our later multi-server design: keep the single-owner-of-state-per-environment model; put
identity (`EnvironmentId`), protocol version and capability descriptor at the connection boundary.

---

## 19. Persistence details

* SQLite via `node:sqlite` wrapper; pragmas: `busy_timeout=5000`, `foreign_keys=ON`, `journal_mode=WAL`,
  `journal_size_limit=32 MiB`; migrations run on boot (`runMigrations`, numbered 001..056, forward-only,
  "ledger by id" — the migrator compares ids only, so forked migration ids are a documented hazard).
  Memory variant for tests (`SqlitePersistenceMemory`).
* Migration 055 (OrchestrationV2) composes sub-migrations (`persistence/Migrations/OrchestrationV2/*`):
  Foundation (driver / provider_instance_id write-through columns, effect outbox, positions, projection
  metadata), ApplicationEventSource (folds v2 events + project baseline into `orchestration_events`),
  ProviderSessionBindings, Subagents, ScheduledTasks, ThreadLaunchWorkflows, EffectCancellation (adds
  `cancelled` status via table rebuild), RecoveryIndexes, ShellIndexes, LegacyV1ImportState.
* JSON payloads are validated with the same `effect/Schema` as the wire (`...Json` mapped schemas with
  `withDecodingDefault` for fields added later) — schema evolution is "optional + decoding default"
  forever, not versioned upcasters.
* Files outside SQLite: attachments (`attachmentStore`, content-addressed-ish by id; claims during
  message intake: `AttachmentClaims`), terminal history files, provider raw-event rotating logs,
  checkpoints (inside the repository's `.git` as refs), per-provider homes.
* Other tables in the same DB: auth (sessions, pairing links), provider session runtime
  (`ProviderSessionRuntime`), PR "files viewed", usage.

---

## 20. Testing approach

(`docs/orchestration-v2/testing-strategy.md`)

* Prefer a few replay-backed integration tests over mocked unit tests: real Orchestrator, real adapter
  normalizer, real event sink/projection store (in-memory SQLite or `layerMemory`), real checkpoint
  policy. Only substitutes: provider transport (NDJSON transcripts of raw frames with
  `expect_outbound` / `emit_inbound` entries; fixtures derived from real Codex/Claude runs, e.g.
  `CodexReplayFixtures.integration.test.ts`), Effect `TestClock`, deterministic `Random`, temp
  filesystem/worktrees, process supervisor fakes only for process-failure tests.
* Forbidden in integration tests: mocked orchestrator/adapter/normalizer/event sink/reducer/checkpoint
  policy.
* Many `*.test.ts` siblings for pure policy modules (CommandPolicy, QueuedRunOrder, session transition,
  handoff budget) plus targeted regression tests for races (`ProjectionSettlement`, `RunExecutionService`,
  `EffectWorker`, `ThreadLiveEventCoalescer`, memory-profile tests `*.memory.test.ts`).
* `V1ImportBoundary.test.ts` and `Orchestrator.migration.test.ts` guard the legacy cutover.

---

## 21. Strengths, complexity hotspots, lessons for a Rust design

### 21.1 Strengths worth copying

1. **One transaction for receipt + events + projection + outbox**, with publish after commit. Gives
   exactly-once state transitions, idempotent retries by command id, and no projection lag.
2. **Capability-driven policy** as pure functions over a serializable capabilities struct; provider
   names never appear in orchestration logic. Adapters report `planSelectionTransition`.
3. **App ids primary / provider refs secondary** with an explicit native-ref `strength`; deterministic id
   derivation for idempotent multi-entity commands.
4. **Separation of Run vs RunAttempt vs ProviderTurn vs ExecutionNode** — lets steering-by-restart,
   retries, subagents and background work coexist without redefining "turn".
5. **Finalization barrier** (run is `waiting` until checkpoint/streams/subagents are settled; only then
   `completed`) and "root node alone completes the run".
6. **Durable outbox with replay-safe vs process-bound classification** and process-loss reconciliation.
7. **Snapshot + cursor streaming** with bounded replay, fallback to bounded snapshots, coalescing, and
   per-subscriber memory budgets.
8. **Git-ref checkpoints with a temp index**: cheap, no working-tree/index mutation, diffs via plain
   `git diff`, rollback via `restore/clean/reset`, refs hidden under a private namespace, fsync'ed.
9. **Handoffs are first-class, auditable artifacts** with delivery state (no hidden prompt hacks);
   deterministic rendering with token budgets (no extra LLM call needed).
10. **Lazy fork resolution** (record lineage + source point; resolve native vs portable on first run).
11. **Unknown-kind tolerance** for rolling upgrades (fallback union arms, `unknown-event`) and
    **capability descriptor** negotiation per environment.
12. **Restart story is explicit**: cancel in-flight work, expire requests, keep queue held, optionally
    auto-continue with a server-authored prompt, deliver "what we lost" notes once.
13. **Replay-fixture testing** of the whole pipeline with only the transport faked.

### 21.2 Complexity hotspots / things we should avoid reproducing

1. **God modules.** `Orchestrator.ts` (~10k lines) holds the dispatch switch, queue promotion,
   delegated-completion mailbox, subagent finalization and ~35 command planners in one closure;
   `ProjectionStore.ts` (~6k) mixes the reducer, SQL, shell derivation, recovery predicates, history
   paging. Use one module per aggregate behaviour (decide/evolve), not one per layer.
2. **"Entity upserted" events** carrying whole entities. Writers must read-modify-write, so concurrent
   producers (ingestor vs commands vs finalizers) can overwrite each other; the code compensates with
   ad-hoc rules (`preserveRunRecordedFields`, omitting `delegatedCompletion`, `writeIfRunCurrent`,
   `writeIfProviderThreadOwner`, `guardPendingUserInputCancellations`, dedicated patch events like
   `run.background-work-cancelled`). Prefer typed deltas / state-transition events (`RunStarted`,
   `RunWaiting`, `RunCompleted`...) or entity revisions with optimistic concurrency.
3. **Two writers to the log with different guarantees**: commands (serialized per thread, receipts) and
   provider ingestion (direct writes, no per-thread lock). Their interaction needs fences. Better: one
   per-thread actor/mailbox that owns the aggregate, receiving both commands and provider events, with
   an attempt/epoch token in each provider event.
4. **Run status overloading**: `waiting` means "finalizing", approvals do not change run status,
   `preparing` is a pre-start phase. A Rust enum with explicit variants and transition methods would
   make illegal states unrepresentable.
5. **Planner style**: commands are imperative functions pushing onto `Ref<Array<Event>>` while re-reading
   projections with "pending events" folded in. A pure `decide(&State, Command, &Ctx) -> Result<Decision>`
   plus `evolve(State, &Event) -> State` is simpler to test and replay.
6. **Stringly conventions**: command ids / effect ids / message ids built from string prefixes carry
   idempotency semantics and cross-module coupling (`startsWith("command:runtime-reconcile:")` filter).
   Use typed id constructors and an explicit `origin`/`causation` field.
7. **Recovery logic scattered**: `needsRecovery(kind)` predicates, startup loops in Orchestrator,
   `ProviderRuntimeRecoveryService`, `UsageLimitRecoveryWorker`, `ScheduledTaskService`, effect
   reconciliation each reimplement "find stuck state and repair". A single "recovery manifest"
   (states that are only valid with a live process) would help.
8. **Compatibility weight**: nearly every field is `optional` with decoding defaults because the same
   schema is persisted and sent to old clients; legacy fields coexist (`linkedPullRequest` +
   `pullRequests`; `provider` + `providerInstanceId` + `driver` columns). Decide up-front: persisted event
   schemas get explicit versions; wire DTOs are separate from persisted types.
9. **At-least-once delivery edges** (mailbox, provider acceptance vs receipt) are handled by
   stable-message-id dedupe rather than a clear protocol; document these as explicit state machines.
10. **Big projection payloads on the wire** required lots of mitigation (bounded snapshots, wire
    projection, coalescers, budgets). Design read models for paging from the start (timeline separate from
    control-plane state).
11. **Nested checkpoint scopes** and `ContextTransfer` types (`provider_handoff`, `subagent_spawn`,
    `checkpoint_summary`) are modelled more richly than used; avoid speculative generality.
12. **Terminals / worktree-setup state outside the log** is fine, but their lifecycle is tied to thread
    lifecycle by effects (`terminal.cleanup`) and cwd-sharing heuristics (`isCheckpointRestoreIsolated`)
    that scan all threads — consider recording workspace ownership explicitly (Workspace entity with
    owner thread ids) instead of inferring from paths.

### 21.3 Suggested Rust shape (for discussion, not decisions)

* **Aggregates**: `Thread` (with Runs/Attempts/Nodes/Requests/Checkpoint meta as child state) — one
  actor (tokio task + mpsc mailbox) per active thread; `Project`; `ScheduledTask` (plain table).
* **Pipeline**: `Command -> decide() -> (Vec<Event>, Vec<Effect>)` -> single DB transaction
  (`receipt`, `events`, projection upserts, `outbox`) -> broadcast. Keep per-thread serialization and
  global monotonically increasing sequence; keep snapshot+cursor for UI streaming.
* **Events**: transition-oriented, versioned, `#[serde(tag="type")]`; projections as functions
  `fn apply(&mut ReadModel, &Event)` used both inline and for rebuild. Keep full-entity snapshots only for
  slow-changing config entities.
* **Typed state machines**: `enum RunStatus` with `fn can_transition(from, to)`; `RunAttempt` fence
  (`attempt_id`, `epoch`) on all provider events; `Provider events -> thread actor` not direct DB writes.
* **Outbox**: same table shape (lease, attempt count, available_at, classification
  `ReplaySafe | ProcessBound`), worker pool, exponential backoff, startup reconcile; deterministic effect
  ids from `(command_id, kind, target)`.
* **Capabilities**: `ProviderCapabilities` struct stored on the session record; `policy` module of pure
  functions returning `Decision | Unsupported{capability}`.
* **Ids**: newtype ids with deterministic derivation helpers (`Uuid::new_v5`-style) for command-derived
  entities.
* **Checkpoints**: reimplement as `git` subprocess calls (temp `GIT_INDEX_FILE`, `read-tree`, `add -A`,
  `write-tree`, `commit-tree`, `update-ref` under a private ref namespace, `restore/clean/reset`) or
  `gix`; keep the isolated-workspace rule for file restore. Store `{scope, ordinal, ref, files[]}`.
* **Handoffs**: deterministic transcript rendering with token budget; track delivery status per native thread.
* **Terminals**: PTY manager (portable-pty) keyed by `(thread, terminal)`, bounded ring buffer
  (lines+bytes), sequence-numbered attach stream, metadata stream.
* **Scheduler**: single tick loop + due-row query + `running` marker + deterministic command id per
  fire time + startup repair of stuck rows; skip stale fixed-time slots.
* **Recovery**: one startup pass: reconcile runs/requests/sessions/effects in one transaction per thread,
  then optional continuation prompts; queue held until user resumes.
* **Testing**: record/replay of provider transports (NDJSON), in-memory SQLite, injected clock/ids.

---

## 22. Key file index

Docs
* `docs/orchestration-v2/{README,core-graph-and-data-model,entity-ids-and-correlation,feature-lifecycles,thread-lineage-and-context-transfer,provider-switching-and-context,provider-capability-system,orchestrator-mcp-server,testing-strategy}.md` (design intent; implementation differs in places noted above)
* `docs/internals/{overview,remote,connection-runtime,t3-connect,terminal-runtime,context-handoffs,legacy-orchestration-migration,providers,environment-auth,glossary}.md`

Contracts (`packages/contracts/src`)
* `orchestrationV2.ts` (all v2 entities, events, commands, RPC schemas), `applicationEvent.ts` (project events + stored event union), `orchestrationProject.ts`, `project.ts`, `terminal.ts`, `scheduledTask.ts`, `threadPullRequest.ts`, `pullRequest.ts`, `worktreeSetup.ts`, `checkpointDiff.ts`, `environment.ts` (descriptor/capabilities/ScopedRefs), `relay.ts`, `rpc.ts` (WS method names), `providerRuntime.ts`, `providerPolicy.ts`, `modelSelection.ts`, `settings.ts`

Orchestration engine (`apps/server/src/orchestration-v2`)
* Core: `Orchestrator.ts`, `EventSink.ts`, `EventStore.ts`, `ProjectionStore.ts`, `ProjectionMaintenance.ts`, `CommandReceiptStore.ts`, `CommandPolicy.ts`, `ThreadCommandExecutor.ts`, `KeyedSerialExecutor.ts`, `IdAllocator.ts`, `TurnItemPositionStore.ts`
* Outbox: `EffectOutbox.ts`, `EffectWorker.ts`
* Run lifecycle: `ProviderTurnStartService.ts`, `RunExecutionService.ts`, `RunFinalizationService.ts`, `ProviderEventIngestor.ts`, `ProviderTurnControlService.ts`, `RuntimeRequestService.ts`, `QueuedRunOrder.ts`, `ProviderFailure.ts`, `UsageLimitRecoveryWorker.ts`
* Recovery: `ProviderRuntimeRecoveryService.ts`, `RestartContinuation.ts`, `RestartBackgroundNote.ts`
* Providers/policy: `ProviderAdapter.ts`, `ProviderAdapterRegistry.ts`, `ProviderSessionManager.ts`, `ProviderSwitchService.ts`, `ProviderSessionTransitionPolicy.ts`, `ProviderSelectionTransition.ts`, `RuntimePolicy.ts`, `ProviderContinuation{Service,Requests}.ts`
* Context: `ContextHandoffService.ts`, `ContextHandoffBudget.ts`, `ContextHandoffDelivery.ts`, `ThreadForkService.ts`
* Checkpoints: `CheckpointService.ts`, `CheckpointCaptureService.ts`, `CheckpointRollbackService.ts`, `CheckpointRestoreSafety.ts`; `../checkpointing/{CheckpointStore,CheckpointDiffQuery,Diffs}.ts`; git impl `../vcs/GitVcsDriver.ts`
* Subagents/notifications: `SubagentProjection.ts`, `Notification.ts`, `NotificationMailbox.ts`, `DelegatedCompletionDelivery.test.ts`, `PullRequestSyncReactor.ts`, `PullRequestWatchReactor.ts`, `ThreadPullRequestService.ts`
* Thread services: `ThreadManagementService.ts`, `ThreadLaunchService.ts`, `ThreadLifecycleService.ts`, `ThreadSettlementService.ts`, `ThreadDeletion.ts`, `ThreadTitleRegenerationService.ts`, `ThreadMessageIntake.ts`, `ThreadSearch.ts`
* Streaming: `ShellStream.ts`, `ThreadStream.ts`, `WireProjection.ts`, `LiveStreamBudget.ts`, `ThreadLiveEventCoalescer.ts`, `threadHistoryPaging.ts`
* Projects: `ProjectStore.ts`, `ProjectCommands.ts`; `../project/*`
* Legacy: `legacy/LegacyV1ThreadImporter.ts`
* Tests/testkit: `testkit/DeterministicRuntime.ts`, `*ReplayFixtures.integration.test.ts`

Persistence (`apps/server/src/persistence`)
* `Layers/Sqlite.ts`, `Layers/OrchestrationEventStore.ts`, `Layers/OrchestrationCommandReceipts.ts`, `Migrations.ts`, `Migrations/055_OrchestrationV2.ts`, `Migrations/OrchestrationV2/*`, `initializeV2Database.ts`

Other server modules
* Terminal: `terminal/{Manager,PtyAdapter,NodePtyAdapter,OutputProtocol}.ts`
* Scheduling: `scheduling/Scheduler.ts`, `scheduledTasks/{ScheduledTaskService,Schedule}.ts`
* VCS/git/PR: `vcs/*`, `git/{GitManager,GitWorkflowService}.ts`, `pullRequest/*`, `sourceControl/*`, `review/ReviewService.ts`
* Workspace/env: `workspace/*`, `environment/*`, `storageCleanup.ts`, `project/{ProjectService,WorktreeSetupTracker,ProjectSetupScriptRunner,ManagedProjectFolders}.ts`
* Cloud/relay: `cloud/*`, `relay/AgentAwarenessRelay.ts`, `infra/relay/*`, `packages/client-runtime/*`
