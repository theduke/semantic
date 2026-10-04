# t3code: Provider Abstraction Layer and Runtime Event / Data Model

Research notes for a Rust re-implementation of a "run existing coding-agent CLIs and manage
them from a UI" system. Source: `/home/theduke/dev/github.com/pingdotgg/t3code` (read-only,
HEAD `77823bd102`). All paths below are relative to that repo unless absolute.

Scope: the provider abstraction (adapter interface, drivers, registry, sessions), the
canonical event / item / request taxonomy, how each provider is spawned and normalized,
capability flags, lifecycle state machines, and lessons for a Rust redesign.

---

## 0. Executive summary

1. t3code is an Effect-TS monorepo (server in `apps/server`, wire schemas in
   `packages/contracts`). The current architecture is called **orchestration-v2**
   (`apps/server/src/orchestration-v2/`). Design docs live in `docs/orchestration-v2/*.md` and
   `docs/internals/providers.md` and are unusually good; read them first.
2. The provider abstraction has **three layers**:
   - **Driver / Instance** (`apps/server/src/provider/ProviderDriver.ts`): a driver *kind*
     (`codex`, `claudeAgent`, `cursor`, `opencode`, `grok`, `pi`, `acpRegistry`,
     `antigravity`) plus user-configured *instances* (`codex_work`, `codex_personal`...) each
     with own binary path, env, home dir, credentials. Everything routes by `ProviderInstanceId`.
   - **Adapter** (`orchestration-v2/ProviderAdapter.ts`): per instance, `getCapabilities()`,
     `planSelectionTransition()`, `openSession()` returning a `SessionRuntime` with
     `ensureThread / resumeThread / startTurn / steerTurn / interruptTurn / respondToRuntimeRequest /
     readThreadSnapshot / rollbackThread / forkThread / injectHistory / compactThread / unloadThread`
     and an `events` stream.
   - **Session manager** (`ProviderSessionManager.ts`): owns live runtimes, idle release,
     event fan-out, MCP credential lifecycle.
3. **Normalization happens inside the adapter.** Adapters do *not* emit raw provider events or a
   small "runtime event" union. They emit already-normalized **entity upserts**
   (`turn_item.updated`, `node.updated`, `runtime_request.updated`, `provider_turn.updated`, ...)
   plus a `turn.terminal` marker. The server's `ProviderEventIngestor` just maps those to
   persisted domain events. (An older "ProviderRuntimeEvent" union of ~52 event types still
   exists in `packages/contracts/src/providerRuntime.ts`; it is now mostly vestigial in the
   server, its sub-types such as `TurnTokenUsage`, `ThreadTokenUsageSnapshot`, `ToolActivity*`
   are reused. Documented in section 4.5 for taxonomy value.)
4. Provider differences are expressed as a **~60-flag capability matrix** (11 groups) emitted by
   each adapter, plus degradation policies (steer-by-interrupt-restart, synthetic fork via
   context handoff, etc.). Section 6 lists the full matrix across providers.
5. Spawn modes: Codex = `codex app-server` stdio JSON-RPC (generated schema from the upstream
   Rust repo); Claude = `@anthropic-ai/claude-agent-sdk` (in-process TS lib that spawns the
   `claude` CLI; streaming-input + control messages); Cursor = `@cursor/sdk` local agent;
   OpenCode = `opencode serve` HTTP + SSE; Pi = `pi --mode rpc` JSONL stdio; Grok /
   Antigravity / generic registry agents = **ACP** (Agent Client Protocol) JSON-RPC over stdio.
6. The biggest weakness is **size and per-provider special-casing**: ~86k lines in
   `Adapters/` (Claude 7.8k, ACP 7.9k, Codex 6.3k, OpenCode2 4.3k, ...). Most complexity is
   background-work / wake-turn / dedup / ordering logic that arises because the canonical model
   mixes "entity snapshot" with "streaming deltas" and "run vs provider-turn vs attempt".

---

## 1. Key file map

### 1.1 Provider layer (`apps/server/src/provider/`)

| Path | Role |
|---|---|
| `provider/ProviderDriver.ts` | `ProviderDriver<Config,R>` SPI (plain record, not a service) and `ProviderInstance` (snapshot, orchestrationAdapter, textGeneration, auth, acpSessionManagement). |
| `provider/builtInDrivers.ts`, `builtInProviderCatalog.ts` | List of shipped drivers. |
| `provider/Drivers/{Codex,Claude,Cursor,OpenCode,Grok,Pi,Antigravity,AcpRegistry}Driver.ts` | One driver factory per kind: decode config, resolve binary/env, build snapshot provider + adapter + text-generation. |
| `provider/Services/ProviderInstanceRegistry.ts`, `Layers/ProviderInstanceRegistryLive.ts` | Owns `Map<InstanceId, ProviderInstance>`, hot-reload on settings change, "unavailable shadow" snapshots for unknown drivers. |
| `provider/Services/ServerProvider.ts`, `makeManagedServerProvider.ts` | Snapshot publisher: `getSnapshot / refresh / streamChanges / applyUsageLimits`, periodic probe, settings-change probe, async enrichment. |
| `provider/Layers/{Codex,Claude,Cursor,OpenCode,Grok,Pi,Antigravity}Provider.ts` | Status probes: version, auth, models, skills, slash commands, usage limits. |
| `provider/Layers/ProviderAuthService.ts`, `ProviderAuthFlow.ts`, `Services/ProviderAuthService.ts` | Interactive sign-in flows (browser / device code / terminal / credentials). |
| `provider/providerMaintenance*.ts`, `providerInstallation.ts`, `providerCompatibility.ts`, `model-manifest.json` | Update / install / compatibility ranges per provider version. |
| `provider/acp/*` | Shared ACP runtime: `AcpSessionRuntime.ts` (spawn + process-tree control), `AcpRuntimeModel.ts` (session update parsing), `AcpClientPolicy.ts` (permission decisions), `AcpClientTerminals.ts`, `Xai*`, `Antigravity*`, `Acp*Registry*`. |
| `provider/opencodeRuntime.ts`, `OpenCodeServerLedger.ts`, `OpenCodeServerOwner.ts`, `opencode2/*` | OpenCode server spawn, crash-orphan ledger, shared per-instance server. |
| `provider/Codex*.ts` | Managed CODEX_HOME, ChatGPT auth callback, thread revert, MCP elicitation, token usage. |
| `provider/ClaudeModelCatalog.ts`, `ClaudeModelManifest.ts`, `ClaudeTurnTokenUsage.ts` | Claude catalog + usage normalization. |
| `provider/ProviderCredentialStore.ts` | Credential storage for T3-owned credentials. |

### 1.2 Orchestration layer (`apps/server/src/orchestration-v2/`)

| Path | Role |
|---|---|
| `ProviderAdapter.ts` (598 lines) | **The adapter interface**, adapter event union, error taxonomy. |
| `ProviderAdapterDriver.ts`, `builtInProviderAdapterDrivers.ts` | Adapter-level driver factory (config schema + `create`) and built-in list. |
| `ProviderAdapterRegistry.ts` | Instance -> adapter lookup; wraps `openSession` with shared-credential admission control. |
| `ProviderSessionManager.ts` (2056 lines) | Live runtime residency, open/close/release/detach, idle timer, event fan-out, MCP credentials. |
| `ProviderTurnStartService.ts` (1267) | Resolves session + provider thread (ensure / resume / fork / handoff) and launches root run. |
| `ProviderTurnControlService.ts` | interrupt / steer / interruptAndAwaitTerminal against a live session. |
| `RuntimeRequestService.ts` | Responds to approvals / user inputs through the live runtime. |
| `RunExecutionService.ts` | Consumes an adapter's event stream for one run: routing, stop gates, finalization. |
| `ProviderEventIngestor.ts` | Adapter events -> domain events (`OrchestrationV2DomainEvent`) -> event store. |
| `ProviderSelectionTransition.ts`, `ProviderSessionTransitionPolicy.ts` | Decide reuse / switch-in-session / restart-and-resume / create-with-handoff / reject. |
| `ContextHandoff*.ts`, `ProviderSwitchService.ts`, `ProviderContinuationService.ts` | Cross-provider context transfer. |
| `SubagentProjection.ts` | Make child app threads for subagents. |
| `RuntimePolicy.ts` | Resolve cwd + runtime mode from thread + project + provider-supported modes. |
| `ProviderFailure.ts` | Failure sanitation / classification (bounded, redacted). |
| `ProviderRuntimeRecoveryService.ts`, `Restart*.ts` | Post-crash reconciliation. |
| `Adapters/{Codex,Claude,Cursor,OpenCode,OpenCode2,Pi,Grok,Antigravity,AcpRegistry,Acp}AdapterV2.ts` | The per-provider adapters. |
| `Adapters/ProviderTextDeltaCoalescer.ts` | Batches streaming text deltas by (turn,item) with a flush interval. |

### 1.3 Contracts (`packages/contracts/src/`)

| File | Content |
|---|---|
| `orchestrationV2.ts` (3357 lines) | **Canonical data model**: AppThread, Run, RunAttempt, ExecutionNode, Subagent, ProviderSession/Thread/Turn, RuntimeRequest, ConversationMessage, PlanArtifact, TurnItem union, Checkpoint, ContextHandoff/Transfer, capabilities, domain events, commands. |
| `providerRuntime.ts` (1204) | Older "ProviderRuntimeEvent" union (52 types) and shared sub-schemas (`TurnTokenUsage`, `ThreadTokenUsageSnapshot`, tool-activity icons, task linkage). |
| `provider.ts` | Legacy v1 provider session/turn inputs (`ProviderSession`, `ProviderSendTurnInput`, ...). |
| `providerPolicy.ts` | `RuntimeMode`, `ProviderInteractionMode`, approval/sandbox enums, request kinds, decisions. |
| `providerInstance.ts` | Open branded slugs `ProviderDriverKind`, `ProviderInstanceId`, instance config envelope. |
| `modelSelection.ts`, `model.ts` | `ModelSelection { instanceId, model, options[] }`, option descriptors, default models. |
| `server.ts` | `ServerProvider` snapshot (auth, models, skills, slash commands, usage limits, version/compat advisory). |
| `providerSetup.ts` | Auth/install flow state. |
| `providerUsageLimits.ts` | Rolling quota windows. |
| `settings.ts` | Per-driver `*Settings` schemas (`binaryPath`, `homePath`, `launchArgs`, `customModels`, ...). |

### 1.4 Protocol wrapper packages

| Package | Role |
|---|---|
| `packages/effect-codex-app-server` | Typed client for the Codex `app-server` JSON-RPC protocol. Types + method tables are *generated* from upstream JSON schemas (`openai/codex` `codex-rs/app-server-protocol`, pinned commit in `scripts/generate.ts`). 102 client request methods, 8 server request methods, 80 server notifications. Includes replay harness (`replay.ts`) and mock peer. |
| `packages/effect-acp` | Typed client+agent for ACP (schema release `v2.0.0-alpha.3`, protocol version 2, with a v1 compat layer in `compat.ts`). Also generated from upstream schema. |

### 1.5 Docs worth reading verbatim

`docs/orchestration-v2/{README,core-graph-and-data-model,entity-ids-and-correlation,feature-lifecycles,provider-capability-system,provider-switching-and-context,thread-lineage-and-context-transfer,orchestrator-mcp-server,testing-strategy}.md` and `docs/internals/providers.md` ("protocol traps").

---

## 2. Layering and the provider interface

```
UI / WS commands (app ids only)
  -> Orchestrator commands (thread.create, message.dispatch, run.interrupt, runtime-request.respond, ...)
     -> ProviderTurnStartService / ProviderTurnControlService / RuntimeRequestService
        -> ProviderSessionManager (live runtimes by ProviderSessionId)
           -> ProviderAdapterRegistry.get(instanceId) -> ProviderAdapterV2Shape
              -> ProviderAdapterV2SessionRuntime  (one per opened session)
                 -> native protocol (JSON-RPC stdio / HTTP+SSE / SDK / JSONL)
  <- adapter emits ProviderAdapterV2Event stream
     -> RunExecutionService (route + stop gates)
        -> ProviderEventIngestor.normalize -> EventSink (SQLite events + projections)
           -> WS projection stream to UI
```

Principle (from `docs/orchestration-v2/README.md`): *provider ids are evidence, app ids are
identity*. Every command targets app ids; adapters translate to native refs at the edge.

### 2.1 Driver vs instance (identity model)

`packages/contracts/src/providerInstance.ts`:

```ts
// Open branded slug (letters/digits/-/_, starts with letter, <=64). NOT a closed enum.
ProviderDriverKind   // "codex" | "claudeAgent" | "cursor" | "opencode" | "grok" | "pi" | "acpRegistry" | "antigravity" | <unknown from forks>
ProviderInstanceId   // user slug, e.g. "codex_work"; default instance id == driver kind
ProviderInstanceConfig = { driver, displayName?, accentColor?, environment?: {name,value,sensitive}[], enabled?, config?: unknown }
```

- Threads, sessions, model selections reference **instance ids**, never driver kinds.
  Two accounts of the same driver never share mutable session/catalog state.
- Unknown drivers (rolled-back build, fork) must still decode; the registry downgrades them to
  an "unavailable shadow snapshot" (`availability: "unavailable"`, `installed:false`, `enabled:false`).
- `ProviderContinuationIdentity { driverKind, continuationKey }` (default key
  `${driver}:instance:${instanceId}`) decides whether two instances can resume each other's native
  threads (same continuation key = yes; else provider-switch handoff).

Driver SPI (`provider/ProviderDriver.ts`):

```ts
interface ProviderDriver<Config, R> {
  driverKind; metadata: { displayName; supportsMultipleInstances? };
  configSchema: Schema.Codec<Config, unknown>;      // decoded once by registry
  defaultConfig(): Config;
  create(input: {instanceId, displayName, accentColor, environment, enabled, config})
    : Effect<ProviderInstance, ProviderDriverError, R | Scope>;   // scope owns all resources
}
interface ProviderInstance {
  instanceId; driverKind; continuationIdentity; displayName; accentColor; enabled;
  snapshot: ServerProviderShape;                 // status probe publisher
  snapshotForCwd?(cwd); refreshModels?(); invalidateCaches?; consumeResetCredit?();
  orchestrationAdapter: ProviderAdapterV2Shape;  // the thing that runs threads
  textGeneration: TextGeneration;                // one-shot titles/commit/PR text
  auth?: ProviderAuthController;
  acpSessionManagement?: { listSessions, logout, deleteSession, listProviders, setProvider, disableProvider };
}
```

Notable design: no per-driver Context tags (instances of the same driver would collide);
drivers are plain values; "two `create` calls with different instance ids MUST have no shared
mutable state"; failures become unavailable shadow snapshots, never defects.

### 2.2 Adapter interface (verbatim essentials)

`orchestration-v2/ProviderAdapter.ts`:

```ts
interface ProviderAdapterV2Shape {
  instanceId; driver;
  getCapabilities(): Effect<OrchestrationV2ProviderCapabilities>;
  planSelectionTransition(input: { current: ModelSelection; target: ModelSelection;
                                   sessionCapabilities }): Effect<ProviderSelectionTransitionPlan>;
  openSession(input: OpenSessionInput): Effect<ProviderAdapterV2SessionRuntime, Err, Scope>;
}
interface OpenSessionInput {
  threadId; providerSessionId; modelSelection; runtimePolicy;
  resumeFromSession?: ProviderSession; initialNativeThreadId?: string; initialProviderItemIdentityVersion?: 2;
}
interface ProviderAdapterV2RuntimePolicy {
  runtimeMode: RuntimeMode;            // "approval-required" | "auto-accept-edits" | "auto" | "full-access"
  interactionMode: ProviderInteractionMode; // "default" | "plan"
  cwd: string | null; approvalPolicy?: unknown; sandboxPolicy?: unknown; reasoningEffort?: string;
}
```

Session runtime:

```ts
interface ProviderAdapterV2SessionRuntime {
  instanceId; driver; providerSessionId; providerSession;
  events: Stream<ProviderAdapterV2Event, Err>;               // single-consumer
  subscribeEvents?: Effect<{ events; close }>;               // manager-owned multi-subscriber fan-out
  hasPendingBackgroundWork?: Effect<boolean>;                // session-wide (idle-release gate)
  hasPendingBackgroundWorkForThread?(providerThread): Effect<boolean>;   // per-thread (stop gate)
  getModelContextWindow?(selection, cwd): number | undefined;
  canReuseContextUsage?(prev, next): boolean;

  ensureThread(input: { threadId; modelSelection; runtimePolicy; providerSessionId?; existingProviderThread? }): ProviderThread;
  resumeThread(input: { providerThread; threadId?; modelSelection?; runtimePolicy? }): ProviderThread;
  injectHistory?(input: { providerThread; messages: HistoricalMessage[]; context: string }): boolean; // false = unsupported
  startTurn(input: TurnInput): void;              // returns once accepted; progress arrives as events
  compactThread?(input: TurnInput): void;
  steerTurn(input: { threadId; runId; providerThread; providerTurnId; message }): void;
  interruptTurn(input: { providerThread; providerTurnId; requestRuntimeRestart?: boolean }): void;
  unloadThread?(input: { providerThread }): void; // detach one thread from a shared runtime
  respondToRuntimeRequest(input: { requestId; decision?; answers?; response? }): void;
  readThreadSnapshot(input: { providerThread }): ThreadSnapshot;   // {providerThread, providerTurns, messages, runtimeRequests, providerPayload?}
  uploadFeedback?(...): { feedbackId };
  rollbackThread(input: { providerThread; target: thread_start|provider_turn; providerThreadTurns }): ThreadSnapshot;
  forkThread(input: { sourceProviderThread; sourceProviderTurns?; providerTurnId?; targetThreadId; ownerNodeId?; modelSelection?; runtimePolicy? }): ProviderThread;
}
interface TurnInput {
  appThread; threadId; runId; runOrdinal; providerTurnOrdinal; restartContinuationOfRunId?;
  attemptId; rootNodeId; providerThread; message: { messageId; text; attachments[]; createdBy; creationSource; scheduledTaskId?; senderThreadId? };
  modelSelection; runtimePolicy;
}
```

Semantics worth copying:

- `startTurn` / `steerTurn` / `interruptTurn` / `respondToRuntimeRequest` return **acknowledgements
  only**. Terminal state arrives through events. The app must *not* mark a run terminal because
  the interrupt request returned (probe-derived: Codex `turn/interrupt` returns first, the
  `interrupted` status arrives later in `turn/completed`).
- `ensureThread` receives `existingProviderThread` and **must adopt that row's identity**
  (otherwise two live provider-thread rows per app thread and `activeProviderThreadId` flaps).
- Resume is a *required* primitive, not a capability. Resume failure is a runtime condition:
  start path logs, calls `ensureThread` with `nativeThreadRef: null`, then creates a
  `provider_resume_fallback` context handoff so the fresh native thread learns prior history.
- `injectHistory` returning `false` means the native protocol explicitly lacks it; the caller
  then inlines the handoff context in the user message text. An *ambiguous* failure
  (transport error) must not fall back to a second delivery; it is tracked as
  `delivery.status: "pending"` and forces a fresh thread on next resume ("uncertain delivery").
- Adapter `Scope` owns the native process; closing the scope kills it.

Adapter event union (`ProviderAdapterV2Event`):

| `type` | Payload | Meaning |
|---|---|---|
| `app_thread.created` | AppThread | Adapter-created thread (subagent child thread). |
| `provider_session.updated` | ProviderSession | Session status changes. |
| `provider_thread.updated` | ProviderThread | Native thread binding / status / background roster / context usage. |
| `provider_turn.updated` | ProviderTurn | Turn lifecycle snapshot (pending/running/terminal) + token usage. |
| `node.updated` | ExecutionNode | Graph node snapshot (tool, approval, subagent...). |
| `subagent.updated` | Subagent | Subagent roster snapshot. |
| `message.updated` | ConversationMessage | User/assistant message snapshot (`streaming` flag). |
| `turn_item.updated` | TurnItem | Timeline item snapshot (the main UI record). |
| `runtime_request.updated` | RuntimeRequest | Pending approval/user-input/dynamic-tool request. |
| `plan.updated` | PlanArtifact | proposed_plan or todo_list artifact. |
| `turn.terminal` | `{providerThreadId, providerTurnId, runOrdinal, status, failure, threadDisposition: reusable\|broken, [failureItemOrdinal, retry, retryStartedAt]}` | The one authoritative terminal marker. `failed` carries a `ProviderFailure`. `threadDisposition: "broken"` forces thread replacement on next run. |

Every event carries `driver`. All entity events are **idempotent upserts keyed by app id**
(streaming = repeated `turn_item.updated` with growing `text` and `streaming: true`).

Adapter error taxonomy (tagged errors, each carries `driver`): `Capabilities`, `OpenSession`,
`CloseSession`, `ResumeThread`, `EnsureThread`, `ReadThreadSnapshot`, `RollbackThread`,
`ForkThread`, `TurnStart`, `SteerRunUnsupported`, `SteerRun`, `Interrupt`,
`RuntimeRequestResponse`, `EventStream`, `Protocol` (with optional raw `payload`).

### 2.3 Adapter registry and shared credentials

`ProviderAdapterRegistry.layerFromProviderInstanceRegistry` resolves adapters *dynamically*
(hot reload visible). It also wraps `openSession` with credential admission: instances sharing a
credential binding (`auth.credentialBinding.{owner,key}`) refuse to open while any of them is
`isChangingCredentials`, and `withAccess` of each peer wraps the open so a credential change
interrupts a peer's in-flight startup. `getMetadata(instanceId)` returns
`{driver, continuationKey, enabled, capabilities}`.

### 2.4 Session manager

`ProviderSessionManagerV2`:

```ts
open({threadId, providerSessionId, modelSelection, runtimePolicy, resumeFromSession?, initialNativeThreadId?}): SessionRuntime
get(providerSessionId): Option<SessionRuntime>
close(id) / closeInstance(instanceId)
release({providerSessionId, reason: idle_timeout|runtime_error|manual_shutdown|server_shutdown, detail?})
detach({providerSessionId, threadId, detail?, revokeMcpCredential?})
shutdown
```

- Owns live-session residency only; **does not resurrect persisted sessions**. Process-loss
  recovery terminalizes provider-bound work; the next user command lazily opens a new session.
- Keyed serial executor on `ProviderSessionId` for open; per-thread attach/detach serialization.
- Idle: `DEFAULT_IDLE_TIMEOUT_MS = 30 min`; release is deferred while
  `hasPendingBackgroundWork` is true, bounded by `DEFAULT_MAX_IDLE_PIN_MS = 4 h`;
  `busyCount` increments around uses. Release scope close timeout 30 s; per-thread unload 10 s.
- `supportsMultipleProviderThreads` sessions (Codex, OpenCode2) attach several app threads;
  `detach` calls `unloadThread` (Codex `thread/unsubscribe`) so the shared process can drop
  native thread state and its MCP servers.
- Event fan-out: each subscriber gets a queue of `{event|failure}` signals; session-scoped
  runtime requests (no provider turn) are persisted by the session pump rather than a run
  subscriber (`sessionScopedRuntimeRequestThreadId`).
- Issues/revokes a per-thread **MCP bearer credential** for T3's own MCP endpoint
  (`McpSessionRegistry`), with reservations protecting the window between "credential handed
  out" and "session entry visible" (race-safe release). Credential reused across re-attach so
  long-lived provider processes that cached the token keep working; rotated when capability set
  (preview/device) flips.

---

## 3. Canonical data model (orchestration-v2)

### 3.1 Entity relationships

```
Project
 AppThread (user-visible conversation; id ThreadId)
   lineage: { parentThreadId, relationshipToParent: fork|subagent, rootThreadId }
   forkedFrom: run | node | provider_thread
   modelSelection, runtimeMode, interactionMode, branch, worktreePath, activeProviderThreadId
   Run (counted user turn; ordinal N)  status: preparing|queued|starting|running|waiting|completed|interrupted|failed|cancelled|rolled_back
     RunAttempt (reason: initial|steering_restart|retry|provider_recovery) -> providerTurnId
       ExecutionNode tree (root_turn, assistant_message, reasoning, plan, todo_list, tool_call,
                          approval_request, user_input_request, subagent, hook, system)
         TurnItems (timeline records) / RuntimeRequests / Subagents / Checkpoints
 ProviderSession (live runtime handle)  status: starting|ready|running|waiting|stopped|error
   ProviderThread (native conversation handle; may be many per session)
     ProviderTurn (native turn; ordinal; token usage)
 ContextHandoff / ContextTransfer (fork, provider_handoff, merge_back, subagent_spawn, subagent_result)
 CheckpointScope / Checkpoint (filesystem snapshots; root and nested)
```

Key idea: **Run (app turn) != ProviderTurn (native turn) != RunAttempt**. One run can have
several attempts (steering by interrupt-restart, retry, provider recovery); child provider
turns (subagents) are real provider turns that never replace the parent turn's id.
"Only the root node can complete the run."

### 3.2 Refs and identity

```ts
OrchestrationV2ProviderRef = { driver, nativeId: string|null, strength: "strong"|"weak"|"none", fingerprint?, ordinal? }
```

- App ids primary; native ids stored as refs for correlation/debugging/routing.
- Scoped correlation: `ProviderThread: provider+session+nativeThreadRef`;
  `Turn: providerThread+nativeTurnRef | +turnOrdinal`; `Item: providerTurn+nativeItemRef | +itemOrdinal`;
  `Request: session+nativeRequestRef | turn+requestOrdinal`.
- Identity strength per kind is a capability: `strong` = use native id; `weak` = native id +
  ordinal/fingerprint; `none` = allocate by scoped ordinal. Replay determinism requirement.
- `nativeMetadata.itemIdentityVersion: 2` scopes item ids by provider instance (a migration).
- `OrchestrationV2ProviderThread.nativeThreadRef` is what resume needs; for Claude it is the
  session id and `nativeConversationHeadRef` the last message uuid (`resumeSessionAt`); for Pi the
  session file path; for ACP the session id; for Codex the thread id.

### 3.3 ProviderSession / ProviderThread / ProviderTurn

```ts
ProviderSession = { id, driver, providerInstanceId, status: starting|ready|running|waiting|stopped|error,
                    cwd, model, capabilities, createdAt, updatedAt, lastError }
ProviderThread  = { id, driver, providerInstanceId, providerSessionId|null, appThreadId|null, ownerNodeId|null,
                    nativeThreadRef, nativeConversationHeadRef,
                    status: not_loaded|idle|active|archived|closed|error,
                    firstRunOrdinal, lastRunOrdinal, handoffIds[], forkedFrom?,
                    pendingBackgroundTasks: ({kind: subagent|command|monitor|background_task, taskId, description?, childThreadId?})[],
                    contextUsage: ThreadTokenUsageSnapshot|null, nativeMetadata: {modelSelection?, title?, updatedAt?, itemIdentityVersion?} }
ProviderTurn    = { id, providerThreadId, nodeId, runAttemptId, nativeTurnRef, ordinal,
                    status: pending|running|completed|interrupted|failed|cancelled,
                    startedAt, completedAt, tokenUsage?, turnTokenUsage? }
```

`pendingBackgroundTasks` uses a "kind union with fallback" decoder (unknown kinds decode to
`background_task`) for forward compatibility.

### 3.4 ExecutionNode and Subagent

```ts
ExecutionNode = { id, threadId, runId|null, parentNodeId|null, rootNodeId,
  kind: root_turn|assistant_message|reasoning|plan|todo_list|tool_call|approval_request|user_input_request|subagent|hook|system,
  status: idle|pending|running|waiting|completed|interrupted|failed|cancelled|rolled_back,
  countsForRun, providerThreadId, providerTurnId, nativeItemRef, runtimeRequestId, checkpointScopeId, startedAt, completedAt }
Subagent = { id(NodeId), threadId, runId, parentNodeId, origin: provider_native|app_owned, createdBy, driver, providerInstanceId,
  providerThreadId, childThreadId, nativeTaskRef, prompt, title, model, completionWake?, completionDelivery?,
  status: idle|pending|running|waiting|completed|failed|cancelled|interrupted, progress?, result, startedAt, completedAt, updatedAt }
```

`idle` (resumable, does not keep a turn or subscription alive) vs `pending|running|waiting`
(active) is distinguished by `isOrchestrationV2WorkActive`. Provider-native subagents are
read-only children (the provider owns their conversation); app-owned ("delegate_task" via T3's MCP)
children accept messages.

### 3.5 RuntimeRequest (approvals, user input, dynamic tool, auth refresh)

```ts
RuntimeRequest = {
  id: RuntimeRequestId, nodeId, providerTurnId|null, nativeRequestRef|null,
  kind: ProviderRequestKind | "dynamic_tool_call" | "user_input" | "auth_refresh",
  status: pending|resolved|expired|cancelled,
  responseCapability:
      | { type: "live", providerSessionId }          // callback held in a live process
      | { type: "message" }                          // answer by sending a new user message (Codex async questions)
      | { type: "not_resumable", reason },           // process gone
  createdAt, resolvedAt, decision?, answers? }

ProviderRequestKind = "command" | "file-read" | "file-change" | "mcp-elicitation" | "permission"
ProviderApprovalDecision = "accept" | "acceptForSession" | "acceptAlways" | "decline" | "cancel"
ProviderApprovalOption = { decision, label, warning? }       // provider-advertised choices; native ids must survive normalization
```

Pending requests are *persisted* but callbacks usually die with the process; on restart they
become `not_resumable`/`cancelled`. `message`-mode requests (Codex `item/tool/requestUserInput`
arrives as notification-ish, answered with a new message) survive restart and are answered by
`runtime-request.respond` committing "resolution + user message" in one transaction, idempotent
by command receipt.

### 3.6 TurnItem union (the timeline)

Base fields on every item:

```ts
{ id: TurnItemId, threadId, runId|null, nodeId|null, providerThreadId|null, providerTurnId|null,
  nativeItemRef|null, parentItemId|null, ordinal: number, status: TurnItemStatus,
  title: string|null, startedAt, completedAt, updatedAt,
  toolSurface?: "browser"|"computer", toolIcon?: website|native-app|themed-logo, toolSource?: {key,name,kind,icon?} }
TurnItemStatus = idle|pending|running|waiting|completed|failed|cancelled|interrupted
```

| `type` | Extra fields |
|---|---|
| `user_message` | messageId, createdBy (user\|agent\|system), creationSource (web\|mobile\|mcp\|provider\|server), scheduledTaskId?, senderThreadId?, `inputIntent` (turn_start\|queued_turn\|steer\|promoted_queued_to_steer), text, context?, attachments[] |
| `assistant_message` | messageId, text, attachments?, streaming |
| `reasoning` | text, streaming |
| `proposed_plan` | planId, markdown, streaming |
| `todo_list` | planId, steps[{id,text,status: pending\|running\|completed, durationAnchorAt?, durationMs?}], explanation? |
| `user_input_request` | requestId, questions[{id,header,question,options[{label,description,value?}],multiSelect?,allowCustomAnswer?,required?}], questionAnswer?, responseMode?: "message" |
| `approval_request` | requestId, requestKind, prompt?, appName?, options?[ProviderApprovalOption] |
| `command_execution` | input, output?, outputIndicatesFailure?, exitCode? |
| `file_change` | fileName, additions?, deletions?, diffStr?, oldStr?, newStr?, changes?[{operation, path, oldPath?, fileType?, mimeType?}] |
| `file_search` | pattern?, results?[{fileName,line?,column?,preview?}] |
| `web_search` | patterns?[], results?[{title?,url?,snippet?}] |
| `dynamic_tool` | toolName|null, viewedImagePath?, input: unknown, output?: unknown |
| `subagent` | subagentId, origin, driver, providerInstanceId, childThreadId|null, prompt, progress?, result|null |
| `notification` | source (delegated_task\|subagent\|command\|background_task), outcome (completed\|failed\|cancelled\|updated\|unknown), summary, detail? |
| `error` | failure: ProviderFailure, retry?: ProviderRetry |
| `compaction` | driver|null, summary?, beforeTokenCount?, afterTokenCount? |
| `checkpoint` | checkpointId, scopeId, files[{path,kind,additions,deletions}] |
| `handoff` | contextHandoffId, from/to provider thread+instance ids, strategy, summary?, fromModelSelections?, toModel? |
| `fork` | source (run\|node\|provider_thread), targetThreadId, providerThreadId? |
| `thread_created` | targetThreadId, targetRunId, targetProviderInstanceId, targetModel |
| `run_interrupt_request` / `run_interrupt_result` / `system_notice` | message |

Observations: file_change carries diff as `diffStr` and/or `oldStr/newStr`; the app also
derives real diffs from filesystem **checkpoints** (git-backed) because provider diffs are
inconsistent (Codex `turn/diff/updated` is explicitly *opted out* at initialize). Generic tool
output is `dynamic_tool.output: unknown`. Command output is a single string, streamed by
repeated upserts.

### 3.7 PlanArtifact

```ts
PlanArtifact = { id: PlanId, threadId, runId|null, nodeId, status: draft|active|completed|superseded, detailInTurnItem? } &
  ( { kind: "proposed_plan", markdown } | { kind: "todo_list", steps: PlanStep[], explanation? } )
```

Three separate concepts: *proposed plan* (acceptable artifact), *todo list* (live progress),
*questions* (user-input request). The ingestor derives per-step durations server-side
(`withPlanStepDurations`) because provider step ids may be positional.

### 3.8 ProviderFailure and retry

```ts
ProviderFailure = { class: usage_limit|provider_error|transport_error|permission_error|validation_error|unknown,
                    message(<=4096), code|null(<=128), retryable: boolean|null, resetAt?: IsoDateTime|null }
ProviderRetry   = { attempt, maxAttempts|null, retryDelayMs|null }   // Claude exposes all; Codex only willRetry
```

Producers must redact credentials/URLs (query strings stripped) and map defect causes to
fixed safe messages (`ProviderFailure.ts`). `usage_limit` + `resetAt` feeds a thread-level
`limitRecovery {runId, resetAt, autoResume, snooze}`.

### 3.9 Token / context usage

Two distinct shapes:

```ts
ThreadTokenUsageSnapshot = { usedTokens, totalProcessedTokens?, maxTokens?, inputTokens?, cachedInputTokens?, outputTokens?,
  reasoningOutputTokens?, last*Tokens?, toolUses?, durationMs?, compactsAutomatically?, autoCompactThreshold?, cost?: {amount, currency} }
TurnTokenUsage = { usageScope: "main_agent", hasSubagents: boolean,
  usageStatus: "complete" (inputTokens, outputTokens required) | "partial" | "unavailable",
  cachedInputTokens?, cacheCreationTokens?, reasoningTokens? }     // input includes cache reads+writes; output includes reasoning
```

Per-provider normalizers: `ClaudeTurnTokenUsage.ts` (from SDK result message), `CodexTurnTokenUsage.ts`
(Codex reports cumulative thread totals; the adapter computes per-turn deltas from `total` vs `last`,
handling resume/rollback baselines). Account-level quota windows (`ServerProviderUsageWindow`
{id, kind session|weekly|monthly|other, label, usedPercent, resetsAt, windowDurationMins}) are
separate and live on the provider snapshot.

### 3.10 Domain events (persisted)

Adapter events are mapped (`ProviderEventIngestor.normalize`) to `OrchestrationV2DomainEvent`:
`thread.created/…`, `run.created|updated|background-work-cancelled`, `run-attempt.created|updated`,
`node.updated`, `subagent.updated`, `provider-session.attached|updated|detached`,
`provider-thread.updated`, `provider-turn.updated`, `runtime-request.updated`, `message.updated`,
`turn-item.updated`, `plan.updated`, `checkpoint-scope.created`, `checkpoint.captured`,
`checkpoint.rollback-requested`, `context-handoff.updated`, `context-transfer.created|updated`.
Envelope: `{ id, type, threadId, runId?, nodeId?, driver?, providerInstanceId?, rawEventId?, occurredAt, payload }`.
Writes use guarded variants: `writeIfRunCurrent` (reject mutable events from an attempt that
lost ownership) and `writeIfProviderThreadOwner`. On terminal provider turn events, pending
native user inputs for that turn are auto-cancelled (`dismissNativeUserInputs`).

Raw diagnostic frames: `OrchestrationV2RawProviderEvent { id, driver, providerInstanceId, providerSessionId,
sequence, direction incoming|outgoing, messageKind request|response|notification|error, method, jsonRpcId, payload, observedAt }`.
**Not** a durable DB; written as rotating NDJSON logs with a 64 KiB per-payload budget,
token deltas filtered, structural summaries for big payloads.

### 3.11 Commands relevant to providers

`message.dispatch` with `mode`:
`defer_start | steer_active{targetRunId} | restart_active{targetRunId} | queue_after_active | start_immediately`;
`run.interrupt`, `queued-message.promote-to-steer`, `queue.resume`, `queued-run.reorder|cancel|edit`,
`runtime-request.respond`, `thread.user-input.dismiss`, `checkpoint.rollback`, `thread.fork`,
`thread.merge_back`, `provider.switch`, `provider-session.detach`, `thread.runtime-mode.set`,
`thread.interaction-mode.set`, `thread.model-selection.set`, `delegated_task.request`, ...
Command ids give idempotent receipts (duplicate command id returns same result, no repeat side effects).

---

## 4. Policy enums: runtime modes, interaction modes, model selection

### 4.1 Runtime mode (T3's unified permission notion)

```ts
RuntimeMode = "approval-required" | "auto-accept-edits" | "auto" | "full-access"   // default full-access
ProviderInteractionMode = "default" | "plan"
ProviderApprovalPolicy = "untrusted" | "on-failure" | "on-request" | "never"     // codex-style
ProviderSandboxMode = "read-only" | "workspace-write" | "danger-full-access"
```

Provider snapshot publishes `supportedRuntimeModes`; `RuntimePolicy.resolve` downgrades an
unsupported mode to `approval-required` (the safest) rather than imitating it.
`runtimePolicy.enforcement: "native" | "client-boundary"` capability: "native" = provider gets
approval+sandbox policy per turn and confines itself; "client-boundary" = T3 only enforces what
it mediates (permission requests, client fs/terminal handlers); provider-owned execution is not
confined, so guarantees are weaker. Persisted capability decoding defaults to the weaker value.

Mapping per provider:

| Runtime mode | Codex (`turn/start` params) | Claude (`permissionMode`) | ACP |
|---|---|---|---|
| approval-required | approvalPolicy `untrusted`, reviewer `user`, sandbox `readOnly` | `default` (permission callback asks) | permission requests surfaced as approvals |
| auto-accept-edits | `on-request`, reviewer `user`, sandbox `workspaceWrite` | `acceptEdits` (+callback for non-edits) | policy auto-allows workspace edits, asks for others |
| auto | `on-request`, reviewer `auto_review`, `workspaceWrite` | `auto` | provider-specific (Grok "auto": provider classifier asks; `permissionDisposition` override) |
| full-access | `never`, reviewer `user`, `dangerFullAccess` | `bypassPermissions` (+ `allowDangerouslySkipPermissions`) | auto-allow |

Plan interaction mode: Codex -> `collaborationMode: {mode: "plan", settings: {model, reasoning_effort, developer_instructions}}`
on `turn/start` (always sent explicit; reviewer also always explicit because omitting it leaves
the previous value sticky on resumed threads); Claude -> `permissionMode: "plan"` (+ read-only
tool list); ACP -> native session mode named `plan` or `architect` if advertised via
`session/set_mode`, else config option; Grok/Claude `ExitPlanMode`/`x.ai/exit_plan_mode` are
intercepted, plan markdown captured into a `proposed_plan` item, native gate denied/abandoned.

### 4.2 ModelSelection and options

```ts
ModelSelection = { instanceId: ProviderInstanceId, model: string, options?: { id: string, value: string | boolean }[] }
// legacy decode: { provider, model, options: {effort:"max", fastMode:true} } -> canonical array
ProviderOptionDescriptor =
   { type: "select", id, label, description?, options: [{id,label,description?,isDefault?}], currentValue?, promptInjectedValues? }
 | { type: "boolean", id, label, description?, currentValue? }
ModelCapabilities = { optionDescriptors?: ProviderOptionDescriptor[] }
ServerProviderModel = { slug, name, shortName?, subProvider?, aliases?, badge?, isCustom, isDefault?, isLegacy?, capabilities: ModelCapabilities|null }
```

- Reasoning effort and friends are **data-driven option descriptors per model**, not typed fields:
  known option ids include `reasoningEffort` (Codex), `effort` (Claude), `fastMode`, `serviceTier`,
  `thinking`, `contextWindow`. UI renders descriptors generically. `promptInjectedValues`
  = values that are applied by prepending text to the prompt (Claude's "ultrathink"-style effort,
  `applyClaudePromptEffortPrefix`).
- Each adapter has a compile step at the provider boundary (e.g. `compileClaudeModelSelection` ->
  `{apiModelId, effort, promptEffort, settings{alwaysThinkingEnabled,fastMode,ultracode}, queryIdentity}`).
  `queryIdentity` is a stable hash used to decide whether the live Claude query can be reused or
  must be replaced.
- Model catalogs: Codex from `model/list` (paginated `requestAllCodexModels`), Claude from a bundled
  + remotely refreshed JSON manifest (`provider/model-manifest.json`, `updatedAt`-ordered, validated
  by catalog + adapter schema before replacing cache; also holds per-driver version compatibility
  ranges `supported|graceful|unsupported|broken`), Cursor from SDK catalog, OpenCode from server,
  ACP from `session/new` response `models`/config options, Pi from user's settings.
- Custom models: `customModels: (string | {slug,name?,capabilities?})[]` per instance.
- Selection transition plan (`planSelectionTransition`): adapter classifies a model/option change:
  `apply_on_next_turn | restart_session | create_with_handoff | reject{reason}`. Session
  transition policy (`ProviderSessionTransitionPolicy`) then picks `reuse | switch_model_in_session |
  restart_and_resume | create_with_handoff | reject` using continuation identity, instance,
  runtime mode and workspace change.

---

## 5. Normalization: native -> canonical, per provider

### 5.1 Spawn / connect matrix

| Provider (driver kind) | Transport | Process model | Resume handle |
|---|---|---|---|
| Codex (`codex`) | JSON-RPC 2.0 NDJSON over stdio to `codex app-server [launchArgs]` | One process per **provider session**, multiplexes many threads (`supportsMultipleProviderThreadsPerSession`). `CODEX_HOME` override per instance; optional shadow home. Client `initialize` + `initialized` notify, capabilities `{experimentalApi:true, optOutNotificationMethods:["turn/diff/updated"]}`. | thread id (`thread/resume`, `excludeTurns:true` for metadata reads) |
| Claude (`claudeAgent`) | `@anthropic-ai/claude-agent-sdk` `query({prompt: AsyncIterable<SDKUserMessage>, options})` which spawns the `claude` binary (`pathToClaudeCodeExecutable`, `CLAUDE_CONFIG_DIR`) | One live `query` (CLI process) per native thread; streaming-input mode; closing the query ends the CLI and its background shells. | session id (`resume`), `resumeSessionAt` = message uuid; fork via SDK `forkSession`; history read from CLI session storage |
| Cursor (`cursor`) | `@cursor/sdk` local agent (`Agent.create/resume`, `agent.send`, `InteractionUpdate` deltas) | In-process SDK; `settingSources` omitted so Cursor's own rules/hooks/MCP layers are not loaded | agentId |
| OpenCode (`opencode`) | HTTP REST + SSE (`@opencode-ai/sdk`, `session.promptAsync`, `event.subscribe`, `session.abort/fork/revert`), v2 via `@opencode/client` | 1.x: T3-managed `opencode serve --hostname --port` **per thread**; 2.x: one server per **instance** for all directories; external server URL supported; basic-auth password | session id |
| Pi (`pi`) | JSONL over stdio of `pi --mode rpc` (own LF-framed protocol, correlated by `id`) | One process per thread; injects T3-owned MCP bridge extension via `--extension` (Pi has no MCP client) | session file path |
| Grok (`grok`) | ACP over stdio | one process per session, ACP flavor with xAI extensions (`x.ai/ask_user_question`, `x.ai/exit_plan_mode`, background-task tracking) | ACP session id |
| Antigravity (`antigravity`) | ACP over stdio | profile-isolated home per instance, managed installer with version leases | ACP session id |
| ACP Registry (`acpRegistry`) | ACP over stdio | agent from the official ACP registry; binary/npx/uvx distributions, cached/versioned installer; no agent-specific code allowed in this driver (new quirks get a dedicated flavor/driver) | ACP session id |

All process spawns go through a common resolver (`resolveSpawnCommand`: Windows shell wrapping,
`.cmd` shims), `detached: true` + process-group signalling on POSIX so entire trees die together.

### 5.2 Codex app-server (richest protocol)

Wrapper (`packages/effect-codex-app-server`): `CodexAppServerClient.request(method, params)`
(typed by generated tables), `notify`, `handleServerRequest(method, handler)`,
`handleServerNotification(method, handler)`, plus unknown-method handlers; patched JSON-RPC
protocol layer (`protocol.ts`) with termination-error propagation (process exit becomes a typed
error), ring of raw messages for diagnostics, in-memory stdio and a **replay driver**
(`replay.ts`: recorded transcript as the only mocked boundary in tests).

Client request methods used by the adapter: `initialize`, `thread/start|resume|fork|read|archive|
unsubscribe|compact/start|inject_items|backgroundTerminals/{list,terminate}`, `turn/start|steer|interrupt`,
`model/list`, `account/read`, `account/rateLimits/read`, `skills/list`, `feedback/upload`, `review/start`.
Server -> client requests (need a response): `item/commandExecution/requestApproval`,
`item/fileChange/requestApproval`, `item/permissions/requestApproval`, `mcpServer/elicitation/request`,
`item/tool/requestUserInput`, `item/tool/call` (dynamic tool callbacks), `account/chatgptAuthTokens/refresh`,
`attestation/generate`, and legacy `execCommandApproval`, `applyPatchApproval`.

Key notifications (80 total): `thread/started|status/changed|tokenUsage/updated|name/updated|compacted`,
`turn/started|completed|plan/updated|diff/updated`, `item/started|completed`,
`item/agentMessage/delta`, `item/reasoning/{textDelta,summaryTextDelta,summaryPartAdded}`,
`item/plan/delta`, `item/commandExecution/outputDelta|terminalInteraction`,
`item/fileChange/outputDelta|patchUpdated`, `serverRequest/resolved`, `account/rateLimits/updated`,
`model/rerouted`, `mcpServer/startupStatus/updated`, `hook/started|completed`, `thread/realtime/*`.

Native item types seen by the adapter: `userMessage`, `agentMessage`, `reasoning`, `commandExecution`,
`fileChange`, `mcpToolCall`, `dynamicToolCall`, `webSearch` (actions search/openPage/findInPage/other),
`plan`, `contextCompaction`, `collabAgentToolCall` (tool `spawnAgent` etc., `receiverThreadIds`),
`subAgentActivity`.

Mapping to canonical:

| Codex | Canonical |
|---|---|
| `thread/start|resume` response | ProviderThread (`nativeThreadRef strong`), `status idle` |
| `turn/start` response `turn.id` | ProviderTurn (strong id), root run registered; wait for `turn/started` if `startedAt` null |
| `item/agentMessage/delta` | coalesced (`ProviderTextDeltaCoalescer`) -> `assistant_message` upserts `streaming:true` |
| `item/started|completed` commandExecution | `command_execution` item + `tool_call` node |
| mcpToolCall / dynamicToolCall | `dynamic_tool` item (+ `toolSource`/icon presentation from `CodexToolPresentation.ts`) |
| fileChange | `file_change` item (diff from `fileChange/outputDelta`/`patchUpdated`) |
| plan item + `item/plan/delta` | `proposed_plan` |
| `turn/plan/updated` | `plan.updated` kind `todo_list` |
| `*/requestApproval` | node(approval_request) + RuntimeRequest(kind command/file-change/permission/mcp-elicitation, `live`) + `approval_request` item; handler awaits a `Deferred` resolved by `respondToRuntimeRequest`; `acceptAlways` is sent as `acceptForSession` |
| `item/tool/requestUserInput` | `user_input_request` + RuntimeRequest `user_input`; reply maps answers by question id |
| `turn/completed` (status completed/interrupted/failed, error code `usageLimitExceeded`/`rateLimitExceeded` -> `usage_limit`) | `turn.terminal` — **the** authoritative terminal event (`thread/status/changed` going idle is not) |
| `collabAgentToolCall` `spawnAgent` + child `turn/*` | `subagent` node/item, child ProviderThread per `receiverThreadIds`, child app thread (`app_thread.created`), child turn completions never close the parent |
| `thread/tokenUsage/updated` | `ThreadTokenUsageSnapshot` + per-turn `TurnTokenUsage` deltas |
| `account/rateLimits/updated` | provider snapshot `usageLimits` window update |
| `model/rerouted`, `thread/compacted`, `contextCompaction` | `system_notice` / `compaction` item |

Control:
- `steerTurn`: `turn/steer {threadId, expectedTurnId, input}` — requires active turn; error if not.
- `interruptTurn`: `turn/interrupt {threadId, turnId}`; terminal arrives later; also terminates background terminals (`thread/backgroundTerminals/terminate`).
- `injectHistory`: `thread/inject_items` (method-not-found `-32601` => returns false => inline in prompt).
- `compactThread`: `thread/compact/start`.
- `rollbackThread`: `thread/revert`-style (authoritative snapshot returned; `CodexThreadRevert.ts`).
- `forkThread`: `thread/fork` (from turn, from subagent thread).
- `unloadThread`: `thread/unsubscribe`.

Developer instructions: when T3's MCP is attached, `developer_instructions` and `additionalContext`
(model, effort, tool availability) are sent each turn through `collaborationMode.settings` / `additionalContext`.
T3's MCP server is injected via `-c mcp_servers.t3-code.url=...` /
`bearer_token_env_var="T3_MCP_BEARER_TOKEN"` launch overrides.

Status probe (`Layers/CodexProvider.ts`): short-lived `codex app-server`, `initialize` (parse
`userAgent` for version), `account/read` (if `!account && requiresOpenaiAuth` => unauthenticated), then
parallel `skills/list`, `model/list` (all pages), `account/rateLimits/read` (timeout-bounded enrichment,
failure degrades to "no usage").

### 5.3 Claude (Agent SDK)

- One `query` per native thread with `prompt` = async iterable fed from a queue: `offer(SDKUserMessage)`
  is how new turns **and steering** are delivered. Steering message has `priority: "now"`; adapter
  records `steeredTurns`. Interrupt = `query.interrupt()` then `close()`; wait up to 10 s for close,
  else finalize turn as `interrupted` itself.
- Runtime policy -> `permissionMode` (see 4.1). `canUseTool` callback installed only when
  approvals are needed (`installPermissionCallback`). The callback special-cases:
  - `AskUserQuestion` -> `user_input_request` + `user_input` RuntimeRequest (blocks on answers; allow with `updatedInput.answers`).
  - `ExitPlanMode` -> captured as `proposed_plan`; **denied** with a message telling the model to stop and wait; plan mode state restored on the live query afterward.
  - `Agent` (subagent launch) -> remembered for subagent correlation.
  - everything else -> approval `RuntimeRequest` (kind from tool name) when required.
- Live-query reuse is keyed by `claudeEffectiveQueryPolicyKey` (permissionMode, tools, allowedTools, MCP preapprovals, model `queryIdentity`). Changing model/settings while background agents/commands are pending is refused (`ClaudeBackgroundWorkBlocksQueryReplacementError`) because replacing the query would kill them.
- SDK messages handled: `system` (`init`, `status`, `task_started`, `task_progress`, `task_notification`, `compact_boundary`, `api_retry`, `model_refusal_fallback`), `stream_event` (partial deltas; thinking blocks), `assistant`, `user` (tool results), `result` (`subtype` + `is_error`, `api_error_status` 429/529 -> usage_limit / overload), `rate_limit_event`. `parent_tool_use_id` attributes messages to subagents.
- Tool classification -> `command_execution` (Bash), `file_change` (Edit/Write/MultiEdit/NotebookEdit; `diffStr` from tool result text), `web_search` (WebSearch/WebFetch), `file_search`, else `dynamic_tool`; `TodoWrite` (root only) -> `todo_list` plan.
- Background work: Claude can run background shells/subagents/monitors that outlive the root turn and wake the model with a new turn ("wake turn", `task-notification` result origin). The adapter keeps a **roster** (`pendingBackgroundTasks`) and exposes `hasPendingBackgroundWork*` so (a) idle release is deferred, (b) run ingestion doesn't stop at root terminal while child work is pending, (c) restart recovery can tell the next turn what was cancelled (`restartCancelledBackgroundWork`).
- Rollback/fork: `resumeSessionAt` message-uuid; SDK `forkSession`; no thread snapshot read (`canReadThreadSnapshot:false`) — history is reconstructed from CLI session storage by a worker (`claude-history-worker.ts`).
- MCP: HTTP server `t3-code` with bearer header in query options, plus `allowedTools: ["mcp__t3-code__*"]`.
- Status probe: `claude auth status` + SDK `initializationResult()` using a never-yielding prompt (no API request) to get account info, models, slash commands, skills; separate timeout for the experimental `usage` call.

### 5.4 ACP (generic) and flavors

`AcpAdapterV2` is one shared adapter (7.9k lines) parameterized by an `AcpAdapterV2Flavor` record:
`driver`, `capabilities`, `makeRuntime`, `normalizeSessionUpdate`, `normalizeToolCall`,
`permissionDisposition`, `approvalOptions`, `extractPermissionQuestion`, `sessionModeForPolicy`,
`applyModelSelection`, `resolveModelId`, `clientFileSystem` (opt-in ACP `fs/*` handlers),
`extractSubagentUpdate`, `extractProposedPlanMarkdown`, `registerExtensions`, `supportsCompaction`,
`preferResumeSession`, `promptFailure`, `subagentsIdleOnTurnCompletion`, `onSessionEvent`, ...
Grok, Antigravity, and generic registry agents are flavors. **This "base adapter + flavor hooks"
structure is the best-factored part of the codebase.**

`AcpSessionRuntime` (`provider/acp/AcpSessionRuntime.ts`) responsibilities: spawn (+optional
detached process group, descendant process-group ownership, Linux cgroup lease wrapper
`t3-acp-<pid>-*` with stale-sibling sweep, POSIX ownership ledger, Windows `taskkill`), `initialize`
(client capabilities/info/meta), auth (`authenticate` when `-32000` auth-required, or eagerly),
`session/new|load|resume|fork`, model/config option application, `session/prompt`,
`session/cancel` (`cancelBehavior: interrupt|wait-for-prompt`, 15 s timeout), stderr capture
(32 KiB chunk cap, redaction), session-load replay idle-gap detection (history replay must be
drained before first prompt), event stream with barriers.

ACP `session/update` -> parsed events (`AcpParsedSessionEvent`): `ModeChanged`,
`AvailableCommandsUpdated`, `ConfigOptionsUpdated`, `AssistantItemStarted|Completed`, `PlanUpdated`
(kinds items / markdown / file / unknown / removed), `ToolCallUpdated`, `ContentDelta`,
`ThoughtDelta`, `UsageUpdated`, `SessionInfoUpdated`, `UnknownUpdate`. Tool kind mapping
(`canonicalItemTypeFromAcpToolKind`): `execute`->`command_execution`; `edit|delete|move`->`file_change`;
`search|fetch`->`web_search`; `read` and unknown->`dynamic_tool_call`. `session/request_permission`
is answered by `AcpClientPolicy` (`allow|ask|deny` by runtime mode, tool kind, path containment with
symlink-safe canonicalization that fails closed) or surfaced as an approval RuntimeRequest.

ACP limitations as modeled (`AcpProviderCapabilitiesV2`): no active steering (cancel-and-restart),
no native subagents, no MCP-by-default (`supportsMcpTools` false unless the agent advertises),
`enforcement: "client-boundary"`, weak turn/item/request ids, no thread snapshot read, fork only if
agent advertises `session/fork`. ACP v2 draft also adds providers/list|set|disable, `session/delete`,
elicitation, `mcp/connect|message|disconnect` (ACP-native MCP over the client connection).

### 5.5 Cursor, OpenCode, Pi (brief)

- **Cursor SDK**: no approvals (`supportsCommandApproval:false`, `enforcement:"native"` via SDK
  sandbox/approval mode), no fork/rollback, strong turn ids, structured plan streaming,
  steering via interrupt-restart, `InteractionUpdate` deltas map to items; transport failures
  classified in `CursorTransportFailure.ts`.
- **OpenCode 1.x/2.x**: SSE events `server.connected`, `message.updated`, `message.part.updated|delta`
  (part types `text`, `reasoning`, `tool`, `step-finish`, `file`), `todo.updated`, `permission.asked|replied`,
  `question.asked|replied|rejected`, `session.created|updated|deleted|status|idle|error|compacted`.
  Permissions are directory/project-scoped on the server ("always" grants affect the whole project, so
  T3 replies `once` for auto-approvals). Orphan protection: spawned `opencode serve` groups are recorded
  in an on-disk **ledger** (`pgid`, pid, start time, command, port, owner identity) and swept on next
  server start if the T3 server crashed.
- **Pi**: events `agent_settled` (the only terminal signal; `agent_end` only closes one low-level run
  because compaction/auto-retry/queued continuations may follow, and the adapter double-checks idle),
  `extension_ui_request` (dialogs -> `confirm` => approval_request; `select|input|editor` =>
  user_input_request; `notify` => completed activity), responses via `extension_ui_response`.
  Session file path is the native thread ref so TUI and T3 can share sessions. Fork uses Pi's CLI in
  the destination cwd (RPC session switching retains source cwd).

### 5.6 Cross-provider event-ordering machinery

Adapters buffer and reconcile a lot:
- **Text delta coalescing** per `(turnId,itemId)` with a flush interval, complete-with-final-text, `flushTurn` at terminal.
- **Held root frames** (Claude): output arriving before the prompt echo may belong to a queued wake turn; frames are held until the echo identifies the owning turn.
- **Item ordinals**: `resolveItemOrdinal(context, key)` gives replay-deterministic `ordinal` per turn; `ordinal` orders the timeline independent of arrival order.
- **Run ownership routing** (`RunExecutionService.routeProviderEvent`): per-run state of `ownedThreadIds`, `ownedProviderThreadIds`, `ownedProviderTurnIds`, `rootProviderTurnId`, `inheritedBackgroundTurnItems` decides which run an event belongs to (child threads' events attach to the parent run; background items inherited across runs).
- **Stop gate** for ingestion: `Stream.takeUntilEffect(shouldStopProviderEventIngestion)`: stop only after root `turn.terminal` *and* no pending background work for that provider thread.

---

## 6. Capability flags and feature differences

### 6.1 Flag groups (`OrchestrationV2ProviderCapabilities`)

| Group | Flags |
|---|---|
| `sessions` | supportsMultipleProviderThreadsPerSession, supportsModelSwitchInSession, supportsProviderSwitchingViaHandoff, supportsRuntimeModeSwitchInSession, pendingRequestsSurviveRestart |
| `threads` | canCreateEmptyThread, canReadThreadSnapshot, canRollbackThread, canForkThread, canForkFromTurn, canForkFromSubagentThread, exposesNativeThreadId |
| `turns` | exposesNativeTurnId, emitsTurnStarted, emitsTurnCompleted, supportsInterrupt, supportsActiveSteering, supportsSteeringByInterruptRestart, supportsQueuedMessages, terminalStatusQuality (strong\|weak\|none) |
| `streaming` | streamsAssistantText, streamsReasoning, streamsToolOutput, streamsPlanText, emitsMessageCompleted |
| `tools` | exposesToolItemIds, emitsToolStarted, emitsToolCompleted, emitsToolOutput, supportsMcpTools, supportsDynamicToolCallbacks |
| `approvals` | supportsCommandApproval, supportsFileReadApproval, supportsFileChangeApproval, supportsApplyPatchApproval, approvalsHaveNativeRequestIds, approvalCallbacksAreLiveOnly, approvalsCanOriginateFromSubagents |
| `planning` | emitsPlanUpdated, emitsTodoList, emitsProposedPlan, supportsStructuredQuestions, planDeltasHaveItemIds |
| `subagents` | supportsSubagents, exposesSubagentThreadIds, emitsSubagentLifecycle, canWaitForSubagents, canCloseSubagents, canForkSubagentThread |
| `context` | acceptsSystemContext, acceptsDeveloperContext, acceptsSyntheticUserContext, canGenerateSummaries, canConsumeHandoffSummaries, supportsDeltaHandoff, supportsFullThreadHandoff, maxRecommendedHandoffChars |
| `checkpointing` | appCanCheckpointFilesystem, supportsNestedCheckpointScopes, providerCanRollbackConversation, providerRollbackReturnsSnapshot, providerCanReadConversationSnapshot |
| `identity` | nativeThreadIds / nativeTurnIds / nativeItemIds / nativeRequestIds : strong\|weak\|none |
| `runtimePolicy` | enforcement: native\|client-boundary |

### 6.2 Matrix (flags that differ; extracted from each adapter's `*ProviderCapabilitiesV2`)

Columns: Cx=Codex, Cl=Claude, ACP=generic ACP, Cu=Cursor, OC=OpenCode 1.x, OC2=OpenCode 2.x, Pi.
Grok = ACP + `sessions.supportsModelSwitchInSession`, `tools.supportsMcpTools`,
`checkpointing.providerCanReadConversationSnapshot`; `supportsSubagents` + thread ids exposed via
orchestrator-owned children; `canFork*` false. Antigravity = ACP + model switch, runtime-mode switch,
MCP, subagents.

| Flag | Cx | Cl | ACP | Cu | OC | OC2 | Pi |
|---|---|---|---|---|---|---|---|
| multiple provider threads / session | Y | N | N | N | N | Y | N |
| model switch in session | Y | Y | N | Y | Y | Y | Y |
| runtime-mode switch in session | Y | N | N | N | N | Y | N |
| read thread snapshot | Y | N | N | Y | Y | Y | Y |
| rollback thread | Y | Y | Y | N | Y | Y | Y |
| fork / fork-from-turn | Y/Y | Y/Y | N/N | N/N | Y/Y | Y/Y | Y/Y |
| fork from subagent thread | Y | N | N | N | Y | N | N |
| native turn ids | Y | N | N | Y | N | N | N |
| active steering | Y | Y | N | N | Y | Y | Y |
| steer by interrupt+restart | Y | N | Y | Y | Y | Y | N |
| stream tool output | Y | N | Y | Y | Y | N | Y |
| stream plan text | Y | N | N | Y | N | N | N |
| MCP tools | Y | Y | N | Y | Y | N | Y |
| dynamic tool callbacks | Y | Y | N | N | N | N | N |
| command / file-change approvals | Y/Y | Y/Y | Y/Y | N/N | Y/Y | Y/Y | Y/Y |
| file-read approval | Y | Y | Y | N | Y | Y | N |
| apply-patch approval | Y | N | N | N | Y | Y | N |
| native request ids | Y | Y | N | N | Y | Y | Y |
| approvals from subagents | Y | N | N | N | Y | Y | N |
| plan updated / todo | Y/Y | Y/Y | Y/Y | Y/Y | Y/Y | N/N | N/N |
| proposed plan | Y | Y | N | Y | N | N | N |
| structured questions | Y | Y | Y | N | Y | Y | Y |
| plan deltas have item ids | Y | N | N | Y | N | N | N |
| subagents | Y | Y | N | Y | Y | Y | Y |
| subagent thread ids | Y | N | N | N | Y | Y | N |
| wait / close subagents | Y/Y | N/N | N/N | Y/N | Y/N | Y/N | N/N |
| accepts system / developer context | Y/Y | Y/Y | N/N | N/N | N/N | N/N | N/N |
| can generate summaries | Y | Y | Y | Y | Y | N | N |
| nested checkpoint scopes | Y | Y | Y | Y | Y | Y | N |
| provider rollback / snapshot return | Y/Y | Y/Y | Y/Y | N/N | Y/Y | Y/Y | Y/Y |
| identity strength thread/turn/item/request | S/S/S/S | S/W/S/S | S/W/W/W | S/S/W/none | S/W/S/S | S/W/S/S | S/W/S/S |
| runtime policy enforcement | native | native | client-boundary | native | native | (n/a) | (n/a) |

Always true for all: `canCreateEmptyThread`, `exposesNativeThreadId`, `emitsTurnStarted/Completed`,
`supportsInterrupt`, `supportsQueuedMessages`, streams assistant text and reasoning,
tool ids/started/completed/output events, `supportsProviderSwitchingViaHandoff`,
`supportsSyntheticUserContext`, delta + full handoff, `appCanCheckpointFilesystem`, `pendingRequestsSurviveRestart:false`.
`terminalStatusQuality` is "strong" everywhere in practice (hard-won by per-adapter terminal
inference, e.g. Pi `agent_settled`).

### 6.3 Snapshot-level (non-session) capability fields on `ServerProvider`

`showInteractionModeToggle`, `reportsContextWindow`, `supportedRuntimeModes[]`,
`requiresNewThreadForModelChange`, `supportsConversationRollback`, `supportsTextGeneration`,
`setup {canAuthenticate, canInstall, documentationUrl}`, `nativeSessions {canList, canLoad, canResume, canDelete}`,
`configurableProviders`, `runtimePaths {homePath, shadowHomePath}`, `continuation {groupKey}`,
`iconUrl`, `badgeLabel`. UI affordances are driven from these + session capabilities.

### 6.4 Degradation policies (from `provider-capability-system.md`)

```
interrupt unsupported          -> stop session if allowed, else mark unsupported
active steering unsupported    -> interrupt active turn, restart run as steering replacement attempt (RunAttempt reason steering_restart)
fork unsupported               -> synthetic fork from app projection (ContextTransfer portable_context)
rollback unsupported           -> restore filesystem checkpoint, restart provider context, mark provider state divergent
nested checkpoint unsupported  -> only root-run checkpoint; child filesystem activity recorded as uncheckpointed
provider-switch return unsupp. -> fresh provider thread with full app-thread summary
structured approvals unsupp.   -> run under configured sandbox, no approval UI
plan_updated unsupported       -> no todo UI; rely on assistant text
streaming message-complete missing -> normalizer closes assistant messages at root terminal
```

Caveat discovered in code: several capability reads are still provider-name conditionals inside
adapters (e.g. `Agent`/`ExitPlanMode` tool names, `x.ai/*` extensions), and the ACP registry
adapter explicitly forbids adding more `agentId === ...` branches without maintainer approval,
which shows the capability model does not fully remove per-agent quirks.

---

## 7. Lifecycle state machines

### 7.1 Provider session (live runtime)

```
(none) --open--> starting --initialize/auth ok--> ready
ready --startTurn--> running --turn.terminal--> ready
running --pending approval/user-input--> waiting --respond--> running
ready|running|waiting --idle_timeout(30m; deferred while background work pending, max 4h)--> stopped (released)
any --process exit / protocol error / release(runtime_error)--> error | stopped
stopped|error --next user command--> open (new ProviderSessionId) --resumeThread--> ready
```

Process loss while turns/requests are in flight: `ProviderRuntimeRecoveryService` terminalizes
provider-bound runs, stops sessions, closes requests (`not_resumable`/`cancelled`), retires
non-replayable outbox effects, requeues replay-safe ones, and appends a restart continuation note
(`restartCancelledBackgroundWork`) to be delivered with the next provider turn.

### 7.2 Provider thread

```
not_loaded --ensureThread (no native ref)--> idle (native thread created lazily, may be at first turn)
not_loaded --resumeThread(nativeThreadRef)--> idle
idle --startTurn--> active --turn.terminal--> idle
any --unloadThread / detach--> not_loaded      (native ref kept)
any --archive--> archived ; --close--> closed
any --resume failure / threadDisposition "broken"--> error ; next run: ensureThread with nativeThreadRef=null + provider_resume_fallback handoff
```

### 7.3 Run / attempt / provider turn

```
Run:     preparing -> queued -> starting -> running <-> waiting -> completed | interrupted | failed | cancelled ; (completed) -> rolled_back (checkpoint rollback)
Attempt: pending -> running -> completed | interrupted | failed | cancelled | superseded   (steering_restart / retry / provider_recovery create the next attempt)
Turn:    pending -> running -> completed | interrupted | failed | cancelled
```

Start sequence (`ProviderTurnStartService.start`):

```
load projection (run, message, thread, handoffs, transfers)
-> resolve RuntimePolicy (cwd, runtimeMode clamped by supportedRuntimeModes)
-> ProviderAuth check -> ProviderSessionManager.open(session) [retryable: willRetry => surface error to caller; else settle run failed]
-> thread binding:
     native fork transfer pending?   -> session.forkThread(source)
     else nativeThreadRef == null    -> session.ensureThread(existingProviderThread)
     else uncertain history delivery -> fail resume deliberately
     else                            -> session.resumeThread ; on failure -> ensureThread(nativeThreadRef=null) + resume-fallback handoff
-> prepare ContextHandoff delivery (budget-aware): session.injectHistory? else inline in user text
-> [compact path: compactThread] else session.startTurn(turnInput with handoff context + restart note)
-> RunExecutionService.startRootRun: subscribe events, route/ingest until stop gate
```

Run finalization barrier: root `turn.terminal` -> flush assistant text buffers -> close streams ->
finalize plans/todos/questions -> terminalize open non-live child nodes -> capture checkpoint ->
publish run terminal. Early checkpoint capture is the bug class this prevents.

### 7.4 Steering and interruption

```
message.dispatch mode:
  start_immediately | defer_start        : new run (startTurn)
  queue_after_active                     : new queued run
  steer_active{targetRunId}              : adapter.steerTurn (needs supportsActiveSteering); error "turn ended before steering message delivered" if turn not running
  restart_active{targetRunId}            : interruptAndAwaitTerminal(old attempt) -> new RunAttempt(reason steering_restart) on same Run
run.interrupt                            : adapter.interruptTurn(requestRuntimeRestart:true) ; terminal arrives as turn.terminal(interrupted)
```

`interruptAndAwaitTerminal` polls projection up to 1000 times, yielding to the event loop each
time (not clock sleeps, to avoid deadlock with deterministic test clocks) until both providerTurn
and attempt are non-running. If no live session exists, interrupt/restart are treated as
already-stopped (no failing durable effect, no retries). `requestRuntimeRestart` lets an adapter
respawn its runtime on the next `startTurn` (Grok stop recovery); Claude uses it to close the
CLI process so orphaned background shells die.

### 7.5 Runtime request (approval / user input)

```
provider asks (server->client request/callback)
 -> adapter creates Deferred, emits node.updated(approval_request|user_input_request) + runtime_request.updated(pending, live)
    + turn_item.updated(approval_request|user_input_request)    [run status -> waiting]
user responds: runtime-request.respond(RuntimeRequestId, decision|answers)
 -> command handler validates pending, persists status=resolved (+ answers) atomically
 -> RuntimeRequestService.respond requires status==resolved, responseCapability.live for same providerSessionId,
    session present -> runtime.respondToRuntimeRequest -> Deferred resolves -> native response returned
terminal turn event / session loss -> pending requests => cancelled / not_resumable ; node + item => cancelled
```

Subtle: persistence of the resolution happens *before* the side effect (the outbox executes the
provider call), so a crash between them yields a resolved-but-undelivered response, which is
reported as `request-not-resumable` rather than lost silently.

### 7.6 Auth flow (provider setup)

```
ProviderAuthState.phase: idle -> starting -> waiting (interaction: browser|deviceCode|terminal|credentials) -> verifying -> succeeded | failed | cancelled
methods: [{ id, name, description, type: agent|terminal|credentials, accountEmail? }]
```

Flows have owner session ids, lifetime timeouts, optional client-side callback mode (loopback listener
may be on another machine), and `credentialOwner: provider|t3`. Sign-out closes admission to new
processes and stops existing ones *before* clearing metadata (otherwise a resumed session retains the
old account).

---

## 8. Capabilities discovery, model listing, auth status

Snapshot (`ServerProvider`) assembled by `makeManagedServerProvider`:
`checkProvider` effect (probe), `initialSnapshot(settings)` (cheap placeholder), optional
`enrichSnapshot` (async slow data, generation-counted so stale enrichment is dropped),
`refreshInterval`, `checkProviderOnSettingsChange`, `streamChanges` PubSub.
`BackgroundPolicy` throttles background probes. Maintenance: `resolveMaintenance({fresh})` returns
ownership-derived update capability (npm/brew/native-path proof; only offer one-click update when the
resolved executable path proves which installer owns it; otherwise manual-only but still show version gap).

Per provider:

| Provider | Version / install | Auth | Models | Extras |
|---|---|---|---|---|
| Codex | `initialize.userAgent` | `account/read` (+ `requiresOpenaiAuth`), managed ChatGPT sign-in (callback server, optional primary handoff) | `model/list` pages; preferred-default list; overlay manifest | skills, rate limits, reset credits |
| Claude | `claude --version` | `claude auth status`, plus SDK init `accountInfo`; subscription type | bundled+remote manifest catalog with option descriptors (effort, fastMode, thinking, context) | slash commands, skills (user-invocation-only flags), usage windows (five_hour, seven_day...) |
| Cursor | SDK | credential store / keychain token | SDK catalog (`CursorSdkCatalog`) | usage limits |
| OpenCode | `opencode --version` probe, 1.x vs 2.x runtime pick | server-side | server inventory | permissions scoping |
| Pi | `pi --version`, `launchArgs` | user's own Pi auth | user's settings.json models | thinking capabilities |
| Grok | CLI probe that avoids creating sessions | CLI auth | ACP config options | xAI usage |
| Antigravity | managed installer w/ leases | per-instance profile; OAuth browser/API key/Agent Platform | catalog only after explicit setup | profile isolation |
| ACP registry | registry catalog + cached distribution | disposable `session/new` success == authenticated | models from session/new | sessions list/delete, providers list/set |

Rule (docs/internals/providers.md): **setup must not happen as a health-check side effect.**
Opening a session can start MCP servers, run hooks, or open a login browser; probes use initialize-only
or never-yielding prompts. Compatibility manifest per driver gives `supported|graceful|unsupported|broken` by version range.

---

## 9. T3-owned MCP bridge (cross-cutting adapter feature)

T3 runs an authenticated HTTP MCP endpoint `http://127.0.0.1:<port>/mcp` (server key `t3-code`).
Per provider session `McpSessionRegistry` issues a bearer credential scoped to
{environment, thread, provider instance, capabilities: orchestration | worktree | pull-requests | preview | device};
expires by max lifetime and idle, revoked on release/detach. Each adapter injects it its own way:

| Provider | Injection |
|---|---|
| Codex | `-c mcp_servers.t3-code.url=...` + bearer env var |
| Claude | `mcpServers` http + `allowedTools: mcp__t3-code__*` |
| Cursor | SDK `mcpServers` option |
| Grok/ACP | `mcpServers` in `session/new|load|fork` |
| OpenCode 2 | per-thread `t3-code-<thread>` MCP entry; session permission rules deny other threads' entries |
| Pi | generated extension that bridges MCP tools via `pi.registerTool` |

Tools include `delegate_task` (create an app-owned subagent on any provider instance), thread
launch/list/read/rename/send/wait/interrupt, PR linking, worktree, preview/device. Purpose: uniform
cross-provider orchestration + "subagents on any provider" without native subagent support.

---

## 10. Context handoff, switching, forking (provider-agnostic continuity)

- Provider switching between runs is a first-class `ContextHandoff` artifact (strategies:
  `delta_since_target_last_seen`, `fork_delta_summary`, `full_thread_summary`, `checkpoint_summary`,
  `manual_context`), shown in the timeline as a `handoff` item. Returning to a provider resumes its
  previous native thread and injects only the off-provider delta.
- Delivery: `injectHistory` if supported, else inline in the next user message; `delivery.status:
  pending|injected|inline` plus `itemIds` / `omittedItemIds` track what the native model has seen;
  a budget (`ContextHandoffBudget`) limits size by model window and `maxRecommendedHandoffChars`.
- Fork = new AppThread with a pending `ContextTransfer(type fork)`; resolved lazily on first run:
  `native_fork` (same provider and `canForkFromTurn`) else portable context. Stable fork points:
  completed run, checkpoint, idle provider thread; unstable: streaming deltas, active tool, pending
  approval.
- `ContextTransferType`: fork | provider_handoff | merge_back | subagent_spawn | subagent_result.
- Checkpoints are app-owned (git-backed filesystem snapshots, nested scopes for subagents); rollback
  restores files then asks the provider to roll back conversation (Codex returns an authoritative
  snapshot; others restart from reconstructed context and mark divergent). Capability gating example:
  Antigravity can checkpoint but not roll back conversation => revert rejected *before* touching files.

---

## 11. Design strengths worth keeping

1. **Instance vs driver split**, routing by instance, open slug for driver kinds, "unavailable
   shadow" instead of decode failure. Multi-account support falls out for free.
2. **App ids are identity; provider ids are refs**, with explicit strength (strong/weak/none) and
   scoped ordinal/fingerprint correlation. Makes weak protocols (ACP) workable and replay deterministic.
3. **Run / Attempt / ProviderTurn / ExecutionNode separation**: steering-by-restart, retry,
   subagents and wake turns all fit without hacks. "Only root terminal completes a run."
4. **Entity-upsert event stream from adapters** (idempotent snapshots keyed by app id) is simple to
   persist, replay, and render; no delta-merging logic downstream except the one text-coalescer.
5. **Capability flags + degradation policies** instead of provider-name `if`s; UI gets typed affordances.
   `planSelectionTransition` makes "can I change model mid-session?" the adapter's decision.
6. **Authoritative terminal event** (`turn.terminal` with `threadDisposition`) separate from
   acknowledgements; control calls never mark terminal state.
7. **ACP base adapter + Flavor hooks** with a hard rule against agent-specific branching in the
   generic registry driver. Gives many agents "for free" via the open ACP standard.
8. **Protocol wrappers generated from upstream schemas** (Codex JSON schemas; ACP schema release)
   and a **replay harness** where only the transport is faked; tests assert whole-pipeline behaviour.
9. **Safety/robustness details**: process-group + cgroup + ledger cleanup of orphaned agent trees;
   path-containment checks with symlink canonicalization failing closed; redaction/bounded failures;
   bounded raw logs (64 KiB, deltas dropped); credential admission locks around sign-in changes;
   persisted vs live-only request distinction (`responseCapability`).
10. **Unified runtime mode** (4 levels) mapped to each provider's native approval/sandbox model with
    a defined fallback (`approval-required`) and an explicit native-vs-client-boundary enforcement flag.
11. **Auditable cross-provider continuity** (ContextHandoff items) rather than hidden prompt hacks.
12. **Model options as data** (descriptors) so new effort/fast/thinking knobs need no new typed UI.

## 12. Weaknesses and pitfalls to avoid in a Rust redesign

1. **Adapter bloat**. 86k lines; Claude/ACP/Codex adapters are 6-8k lines each with dense
   interleaved concerns (wire decoding, ordering, dedup, background tasks, ids, permissions,
   projection building). Split each adapter into: (a) wire client (typed, generated), (b) a pure
   *reducer* `fn(state, native_msg) -> (state, Vec<CanonicalEvent>)` that is unit-testable with
   fixtures, (c) thin effectful shell. T3's adapters mix (b) and (c).
2. **Adapters emit full domain snapshots** (TurnItem with all base fields, ExecutionNode,
   RuntimeRequest) so every adapter must know about app ids, ordinals, checkpoint scopes, run
   ordinals, parent nodes. A narrower canonical event enum emitted by adapters (item started /
   delta / completed with *native* ids, request opened, turn terminal) plus a single shared
   normalizer that allocates app ids and builds nodes/items would remove most duplication and make
   fixtures provider-neutral. (The legacy 52-type `ProviderRuntimeEvent` was that design; T3 moved
   away from it for fidelity, at a large cost in adapter code. A middle path is better.)
3. **Too many near-duplicate entities**: ExecutionNode, TurnItem, RuntimeRequest, PlanArtifact,
   Subagent, Message describe overlapping facts (an approval is a node + a request + an item).
   Consider one item-with-kind-specific-state record plus a request table. Keep the graph only where needed.
4. **Capability explosion**: ~60 booleans across 11 structs, 7 adapters each hand-writing a full
   copy (spread-merge from ACP base). Several never vary (always true). Prefer a smaller set
   (e.g. steering mode enum {native, interrupt_restart, none}; fork {native_turn, native_thread,
   none}; approvals {per-kind set}; ids {strength x4}) and defaults with overrides.
5. **Hidden provider-name branches** despite the capability system: tool names (`Agent`,
   `ExitPlanMode`, `TodoWrite`, `AskUserQuestion`), `x.ai/*` extensions, Codex `acceptAlways->
   acceptForSession`, OpenCode "always means project-wide" rule. These need a named place in the
   adapter (a quirks table) and tests.
6. **Wake turns / background tasks (esp. Claude)**: background shells/subagents outliving the root
   turn force held-frame buffering, rosters, pending-work probes in idle release and stop gates,
   restart notes for cancelled background work, and replacement-query refusal. A Rust design should
   decide upfront how a "turn" relates to background work (e.g. treat the session as a long-lived
   stream of *activity* with explicit run boundaries) instead of retrofitting.
7. **Dual event vocabularies** (legacy runtime events vs v2 adapter events vs domain events vs raw
   frames) with partial overlap; some contract types still import from the legacy file. Keep exactly
   one canonical vocabulary and one raw-log format.
8. **Polling for terminalization** (`interruptAndAwaitTerminal` loops over projection reads 1000x,
   yielding to the event loop). In Rust use a watch channel / oneshot keyed by (turn id) with timeout.
9. **Ambiguous at-least-once delivery** for history injection and approvals persist-then-effect;
   needs explicit "uncertain delivery" markers (which T3 does add after the fact). Design the outbox
   and idempotency keys up front; T3 flags it as a "tracked follow-up".
10. **SDK-in-process dependencies** (Claude Agent SDK, Cursor SDK, OpenCode SDK) have no Rust
    equivalent. Options: spawn `claude` directly with `--input-format stream-json --output-format
    stream-json` and implement the control protocol (permission callbacks `can_use_tool`,
    `interrupt`, `set_model`, `set_permission_mode`, MCP config) yourself, or prefer ACP wrappers
    where available. Budget for protocol drift: T3 maintains a version-range compatibility manifest per
    CLI, because wrappers break on CLI updates.
11. **Process lifetime is the hard part**: detached groups, cgroup leases, orphan ledgers, Windows
    taskkill, stale server cleanup, port allocation races for HTTP servers (OpenCode), stderr
    flooding and secrets in stderr/URLs (32 KiB cap + redaction). Plan a single `ManagedProcess`
    abstraction (own process group, kill-tree on drop, ledger for crash recovery) used by all adapters.
12. **Authorization semantics differ and "always" is dangerous**: provider "always allow" grants
    can widen a project or global policy (OpenCode); T3 maps to `once`/session-only. Define the
    canonical `acceptAlways` to mean "T3-scoped session rule", never a provider-global grant.
13. **Persisted shape evolution cost**: `kindUnionWithFallback`, many `withDecodingDefault`s,
    `Json` twin schemas (`...Json` mirrors of every entity) and legacy decode paths (v1 import,
    `{provider,model}` selection, object-valued options). Version events from day one and keep
    wire types separate from storage types.
14. **Test strategy depends on recorded transcripts per provider** (replay harness); excellent, but
    fixtures are large and version-specific. Keep fixtures normalized (method/payload pairs) and
    tag with CLI version.
15. **Plan mode is implemented by trickery per provider** (deny ExitPlanMode, abandon native gate,
    sniff plan.md, set native mode by name `plan`/`architect`). There is no protocol-level plan
    mode in ACP; expect to keep per-provider shims and a capability bit like `proposed_plan_source`.
16. **Model identity strings are vendor-defined and drift** (aliases, legacy slugs, `auto`,
    `default`, `grok-build` meaning "session's current model"): keep model resolution (alias tables,
    "do not send this id to the agent" sentinels) behind the adapter and manifest-driven.

---

## 13. Suggested shape for a Rust re-design (derived from the above)

Not a spec, a starting point consistent with the findings.

```rust
// Identity
struct ProviderKind(String);           // open slug, serde-transparent; unknown kinds must round-trip
struct InstanceId(String);
struct ModelSelection { instance: InstanceId, model: String, options: Vec<(String, OptionValue)> }

// Static per-driver registration
trait ProviderDriver: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn config_schema(&self) -> ConfigSchema;                 // decoded once into typed config
    async fn create(&self, input: DriverCreateInput) -> Result<Arc<dyn ProviderInstance>, DriverError>;
}

#[async_trait]
trait ProviderInstance: Send + Sync {
    fn id(&self) -> &InstanceId;
    fn capabilities(&self) -> Capabilities;
    fn snapshot(&self) -> watch::Receiver<ProviderSnapshot>;         // version, auth, models, skills, usage
    async fn refresh(&self) -> Result<ProviderSnapshot, ProbeError>; // never starts sessions / auth flows
    fn plan_selection_change(&self, cur: &ModelSelection, next: &ModelSelection, caps: &Capabilities) -> SelectionPlan;
    async fn open_session(&self, req: OpenSession) -> Result<Box<dyn SessionRuntime>, OpenError>;
    fn auth(&self) -> Option<&dyn AuthController>;
}

#[async_trait]
trait SessionRuntime: Send {
    fn events(&mut self) -> mpsc::Receiver<AdapterEvent>;            // or Stream
    async fn ensure_thread(&self, req: EnsureThread) -> Result<ProviderThread, _>;
    async fn resume_thread(&self, req: ResumeThread) -> Result<ProviderThread, _>;   // required primitive
    async fn start_turn(&self, req: StartTurn) -> Result<TurnAccepted, _>;            // ack only
    async fn steer_turn(&self, req: Steer) -> Result<(), SteerError>;                 // NotSupported distinct
    async fn interrupt_turn(&self, req: Interrupt) -> Result<(), _>;                  // ack only
    async fn respond(&self, req: RequestId, answer: RequestAnswer) -> Result<(), _>;
    async fn read_snapshot(&self, t: &ProviderThread) -> Result<Snapshot, _>;         // optional via capability
    async fn rollback(&self, ..) / fork(&self, ..) / inject_history(&self, ..) / compact(&self, ..) / unload(&self, ..);
}

// Adapter output: small, native-id-based, replayable
enum AdapterEvent {
    ThreadBound { native_thread: NativeRef, .. },
    TurnStarted { turn: NativeRef, started_at },
    ItemStarted { item: NativeRef, turn: NativeRef, parent: Option<NativeRef>, kind: ItemKind, title },
    ItemDelta { item: NativeRef, stream: StreamKind /*assistant|reasoning|plan|cmd_output|file_diff*/, text },
    ItemCompleted { item: NativeRef, status, payload: ItemPayload /*cmd|file_change|web_search|tool|subagent*/ },
    PlanUpdated { .. }, UsageUpdated { .. },
    RequestOpened { request: NativeRef, kind: RequestKind, options: Vec<ApprovalOption>, questions: Vec<Question>, response: ResponseMode /*Live|Message*/ },
    RequestResolved { .. },
    TurnTerminal { turn: NativeRef, status, failure: Option<Failure>, thread_disposition: Reusable | Broken },
    Notice { level, message }, RateLimits { .. },
}
```

Principles to carry over: app ids allocated by one shared normalizer; native refs with strength;
single terminal marker per turn; requests carry `ResponseMode::{Live, Message}`; reducer-style
adapters with fixture replay; one `ManagedProcess` abstraction with kill-tree and crash ledger;
generate protocol bindings from upstream schemas (Codex JSON schema; ACP schema crate may exist);
treat ACP as the default integration path and Codex app-server as the reference rich protocol;
keep the capability set small and policy-driven; keep probes side-effect free; store raw frames as
bounded rotating logs only.
