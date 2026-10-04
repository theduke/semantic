# t3code: the custom `t3-code` MCP server exposed to coding agents

Research report. Source: `/home/theduke/dev/github.com/pingdotgg/t3code` (read-only).
Date: 2026-10-03. Purpose: inform a Rust re-implementation (likely on the `rmcp` crate).

Paths below are relative to the t3code repo unless absolute. `S` = `apps/server/src`,
`C` = `packages/contracts/src`.

---------------------------------------------------------------------------------------------

## 0. TL;DR

* t3code runs ONE in-process HTTP MCP server (`POST/DELETE {server}/mcp`, MCP protocol
  `2025-06-18`, streamable HTTP) inside its server process. The server is named
  "T3 Code"; every agent sees it under the MCP server name **`t3-code`**.
* Every agent session (one per thread) gets its OWN opaque bearer token minted at session
  prepare time. The token resolves (via a registry) to an immutable `McpInvocationScope`
  `{environmentId, threadId, providerSessionId, providerInstanceId, capabilities, issuedAt}`.
  Tool handlers never take a "who am I" argument; identity comes from the credential.
  Capabilities (`preview`, `orchestration`, `worktree`, `device`, `pull-requests`) are
  baked into the token and gate tool families.
* The same server serves ~60 tools in 10 toolkits: browser automation (`preview_*`),
  device (simulator/emulator) tools, orchestration (delegate sub-agent tasks, spawn
  threads, send/wait/interrupt/read threads, scheduled tasks), thread metadata (rename,
  PR link), worktree handoff, PR link/watch, project/environment/attachment management,
  queue/pending-request management.
* Attachment per provider: HTTP MCP config (Claude Code, Codex, Cursor, OpenCode), stdio
  bridge child process (`t3 acp-mcp-bridge`) for ACP agents, MCP-over-ACP (unstable ACP
  transport), and a generated TS extension for Pi (which has no MCP client).
* Browser tools are NOT implemented in the server: the server is a broker. It pushes
  requests down a WebSocket RPC stream (`previewAutomation.connect`) to the connected UI
  host (Electron desktop app, webview + CDP + injected Playwright runtime), and the host
  answers via `previewAutomation.respond`. The user watches the same browser tab live.
* Instructions are injected per-provider telling the agent to prefer these tools, with
  availability-gated text (never describe tools the credential does not carry).
* Completion of delegated sub-tasks is *pushed back* to the parent thread as a durable
  "mailbox" steer/continuation message; the tool descriptions tell the agent to end its
  turn instead of polling.

---------------------------------------------------------------------------------------------

## 1. Architecture and file map

```
 Provider CLI/SDK process (claude / codex / cursor / opencode / ACP agent / pi)
        |  HTTP MCP, Authorization: Bearer <per-thread token>
        |  (ACP: via stdio bridge or MCP-over-ACP;  Pi: via generated extension)
        v
 S/mcp/McpHttpServer.ts   (effect `McpServer.layerHttp`, path "/mcp", protocol 2025-06-18)
   |  auth middleware: Bearer -> McpSessionRegistry.resolve -> McpInvocationContext (per request)
   v
 Toolkits (S/mcp/toolkits/*)  ->  services (OrchestratorMcpService, WorktreeMcpService,
                                   ThreadMetadataMcpService, DeviceService, PreviewManager,
                                   Orchestrator (PR link), ProjectService, ...)
   |
   +-- preview_* -> PreviewAutomationBroker -> WS RPC stream -> web UI/Electron host
```

Key files:

| File | Role |
|---|---|
| `S/mcp/McpHttpServer.ts` (729 lines) | Wires toolkits into the HTTP MCP server; auth middleware; custom registration for image-returning tools (`preview_snapshot`, `device_screenshot`); snapshot size bounding; error shaping |
| `S/mcp/McpSessionRegistry.ts` | Token mint/resolve/revoke/touch; SHA-256-hashed token store; 24h liveness window |
| `S/mcp/McpProviderSession.ts` | Process-global `threadId -> McpProviderSessionConfig {endpoint, authorizationHeader, capabilities, browserToolsAvailable, agentDeviceEnvironment}` that adapters read when building the provider launch config |
| `S/mcp/McpInvocationContext.ts` | The `McpInvocationScope` service and `requireMcpCapability(cap)` |
| `S/mcp/threadAccess.ts` | `readCaller`, `readMutationCaller` (adds "live caller" check), `readThread` (project-scoped lookup), `readWritableThread` (adds mode-escalation checks) |
| `S/mcp/OrchestratorMcpService.ts` (1952 lines) | delegate_task, task_status/cancel, create_threads, thread list/read/send/wait/interrupt, scheduled tasks, capabilities |
| `S/mcp/ThreadMetadataMcpService.ts` | `t3_thread_update` (rename, regenerate title, PR link/unlink) |
| `S/mcp/WorktreeMcpService.ts` | `t3_worktree_handoff`, `t3_worktree_status` |
| `S/mcp/PreviewAutomationBroker.ts` | Request router between MCP calls and UI hosts (leases, tab assignment, timeouts) |
| `S/mcp/AcpMcpStdioBridge.ts`, `AcpMcpOverAcpBridge.ts`, `S/cli/acpMcpBridge.ts` | Transport adapters for ACP agents |
| `S/mcp/toolkits/{preview,previewControls,device,orchestrator,thread,worktree,pullRequests,project,environment,attachment}/{tools,handlers}.ts` | Tool declarations (schema + description + annotations) and handlers |
| `C/orchestratorMcp.ts`, `C/threadMetadataMcp.ts`, `C/worktreeMcp.ts`, `C/previewAutomation.ts`, `C/device.ts` | Shared schemas (inputs/results/errors) |
| `S/preview/Manager.ts`, `S/preview/PortScanner.ts` | Server-side preview tab registry (per thread) and local dev-server discovery |
| `S/device/*` | Device hosts (local, SSH), toolchain installer, hub proxy, `agent-device` shim |
| `S/provider/T3OrchestrationInstructions.ts`, `RuntimeInstructions.ts`, `CodexDeveloperInstructions.ts` | Prompt injection |
| `S/orchestration-v2/ProviderSessionManager.ts` | Mints/rotates/revokes credentials around session lifecycle |
| `S/orchestration-v2/Adapters/{Claude,Codex,Cursor,OpenCode,OpenCode2,Acp,Pi}AdapterV2.ts` | Per-provider attach code |
| `apps/web/src/components/preview/PreviewAutomationHosts.tsx` (+ siblings) | UI host that executes preview automation requests |
| `apps/desktop/src/preview/Manager.ts` | Electron main-process browser control (CDP debugger, Playwright injected runtime, recording) |

Notes on tech: the TypeScript code uses the `effect` library (v4 "unstable" modules:
`effect/unstable/ai` `Tool`/`Toolkit`/`McpServer`, `effect/unstable/http`). Tools are
declared with `Tool.make(name, {description, parameters, success, failure, failureMode:
"return", dependencies})` plus MCP annotations (`Title`, `Readonly`, `Destructive`,
`Idempotent`, `OpenWorld`). `failureMode: "return"` means typed domain failures are
returned as tool results (`isError`) rather than as JSON-RPC errors.

---------------------------------------------------------------------------------------------

## 2. Authentication, invocation context, lifecycle

### 2.1 Credential model

* `McpSessionRegistry.issue({threadId, providerInstanceId, browserToolsAvailable?, capabilities?})`:
  * `providerSessionId = randomUUIDv4`, `rawToken = 32 random bytes, base64url`.
  * Stores only `sha256(token)` (hex) -> `{scope, lastAliveAt}` in an in-memory map
    (`SynchronizedRef`). Nothing is persisted; a server restart invalidates all tokens,
    and sessions get re-issued on next prepare.
  * Base capabilities are always `orchestration`, `worktree`, `pull-requests`; `preview`
    and `device` are added based on settings (`enableAgentBrowserAccess` default true,
    `enableAgentDeviceAccess` default false, both overridable per project).
  * Returns `McpProviderSessionConfig`: `{endpoint, authorizationHeader: "Bearer <tok>",
    environmentId, threadId, providerSessionId, providerInstanceId, capabilities,
    browserToolsAvailable}`.
* `endpoint` is computed from the HTTP server's bound address:
  `http://127.0.0.1:<port>/mcp` (wildcard binds are announced as loopback because the
  provider subprocesses run locally).
* Resolution: `Authorization: Bearer <token>` header -> `registry.resolve(token)` hashes,
  prunes dead records, refreshes `lastAliveAt`, returns scope or `undefined`.
  On failure the server answers `401` `{error: "invalid_mcp_credential"}` with
  `www-authenticate: Bearer` and logs a warning (comment: otherwise the only symptom is
  the agent silently losing the entire toolkit for the rest of its session).
* The `/mcp` route is mounted OUTSIDE the normal environment auth stack and is reachable
  on whatever interface the server binds, so the token is the only guard. Hence the
  liveness window.

### 2.2 Invocation context

`McpInvocationScope` is provided per HTTP request through the effect `Context`
(`HttpRouter.middleware<{provides: McpInvocationContext}>`) and is read by every handler:

```ts
interface McpInvocationScope {
  environmentId; threadId; providerSessionId; providerInstanceId;
  capabilities: ReadonlySet<"preview"|"orchestration"|"worktree"|"device"|"pull-requests">;
  issuedAt: number;
}
```

* `requireMcpCapability(cap)` fails with `PreviewAutomationUnavailableError` (for
  `preview`) or `McpCapabilityUnavailableError`. The preview error message is written for
  the agent: "browser preview tools are off for this thread. Do not retry them. To check
  a page, use a headless browser from the shell, such as Playwright ...". Error text
  is part of the agent UX.
* Thread-scoped authorization (`threadAccess.ts`):
  * `readCaller`: needs the `orchestration` capability, loads the caller thread shell,
    404-like `thread_not_found` if deleted.
  * `readMutationCaller`: additionally asserts the caller is *live*: not archived, has an
    `activeRunId`, and `caller.providerInstanceId === scope.providerInstanceId`
    (`parent_not_active` otherwise). This prevents a stale/detached provider session from
    mutating state after its run ended.
  * `readThread(threadId?)`: looks up the target thread **within the caller's project**
    only (`getProjectThreadRecords({projectId: caller.projectId, threadId})`). Cross-project
    access is impossible by construction ("Threads from other projects are never exposed").
  * `readWritableThread`: plus escalation checks (below).
* Privilege escalation guard: `resolveRuntimeMode(parent, requested)` ranks
  `approval-required(0) < auto-accept-edits(1) < auto(2) < full-access(3)`; a child
  (delegated task, sent message target) may never exceed the parent
  (`runtime_mode_escalation_denied`). Interaction mode `plan(0) < default(1)` likewise
  (`interaction_mode_escalation_denied`). Some tools additionally require a
  "full-access/default caller" (t3_thread_launch, run_scheduled_task_now,
  t3_environment_preferences_update).

### 2.3 Lifecycle

Managed in `ProviderSessionManager.prepareMcpSession(threadId, providerInstanceId)`:

1. Serialized per thread (`makeKeyedSerialExecutor`) so concurrent prepares cannot revoke
   each other's fresh credential.
2. Resolve effective agent-access settings -> capability set.
3. If a stored `McpProviderSession` for the thread exists AND its token still resolves AND
   thread/provider-instance match AND `preview`/`device` capability flags still equal the
   current settings, **reuse** it (no rotation). Reason (source comment): long-lived
   provider processes such as `codex app-server` build their MCP client once per
   conversation and keep the token; a thread that detaches/re-attaches across a worktree
   handoff must come back to the same token.
4. Else `revokeThread`, `issue`, `setMcpProviderSession`.
5. A *reservation* count per `(thread, credentialId)` protects the credential between
   `prepare` and the adapter actually using it; released exactly once.
6. `clearMcpSession(threadId, credentialId?)`: with an id, `revokeProviderSession(id)` and
   clear the process-global slot only if it still holds that id (a replacement session's
   newer credential survives); without an id, revoke thread-wide.
7. `RunExecutionService` calls `touchActiveMcpThread(threadId)` on every provider turn:
   liveness is refreshed by MCP traffic AND by turns, so a long-running session that does
   not use MCP tools never expires. Default window 24h; normal paths (`stopSession`,
   `stopAll`) revoke eagerly.
8. `configureMcp === false` option: sessions (e.g. internal text-generation helpers) can
   opt out; the slot is cleared and no credential minted.

The registry itself is a process-global (`activeMcpSessionRegistry`) so adapters not
inside the effect layer graph can call `issueActiveMcpCredential` / `touchActiveMcpThread`.
(A design smell: ambient global state; a Rust port should pass an `Arc<Registry>` instead.)

### 2.4 HTTP transport details

* `McpServer.layerHttp({name:"T3 Code", version, path:"/mcp", protocols:[v2025_06_18]})`.
  * Stateful sessions with `mcp-session-id` header; `DELETE /mcp` terminates
    (400 missing id, 404 unknown id, 204 on success; test `terminates HTTP MCP sessions
    with DELETE`). The bearer token and the MCP session are separate: the token identifies
    the agent thread, the session id is protocol-level state.
  * Responses can be JSON or SSE.
  * `normalizeMcpHttpResponse`: a `200` with empty body is rewritten to `202` (spec:
    notifications must be acknowledged with 202; some clients choke otherwise).
* Claude-specific: Claude Code aborts HTTP MCP calls after 60 s by default, but
  `t3_thread_wait`/`delegate_task mode=wait` can legitimately block up to 60 min
  (`MAX_WAIT_TIMEOUT_MS`); the Claude adapter sets the MCP server `timeout` to 65 min
  (`CLAUDE_T3_MCP_TOOL_TIMEOUT_MS`).

---------------------------------------------------------------------------------------------

## 3. Attaching the server to each provider

All adapters read `McpProviderSession.readMcpProviderSession(threadId)` and, if present,
attach. Absent session => no MCP and no instructions.

| Provider | Mechanism | Detail |
|---|---|---|
| **Claude Code** (SDK) | `mcpServers: {"t3-code": {type:"http", url, headers:{Authorization}, timeout: 65min}}` in query options | Also appends `allowedTools`. Non-read-only: `mcp__t3-code__*` pre-approved (headless modes like `dontAsk` deny anything not pre-approved). Read-only sandbox: only an explicit allow-list of read-only tools (`orchestrator_capabilities`, `list_scheduled_tasks`, `t3_thread_list`, `t3_thread_wait`, `t3_pending_request_list/read`, `t3_thread_configuration`, `t3_thread_transfers`, `t3_worktree_status/list`, `t3_project_list/read`, `t3_thread_search`, `t3_preview_list`, `t3_environment_read`, `t3_queue_list/read`) so a read-only session cannot spawn threads or schedule tasks |
| **Codex** (app-server) | per-thread config `mcp_servers: {"t3-code": {url, http_headers:{Authorization}}}` | Codex raises `mcpServer/elicitation/request` for tool approvals; the adapter maps these into T3 approval requests (`requestKind: "mcp-elicitation"`) and shows them in the UI, declining unsupported shapes or when no active turn context |
| **Cursor** | `mcpServers: {"t3-code": {type:"http", url, headers}}` in agent options | same shape as Claude |
| **OpenCode (1.x SDK)** | `client.mcp.add({name:"t3-code", config:{type:"remote", url, headers}})` at session | skipped for `connection.external` servers (may not reach T3 endpoint) |
| **OpenCode2** | `client.mcp.add` per *thread*-scoped server name `t3McpServerName(threadId)` (OpenCode registers MCP per directory, not session) with `oauth:false`; permission rules `<server>_*` allow-listed per thread; registration removed on session end and re-added if credential/directory changes or the server restarts | MCP is "an addition": failure to add logs a warning and the turn still runs |
| **ACP agents** (generic, Grok, Antigravity, ...) | **stdio bridge**: ACP `session/new.mcpServers` gets `{name:"t3-code", command:<t3 self>, args:["acp-mcp-bridge"], env:[ELECTRON_RUN_AS_NODE, T3_ACP_MCP_ENDPOINT, T3_ACP_MCP_AUTHORIZATION]}` | Rationale (source comment): stdio is the only mandatory ACP MCP transport; agents that advertise optional http support routinely drop injected http servers (codex-acp 1.2.0, pi-acp). Credential travels via env vars, never argv |
| ACP unstable transport | **MCP-over-ACP**: also announces `{type:"acp", name:"t3-code", serverId:"t3-code"}`; `AcpMcpOverAcpBridge` implements `connect/message/notification/disconnect` ACP requests by forwarding JSON-RPC to the HTTP endpoint (max 16 connections, 8 MiB messages, per-connection mutex, replays `mcp-session-id` and `mcp-protocol-version`) | Only serverId `t3-code` accepted |
| ACP terminal fallback | `t3 acp-mcp-call <tool> '<json>'` CLI does `initialize` + `notifications/initialized` + `tools/call` on a fresh HTTP session; env `T3_ACP_MCP_NODE`/`T3_ACP_MCP_ENTRYPOINT` exposed to the shell; the orchestration instructions teach the agent this fallback if the tools do not show up | "terminal fallback for ACP agents that accept `mcpServers` but fail to expose those tools to their model" |
| **Pi** | Pi has no MCP client. T3 materializes a TS extension (`pi-t3-mcp-extension.ts`, written under the provider status cache dir, loaded with `--extension`) that speaks MCP over HTTP itself (env `T3_MCP_URL`, `T3_MCP_BEARER_TOKEN`, `T3_PI_RUNTIME_MODE`), registers the tools into Pi, also implements Pi's permission hook | The extension is materialized even without an MCP credential, because it also enforces runtime-mode permissions (Supervised must never degrade to unrestricted) |

`agentDeviceEnvironment` (device capability): adapters call
`withAgentDeviceEnvironment(base, config)` to prepend a shim dir to the provider
subprocess `PATH` and set daemon env so `agent-device` just works.

---------------------------------------------------------------------------------------------

## 4. Tool catalogue

Tool names are exact. "RO/D/I/OW" = MCP annotations readOnly / destructive / idempotent /
openWorld. Unless noted, tools use `failureMode: "return"` (typed failure in the result).
Registration order in `McpHttpServer.layer`: Preview (standard + snapshot), Orchestrator,
Thread, Attachment, Project, Environment, PreviewControls, Worktree, PullRequests, Device.

> Note: the MCP tools visible in the *current* Claude Code session of the researcher
> (`mcp__t3-code__*`) are a subset: `device_*`, `link_pull_request`,
> `list_thread_pull_requests`, `unlink_pull_request`, `preview_*`. The checked-out source
> additionally exposes orchestrator/thread/project/etc. toolkits and `watch_pull_request`;
> the installed app may be an older build or filter by capability.

### 4.1 Browser / preview automation (capability `preview`)

All operate on a "collaborative browser tab": a real browser tab (Electron webview) that
the user can see in the UI as an inline panel / floating mini-player. Every tool takes an
optional `tabId` (a `PreviewTabId`); omitted = "this agent session's current tab" (tracked
server-side per provider session, see 5.3). Most results also carry an optional `toolIcon`
(website favicon hint) for UI presentation. Common input fields: `timeoutMs` (int, default
15000, max 60000).

| Tool | Input (key fields) | Result | Annotations |
|---|---|---|---|
| `preview_status` | `tabId?` | `PreviewAutomationStatus {available, visible, tabId\|null, url\|null, title\|null, loading, viewportSetting?, viewport? {width,height}}` | RO, !D, I |
| `preview_open` | `tabId?`, `url?` (http(s) or schemeless host, e.g. `localhost:5173`; public schemeless -> https, loopback -> http), `open?` (default true: show inline preview to user; false = background-only), `show?` (deprecated alias), `reuseExistingTab?` (default true; cannot combine `tabId` with `false`) | Status | OW, !D |
| `preview_navigate` | `tabId?`, exactly one of `url` or `target: {kind:"url",url} \| {kind:"environment-port", port, protocol?: http\|https, path?}`; `readiness?: load(default)\|domContentLoaded\|none`; `timeoutMs?` | Status | OW, !D |
| `preview_resize` | `tabId?`, `mode: fill\|freeform\|preset`; freeform needs `width`,`height` (max area cap `PREVIEW_VIEWPORT_MAX_AREA`); preset needs `preset` (Chrome DevTools device catalog id) + optional `orientation` | `{tabId, setting, viewport}` | OW, !D, I |
| `preview_set_appearance` | `tabId?`, `colorScheme: system\|light\|dark` (emulated `prefers-color-scheme` via CDP `Emulation.setEmulatedMedia`) | `{tabId, colorScheme}` | OW, !D, I |
| `preview_snapshot` | `tabId?`, `includeImage?` (default true), `save?` (write PNG to disk, return `screenshotPath`) | see below | RO, !D, I, OW |
| `preview_click` | `tabId?`, exactly one of `locator` (Playwright selector, e.g. `role=button[name='Send']`) / `selector` (legacy CSS) / `x`+`y` (viewport CSS px); `timeoutMs` | `{toolIcon?}` | OW, D |
| `preview_type` | `tabId?`, `text`, `locator?`/`selector?` (at most one; neither = focused element), `clear?` | `{toolIcon?}` | OW, D |
| `preview_press` | `tabId?`, `key` (`Enter`, `Escape`, `a`...), `modifiers?: [Alt, Control, Meta, Shift]` | `{toolIcon?}` | OW, D |
| `preview_scroll` | `tabId?`, `deltaX?`/`deltaY?` (at least one), `locator?`/`selector?` container | `{toolIcon?}` | OW, !D |
| `preview_evaluate` | `tabId?`, `expression` (<=64000 chars, main frame), `awaitPromise?` (true), `returnByValue?` (true) | `{value}` (wrapped in an object because MCP `structuredContent` MUST be an object and Claude Code rejects the entire result otherwise; value capped 64 KB) | OW, D |
| `preview_wait_for` | `tabId?`, any of `locator`/`selector`/`text` (case-sensitive visible text substring)/`urlIncludes`; at least one; `timeoutMs` | `{toolIcon?}` | RO, I, OW |
| `preview_recording_start` | `tabId?` | `{tabId, recording, startedAt}` | OW, !D |
| `preview_recording_stop` | `tabId?` | `{id, tabId, path, mimeType, sizeBytes, createdAt}`; recording (<=50 MiB, stop timeout 120 s) is uploaded by the UI to the server attachments dir (`uploadedAttachmentId`), then the server claims it (`claimPreviewRecording`: validates size/type, renames from pending to thread-scoped id) and returns an env-local path | OW, !D |

`preview_snapshot` result design (important for token economy and client quirks):

* Underlying `PreviewAutomationSnapshot`: `url,title,loading,visibleText,
  interactiveElements[{tag,role,name,selector,x,y,width,height}], accessibilityTree,
  consoleEntries[{level,text,timestamp,source?}], networkEntries[{url,method,status,failed,
  errorText?,timestamp}], actionTimeline[{id,action,status,startedAt,completedAt?,error?}],
  screenshot{mimeType:"image/png",data(base64),width,height}`.
* Registered by hand (not via the generic toolkit adapter) so that the PNG is returned as
  an MCP `image` content block and everything else as JSON metadata.
* Bounded to ~20 KB (`MAX_SNAPSHOT_TEXT_BYTES`): drops `accessibilityTree`, caps
  visibleText at 8000 chars, element names 200, url/title 2048, keeps newest 40 log entries
  of 500 chars each, then halves lists iteratively until the JSON fits (logs first, then
  visible text, locators last). An `omitted` array (also a trailing text block) tells the
  agent what was cut and to use `preview_evaluate` for more. Rationale: Claude Code moves
  oversized MCP results into a file and hands the model a notice, losing the locators.
* Claude Code shows the model `structuredContent` instead of `content` when both exist,
  so both carry the same bounded snapshot; other clients show only text content. Always
  duplicate important info across both.
* `save=true` writes `browser-screenshot-<site-slug>-<ms36>-<uuid8>.png` to
  `config.browserArtifactsDir`; the description tells the agent to embed it as
  `![alt](screenshotPath)` "This is the only way to show the user a screenshot".
  With `includeImage=false, save=true` only `{url, screenshotPath}` returns.
* Errors: a custom failure shaper returns `isError:true` with `structuredContent.error
  {_tag, operation, failureCount, message?}` and a text block, and logs. Preview errors
  build their message server-side (never from page output), and the message tells the
  agent what to do next (e.g. fall back to a shell browser).
* Generic image tools (`device_screenshot`) use `registerImageTool`: error text is only
  the tag (remote messages "may carry renderer or device output the agent should not see").

Additional post-processing in `handlers.ts`: after every action other than
`status|open|navigate|snapshot` and (for evaluate) the server fires a best-effort
`status` request (500 ms timeout, `updateCurrentTab:false`) purely to compute a
`toolIcon {_tag:"website", pageUrl}` so the UI timeline row shows the site favicon.

`preview_open` deliberately leaves `open` unstated if the agent did not state it: whether
an unrequested preview surfaces is the user's desktop-local `browserAutoShowFloatingPreview`
preference, unreadable from the server.

### 4.2 Preview controls (capability: none beyond `orchestration` check at the handler)

| Tool | Input | Result |
|---|---|---|
| `t3_preview_list` | `cursor?`, `limit?` (1..50) | `PreviewListResult` + `nextCursor`: this thread's preview tabs (RO) |
| `t3_preview_close` | `tabId` | `{}`; closes a tab owned by this thread via the normal server/host tab lifecycle (D) |

### 4.3 Device tools (capability `device`; default OFF)

Deliberately tiny surface (source comment: driving via the external `agent-device` CLI
"has the semantic snapshot model agents need and stays current with its own releases.
Wrapping its commands here would only lag behind it").

| Tool | Input | Result |
|---|---|---|
| `device_list` | `hostId?` | `{hostStatuses, hosts[], devices[], open[{hostId,deviceId}]}`: iOS Simulators and Android Emulators across device hosts, platform availability, devices already open in this thread's Device panel. Fails with a helpful reason when device support is disabled |
| `device_open` | `deviceId?` (udid/serial), `platform?` (required if no deviceId and both platforms exist), `hostId?` (default `local`) | `{device: DeviceSummary, agentDevice: {command, targetArgs[]}, quickStart: string}`: boots if needed, starts live stream, shows it in the user's Device panel. `command` is the absolute path of the shim launcher; `targetArgs` are `--platform ios --udid X` (or `--serial`) plus `--config`/`--session` flags. `quickStart` is just-in-time prose telling the agent how to drive it (open/snapshot -i/click @eN/fill/screenshot/install). Not in the always-on prompt so threads that never open a device pay nothing |
| `device_screenshot` | `deviceId?`, `hostId?` (default: most recently opened in this thread) | image content block + `{device, screenshot{mimeType,width,height}}` |
| `device_close` | `deviceId?` (omit = all in thread), `hostId?`, `shutdown?` (power off) | `{}` |

Supporting machinery (`S/device/*`): `DeviceService` (sessions per thread, consent, state),
`DeviceHost` interface (local and SSH hosts; "a future cloud host slots in beside"),
`DeviceToolchain` (pinned npm installs of `expo-device-hub@0.12.0` for streaming and
`agent-device@0.21.12` for driving, installed lazily and only after user consent, staged
temp dir + sentinel + rename), `DeviceHubProxy` (same-origin allow-listed proxy to the
loopback-only hub; requires environment-session read/operate scope, never exposes hub
exec routes), `AgentDeviceShim` (generated `agent-device` launcher that refuses to run
without `--config` and `--session`, i.e. "call device_open first", and strips daemon
env so the pinned version and the daemon assigned to the thread are used).

### 4.4 Orchestrator toolkit (capability `orchestration`)

#### Delegation (sub-agents owned by the parent thread)

* `orchestrator_capabilities` (RO, I; no input). Result: `{parentThreadId,
  inheritedProviderInstanceId, inheritedModel, runtimeMode, interactionMode,
  providers[{providerInstanceId, driverKind, displayName, models[{id,label,options?
  [ProviderOptionDescriptor]}], canRunChildTask, canRunCrossProviderChildTask,
  constraints[]}], features{appOwnedSubagents, asyncPolling, cancellation,
  batchThreadCreation, threadManagement, incrementalThreadRead, scheduledTasks,
  maxBatchThreads}}`. Same live catalog as the UI composer, including custom models.
* `delegate_task` (D, OW). Input `OrchestratorMcpDelegateTaskInput`:
  * `task` (1..120000 chars; "self-contained task for one delegated child agent")
  * `target?: {providerInstanceId?, driverKind?, model?, options?}`; `options` is either
    `[{id,value}]` or the shorthand record `{id: value}` (strictly decoded; non-string/bool
    values reject). If omitted, options inherit from parent only when child has the
    parent's provider+model.
  * `title?` (<=512), `role?: implementation|research|review|design|test|general`,
    `mode?: async(default)|wait`, `timeoutMs?` (wait budget; default 10 min; clamped to
    max 60 min), `clientRequestId?` (<=256, idempotency key), `runtimeMode?`,
    `interactionMode?` (`inherit` or a concrete mode; escalation denied).
  * Result `OrchestratorMcpDelegateTaskResult`: `{taskId (NodeId), childThreadId,
    childRunId|null, childNodeId, status: queued|running|waiting|completed|failed|
    cancelled|interrupted, workState: working|waiting_for_children|result_available,
    hasPendingChildRuns, latestTerminalRunId|null, latestTerminalStatus|null,
    latestTerminalSummary|null, latestTerminalResultContextTransferId|null,
    providerInstanceId, model|null, summary|null, resultContextTransferId|null,
    waitTimedOut}`.
  * Semantics: the child runs with ONLY the supplied prompt, no parent history
    (`"without copying parent conversation history"`). The child thread
    (`childThreadId`) is "backing storage"; further rounds must be new `delegate_task`
    calls (distinct `clientRequestId` per round; the prompt must carry prior findings),
    NOT `t3_thread_send` on the child. Requires an active parent run owned by this
    credential's provider instance (`parent_not_active`).
  * Implementation: dispatches an orchestrator command `delegated_task.request`
    (`createdBy:"agent"`, `creationSource:"mcp"`, deterministic `commandId` derived from
    `(scope.providerSessionId, requestKey, "delegate-task")` so retries are idempotent)
    carrying `parentThreadId/RunId/NodeId`, resolved model selection, modes, and
    `completionWake: "always"` (async) or `"settled_only"` (wait). Then reads the task
    projection back (`readTask`).
  * `mode="wait"` polls the projection (50 ms) until terminal or timeout. On timeout the
    wait no longer owns delivery, so it dispatches `delegated_task.wake-policy` upgrading
    `completionWake` to `always`, so a later terminal still wakes the parent.
* `task_status` (not RO because it acknowledges; I). Input `{taskId}`. Same shape as the
  delegate result. Reading a terminal result *acknowledges the automatic parent
  delivery* (dispatches `delegated_task.completion-delivery.acknowledge`) so the parent
  is not woken redundantly.
* `task_cancel` (D). Input `{taskId, reason?(<=2000), clientRequestId?}`. Result
  `{taskId, status: cancel_requested|completed|failed|cancelled|interrupted}`. Also
  disposes the automatic parent delivery (`...completion-delivery.dispose`).

#### Thread control (any thread in the caller's PROJECT)

* `create_threads` (D, OW): batch (1..20) of ordinary top-level threads sharing the
  caller's checkout; each `{prompt?, title?, target?, runtimeMode?, interactionMode?}`;
  project/branch/worktree always inherited. Result `threads[{threadId, runId|null, status,
  title, createdBy, creationSource, providerInstanceId, model}]`. Description insists on
  "Both require the user to request separate/new/top-level threads".
* `t3_thread_launch` (project toolkit; D, OW): single top-level thread with explicit
  workspace: `{projectId?, scratch?, title, modelSelection?, runtimeMode?,
  interactionMode?, workspaceStrategy?: {type:"worktree",baseRef,branch,startFromOrigin?}
  | {type:"existing_worktree",worktreePath,branch} | {type:"root"}, message?,
  attachments?[<=8]}` -> `{threadId, projectId, modelSelection, runId|null, status|null}`.
  No idempotency key (description tells agent to inspect `t3_thread_list` after errors).
  "Omitted workspaceStrategy means the project root, NOT the caller's worktree."
* `t3_thread_list`: filter `statuses[]` (idle + run statuses), `titleContains`, `settled`,
  `includeSubagents`, `cursor`, `limit<=100`. Item: `{threadId,title,createdBy,
  creationSource,status,latestRunId,providerInstanceId,model,runtimeMode,interactionMode,
  linkedPullRequest,settled,settledAt,parentThreadId,relationshipToParent: fork|subagent|
  null,itemCount,createdAt,updatedAt}`; page: `{projectId,currentThreadId,threads,
  nextCursor,total}`.
* `t3_thread_read` (not RO: may acknowledge delegated delivery): `{threadId, itemId?,
  textOffset?, view?: messages|activity, afterPosition?, limit<=100, runLimit<=50,
  maxCharsPerItem<=50000}` -> `{thread: detail, recentRuns, items[{position, visibility:
  local|inherited|synthetic, sourceThreadId, itemId, runId, messageId, type, status,
  title, text, textTruncated, nextTextOffset?}], nextPosition, hasMore}`. Pagination is by
  `afterPosition`; long items recoverable with `itemId`+`textOffset` (UTF-16 code units).
  Can read "a thread the user attached to this conversation as context".
* `t3_thread_send` (D, OW): `{threadId, message(<=120000), mode?: auto|queue|steer|
  restart, clientRequestId?}` -> `{threadId, messageId, runId, status, delivery: started|
  queued|steered|restarted}`. `auto` = start if idle, steer if fully active, queue if not
  yet steerable.
* `t3_thread_wait` (RO, I): `{threadId, runId?, timeoutMs?}` -> `{threadId, runId|null,
  status, timedOut}`; idle thread returns immediately; timeout does not interrupt work;
  reports status only (does not ack delegated result).
* `t3_thread_interrupt` (D): `{threadId, runId?, reason?, clientRequestId?}` ->
  status `interrupt_requested|no_active_run|<terminal>`.
* `t3_thread_update` (metadata; see 4.5).
* Thread toolkit (all share the project-scoped access rules):
  `t3_thread_organize` (`pin|unpin|snooze|unsnooze|settle|unsettle|archive|unarchive|
  mark_unread`, `snoozedUntil`), `t3_queue_list|read|edit|cancel|reorder|
  promote_to_steer` (manage queued messages by `queuedRunId`), `t3_pending_request_list|
  read|respond` (answer pending *user-input questions* of another thread; explicitly
  cannot approve permission requests), `t3_thread_configuration` (read provider/model/
  modes), `t3_thread_configure` (set own model selection), `t3_thread_fork` (fork from run/
  checkpoint `sourcePoint`), `t3_thread_merge_back`, `t3_thread_transfers` (context
  transfers), `t3_thread_search` (bounded global search filtered to the project),
  `run_scheduled_task_now`.

#### Scheduling (persistent app scheduler; runs without an active turn)

* `schedule_task` (D, OW): `{prompt, schedule, title?, enabled?(true),
  bindToCurrentThread?(true), clientRequestId?}`. `schedule` =
  `{type:"interval", everyMs}` or `{type:"fixed_time", timeOfDay:"09:00", weekdays?:
  [1..7]}`; the schema ALSO accepts a JSON string of that object ("OpenCode 1.15 has been
  observed serializing nested MCP union objects as JSON strings") — decode tolerance for
  client bugs, documented as compatibility-only. Result `{scheduledTaskId,title,prompt,
  enabled,projectId,boundThreadId|null,schedule,nextRunAt|null,lastRunStatus}`.
  `bindToCurrentThread=true` wakes THIS thread on each run; false creates a fresh
  top-level thread per run.
* `list_scheduled_tasks`, `update_scheduled_task`, `delete_scheduled_task`.

### 4.5 Thread metadata (capability `orchestration`)

`t3_thread_update` (D, not idempotent): `{threadId? (default caller), action: rename |
regenerate_title | link_pull_request | unlink_pull_request, title? (rename only, <=512),
pullRequest? {repository "owner/name", number, url (http/https)}, clientRequestId?}` with
cross-field validation (rename requires title and forbids pullRequest; regenerate/unlink
forbid both). Result `{threadId, action, commandId, sequence, title, titleRegeneration,
linkedPullRequest, updatedAt}`. Implemented by dispatching `thread.metadata.update`
commands whose `commandId` embeds `(providerSessionId, "thread-update", threadId, action,
requestKey)` for idempotency; `sequence` is the event-store sequence number of the write.

### 4.6 Worktree (capability `worktree`)

* `t3_worktree_status` (RO, I; no params): `{attached, worktreePath|null, branch|null,
  projectWorkspaceRoot, defaultStartFromOrigin}`.
* `t3_worktree_list` (RO): refs + checkout paths (`query,cursor,limit,refKind,
  includeMatchingRemoteRefs`), reuses the git ref inventory (`VcsListRefs`).
* `t3_worktree_handoff` (D, !I, OW): `{branch, baseRef?, startFromOrigin?, path? (absolute
  only, regex-checked), runSetupScript? (default true), continuationPrompt?
  (<=120000)}` -> `{worktreePath, branch, baseRef, startedFromOrigin, setupScript:
  started{scriptName,terminalId}|no-script|skipped|failed{detail}, continuation:
  scheduled{delivery}|skipped|failed{detail}, note}`. Semantics worth copying:
  * Moves the *calling* thread into a freshly created git worktree. Changing the
    workspace *detaches the live provider session*, so the current turn ends shortly after
    the call: "call this as the last action of the turn". `continuationPrompt` is queued as
    the thread's next message so the conversation resumes inside the worktree with
    history preserved.
  * Per-thread in-flight guard (`handoffThreadsInFlight` set -> `handoff_in_progress`) so
    two concurrent calls cannot create two worktrees; fails `already_in_worktree`;
    recheck-then-bind; failure after worktree creation removes it again; setup script and
    continuation are best-effort and reported in the result instead of failing the call.
  * Failure type `WorktreeMcpFailure.code`: `capability_denied | thread_not_found |
    project_not_found | already_in_worktree | handoff_in_progress | invalid_request |
    operation_failed`.

### 4.7 Pull requests (capability `pull-requests`)

All take `PullRequestTargetInput {url? | repository+number (+host?)}`. The URL is parsed
(`parseChangeRequestUrl`); repo+number are completed with the project's remote host.
Normalized to a host-level identity key (lowercased).

* `link_pull_request` (idempotent): -> `{host,repository,number,url,alreadyLinked}`.
  Links the PR to the thread so T3 tracks it, shows status beside the thread, and
  *settles the thread when it merges*. Description: "Register every pull request you
  open for this thread, including each layer of a stack, right after creating it."
* `unlink_pull_request` (D, idempotent) -> `{host,repository,number,wasLinked}`.
* `list_thread_pull_requests` (RO) -> `{pullRequests[{host,repository,number,url,source,
  watching,state|null,title|null,headBranch,baseBranch,isDraft,stack: {kind:
  native|derived, position(1-based, bottom first), size}|null}], chains[{kind,numbers
  (bottom->top)}]}`.
* `watch_pull_request` / `unwatch_pull_request` -> `{...identity, watching, wasWatching}`.
  Server-side `PullRequestWatchReactor` sweeps once per minute; `evaluatePullRequestWatch`
  diffs each watched PR against what the agent was last told (new failing checks;
  required-checks-passed; comments/reviews by someone other than the agent's own
  account; newly conflicting) and, when there is news, wakes the thread with a message.
  Safeguards: wake limit of 10 comment-only wakes in a row (stops bot loops), watch ends
  on merge/close, after 15 failed reads (15 min), or on `unwatch`; settled threads wait
  until active again. Tool errors are a typed union
  (`PullRequestUrlInvalidError, ...TargetIncompleteError, ...HostRequiredError, ...
  ThreadNotFoundError, ...LinkFailedError, ...NotOpenError, McpCapabilityUnavailableError`)
  whose messages are agent-facing instructions ("Pass either url, or both repository and
  number.").
* `t3_thread_update{action:link_pull_request}` is a second, older route to the same
  concept.

### 4.8 Project, environment, attachment toolkits

* Project: `t3_project_list|read|create|update|delete|clone`. `create` with no
  `workspaceRoot` makes a fresh git repo in a managed folder (README, icon, first commit;
  `commitError` reported but project exists). `delete` requires `force` for non-empty.
  `clone` only clones; register with `create`.
* Environment: `t3_environment_read` (identity, version, platform, selected preferences),
  `t3_environment_preferences_update` (patch of default thread env mode, start-from-origin,
  provider update checks, background activity profile, source-control writing style;
  requires live full-access caller).
* Attachment: `t3_attachment_prepare_upload` (returns a signed upload URL; agent POSTs raw
  bytes to the MCP server's HTTP origin), `t3_attachment_discard`,
  `t3_thread_send_attachments` (send up to 8 uploaded attachments + message to this or
  another thread; target modes may not exceed caller's). Attachment input schema
  deliberately omits the recursive `source` accessibility tree because it "breaks model
  tool schemas" (note for schema hygiene: no recursive/`anyOf[object,array]` schemas).

### 4.9 Schema hygiene lessons embedded in the code

* An empty `Schema.Struct({})` serializes as `anyOf: [object, array]`, "not a valid MCP
  tool input schema and makes clients reject the whole server" (`t3_worktree_status`
  omits `parameters`; `device_list` gets an optional `hostId` instead of an empty object).
  A Rust port must guarantee every `inputSchema` has top-level `"type":"object"`.
* Providers drop ALL tools of a server when one tool schema is invalid.
* MCP `structuredContent` must be a JSON object (wrap scalars/arrays/null).
* Tool annotations are used by clients for auto-approval (Claude read-only sandbox
  allow-list) and by UI.

---------------------------------------------------------------------------------------------

## 5. Browser automation: how MCP calls reach the UI

### 5.1 Topology

Server (`PreviewAutomationBroker`) <-> WebSocket RPC <-> web UI (`PreviewAutomationHosts`)
<-> Electron preload IPC `previewBridge.automation.*` <-> desktop main
(`apps/desktop/src/preview/Manager.ts`: webview `WebContents`, Chrome DevTools Protocol
via `wc.debugger.attach("1.3")`, an injected Playwright runtime
(`PlaywrightInjectedRuntime.ts`, `__t3PlaywrightInjected`) that resolves Playwright-style
`locator` strings, recording, annotation overlay).

RPC methods (in `C/rpc.ts`, auth scope `AuthOrchestrationOperateScope`):

* `previewAutomation.connect` : payload `PreviewAutomationHost {clientId, environmentId,
  supportedOperations?}` -> server-push **stream** of `PreviewAutomationStreamEvent`:
  `{type:"connected", connectionId}` then `{type:"request", connectionId, request:
  {requestId, threadId, tabId?, tabIdExplicit?, operation, input, timeoutMs}}`.
* `previewAutomation.respond` : `PreviewAutomationResponse {clientId, connectionId,
  requestId, ok, result?, error?{_tag,message,detail?}}`.
* `previewAutomation.focusHost` : host reports `{clientId, environmentId, connectionId,
  focused, liveTabs[{threadId, tabId, visible?}]}` so the broker can pick the right host.

Operations: `status, open, navigate, snapshot, click, type, press, scroll, evaluate,
waitFor, recordingStart, recordingStop` (V1) + `resize, setColorScheme` (V2). Hosts
advertise `supportedOperations`; absence = V1 set, enabling mixed-version rollout
(a newer server with an older desktop).

### 5.2 Broker algorithm (`PreviewAutomationBroker.invoke`)

State (one `SynchronizedRef`): `clients` (by clientId: connection, supported ops, focus
flag/order, liveTabs, outbound `Queue`), `assignments` (key `environmentId\0
providerSessionId` -> `{clientId, connectionId, queue, tabId?, tabSequence?}`), `pending`
(requestId -> `Deferred` + error context), counters.

1. Prune assignments whose connection is gone/replaced.
2. If the session has a live assignment in this environment: use that host (if it
   supports the op; else fail with a capability error rather than silently migrating).
   Rationale (comment): "Keep one provider session on one physical desktop runtime so a
   multi-step browser interaction cannot jump between independent Electron cookie/DOM
   state." The lease has no clock; it lives exactly as long as the connection.
3. Otherwise choose among hosts in the environment supporting the op, sorted by: owns the
   target tab *visibly* for this thread > owns it at all > window focused > most recent
   focus order.
4. Resolve `tabId`: explicit input, else the assignment's remembered current tab.
5. Allocate `requestId = preview-<seq>`, store pending, `Queue.offer` the request.
6. `Deferred.await` with timeout (default 15 s). On timeout: **evict the connection** (an
   unanswered request invalidates it; completes the stream so a responsive desktop can
   re-register) and fail `PreviewAutomationTimeoutError`. Never replay an action: "the
   client may have applied them before becoming unreachable".
7. After success, update the session's current `tabId` from the response (monotonic
   `tabSequence` guard against out-of-order completions).
8. Reconnect semantics: a new `connect` from the same `clientId` replaces and shuts down
   the previous queue, failing its pending requests with `ClientDisconnected`.
9. Errors are a rich tagged union (NoAvailableHost, UnsupportedClient, TabNotFound,
   Timeout, ControlInterrupted, InvalidSelector, TargetNotEditable, ResultTooLarge,
   RecordingTooLarge/DeadlineExpired/TransferError/DesktopUpdateRequired,
   MalformedResponse, ClientDisconnected, RequestQueueClosed, ExecutionError ...) with
   diagnostics (selector kind/length, never page content) in the log.

### 5.3 UI side (`apps/web/src/components/preview/*`)

* `PreviewAutomationHosts.tsx` (868 lines): mounts when the environment connects; creates
  a stable `clientId`; subscribes to `previewAutomation.connect` via an Effect Atom
  (`createPreviewAutomationRequestConsumerAtom`): each request -> `handle(request)` ->
  result or serialized error -> `previewAutomation.respond`. Handles `open` by driving the
  thread-bound preview state store (`previewStateStore`), the floating mini-player
  (`previewMiniPlayerStore`), and waiting (bounded by deadlines) for the Electron webview
  to be rendering/visible and for CDP to attach (`waitForDesktopOverlay`,
  `waitForRenderedViewport`, `waitForNavigationReadiness`), viewport mutation with
  rollback (`shouldRollbackPreviewViewport`), recording start/stop + upload
  (`uploadBrowserRecording` -> server attachments, then the MCP side claims it),
  preview "keep host focus" click handling.
* The user therefore sees the agent working in the same tab (it can be inline in the
  thread, floating, or hidden when `open=false`). The user can interact with the same
  page; `actionTimeline` in snapshots records agent actions and their status
  (`running|succeeded|failed|interrupted` — "interrupted" when the user takes control).
* `focusHost` reporting enables "agent browser goes to the window the user is looking
  at" when multiple desktop/web clients are connected.
* Non-Electron web clients do not host automation (the preview webview requires the
  Electron bridge): `PreviewAutomationNoAvailableHostError` -> tool message tells agent
  to use a headless browser from the shell.
* Tool-call rows in the timeline render MCP tool calls with presentation helpers
  (`packages/client-runtime/src/work-log/presentation.ts`, `ToolActivityIcon` with
  `toolIcon` = website favicon).
* Server `PreviewManager` (`S/preview/Manager.ts`) is the in-memory source of truth for
  tab sessions keyed `(threadId, tabId)` (multiple tabs per thread; tab lifecycle owned by
  the renderer; events via PubSub). `PortScanner` discovers local dev servers (lsof on
  Unix, curated port list on Windows; publishes only ports that answer an HTML probe;
  ref-counted polling) for the "environment-port" navigation target and UI suggestions.
  `navigate{target:{kind:"environment-port",port}}` is resolved relative to the *agent's
  execution environment* (local or remote SSH), which matters when the UI and the
  environment are on different machines.

### 5.4 Settings gating

`enableAgentBrowserAccess` (default true) and `enableAgentDeviceAccess` (default false),
with per-project overrides in `projectSettingsOverrides`. If settings cannot be resolved
the code fails closed (`browser:false, device:false`). A flipped setting forces credential
rotation (see 2.3 step 3) so the scope reflects it; the instruction text for the session
is also computed from the actual credential capabilities (`browserToolsAvailable`).

---------------------------------------------------------------------------------------------

## 6. Orchestration interplay: delegation, notifications, delivery

### 6.1 Data model (names from the code)

* Thread (project-scoped, has `activeRunId`, runs with ordinals, `runtimeMode`,
  `interactionMode`, provider instance + model). Run -> execution nodes (`root_turn`,
  `assistant_message`, `tool_call`, `subagent`, `approval_request`, `user_input_request`
  ...). `OrchestrationV2Subagent` has `origin: provider_native | app_owned`; delegated
  tasks are `app_owned`, have `childThreadId`, `completionWake: always|settled_only`,
  `completionDelivery.state` (`pending|claimed|delivered|acknowledged|disposed`...).
* Parent run holds `delegatedCompletion` cohort `{disposition: open|..., delivery:
  {messageId, taskIds[]}}`.

### 6.2 Completion delivery ("mailbox")

When a child run terminates, `Orchestrator` (around lines 8440-9140) decides:

* `completionWake: "always"` (async delegation): offer a continuation to the parent on
  EVERY child terminal. If the parent run is live, the notification is *steered* into the
  active turn when the provider supports steering, else *queued* (`queue_after_active`)
  behind it. If the parent has settled, a new run is started (a "wake").
* `settled_only` (wait mode): only wake if the parent has no live run (tool result is the
  delivery path; if the parent turn ended/was interrupted first, wake covers it).
* Delivery is a persisted user-role message with a `delegatedCompletion` marker and
  at-least-once semantics (`NotificationMailbox.isUndeliveredMailboxSteer`: "provider
  acceptance and our receipt cannot commit atomically. Reusing the message ID keeps
  recovery from duplicating timeline items").
* Acknowledgement: `task_status` and `t3_thread_read` of the child's untruncated terminal
  result acknowledge delivery so the parent is not woken redundantly; `task_cancel`
  disposes it. `t3_thread_wait` explicitly does not acknowledge.
* Agent guidance (tool descriptions + instructions): "An async child's completion wakes
  this thread through a notification ... so end the turn instead of polling or spawning
  watchers; use task_status only when the result is needed mid-turn."
* Related notification mechanisms using the same mailbox: scheduled tasks
  (`bindToCurrentThread`) and PR watch wakes (message built by
  `pullRequestWatchMessage`, kind `OrchestrationV2Notification`, also "monitor" source
  notifications that are excluded from delegated-task results).

### 6.3 Idempotency convention

Every mutating tool takes `clientRequestId` (<=256 chars; paired-surrogate validated in
thread metadata) and derives deterministic command/thread/message IDs from
`(providerSessionId, operation, requestKey)` (`stableCommandId`, `stableThreadId`,
`stableMessageId`). Retrying the same call replays the same command (the command bus
returns the stored receipt or `OrchestratorCommandPreviouslyRejectedError`). Without a key,
a random `mcp:<uuid>` command id is used. Tools lacking a key say so loudly in their
description ("Each call creates a new launch with no retry key; inspect t3_thread_list
before retrying").

### 6.4 User prompts / approvals through the MCP boundary

* The MCP server never prompts the user itself (no MCP elicitation or sampling server
  features were found). It *exposes* the pending user-input requests of other threads
  (`t3_pending_request_*`) but explicitly refuses to answer approval requests.
* Approvals for *calling* MCP tools are the provider's responsibility and are bridged by
  the adapters: Claude via `allowedTools` pre-approval (tools pre-approved because the
  user already trusts the t3-code server in a T3 thread; read-only sandboxes limited to
  read-only tools), Codex via `mcpServer/elicitation/request` -> T3 approval UI card,
  OpenCode2 via per-thread permission rules, Pi via the extension permission hook.
* Runtime-mode escalation checks prevent an agent from using the MCP server to obtain
  broader permissions than its own thread has (children can only equal or narrow).

---------------------------------------------------------------------------------------------

## 7. Instruction / system-prompt injection

Different per provider because channels differ. Content in
`S/provider/T3OrchestrationInstructions.ts`, `RuntimeInstructions.ts`,
`CodexDeveloperInstructions.ts`.

Blocks:

1. `T3_CODE_ORCHESTRATION_INSTRUCTIONS` — distinguishes delegated sub-agents (delegate_task,
   task_status, task_cancel) from ordinary top-level threads (`t3_thread_launch`,
   `create_threads`, only on explicit user request), says "prefer native subagent tools
   only when they support the chosen model", review-round protocol (new `delegate_task`
   per round, distinct `clientRequestId`), `schedule_task` usage with a literal structured
   `schedule` example, workspace selection (`t3_thread_launch` with `workspaceStrategy`,
   examples with JSON), retry guidance, and robustness notes: tool names may carry harness
   prefixes (`mcp__t3_code__delegate_task`); some harnesses attach MCP servers lazily so
   "make one bounded direct attempt using the known tool name" before concluding the
   capability is absent; the ACP `acp-mcp-call` fallback command.
2. `T3_CODE_BROWSER_TOOL_INSTRUCTIONS` — "You are running inside T3 Code. The `t3-code`
   MCP server is the product-native collaborative browser shared with the user... first
   call `preview_status`; if no preview call `preview_open` before concluding the browser
   is unavailable ... Do not switch to global browser skills, Chrome, Node REPL browser
   automation, standalone Playwright, or agent-browser merely because the preview is
   initially closed or a first call fails." Included only when the `preview` capability
   exists.
3. `T3_CODE_DEVICE_TOOL_INSTRUCTIONS` — short pointer to `device_list` -> `device_open`
   and the exact launcher path returned (details deliberately deferred to the
   `quickStart` field of the tool result). Only when `device` capability.
4. `PULL_REQUEST_LINKING_INSTRUCTIONS` (`<pull_request_linking>`) inside
   `buildRuntimeInstructions` — must call `link_pull_request` right after creating/starting
   work on a PR, for every layer of a stack, also when created through `gh` or other
   CLIs; call `list_thread_pull_requests` before finishing; do not link unrelated PRs;
   report linking failures honestly; use `watch_pull_request` and end the turn rather than
   polling.
5. `<runtime_info>` — "In case you're asked: you are running in T3 Code through the
   <harness> harness, as <model> with <effort> reasoning effort. ... You can embed images
   and videos in your response using Markdown with absolute file paths." This is what lets
   the screenshot/recording paths from tools render in the transcript.
6. ACP-only interaction mode text (default vs plan), since ACP has no developer prompt.

Delivery channel per provider:

* Claude: appended to the system prompt only when `mcpServers` set (`T3_CODE_ORCHESTRATION_INSTRUCTIONS` gate).
* Codex: `turn/start.additionalContext` entries `t3_code_orchestration`, `t3_code_runtime`,
  `t3_code_tools` (kind `application`; Codex resends only on change; kept SEPARATE from the
  collaboration-mode developer instructions because newer model catalogs ship their own mode
  text and Codex then drops the client's `developer_instructions`; separate keys keep each
  under Codex's per-entry token cap).
* OpenCode2: `session.instructions.entry.put` keyed `INSTRUCTIONS_KEY`, updated when the
  composed string changes (and removed with the MCP server).
* OpenCode 1.x: `t3OrchestrationSystemPrompt(hasT3Mcp)` as system prompt.
* Cursor/ACP/others without a system channel: wrapped in the first user prompt:
  `<t3_code_instructions>...</t3_code_instructions>\n\n<user_request>\n...\n</user_request>`
  (re-sent whenever MCP availability or interaction mode changes; skipped if the prompt
  starts with `/`, so native slash commands keep working). First-run-only variant
  `t3OrchestrationPromptForFirstRun`.
* Pi: the extension holds the instructions string and injects it (`ORCHESTRATION_INSTRUCTIONS`
  constant embedded at materialization time).

Rule baked into the code: instruction blocks are omitted when the matching tools aren't
attached ("Describing `preview_*` ... tools that aren't in the turn's tool list would be
worse than saying nothing: the instructions actively steer the model away from
Playwright..."), and instruction state is derived from the *actual session config*, not
from re-reading settings.

Also relevant: tool descriptions are long and behavioral (they carry most of the
guidance: when to use, what NOT to use, retry semantics, how to follow up). The two-layer
approach (short always-on prompt + rich descriptions + just-in-time `quickStart` in
results) keeps context cost low.

---------------------------------------------------------------------------------------------

## 8. Testing approach worth copying

* `McpHttpServer.test.ts` spins the real HTTP MCP server with a fake broker/host and
  asserts: 202 normalization, snapshot bounding invariants (several adversarial cases:
  wide chars, huge logs, huge titles), DELETE session termination (400/404/204), auth
  context preserved into handlers, annotated tool registration, tagged error shaping.
* `McpSessionRegistry.test.ts` (injectable `now`), `McpInvocationContext.test.ts`,
  `toolkits/*/tools.test.ts` (schema validity such as "no anyOf[object,array]" checks),
  `OrchestratorMcpToolkit.integration.test.ts` (3.8k lines, tool-level end-to-end against
  an in-memory orchestrator), `PreviewAutomationBroker.test.ts` (1.5k lines: leases,
  failover, timeouts, reconnect races), `AcpMcpStdioBridge.test.ts`.

---------------------------------------------------------------------------------------------

## 9. Design lessons for a Rust implementation

### 9.1 Architecture

1. **One MCP server per app process, identity from credential, not arguments.** Mint a
   random 256-bit token per (agent session) and keep `sha256(token) -> Scope` in memory
   (`DashMap`/`RwLock<HashMap>`). Never trust caller-supplied thread ids for the caller's
   own identity. For cross-thread operations, resolve the target within the caller's
   project/workspace scope server-side (t3code's `readThread`).
2. **Capabilities in the token**, enforced both by (a) not advertising the tool family
   (t3code advertises everything and rejects at call time, which needs agent-friendly
   rejection messages; a better approach in rmcp is per-connection tool lists filtered
   by scope in `list_tools`, while still enforcing in `call_tool`) and (b) the instruction
   text being generated from the same capability set.
3. **Liveness + eager revoke**: revoke on session stop; refresh on every turn and on MCP
   traffic; prune after a generous window (24h) as a backstop. Reuse the token across
   detach/re-attach of the same thread/provider-instance (long-lived provider processes
   hold on to the token they were launched with).
4. **Prefer loopback bind + token**, and make the endpoint address-aware (wildcard bind ->
   announce 127.0.0.1). Consider a unix-domain socket or random per-session path for
   extra protection, but note most CLIs only accept `http(s)` URLs; rmcp also has a
   streamable HTTP client over unix sockets (feature flag
   `transport-streamable-http-client-unix-socket`) but agents' own clients would not.
5. **Transport matrix** is the real complexity: HTTP for Claude/Codex/Cursor/OpenCode;
   stdio proxy for ACP agents (a tiny `semantic acp-mcp-bridge` subcommand which forwards
   JSON-RPC lines over HTTP with the bearer token from env vars, replays `mcp-session-id`
   and `mcp-protocol-version`); MCP-over-ACP optional; and an escape hatch CLI
   (`acp-mcp-call <tool> <json>`) for agents that accept `mcpServers` but never surface
   the tools. In Rust the stdio bridge can be implemented using rmcp's proxy-ish pieces
   or a trivial line-oriented forwarder with `reqwest`; keep its dependency graph lean
   (t3code fast-paths it before loading the CLI graph because it sits on the ACP
   first-message critical path).
6. **Broker pattern for UI-backed tools** (browser, user-visible panels): the server holds
   an outbound request queue per UI host and a pending-response map keyed by request id.
   Hosts connect over the app's existing duplex channel (WS/RPC stream) and answer
   asynchronously. Copy: host capability negotiation (`supportedOperations`), session ->
   host lease with no independent clock, timeout => evict connection and never replay,
   generation counters/queue identity to make reconnect races safe, focus-based host
   selection, never log page content.
7. **Split "tool surface" from "service layer"**: tools are thin handlers calling
   services with `(scope, input)`; services dispatch *commands* on the app's command bus
   with deterministic IDs derived from `(session, op, clientRequestId)` so retries are
   idempotent. For `semantic`, that maps to DB mutations/"command log" entries.
8. **Push, don't poll.** The delegated-completion mailbox (persisted message, steer-if-
   active-else-queue-else-wake, acknowledge-on-read, dispose-on-cancel, at-least-once with
   stable message id) is the highest-leverage idea for sub-agent orchestration. Combine with
   tool descriptions that say "end the turn".
9. **Escalation rules**: child runtime/interaction mode <= parent; read-only sessions
   only get read-only tools allow-listed (derived from the MCP `readOnlyHint`
   annotation rather than a hand-maintained list, if possible).
10. **Keep the "agent action" surface small where an external CLI exists** (devices: 4
    tools + CLI shim), and return just-in-time docs in a result field rather than the
    system prompt.

### 9.2 Tool design details to replicate

* Every input schema: top-level `type: object`, no `anyOf[object,array]`, no recursive
  types (`schemars` will produce `$ref`/`$defs`; test that inlining is not required by
  target clients — Claude/Codex/OpenCode vary; add a unit test that walks every tool's
  `input_schema` and asserts top-level object, no empty-struct pitfall, no unsupported
  keywords you've seen clients reject).
* Return BOTH `content` (text JSON) and `structuredContent` (object) for every tool; some
  clients show only one. Large payloads: cap (~20 KB for snapshots) and include an
  `omitted` list so the model can ask for more.
* Images: use `Content::image(base64, "image/png")` blocks, plus a text block with
  metadata; allow `includeImage=false` for text-only.
* Give agents *file paths* for artifacts (screenshots, recordings) and tell them to
  embed with Markdown; that requires the UI to render local absolute-path images.
* Annotations: set `readOnlyHint/destructiveHint/idempotentHint/openWorldHint` and `title`.
* Typed failures with `is_error: true` and agent-actionable messages ("Do not retry. To
  check a page, use ...") instead of JSON-RPC protocol errors; hide remote/renderer
  messages from the agent where they could leak page/device output (return the tag only).
* Defensive decoding: accept clients that stringify nested objects (OpenCode quirk) — a
  `#[serde(deserialize_with)]` shim for union-typed params; document as compatibility.
* Per-tool timeouts and the *client's* MCP timeout (Claude: 60 s default) — blocking
  tools (`wait`) need the client `timeout` raised in the attach config, or should return
  early with `timedOut: true` and let the agent call again.
* Pagination convention: `cursor`/`limit`, `nextCursor` and `afterPosition`/`nextPosition`
  for timelines; offset-based text recovery for long items.
* `clientRequestId` on all mutating tools; say in the description when absent.

### 9.3 Instruction injection

* Build instruction text from the session's actual capability set; omit blocks for
  absent tools.
* Support per-provider channels (system prompt append, developer/additional-context,
  first-user-message wrapper with tags, extension-held constant). Re-send on change only.
* Keep a robust fallback paragraph for lazily-attached MCP servers and prefixed tool
  names.
* Put rules about competing mechanisms (native subagents vs `delegate_task`, shell
  Playwright vs `preview_*`) in prompt, and make the preferred path the one that is
  user-visible (shared browser tab, tracked PR, schedulable thread).

### 9.4 Things to avoid / improve over t3code

* Ambient process-global registries (`activeMcpSessionRegistry`, `sessionsByThread`) —
  use an explicit `Arc<McpState>` injected where needed.
* Full tool list always advertised; consider per-scope `list_tools`.
* Very long single-file services (OrchestratorMcpService 1.9k lines): split per tool.
* Token in process env vars of the ACP bridge is visible to same-user `ps e`/`/proc`;
  acceptable on single-user machines (loopback + same-user), but note it; per-session
  tokens with revocation limit blast radius.
* `/mcp` mounted outside normal auth — in `semantic` mount it on a separate loopback
  listener (or path with its own middleware) so it never inherits remote exposure.
* Tool count (~60) is large for model context; t3code puts rare administrative tools
  (project/environment/attachment/queue) in the same server. Consider several
  toolsets selectable by capability so a default session only gets the high-value set
  (preview/pr/delegate/thread-update/worktree).

---------------------------------------------------------------------------------------------

## 10. Rust MCP ecosystem check: `rmcp`

Checked 2026-10-03 via crates.io API, docs.rs and the GitHub repo
(`modelcontextprotocol/rust-sdk`). `~/.cargo/registry` has no MCP crates locally, and the
semantic `Cargo.lock` has no `rmcp`, so it would be a new dependency.

* **Crate**: `rmcp` (official Rust SDK, Tokio-based). **Latest: 3.5.0** (2026-09-28); recent
  releases: 3.4.1 (09-23), 3.4.0 (09-15), 3.3.0 (09-10), 3.2.0 (08-31), 3.1.4 (08-20),
  3.1.3 (08-17), 3.1.2 (08-07). Created 2025-03-16. ~31.7M total downloads, ~17M recent.
  GitHub: ~4k stars, ~650 forks, ~730 commits. Frequent releases (roughly weekly) and a
  recent major bump to 3.x: **expect API churn**; pin the version and re-read docs.rs
  before coding (some examples below are from the README and I did not compile them).
* **Protocol**: README claims stable MCP spec `2026-07-28` with backward compatibility to
  `2025-11-25` and older. t3code only speaks `2025-06-18`; the agents we target (Claude
  Code, Codex, Cursor, OpenCode) all support at least `2025-06-18` streamable HTTP, so
  rmcp negotiation should be fine; verify each client in an integration test.
  Features advertised in README: stateless HTTP serving (new spec) and "legacy session
  mode" (`StreamableHttpServerConfig::default().with_legacy_session_mode(false)
  .with_json_response(true)`), server discovery, subscriptions, multi-round-trip
  requests, response caching hints, tasks extension (long-running operations with
  polling), pagination helpers (`list_all_tools()`), elicitation (`ctx.peer.elicit::<T>(..)`
  with `elicit_safe!`), progress notifications (`peer.notify_progress`), sampling
  (deprecated by SEP-2577).
* **Feature flags** (3.5.0): defaults `base64, macros, server, schemars,
  transport-async-rw, uuid`. Server HTTP: `server-side-http`, `tower`,
  `transport-streamable-http-server`, `transport-streamable-http-server-session`.
  Client: `client`, `client-side-sse`, `elicitation`, `request-state`,
  `transport-streamable-http-client`, `transport-streamable-http-client-reqwest`,
  `transport-streamable-http-client-unix-socket`. Other transports: `transport-io`
  (stdio), `transport-child-process`, `transport-worker`, `which-command`. Auth:
  `auth`, `auth-client-credentials-jwt`, `auth-enterprise-managed`. Misc: `local`.
* **Server programming model**: attribute macros `#[tool]`, `#[tool_router]`,
  `#[tool_handler]` (README shows `#[tool_router(server_handler)] impl Calculator {
  #[tool(description = "...")] fn add(&self, Parameters(AddParams{a,b}): Parameters<AddParams>)
  -> String }`). Parameter types derive `serde::Deserialize + schemars::JsonSchema` and the
  input schema is generated (JSON Schema 2020-12). Results: plain `String`, or
  `CallToolResult::success(vec![ContentBlock::text(..), ContentBlock::image(b64, "image/png")])`;
  errors follow "tool-level failures return `Ok(CallToolResult::error(..))`, protocol
  errors return `Err(McpError)`" — exactly the `failureMode:"return"` semantics t3code uses.
  Handlers receive `RequestContext<RoleServer>` with `peer` and client capabilities, and
  HTTP metadata through `http::request::Parts` in request extensions.
* **Streamable HTTP server**: `StreamableHttpService` is a Tower service, mountable in
  `axum` (`Router::nest_service("/mcp", service)`), so a custom axum middleware layer can
  validate the bearer token and inject the `Scope` into request extensions, from where
  tool handlers read it via `context.extensions.get::<http::request::Parts>()` /
  your own extension type. **Verify the exact accessor in the current docs.rs** — this is
  the crux for per-session identity. Alternative that avoids extension plumbing: create
  one `ServerHandler` instance **per session/connection** via the service factory
  closure (the factory in `StreamableHttpService::new(|| Ok(handler), ...)` runs per MCP
  session; if the factory can see the request/parts, bind the Scope there; otherwise use
  the extension approach).
* **Per-scope tool lists**: implement `ServerHandler::list_tools` yourself (or compose
  routers with `ToolRouter` and combine) to filter by scope capabilities; `#[tool_router]`
  generates a `ToolRouter<Self>` that can be `+`-combined, enabling per-toolset routers
  (`preview_router() + orchestrator_router() + ...`) gated by capability.
* **Stdio**: `transport-io`/`stdio()` for a stdio server; `transport-child-process` for
  clients spawning servers. For our `acp-mcp-bridge` we don't need rmcp's server at all —
  a raw JSON-RPC line forwarder is simpler and keeps startup fast; use rmcp client
  transport only for tests (`transport-streamable-http-client-reqwest`) to exercise the
  server end to end.
* **Images/structured content**: `ContentBlock::image/audio`, resources, plus structured
  output (check docs for `Json<T>` returns / `CallToolResult::structured` and output
  schema generation in 3.x; keep emitting duplicate text content for clients that ignore
  `structuredContent`).
* **Maturity assessment**: official SDK, large adoption (Goose, Apollo, others), very
  active, transports we need exist (streamable HTTP server over axum/tower, stdio, client).
  Risks: rapid major versions (3.x within ~18 months), spec-version churn (2026-07-28
  stateless mode vs legacy sessions — pick `legacy_session_mode`/stateful vs stateless
  explicitly and test against each agent; t3code's server is stateful with
  `mcp-session-id` and `DELETE`), and macro-generated schemas can diverge from client
  expectations (empty params, `$ref` handling, optional fields as `nullable`/`anyOf`):
  add schema-lint tests like t3code's.
* **Alternatives**: none comparable at official quality; community crates exist
  (`rust-mcp-sdk`, `mcp-server`-style crates) but rmcp is the reference. Writing a minimal
  JSON-RPC/HTTP MCP server by hand is feasible (initialize, tools/list, tools/call, ping,
  session header, SSE/JSON response) if rmcp's per-request context proves awkward, as
  the protocol surface t3code uses is small: only tools (no resources, prompts, sampling,
  or elicitation were found in the server). Recommendation: start with rmcp, keep the
  tool logic in transport-agnostic traits so swapping is cheap.

### 10.1 Suggested Rust skeleton (sketch, not compiled)

```rust
// Shared, injected state (no globals)
pub struct McpState {
    registry: Arc<SessionRegistry>,   // token_hash -> Scope, liveness, revoke
    broker: Arc<UiBroker>,            // preview/ui host requests
    services: Arc<Services>,          // orchestrator, db, worktrees, ...
}

#[derive(Clone)]
pub struct Scope {
    pub environment_id: EnvId,
    pub thread_id: ThreadId,
    pub provider_session_id: Uuid,
    pub provider_instance_id: ProviderInstanceId,
    pub capabilities: Arc<HashSet<Capability>>,
    pub issued_at: Instant,
}

// axum: bearer middleware -> insert Scope into request extensions -> rmcp StreamableHttpService
// per-tool: fn handler(&self, ctx: RequestContext<RoleServer>, Parameters(p): Parameters<P>)
//           let scope = ctx.extensions.get::<Scope>().ok_or(unauthenticated)?;
//           scope.require(Capability::Preview)?;
```

Mapping of t3code concepts to a `semantic` implementation (to be decided with the wider
design):

| t3code | Rust analogue |
|---|---|
| `McpSessionRegistry` | `SessionRegistry` with `sha2` hashed tokens, `parking_lot::RwLock`/`DashMap`, tokio interval prune, `touch(thread)` |
| `McpInvocationContext` | axum request extension `Scope` |
| `PreviewAutomationBroker` | `UiBroker` with `tokio::sync::mpsc` per host, `oneshot` per pending request, generation id per connection |
| `previewAutomation.connect` stream | whichever duplex RPC the Dioxus UI already uses (WS/SSE + POST response) |
| Effect `Deferred` + timeout + eviction | `tokio::time::timeout` on `oneshot::Receiver`; on elapse drop host connection |
| Command bus with stable ids | DB command/event log with idempotency key `(session, op, client_request_id)` |
| Mailbox delivery | persisted notification row + steer/queue/wake policy per thread |
| `acp-mcp-bridge` subcommand | tiny clap subcommand: stdin lines -> `reqwest` POST -> stdout lines |
| Instruction builders | functions taking `&Scope`/capability set and returning prompt fragments per provider channel |

---------------------------------------------------------------------------------------------

## 11. Appendix

### 11.1 Complete tool name list (source of truth in `S/mcp/toolkits/*/tools.ts`)

Preview: `preview_status, preview_open, preview_navigate, preview_resize,
preview_set_appearance, preview_snapshot, preview_click, preview_type, preview_press,
preview_scroll, preview_evaluate, preview_wait_for, preview_recording_start,
preview_recording_stop`.
Preview controls: `t3_preview_list, t3_preview_close`.
Device: `device_list, device_open, device_screenshot, device_close`.
Orchestrator: `orchestrator_capabilities, delegate_task, task_status, task_cancel,
schedule_task, list_scheduled_tasks, update_scheduled_task, delete_scheduled_task,
create_threads, t3_thread_list, t3_thread_read, t3_thread_update, t3_thread_send,
t3_thread_wait, t3_thread_interrupt`.
Thread: `run_scheduled_task_now, t3_thread_search, t3_thread_fork, t3_thread_merge_back,
t3_thread_transfers, t3_thread_configuration, t3_thread_configure,
t3_pending_request_list, t3_pending_request_read, t3_pending_request_respond,
t3_thread_organize, t3_queue_list, t3_queue_read, t3_queue_edit, t3_queue_cancel,
t3_queue_reorder, t3_queue_promote_to_steer`.
Worktree: `t3_worktree_handoff, t3_worktree_status, t3_worktree_list`.
Pull requests: `link_pull_request, unlink_pull_request, list_thread_pull_requests,
watch_pull_request, unwatch_pull_request`.
Project: `t3_thread_launch, t3_project_list, t3_project_read, t3_project_create,
t3_project_update, t3_project_delete, t3_project_clone`.
Environment: `t3_environment_read, t3_environment_preferences_update`.
Attachment: `t3_attachment_prepare_upload, t3_attachment_discard,
t3_thread_send_attachments`.

Count: 14 + 2 + 4 + 15 + 17 + 3 + 5 + 7 + 2 + 3 = 72.

### 11.2 Orchestrator failure codes (`OrchestratorMcpFailure.code`)

`capability_denied, parent_not_active, provider_unavailable, model_unavailable,
runtime_mode_escalation_denied, interaction_mode_escalation_denied, task_not_found,
task_not_cancellable, thread_not_found, run_not_found, thread_not_sendable,
thread_not_interruptible, invalid_request, orchestration_error`.
Note `unavailable()` helper returns a generic message ("The operation could not be
completed.") hiding internals for thread-lookup errors.

### 11.3 Constants

| Constant | Value |
|---|---|
| MCP protocol | `2025-06-18` |
| Endpoint path | `/mcp` |
| MCP server name (agent-visible) | `t3-code` (display name "T3 Code") |
| Credential liveness | 24 h |
| Claude MCP call timeout | 65 min |
| Default/max task & thread wait | 10 min / 60 min |
| Preview request default/max timeout | 15 s / 60 s |
| Recording stop timeout / max size | 120 s / 50 MiB |
| Snapshot text cap | ~20 KB (visible text 8000 chars; 40 log entries x 500 chars) |
| `preview_evaluate` expression / result | 64000 chars / 64 KB |
| Thread list default/max | 50 / 100 |
| Thread read default max chars per item | 20 000 (max 50 000) |
| Batch thread creation max | 20 |
| Attachments per send | 8 |
| ACP over-ACP connections / message size | 16 / 8 MiB |
| PR watch sweep / wake limit / read-failure limit | 1 min / 10 comment-only wakes / 15 passes |
| Pinned device tools | `expo-device-hub@0.12.0`, `agent-device@0.21.12` |

### 11.4 Questions / items not fully verified

* Exact semantics of `delegated_task` ack/dispose state machine beyond the paths read
  (Orchestrator.ts is ~9.4k lines; I read the offer/claim sections only).
* Device panel UI components in `apps/web` not inspected; behavior inferred from
  `DeviceService`/contracts and tool descriptions.
* `McpServer.layerHttp` internals live in the `effect` library (not in this repo); session
  and SSE behavior inferred from the tests.
* The rmcp API snippets come from the upstream README via fetch, not from compiling
  against 3.5.0; verify against docs.rs (especially request-extension access and
  stateless vs legacy-session configuration).
