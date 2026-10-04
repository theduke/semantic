# t3code UI research

Scope: the t3code user interface (`apps/web`, the UI-facing parts of `apps/desktop`,
the shared client logic in `packages/client-runtime`, and the wire contracts in
`packages/contracts`). Purpose: input for designing a Rust/Dioxus app that runs
existing coding-agent CLIs and manages them through a UI.

Source root: `/home/theduke/dev/github.com/pingdotgg/t3code` (read-only). All paths
below are relative to it unless absolute.

Conventions: "verified" means I read the code or doc. "Inferred" means I deduced it
from naming or structure and did not read the implementation.

Contents:

1. Executive summary
2. Product shape and architecture boundaries
3. Key file map
4. Client data model and data flow
5. App layout and navigation
6. Sidebar and thread lifecycle
7. Thread view and timeline rendering
8. Composer
9. Approvals, user input, plans, subagents
10. Checkpoints, diff panel, revert, edit-from-here
11. Worktrees, git actions, pull requests
12. Right panel surfaces: terminal, preview browser, devices, files
13. Command palette, keybindings, notifications
14. Settings, onboarding, usage, scheduled tasks
15. Desktop shell aspects
16. Prioritized feature inventory
17. UX lessons
18. Recommendations for a Dioxus implementation
19. Open questions

---

## 1. Executive summary

t3code is a thin, opinionated control surface over existing coding-agent CLIs (Codex,
Claude Code, Cursor, OpenCode, Grok, Antigravity, Pi, plus any ACP-registry agent). It
does not implement an agent. A server process (Node/Effect) owns provider processes,
terminals, git and files. Web, desktop (Electron) and mobile clients are all remote
controllers of that server over an authenticated WebSocket RPC. The desktop app just
bundles and spawns a local server.

The UI is organised around four ideas worth copying:

1. **Thread = durable conversation, run = one user-to-agent cycle.** The UI is a pure
   projection of an event-sourced server model (`thread`, `runs`, `attempts`,
   `turnItems`, `runtimeRequests`, `checkpoints`...). The client keeps a snapshot plus
   a replay cursor and applies idempotent "upsert entity" events.
2. **Inbox-style sidebar.** Threads are sorted into Pinned / Active / (Working) /
   Snoozed / Settled, with a single prioritised status pill per thread (approval >
   input > working > plan ready > completed). A project is a filter scope, not a tree.
3. **Timeline with aggressive collapsing.** Finished turns fold to one line
   ("Worked for 2m 14s"), consecutive tool calls fold into a one-sentence summary
   ("Ran 3 commands, read 5 files"), and only the live tail shows the currently running
   tool. Approvals and questions take over the composer instead of living in the
   timeline.
4. **Composer as the control hub.** Model/effort/permission/plan-mode/workspace/branch
   all live as compact controls around the input. Mid-run input is either queued or
   steered, with a visible, editable, reorderable queue.

Everything else (diff panel, terminal drawer, embedded browser, device simulator,
PR review, usage dashboard, scheduled tasks, remote environments) hangs off a
tabbed right panel and a command palette.

Size of the thing: `ChatView.tsx` 11.3k lines, `ChatComposer.tsx` 7.5k,
`MessagesTimeline.tsx` 5.3k, `Sidebar.tsx` 5.3k. It is a large product; most of the
mass is polish and edge-case handling. A Rust clone should deliberately start from the
core subset (section 16, P0/P1).

---

## 2. Product shape and architecture boundaries

Verified from `docs/internals/overview.md`, `docs/internals/glossary.md`,
`docs/internals/connection-runtime.md`.

| Term | Meaning |
|---|---|
| Environment | One running server plus the machine, credentials and workspace it owns |
| Client | Web / desktop / mobile UI connected to an environment |
| Project | Environment-local workspace rooted at a directory |
| Thread | Durable conversation for a project; survives provider process exits |
| Run (turn) | One user-to-agent cycle. Provider work can end before checkpoint/diff work settles |
| Activity / turn item | Non-message timeline item (tool, approval, error...) |
| Provider instance | One configured provider (several can share a driver, e.g. two Claude accounts) |
| Runtime mode | Permission policy (see 8.4) |
| Interaction mode | `default` or `plan`; orthogonal to permission policy |
| Checkpoint | Workspace state saved as a hidden git ref, used for diffs and restore |

Hard rules the docs state and the code follows:

- Provider processes, terminals, git and files belong to the server. A remote client
  must never substitute its own filesystem or credentials.
- Shared connection and domain state lives in `packages/client-runtime` so web and
  mobile do not diverge on reconnect behaviour. The React views only consume it. For a
  Rust project this maps to: a UI-agnostic client-state crate (our `ui_core`-style
  layer) plus platform shells.
- The RPC contract (`packages/contracts/src/rpc.ts`) is the only boundary between
  independently versioned clients and servers. Capability flags on the environment
  descriptor gate features (for example `threadPullRequests`), never client version.
- Event log is the source of truth. Commands are serialized and idempotent
  (client-chosen `commandId`; the server keeps command receipts). A command ack means
  "intent committed", not "work done".
- Provider-specific behaviour hides behind an adapter. UI branches on **capabilities**,
  not provider names (`docs/orchestration-v2/provider-capability-system.md`,
  "Capability-Driven UI": hide fork when unsupported, show interrupt as
  available / destructive-fallback / unavailable, show approvals as respondable or
  expired, show plan panels only when structured plan artifacts exist, show rollback
  only when an app checkpoint exists).

---

## 3. Key file map

### Web app (`apps/web/src`)

| Area | File |
|---|---|
| Routes | `routes/_chat.tsx` (shell + global shortcut dispatch), `routes/_chat.$environmentId.$threadId.tsx`, `routes/_chat.draft.$draftId.tsx`, `routes/_chat.index.tsx`, `routes/_chat.pull-requests.tsx`, `routes/usage.tsx`, `routes/settings.*.tsx`, `routes/welcome.tsx`, `routes/pair.tsx`, `routes/connect.tsx` |
| Thread screen | `components/ChatView.tsx`, `components/ThreadRouteView.tsx`, `components/chat/ChatHeader.tsx`, `components/chat/ChatCanvas.tsx` |
| Timeline | `components/chat/MessagesTimeline.tsx`, `MessagesTimeline.logic.ts` (row derivation), `session-logic.ts` (timeline entries, pending approvals), `chat/timelineScrollAnchoring.ts`, `chat/timelineMinimapItems.ts`, `chat/WorkLog.tsx`, `chat/TimelineSystemDivider.tsx` |
| Markdown | `components/ChatMarkdown.tsx`, `markdown-incremental.ts` (streaming parse cache), `markdown-links.ts`, `markdown-github-alerts.ts` |
| Composer | `components/chat/ChatComposer.tsx`, `ComposerPromptEditorTiptap.tsx` (Tiptap rich editor with inline chips), `composer-logic.ts` (send intent, triggers), `composerDraftStore.ts`, `chat/ComposerPrimaryActions.tsx`, `chat/QueuedRunsControl.tsx`, `chat/ComposerCommandMenu.tsx`, `chat/ProviderModelPicker.tsx`, `chat/ModelPickerContent.tsx`, `chat/TraitsPicker.tsx`, `chat/ContextWindowMeter.tsx`, `chat/ComposerBannerStack.tsx` |
| Approvals / input / plan | `chat/ComposerPendingApprovalPanel.tsx`, `ComposerPendingApprovalActions.tsx`, `ComposerPendingUserInputPanel.tsx`, `pendingUserInput.ts`, `chat/ProposedPlanCard.tsx`, `ComposerPlanFollowUpBanner.tsx`, `ComposerTasksBadge.tsx`, `proposedPlan.ts` |
| Sidebar | `components/Sidebar.tsx`, `Sidebar.logic.ts` (sections, status pills, drag planning), `Sidebar.drag.ts`, `Sidebar.snooze.ts`, `ThreadStatusIndicators.tsx`, `ThreadHoverCard.tsx`, `sidebar/*`, `lib/threadSort.ts` |
| Right panel | `rightPanelStore.ts` (surface model), `components/RightPanelTabs.tsx`, `RightPanelSheet.tsx`, `rightPanelLayout.ts` |
| Diff | `components/DiffPanel.tsx`, `diffPanelStore.ts`, `components/diffs/*`, `reviewCommentContext.ts`, uses `@pierre/diffs` and `@pierre/trees` |
| Git / worktree | `components/GitActionsControl.tsx` (+ `.logic.ts`), `BranchToolbar*.tsx`, `WorktreeBaseBranchPicker.tsx`, `chat/WorktreeSetupCard.tsx`, `worktreeCleanup.ts` |
| PRs | `components/pullRequest/*`, `PullRequestThreadDialog.tsx` |
| Terminal | `components/ThreadTerminalDrawer.tsx`, `terminalUiStateStore.ts`, `terminal/ghostty/*` (libghostty-vt compiled to WASM) |
| Preview browser | `browser/*`, `components/preview/*`, `previewStateStore.ts` |
| Command palette | `components/CommandPalette*.tsx`, `commandPaletteBus.ts`, `components/search/ProjectContentSearchDialog.tsx`, `components/files/ProjectFilePicker.tsx` |
| Keybindings | `keybindings.ts`, `packages/shared/src/keybindings.ts` (defaults), `packages/contracts/src/keybindings.ts` (command ids) |
| Notifications | `threadNotifications.ts`, `components/ThreadNotificationCoordinator.tsx` |
| Settings | `components/settings/*`, `routes/settings.*.tsx`, `packages/contracts/src/settings.ts` |
| Client state | `state/*` (thin atoms over client-runtime), `uiStateStore.ts`, zustand stores listed per feature |

### Shared logic (`packages/client-runtime/src`)

| Area | File |
|---|---|
| Connection | `connection/supervisor.ts` (retry policy), `connection/registry.ts`, `connection/wakeups.ts`, `rpc/session.ts`, `rpc/client.ts` |
| Shell (sidebar data) | `state/shell.ts`, `state/shellReducer.ts` |
| Thread detail | `state/threads.ts` (subscription + cache), `state/orchestrationV2Projection.ts` (event reducer), `state/threadHistoryMerge.ts`, `state/threadHistoryController.ts`, `state/threadSnapshotHttp.ts` |
| Commands | `state/threadCommands.ts`, `state/composerDispatch.ts` (queue/steer resolution) |
| Work log presentation | `work-log/presentation.ts` (tool grouping summaries), `work-log/toolPresentation.ts`, `work-log/commandLabel.ts`, `state/turnItemPresentation.ts`, `state/subagentDisplay.ts` |
| Terminal | `state/terminal.ts`, `state/terminalSession.ts`, `state/terminalOutput.ts` |
| Misc | `state/gitActions.ts`, `state/vcs*.ts`, `state/pullRequests.ts`, `state/usage.ts`, `state/threadSettled.ts`, `state/threadSnoozed.ts` |

### Contracts (`packages/contracts/src`)

- `orchestrationV2.ts` (3.4k lines): thread shell, projection, turn items, commands,
  events, stream items.
- `rpc.ts` (1.9k lines): ~200 RPC methods grouped by `WS_METHODS` and
  `ORCHESTRATION_V2_WS_METHODS`.
- `providerPolicy.ts` (runtime mode, approval decisions), `settings.ts`,
  `keybindings.ts`, `git.ts`, `worktreeSetup.ts`, `scheduledTask.ts`.

### Desktop (`apps/desktop/src`)

- `main.ts`, `app/DesktopApp.ts`, `backend/DesktopBackendManager.ts` (spawns the local
  server), `preload.ts` + `ipc/channels.ts` (the bridge), `window/*` (menu, quit-hold),
  `preview/*` (guest webview browser with Playwright-style runtime), `snapShot/*`,
  `updates/*`, `ssh/*`, `wsl/*`.

### User docs worth re-reading (`docs/user`)

`composer.md`, `thread-sidebar.md`, `permission-modes.md`, `keybindings.md`,
`source-control.md`, `activity-log.md`, `project-settings.md`, `usage.md`,
`portable-handoffs.md`, `devices.md`, `welcome-wizard.md`.

Design docs: `docs/orchestration-v2/feature-lifecycles.md` (steering, interrupt,
approvals, plans) and `provider-capability-system.md`.

---

## 4. Client data model and data flow

### 4.1 Wire model (verified, `packages/contracts/src/orchestrationV2.ts`)

Two independent streams per environment:

1. **Shell stream** (`subscribeShell`, `OrchestrationV2ShellStreamItem`): lightweight
   data for the sidebar. Items: `synchronized`, `snapshot` (projects + thread shells +
   archived shells), `project.updated`, `project.removed`, `thread.updated`,
   `thread.removed`. Every non-snapshot item carries a monotonically increasing
   `sequence`.
2. **Thread stream** (`subscribeThread`, `OrchestrationV2ThreadStreamItem`): full detail
   for one open thread. Items: `synchronized`, `snapshot` (a full
   `OrchestrationV2ThreadProjection`), `event` (one `OrchestrationV2DomainEvent` plus
   `sequence`), and a decode-only `unknown-event` so older clients skip event types
   they do not know yet while still advancing the cursor.

`ThreadShell` (the sidebar row payload) is rich and precomputed on the server so the
client never needs thread detail for the list. It includes: title, project, model
selection, `runtimeMode`, `interactionMode`, `branch`, `worktreePath`, pull request
links, lineage/fork info, `activeRunId`, `status` (`idle` or run status),
`activityRunStatus`, `lastError` + `lastErrorClass` (`usage_limit` vs others),
`usageLimitResetAt`, `pendingRuntimeRequest` summary, `latestVisibleMessage` preview,
`hasActionableProposedPlan`, `pendingBackgroundTasks`, `providerInstanceHistory`,
counts, `archivedAt`, `settledOverride/settledAt`, `snoozedUntil`, `pinnedAt`,
`pinOrderKey`, `activeOrderKey`, `lastVisitedAt`, `titleRegeneration`, `limitRecovery`.

`ThreadProjection` (the detail snapshot): `thread`, `runs`, `attempts`, `nodes`,
`subagents`, `providerSessions`, `providerThreads`, `providerTurns` (with
`tokenUsage`), `runtimeRequests`, `messages`, `plans`, `turnItems`,
`checkpointScopes`, `checkpoints`, `contextHandoffs`, `contextTransfers`, and
`visibleTurnItems` (server-computed ordered rows the timeline renders).

Run statuses: `preparing, queued, starting, running, waiting, completed, interrupted,
failed, cancelled, rolled_back`. Turn item statuses: `idle, pending, running, waiting,
completed, failed, cancelled, interrupted`.

**Turn item types** (the vocabulary the timeline renders):

| type | Notable fields |
|---|---|
| `user_message` | `text`, `context` (inline chips), `attachments`, `inputIntent` (`turn_start`, `queued_turn`, `steer`, `promoted_queued_to_steer`), `senderThreadId`, `scheduledTaskId` |
| `assistant_message` | `text`, `streaming: bool`, `attachments` |
| `reasoning` | `text`, `streaming` |
| `proposed_plan` | `planId`, `markdown`, `streaming` |
| `todo_list` | `steps[{step,status: pending|inProgress|completed}]`, `explanation` |
| `user_input_request` | `requestId`, `questions[{id, header, question, options[{label, description, value}], multiSelect, allowCustomAnswer, required}]`, answers |
| `approval_request` | `requestId`, `requestKind` (`command`, `file-read`, `file-change`, `mcp-elicitation`, `permission`), `prompt`, `options[{decision,label,warning}]` |
| `command_execution` | `input`, `output`, `exitCode`, `outputIndicatesFailure` |
| `file_change` | `fileName`, `additions`, `deletions`, `diffStr/oldStr/newStr`, `changes[]` |
| `file_search`, `web_search` | patterns + results |
| `dynamic_tool` | `toolName`, `input`, `output`, `viewedImagePath` (MCP/other tools) |
| `subagent` | `subagentId`, `origin`, `childThreadId`, `prompt`, `progress`, `result` |
| `checkpoint` | `checkpointId`, `scopeId`, `files[]` summary |
| `run_interrupt_request` / `run_interrupt_result` | message |
| `system_notice`, `notification` | background-task / subagent notifications |
| `error` | `failure{class,message,code,retryable,resetAt}`, `retry{attempt,maxAttempts,retryDelayMs}` |
| `compaction` | `beforeTokenCount`, `afterTokenCount`, `summary` |
| `handoff`, `fork`, `thread_created` | provider switch / fork lineage markers |

All items share base fields: `id, threadId, runId, nodeId, parentItemId, ordinal,
status, title, startedAt, completedAt, updatedAt` plus tool presentation hints
(`toolSurface`, `toolIcon`, `toolSource`).

**Events** (`OrchestrationV2DomainEvent`) are simple "entity upserted" events:
`thread.*` (created, archived, settled, snoozed, pinned, metadata/runtime-mode/
interaction-mode/model-selection updated, provider-switched...), `run.created/updated`,
`run-attempt.*`, `node.updated`, `subagent.updated`, `provider-session.*`,
`provider-thread.updated`, `provider-turn.updated`, `runtime-request.updated`,
`message.updated`, `turn-item.updated`, `plan.updated`, `checkpoint-scope.created`,
`checkpoint.captured`, `checkpoint.rollback-requested`, `context-handoff.updated`,
`context-transfer.*`. Because each carries the **whole entity** (not a delta), the
client reducer is a trivial `upsert_by_id`.

**Commands** (`OrchestrationV2Command`) include `thread.create/archive/delete/pin/
settle/snooze/fork/...`, `message.dispatch`, `run.interrupt`,
`runtime-request.respond`, `queued-run.edit/reorder/cancel`,
`queued-message.promote-to-steer`, `queue.resume`, `checkpoint.rollback`,
`provider.switch`, `thread.runtime-mode.set`, `thread.interaction-mode.set`,
`thread.model-selection.set`, `thread.metadata.update`, `thread.visit`,
`thread.mark-unread`, `thread.pull-request.link/unlink/watch`.

`message.dispatch` is the central command. Key fields: client-generated `commandId`,
`threadId`, `messageId`, `text`, `context`, `attachments`, optional `titleSeed`,
optional `modelSelection`, `sourcePlanRef` (implement-this-plan), continuation ids
(`restart...`, `usageLimit...`, `manual...`), and
`dispatchMode`: `defer_start | steer_active{targetRunId} | restart_active{targetRunId} |
queue_after_active | start_immediately`, plus a looser `deliveryIntent`
(`auto|steer|restart`) that the server resolves against serialized thread state.

### 4.2 Snapshot + incremental application (verified)

`packages/client-runtime/src/state/orchestrationV2Projection.ts`
`applyOrchestrationV2ProjectionEvent(projection, event, options)` is a pure function:

- Ignores events for other threads.
- `thread.*` events replace `projection.thread` (except `visited`/`marked-unread`,
  which do not bump `updatedAt` because they are read-state, not activity).
- Every other event upserts one entity into the matching array by id.
- `turn-item.updated` additionally maintains `visibleTurnItems` (ordered by
  `ordinal` then id) and handles visibility changes caused by interrupt requests and
  superseded attempts (`run_interrupt_request` can hide other items).
- Special handling for "partial timelines" (bounded snapshots): drop events for items
  older than the loaded window.

Connection flow (`docs/internals/connection-runtime.md`, `state/threads.ts`):

1. A **supervisor** per environment owns transport retry (jittered exponential backoff,
   capped at five minutes, reset only after a connection stays up). Offline and
   auth-failure states wait for a wake-up rather than burning attempts. A socket opening
   is not "ready": the RPC session waits for the initial server config first.
2. **Transport health and data freshness are separate states.** A failed shell
   subscription can coexist with a healthy connection and must not say "reconnecting".
3. **Thread detail** has two lifetimes: subscription lifetime (shared across mounted
   consumers, stops when the last unmounts) and cache lifetime (state + replay cursor
   kept for 5 idle minutes so back-navigation resumes without a new snapshot).
4. Progressive history: a cold open can return a **bounded snapshot** (full control
   plane arrays but only a recent window of timeline rows) with an opaque
   `historyCursor`; "Load earlier" pages fetch older rows over HTTP
   (`threadHistoryHttp.ts`, `threadHistoryMerge.ts`). Expanded timelines are not
   persisted to the offline cache.
5. Cached projections are readable offline but must not imply a live connection and
   must not overwrite newer live data on reconnect.
6. Desktop keeps every starting/running thread subscribed ("keep-alive") so opening a
   running thread needs no replay. Web and mobile do not.
7. Retain cursor and state together only after an update finishes; cancellation must
   not advance the cached cursor past applied data.
8. Mutations are never automatically replayed after reconnect; retry/idempotency is per
   operation (`commandId` receipts).

### 4.3 Optimistic updates (verified, partially)

- The client mints `threadId`, `messageId`, `commandId`. A new thread starts as a
  **draft** (`composerDraftStore.ts`, route `_chat.draft.$draftId`) that exists only
  client-side (persisted to localStorage, including attachments metadata) until the
  first send creates the real thread. `ChatView.logic.ts` keeps "the optimistic
  message and setup progress mounted through the route swap".
- A local-dispatch snapshot (`LocalDispatchSnapshot`) shows an optimistic user message
  and "sending" state until `hasServerAcknowledgedLocalDispatch` observes a new
  latest run / user message, a pending approval/input, or an error. Optimistic
  eviction keys off **visible user turn items**, not `messages`, because
  `message.updated` can arrive one event before `turn-item.updated` (a bug class they
  hit with steer rows).
- Sidebar drag-and-drop uses an `optimisticDrop` state until the server confirms
  (pin/unpin/settle/reorder keys).
- Drafts also track `pendingDraftWork` (image compression, paste downloads): send is
  blocked while any is outstanding, so a message never ships with a chip that has
  nothing behind it.

### 4.4 Client-side stores (not on server)

zustand + localStorage (`persist`): `composerDraftStore` (drafts, per-thread),
`promptStashStore`, `rightPanelStore` (open surfaces per thread), `diffPanelStore`
(selected scope per thread), `terminalUiStateStore`, `previewStateStore`,
`uiStateStore`, `threadSelectionStore` (sidebar multi-select), `browserHistoryStore`.
Server-owned: thread order keys, pins, settle/snooze, visited timestamps (so unread
state syncs across devices), queued messages, settings, keybindings
(`~/.t3/userdata/keybindings.json`).

---

## 5. App layout and navigation

Router: TanStack Router with file routes (`routes/*`, generated `routeTree.gen.ts`).

```
+--------------------------------------------------------------------------+
| titlebar (desktop: drag region, window controls overlay)                  |
+---------------+----------------------------------------+-----------------+
| Sidebar       | Chat header: breadcrumb (project >      | Right panel     |
|  - new thread |   thread title), PR badge, git actions, | (tabs):         |
|  - search +   |   open-in-editor, panel toggles         |  Diff / Files / |
|    project    +----------------------------------------+  File / Browser |
|    scope      | Timeline (virtualized list + minimap)   |  / Device /     |
|  - Pinned     |                                        |  Terminal /     |
|  - Active     +----------------------------------------+  Pull request(s)|
|  - Working    | Banners (errors, limits, provider)      |                 |
|  - Snoozed    | Queued messages strip                   |                 |
|  - Settled    | Composer (approval / question takeover) |                 |
|  - footer:    | Context strip: env, workspace, branch   |                 |
|    PRs, Usage,+----------------------------------------+                 |
|    Settings   | Terminal drawer (bottom, per thread)    |                 |
+---------------+----------------------------------------+-----------------+
```

- Pages: thread (`/$environmentId/$threadId`), draft thread, empty state
  (`NoActiveThreadState`, `NoProjectsHero`), Pull requests list/detail, Usage,
  Settings (General, Providers, Source Control, Connections, Integrations, Keybindings,
  Appearance, Archived, Storage, Projects, Scheduled tasks, Diagnostics, SnapShot,
  Licenses), Welcome wizard, Pairing.
- The right panel is a **tabbed workspace per thread**: `rightPanelStore.ts` models an
  ordered list of surface descriptors (`diff`, `files`, `file:<path>`, `browser:<id>`,
  `device:<id>`, `terminal:<id>` with split groups, `pull-request:<ref>`,
  `pull-requests`) plus an active id. Features own their durable resources; the store
  only points at them. Below 980px width it becomes an overlay sheet
  (`rightPanelLayout.ts`, `RightPanelSheet.tsx`). Tabs have context menus, close
  buttons, scroll overflow, and `mod+w` closes the active tab.
- Chat content has a max width setting (`chatWidth`: comfortable...). Navigation has
  back/forward commands (`mod+[`, `mod+]`), kept like a browser.
- Panel animation duration defaults to **0** (setting `panelAnimationDurationMs`)
  because width transitions cause layout work every frame. Good lesson for Dioxus/
  webview too.

---

## 6. Sidebar and thread lifecycle

Files: `components/Sidebar.tsx`, `Sidebar.logic.ts`, `Sidebar.drag.ts`,
`Sidebar.snooze.ts`, `ThreadStatusIndicators.tsx`, `docs/user/thread-sidebar.md`.
(There is also a `LegacySidebar.tsx` behind a `legacySidebarEnabled` setting, which
groups by project; the current "v2" sidebar is the inbox model below.)

### 6.1 Structure

- Header: "New thread" (shortcut label shown), search field, **project scope combobox**
  (filter the list to one logical project; "Search projects"). Search is dual: local
  title filter plus server-side message search (`useThreadSearch`, starts at 2 chars,
  searches user messages and final agent responses, shows match excerpts).
- Sections in order: **Pinned**, **Active**, optional **Working** (beta, setting),
  **Snoozed** shelf, **Settled** shelf (paged tail: initial N, "show more").
  Shelves are collapsible and remember expansion in localStorage.
- Footer/nav items: Pull requests, Usage, Settings, add project; an update pill.
- Projects: grouped logically across environments (same repo remote = one project
  group, `logicalProject.ts`, `sidebarProjectGrouping.ts`), with favicons/monograms,
  per-project settings, "No project" scratch threads (`~/.t3/scratch/<date>-<slug>-<id>`).
- Context menus per thread: rename, pin, settle, snooze (presets + custom), archive,
  delete, regenerate title, mark unread, copy reference, auto-settle behaviour, open
  in new window, multi-select bulk actions.
- Hover card on rows: model, provider handoff history ("Handed off from X"), terminal
  processes count, error summary ("Usage limit reached" vs "Error occurred"), branch,
  PR status.

### 6.2 Thread status model (verified, `Sidebar.logic.ts`)

Two parallel derivations, worth replicating:

`resolveThreadStatusPill` (what the pill says), in priority order:

1. `Pending Approval` (amber)
2. `Awaiting Input` (indigo)
3. `Working` (sky, pulsing) when run is `running|waiting`
4. `Connecting` (sky, pulsing) when `preparing|starting|queued`
5. `Waiting` (grey) when background work will wake the thread
6. `Plan Ready` (violet) when interaction mode is plan, latest run settled, and an
   actionable proposed plan exists
7. `Completed` (green) when there is an unseen completion (last visited < last done)
8. none

`resolveSidebarThreadStatus` (row styling): `approval | input | working | waiting |
failed | limited | ready`. `limited` is a usage/rate-limit stop and is rendered as a
warning, not an error. A project's indicator is the max-priority of its threads. Rows
**recede** (dimmed) when working, waiting, or ready-and-read; they stay prominent when
they need the user (input, unread, selected, active).

Unread tracking is server-side (`lastVisitedAt`, `thread.visit`, `thread.mark-unread`)
so it syncs across devices; clients fall back to local state against older servers.

### 6.3 Lifecycle features

- **Settle** (move finished work out of Active without deleting). Server-owned
  auto-settle: inactivity (default 3 days), linked PR merged/closed; blocked by running
  work, pending approvals/questions, live background work. Per-thread override
  ("Auto-settle behaviour: Disabled"). Manual settle of an idle thread dismisses
  unanswered async questions and closes idle terminals.
- **Snooze**: hide until a time (presets, custom date/duration), or until a usage limit
  resets (`Snooze until reset`); independent of auto-resume. Wake now.
- **Pin**, **Archive**, **Delete**. Pin/settle/snooze/archive show a 5-second **Undo**
  toast, undone by `mod+z` when no text field is focused (`thread.undo`).
- **Drag and drop**: reorder within Pinned/Active; drag across sections to
  pin/unpin/settle/un-settle/wake; the dragged card previews the verb ("Pin", "Settle"
  ...). Order keys are server-assigned fractional keys (`pinOrderKey`,
  `activeOrderKey`), so reorder survives refresh and syncs. Files dropped onto a thread
  row attach to its composer. Threads can be dragged onto the composer to reference
  them.
- **Usage-limit recovery**: "Resume at reset" schedules a continuation; optional
  auto-resume for limited threads; thread shows `Limited` rather than failed.
- **Sort**: new threads on top of arranged active threads; thread activity does *not*
  reorder the list (stable positions); settled uses settlement time.
- **Working shelf (beta)**: threads that are working but need nothing fold into a
  collapsed bottom section; they return to the top when they finish, fail, or need the
  user. Ordering then switches to "time it came back to you".
- Jump keys: `mod+1..9` open the first nine displayed threads (desktop only, to not steal
  browser tab switching), `mod+shift+[ / ]` previous/next.

---

## 7. Thread view and timeline rendering

Files: `components/chat/MessagesTimeline.tsx` (renderer),
`MessagesTimeline.logic.ts` (pure row derivation, ~1900 lines),
`session-logic.ts`, `packages/client-runtime/src/work-log/*`.

### 7.1 Pipeline

```
projection.visibleTurnItems
  -> deriveTimelineEntriesFromVisibleTurnItems   (session-logic.ts)
       entries: message | work (WorkLogEntry) | event | proposed-plan
  -> deriveMessagesTimelineRows                  (MessagesTimeline.logic.ts)
       rows: message, assistant-meta, work, work-live, work-toggle, turn-fold,
             attempt-fold, working, thinking, context-compaction, event,
             proposed-plan, worktree-setup
  -> computeStableMessagesTimelineRows           (referential stability)
  -> LegendList (virtualized) renders rows
```

Verified design choices:

- **Virtualized list** (`@legendapp/list`) with measured rows, "following end" vs
  "free scrolling" vs "anchoring new turn" scroll modes (`timelineScrollAnchoring.ts`).
  When the user sends, the new turn anchors near the top of the viewport and the
  response grows below; once the user scrolls up, auto-follow stops. Disclosure
  toggles (expanding a fold) suspend end-scroll maintenance so the view does not jump.
- **Row stability**: derived rows are memoised by id so unchanged rows keep identity
  during streaming; row-level context is passed through a React context so callbacks do
  not bust list memoisation. In a Dioxus port this is the equivalent of keyed `for`
  loops over `ReadOnlySignal`s per row plus fine-grained signals.
- **Timeline minimap**: a thin left gutter with a strip per message/turn for
  navigation (`timelineMinimapItems.ts`), hover preview, only on fine-pointer devices.
- **Streaming**: assistant message and reasoning items carry `streaming: bool`; text
  grows via repeated `turn-item.updated` events with full item text. Markdown is parsed
  incrementally (`markdown-incremental.ts`: closed top-level fences followed by blank
  lines are cached as parse boundaries so code-heavy replies are not re-parsed per
  token). `isStreamingMessageTextUpdate` lets the store treat pure text appends as a
  cheap path. A live "Working for 12s" timer row and shimmering "Thinking" row show
  activity; a `DOM`-written timer avoids re-rendering the chat per second (pattern also
  used in `ProviderSubagentBar` and `WorktreeSetupCard`).

### 7.2 Collapsing and grouping rules

1. **Turn fold.** Once a run has produced its terminal assistant message, everything
   between the user's prompt and that final message folds into one row:
   `Worked for 2m 14s` (or `Worked`) with chevron and timestamp. Folds stay open for the
   active run and for failed/interrupted runs. Expansion state is per run id
   (`expandedRunIds`). Result: a long thread reads as a chat of prompts and final
   answers, with work one click away.
2. **Tool group summary.** Consecutive tool-ish entries inside an expanded turn collapse
   into a sentence summarising categories, e.g. `Read 5 files`, `Ran 3 commands`,
   `Searched code 2 times`, `Searched the web 1 time`, `Used <Integration>`,
   `Performed N other actions`; joined as "A, B, and C". A failure flag turns the summary
   red. Group kind picks an icon (`read`, `edit`, `command`, `browser`, `device`,
   `code-search`, `search`, `pull-request`, `reasoning`, `mixed`...). Specific labels
   win when only one kind is present. Activity-log doc: "Commands, file changes... take
   priority over reads and status checks; up to two specific categories and a count of
   the remainder."
3. **Live work row.** While a run is active, contiguous trailing tool entries collapse
   into one `work-live` row that shows the currently running tool (and keeps the latest
   tool in past tense after it finishes instead of vanishing). Reasoning rows show
   "Thinking" / "Thought" with a shimmer while active.
4. **Raw output is not shown inline.** Doc: "Raw command output and tool-result bodies
   are not shown. Use Open diff on a file change." The expanded tool row shows the
   command, status and exit code. (`toolItemForDisplay` strips `output`, `diffStr`,
   `oldStr`, `newStr` for display/copy.) Output is available through the terminal/
   diff surfaces, which keeps the DOM small.
5. **Attempt fold.** A steer that is implemented as interrupt+restart leaves a
   superseded attempt; it renders as "Partial output retained" fold and is hidden by
   default.
6. **Compaction divider.** `Context compacted 82k -> 21k tokens` as a centered
   separator; "Compacting context" with shimmer while running.
7. **Special events** (`V2EventTimelineRow`): errors as a bordered disclosure with tone
   (warning for usage limit, danger otherwise) plus retry progress (attempt/max, delay);
   interrupt request/result; subagent cards (grouped when several); checkpoint rows;
   handoff/fork/thread-created markers; notifications ("subagent finished").
8. **Worktree setup card** (`WorktreeSetupCard.tsx`) renders under the first user
   message with staged progress (stage list with per-stage elapsed time), a Cancel
   button, "Work locally instead" (restarts the same message in the project checkout)
   and "Open terminal" for the setup script.
9. **Assistant message chrome.** Author label ("T3 Code"), copy button (disabled while
   streaming), duration, changed-files summary tree for the turn with "View diff"
   (`ChangedFilesTree.tsx`, expand/collapse all folders), selection toolbar "Cite in
   composer" (`AssistantSelectionToolbar.tsx`, `assistantCitations`), image/media
   attachments in a gallery with a zoom dialog, markdown with GitHub alerts, file-link
   chips that open in the file surface, code blocks with highlighting.
10. **User message chrome.** Text with inline context chips, image thumbnail shelf,
    video/file attachments, queued/steer badges, "Edit from here" (rewind).
11. **Proposed plan card** (`ProposedPlanCard.tsx`): markdown plan with title,
    collapsed preview and "Expand plan", menu to copy / download `.md` / save to
    workspace.
12. **Errors and limits** also surface as composer banners (section 8.9) and sidebar
    state, not only in the timeline.

### 7.3 Timeline details worth noting

- `workEntryDisplayLabel` derives human labels per entry: reasoning shows a one-line
  thought; known tools (`T3 MCP` tools) have first-class names; commands show the
  *unwrapped* command (shell wrappers like `bash -lc "..."` are shortened,
  `work-log/commandLabel.ts`); reads show relative paths ("Read src/foo.rs +2 more");
  searches show the pattern.
- Tool provenance: `toolSource {kind: browser|computer|integration, name, icon}` lets
  the UI show a favicon / app icon / themed logo for MCP-like integrations.
- Timestamps per row with 12/24h setting; relative times in the sidebar.
- Elapsed durations: assistant duration measured from the user message (or steer
  boundary) not from the first output.

---

## 8. Composer

Files: `components/chat/ChatComposer.tsx` (main), `ComposerPromptEditorTiptap.tsx`,
`composer-logic.ts`, `composerDraftStore.ts`, `docs/user/composer.md`,
`docs/user/keybindings.md`, `docs/user/permission-modes.md`.

### 8.1 Layout

Top to bottom, inside one rounded "surface" (`ComposerSurface.tsx`):

1. **Banner stack** (`ComposerBannerStack`): prioritised (`urgent|activity|notice`)
   dismissable banners: provider status/auth problems, usage-limit recovery
   ("Usage limit reached - Resets 5:00 PM" with *Resume at reset* / *Snooze*), server
   update, branch mismatch, clone progress, thread sync status ("Connecting"), plan
   follow-up ("Plan ready: <title>").
2. **Queued messages strip** (`QueuedRunsControl`): rows with image thumbnails, drag
   handle (also arrow keys), promote-to-steer, edit (pencil), remove.
3. **Pending approval / user-input panel**, which *replaces the input* when present
   (see section 9).
4. **Editor**: Tiptap-based rich prompt with inline *chips*; auto-growing; list
   continuation; undo grouping; paste handling.
5. **Footer controls row**: attach, provider+model picker, traits (effort/reasoning/
   fast mode, provider-specific), runtime mode (access), plan toggle, context-window
   meter, usage limits indicator, stash badge, tasks badge, then primary action
   (send / steer / queue / stop / resume / implement plan). Controls collapse to a
   compact overflow menu when narrow (`CompactComposerControlsMenu`,
   `restingComposerControlsMeasurement`).
6. **Context strip** below (for threads in git projects, and on the new-thread hero):
   environment (machine) selector, workspace mode (`Current checkout` | `New
   worktree` | previous worktree), branch picker, base-branch picker for worktrees
   (`BranchToolbar*`, `WorktreeBaseBranchPicker`). Labels collapse to icons when the
   strip is too narrow (with hysteresis).

A **draft hero** (new thread) shows a large headline and the same composer centered.

### 8.2 Triggers and chips

Verified (`composer-logic.ts`): `ComposerTriggerKind = path | pull-request |
slash-command | skill`.

- `@` file/path mention (server-side path search: `projectsSearchEntries` RPC; drag a
  file from the file tree to insert a mention).
- `/` slash commands: T3 commands `/model`, `/plan`, `/default` (work on any line) and
  provider commands (`/compact` etc., must start the message). Skills are optionally
  listed in the slash menu.
- `$` skill mention (provider+environment specific skills).
- `#` pull requests (recent list, number filter, text search) inserting a PR chip.
- `@` thread title: reference another thread (agent reads it on demand).

Chips (`message.context.records`, link form `t3-context://v1/<kind>/<id>`): terminal
excerpt, diff/file **review comment**, preview annotation / picked element, file,
image, PR, thread, assistant citation (quoted response with optional comment). Chips
live at the cursor, are cut/pasteable and copy as Markdown links to other apps. Image
attachments also keep a thumbnail shelf. Chip contract: `docs/internals/composer-context-references.md`.

### 8.3 Attachments

- Up to 100 files per message; images up to 10 MiB (80 MiB total), other files 50 MiB.
- Upload starts immediately on attach (`attachmentUploadQueue`); send is blocked until
  uploads finish; failed uploads retry/remove.
- Pasting >= 32 KiB of text becomes a text-file attachment (hardware shortcut to keep
  it inline). Messages cap at 120,000 chars; the over-limit message names the excess.
- Drag/drop onto the composer, onto a sidebar thread row, or onto a folder; HEIC->JPEG
  conversion and downscale; **SnapShot**: global desktop hotkey captures the active
  window plus accessibility tree and attaches both.
- Video files are passed as paths only (no native video input).

### 8.4 Runtime modes (permissions) and interaction mode

`RuntimeMode`: `approval-required` ("Supervised": ask before commands and file
changes), `auto-accept-edits`, `auto` (provider's automatic review where available),
`full-access` (default). Labels/icons in `chat/runtimeModeConfig.ts`. Defaults at
environment, project override. **Interaction mode** is separate: `default` | `plan`
(toggle `/plan`, keybinding `composer.mode`). Providers enforce modes differently; the
UI states differences in docs rather than hiding the control.

### 8.5 Model picker and traits

- `ProviderModelPicker` + `ModelPickerContent`: provider-instance sidebar (including a
  *Favorites* pseudo-provider), search, keyboard navigation (`mod+shift+m` toggle,
  Left/Shift+Tab to provider list, `mod+1..9` jump to model, `mod+shift+up/down` switch
  provider), favorites pinned first, **multi-select with Shift-click on a new thread to
  fan the same prompt out to several models**, each in its own thread/worktree.
- `TraitsPicker`: renders provider-declared option descriptors generically (effort,
  reasoning effort, variant, agent, fast mode...), so new providers add options without
  UI changes. Option labels are derived from ids as fallback.
- Model defaults: remembered per user, overridden by project. Unset effort/tier means
  "use the provider's own config".
- Custom models can be added in Settings -> Providers.
- Changing provider mid-thread triggers a **context handoff** (see 9.6).

### 8.6 Send semantics, queueing, steering, interrupt (verified)

`composerDispatch.ts` resolves what a send means:

```
running = a turn is in flight
alternate = Mod held / Mod+Enter
default action = Settings "Follow-up behavior": steer | queue   (default: steer)
mode = !running -> auto
       running && !alternate -> default action
       running &&  alternate -> the other one
```

- While running with an **empty** draft the primary button is **Stop/Interrupt**; typing
  turns it into a steer arrow (or queue icon while Mod is held; the tooltip/label tells
  which). Enter sends (setting: Enter | Mod+Enter for multiline | always Mod+Enter);
  Shift+Enter inserts a newline.
- **Steer**: if the provider supports in-place steering, the message joins the active
  run; otherwise the server interrupts and restarts the run with the new message
  (`restart_active`), leaving a superseded attempt. **Queue**: a server-stored list of
  runs that start after the active one; messages survive restarts (held until the user
  presses **Resume**).
- Queue operations: edit in place (original stays until saved, row highlighted;
  composer content restored afterwards), reorder (drag/keyboard), promote to steer
  (`mod+shift+enter` promotes the oldest), remove. `Alt+Up` edits the latest queued.
- Other send variants: `mod+alt+enter` send in background and open a new draft;
  new-thread `mod+enter` same; send to multiple models.
- **Interrupt** is request-then-confirm: the interrupt command is just acked; the run
  turns `interrupted` only when the provider reports the terminal event (design doc).
  `thread.stop` has no default key. A preparing/starting run can be stopped too
  (`deriveCanInterruptRunningThread`).
- Prompt history: `ArrowUp` in an empty composer recalls prior prompts in this thread
  (only text); editing converts it to a normal draft.
- **Prompt stash** (`mod+s`): save current prompt + attachments for later, restore with
  the same key.
- **Edit from here** (rewind) is described in section 10.

### 8.7 Context window meter and usage

`ContextWindowMeter` shows token usage vs the model window from provider turn
`tokenUsage` telemetry; offers manual compaction (`/compact`) when the provider
supports it, with a Claude-specific "resume compaction" prompt when a stale long
session is resumed. `ComposerUsageLimits` shows provider rate-limit windows.

### 8.8 Composer-adjacent controls

- Draft/launch: `Cmd+Enter` start in background. "Start without a project" creates a
  scratch folder.
- `Restart agent session` (command palette) after changing skills/MCP/plugins.
- Provider-native subagent threads show a `ProviderSubagentBar` instead of a composer:
  model, effort, live status ticker, and a link to the parent (you cannot message a
  provider-native subagent; message the parent).

### 8.9 Banners and errors

`ThreadErrorBanner`: glass alert, line-clamped with tooltip for full text, dismissible
per (thread, message) for the session; usage-limit class renders as warning. 
`ProviderStatusBanner`: provider unauthenticated/unsupported/broken version, with setup
action (install/login launches a pre-filled terminal).

---

## 9. Approvals, user input, plans, subagents

### 9.1 Approvals (verified)

- Wire: `approval_request` turn item + `runtimeRequests[]` entry with
  `responseCapability`: `live` (provider callback still alive), `message` (answer by a
  follow-up message), `not_resumable` (provider process is gone).
- UI (`ComposerPendingApprovalPanel` + `Actions`): the **composer is replaced** by a
  compact panel: kind label ("Command approval", "File change approval", "File read
  approval", "App permission approval", "App access approval"), optional app name,
  `1/N` counter when several are queued, then the command/path in monospace (scrollable,
  max 5 rows, focusable). Buttons: primary `Approve`, outline `Decline`; overflow menu
  (`...`) contains `Cancel` and `Always allow this session`. Providers can supply their
  own `options[{decision,label,warning}]`; options with a `warning` show a triangle icon
  and tooltip (used for prompt-injection cautions).
- Decisions: `accept | acceptForSession | acceptAlways | decline | cancel`. Responding
  is `runtime-request.respond`.
- `not_resumable` renders "Provider process is gone - interrupt or restart the run to
  respond" instead of dead buttons.
- Sidebar pill "Pending Approval" (highest priority) and desktop/in-app notification
  (see 13.3). Pending approvals inside subagents are surfaced on the parent thread.

### 9.2 User-input requests / questions (verified)

- `user_input_request` with 1..N questions. Each: header chip, question, options with
  label+description, `multiSelect`, `allowCustomAnswer`, `required`.
- UI (`ComposerPendingUserInputPanel`): stepper card in the composer area. Single-select
  options auto-advance after a short timer with an optimistic selection highlight;
  multi-select toggles; a custom free-text answer (can carry file attachments and
  pasted images). Progress `Q n of N`, answered count, Previous/Next, Submit, Dismiss.
  Collapsible header so a tall prompt does not cover the thread. The card is keyed by
  request id so the next prompt starts expanded. "Message-mode" requests remain
  answerable after the turn ended (they become a normal user message).
- Answers are typed `ProviderUserInputAnswers` (record of question id to value);
  `thread.user-input.dismiss` dismisses without answering. Manual settle auto-dismisses.
- Because answering must not look like composing a normal message, the composer's
  submission target switches (`submissionTarget: "provider-turn" | "pending-user-input"`).

### 9.3 Plans

Three distinct concepts (design doc, verified):

- **Proposed plan** (`proposed_plan` item, `plan.updated` with `kind: proposed_plan`):
  durable markdown artifact the user can accept. Appears as a card in the timeline and
  as the `Plan Ready` sidebar pill and a composer banner. The primary composer action
  becomes **Implement** (starts a new run in the same thread with the plan, and/or
  **Implement in a new thread** with a generated title; `sourcePlanRef` links back) vs
  **Refine** (stay in plan mode and send feedback).
- **Todo list** (`todo_list`, `kind: todo_list`): live progress of the current run.
  Rendered as a `ComposerTasksBadge`: segmented progress bar (max 10 segments) with
  the current step, expandable to a list with Pending / Running / Completed and
  per-step duration. Child/subagent todo lists nest under their node and never replace
  the root plan.
- **Questions**: separate (9.2).

### 9.4 Subagents

- Two kinds: provider-native (child thread created by the provider, read-only to the
  user) and app-owned (T3 MCP tools let an agent create/message threads: "T3
  Orchestrator").
- Timeline: `subagent` cards with prompt, progress and result; several adjacent cards
  group with a status summary (e.g. "3 agents: 2 running"). Items attributed to a
  subagent are re-homed out of the main timeline into an *Agents* surface
  (`ThreadRelationshipsControl.tsx`: Parent / Agents / Previous agents lists, paging for
  big lineages; `ThreadRelationshipIcon`).
- A subagent thread opens like any thread but with the subagent bar instead of a
  composer, and approvals it needs are raised on the parent.

### 9.5 Forks and lineage

`thread.fork` creates a new thread with lineage and a *source point*; provider
selection is deferred to the first run on the fork (native provider fork when
possible, otherwise portable context transfer). The UI shows `fork` and
`thread_created` markers and relationships in the thread details panel.

### 9.6 Provider switching in one thread

Thread keeps one app identity while switching provider/model (`provider.switch`).
Server builds a budgeted **context handoff** (not an agent-written summary) from recent
messages, activity and omitted-history references; the agent can re-read omitted
history with a thread-reading tool. UI shows a `handoff` marker row and sidebar
"Handed off from ...". See `docs/user/portable-handoffs.md`,
`docs/orchestration-v2/provider-switching-and-context.md`.

### 9.7 Thread details panel

`ThreadDetailsPanel` (popover from header): Workspace section (project path, worktree,
branch), Version Control (git status, linked PRs, stack info), thread relationships,
automations (`ThreadAutomationsPanel`: scheduled tasks bound to the thread).

---

## 10. Checkpoints, diff panel, revert, edit-from-here

### 10.1 Checkpoints (server concept, verified)

Hidden git refs capture workspace state after each run (no commits on the user's
branch). `checkpoint` turn items summarise changed files. Checkpoints attach to
*checkpointable execution scopes*; root-run checkpoints advance the run count, child
(subagent) ones nest. Rollback is expressed in app run count and reconciled with
provider conversation state (a provider that cannot roll back its conversation must
reject before touching files).

### 10.2 Diff panel (verified, `DiffPanel.tsx`, `diffPanelStore.ts`)

Scope selector (per-thread, persisted):

- `branch` ("Changes": everything changed since the base ref; base ref pickable),
- `unstaged` (working tree),
- `turn` (a specific run's checkpoint diff; labelled "Latest turn" / "Turn N";
  optionally deep-linked to a file).

Features: unified/split layout setting (`diffLayout`), ignore whitespace toggle,
collapse-files-by-default setting, red/green vs alternative color scheme, file tree
(`DiffFileTree`) with status icons, per-file collapse, lazy per-file patch loading
for large diffs ("truncated" notice), syntax highlighting in a worker pool
(`DiffWorkerPoolProvider`), refresh on window focus, and **line-range review comments**
(`AnnotatableCodeView`, `DiffCommentAnnotation`): select lines -> add comment -> it
becomes a composer chip (`reviewCommentContext.ts`) carrying file path, range label, the
hunk and fence language, which the agent receives with the user's text. This is the
critical "review loop" affordance.

The same diff component powers PR "Code" tab and a "viewed" checkbox per file (synced
with GitHub's viewed state where possible).

### 10.3 Revert / edit-from-here

Beneath each sent user message: **Edit from here** (`MessagesTimeline.tsx:2375`,
`ChatView.tsx:11293`). Dialog "Edit from here?": "Rewind chat to before this message.
Your prompt and attachments return to the composer." Buttons: `Cancel`, `Revert files
too` (destructive), `Revert and keep changes`. File restore only offered for threads in
a worktree and refused if another thread/session shares that directory. Available only
if the provider supports rewind (capability-driven). Drops the message and everything
after from the active thread and provider history. This is "checkpoint rollback" with
the UI minimized to one entry point per message instead of a checkpoint list.

---

## 11. Worktrees, git actions, pull requests

### 11.1 Workspace modes

Composer context strip: `Current checkout` | `New worktree` (base branch picker,
auto-generated branch names with prefix/semantic/custom-instruction options
configurable in Source Control settings) | `Previous worktree` | existing worktree.
Worktree creation runs a staged bootstrap shown in `WorktreeSetupCard`:
stage ids in `packages/contracts/src/worktreeSetup.ts` with per-stage status/timing,
optional setup script from `t3.json` (`scripts[].runOnWorktreeCreate`), submodule
policy (`recursive | top-level | none`). Cancel and "Work locally" fallbacks. Worktree
cleanup policies (inactive N days, merged, no commits; safe-only) in Settings ->
Storage. Without a git repo, branch/diff controls are hidden.

### 11.2 Git actions control (header)

`GitActionsControl.tsx` + `.logic.ts`: one split-button whose primary label is derived
from repo state: `Commit`, `Commit & push`, `Commit, push & create PR`, `Push`,
`Push & create PR`, `Create PR`, `Pull`, `Sync ref`, `Publish repository`. Warns when
acting on the default branch ("Commit & push to default ref?" confirmation). Server
action `git.runStackedAction` with kinds `commit | push | create_pr | commit_push |
commit_push_pr` and streamed progress events (`action_started`, `phase_started`
[branch/commit/push/pr], `hook_started/output/finished`, `action_finished/failed`)
rendered in a toast with CTA ("Open PR"). Commit messages, PR titles and bodies are
generated by a text-generation model chosen in settings, optionally following repo
conventions. Hook output (pre-commit) is streamed.

### 11.3 Pull requests

Multi-host (GitHub, GitLab, Forgejo/Gitea, Bitbucket, Azure DevOps) via the host CLIs
installed on the server (`gh`, `glab`, `fj`/`tea`, tokens for Bitbucket, `az`).
Features: PR list page with filters, detail panel with Summary / Timeline / Code /
checks tabs, comments and reviews, reviewers, labels, merge methods, auto-merge, stack
(GitHub stacks) navigation, files-viewed marks, **link PR to thread** (many-to-many,
auto-detected from branch, agent tool `link_pull_request`), PR badges in the sidebar
and header, `watch_pull_request` agent tool (server polls and wakes the agent on failing
checks/comments/conflicts), auto-settle on merge. Largest self-contained subsystem;
treat as post-MVP.

### 11.4 Projects

Add/clone/publish projects from the command palette; clone runs in the background and
the thread is usable immediately (send waits). Project actions/scripts
(`ProjectScriptsControl`, keybinding `script.<id>.run`) run in terminals; `t3.json` in
the repo can declare scripts, icon, workspace defaults, submodule policy.

---

## 12. Right panel surfaces

### 12.1 Terminal

- Per-thread terminal drawer (bottom) and also as right-panel tabs. Multiple terminals,
  split groups (horizontal/vertical, max per group), restart/clear/close, persisted
  layout (`terminalUiStateStore`, key `t3code:terminal-state:v1`).
- Rendering: **libghostty-vt compiled to WASM** (`terminal/ghostty/*`, vendored `.wasm`),
  selection-action floating toolbar that turns a selection into a composer "terminal
  excerpt" chip (`TerminalContextInlineChip`, `lib/terminalContext.ts`). The terminal
  state is server-held PTY with 5,000 lines / 8 MiB scrollback; RPCs: `terminalOpen`,
  `terminalAttach`, `terminalWrite`, `terminalResize`, `terminalClear`,
  `terminalRestart`, `terminalClose`, plus event/metadata subscriptions. Output is
  delivered as cursor-based incremental updates (`readTerminalOutputUpdate`).
- Terminal status indicator in the sidebar row ("Terminal process running") and
  `settle` closes idle prompts but keeps dev servers. Terminal links can open in the
  preview browser. `!terminalFocus` is a context in most default shortcuts so keystrokes
  are not stolen.

### 12.2 Preview browser (desktop-first)

- Electron `<webview>` tabs (`browser/*`, `desktop/src/preview/*`) with address bar,
  back/forward/refresh, zoom, viewport presets and resize handles, device-toolbar, mute,
  favicon capture, crash recovery, per-profile sessions (Default / Incognito / custom,
  with browser-data import).
- **Agent-controlled**: the same browser is exposed to the agent as MCP tools
  (`preview_open`, `preview_navigate`, `preview_snapshot`, `preview_click`,
  `preview_type`, `preview_evaluate`, recording etc.), with a visible agent cursor
  (`AgentBrowserCursor`), a floating mini-player, optional auto-show, and screen
  recording with key/mouse overlays.
- **Picker/annotation**: user can pick an element or annotate on the page; the result
  becomes a composer chip (preview annotation / element context) the agent gets.
- Discovered local servers (`subscribeDiscoveredLocalServers`) are offered as one-click
  cards in an empty browser tab.
- On non-desktop clients, the browser is limited (iframe/`isPreviewSupportedInRuntime`).

### 12.3 Devices

iOS Simulator / Android Emulator streaming panel with touch input and `device_*` agent
tools (via a device hub). Nice-to-have, out of scope for us initially.

### 12.4 Files

`files` tree surface and `file:<path>` tabs: syntax-highlighted editor/preview with
inline save, rendered Markdown/HTML/CSV/PDF views, line reveal from links
(`path:line`), read-only for files outside the workspace, attachment previews. File
picker overlay (`mod+p`) and project content search (`mod+shift+f`) with highlighted
results; review comments also attach to files (`fileCommentAnnotations`).

---

## 13. Command palette, keybindings, notifications

### 13.1 Command palette (verified)

`CommandPalette*.tsx`; one overlay with three mutually exclusive modes reduced by a
single state machine (`reduceCommandPaletteUiState`): `command` (`mod+k`), `files`
(`mod+p`), `content` (`mod+shift+f`). Re-pressing the mode's shortcut toggles it closed.
Content: actions, projects, threads (recent 12 by default, message search across
environments, min 2 chars), `>` prefix shows only actions, number shortcuts select
entries, "New thread in...", "Add project" (including filesystem browse and clone),
"Change theme", "Link pull request", "Restart agent session", "New project", and
linked-thread lookups from a PR. Focus returns to the composer when closed.

### 13.2 Keybindings (verified)

- User-editable JSON array at `~/.t3/userdata/keybindings.json`, rules `{key, command,
  when?}`; **last matching rule wins**; new defaults are appended on upgrade without
  overriding user customisation; invalid rules ignored.
- Key syntax: `mod` (Cmd on macOS, Ctrl elsewhere), `ctrl`, `alt`, `shift`, `meta`.
- `when` expressions with `!`, `&&`, `||`, parentheses over context keys:
  `terminalFocus, terminalOpen, previewFocus, previewOpen, modelPickerOpen,
  usagePageOpen, composerFocus, composerDraft, turnRunning, editableFocus, isWeb,
  isDesktop`.
- Settings page lists all command ids and edits them; project scripts get
  `script.<id>.run`.
- Defaults (`packages/shared/src/keybindings.ts`): sidebar toggle `mod+b`, back/
  forward `mod+[` `mod+]`, terminal toggle `mod+j`, split `mod+d`/`mod+shift+d`
  (terminal focus), new terminal `mod+n`, close `mod+w` (terminal or panel tab), diff
  toggle `mod+d`, right panel `mod+alt+b`, preview toggle `mod+shift+j`, command
  palette `mod+k`, file picker `mod+p`, project search `mod+shift+f`, usage `mod+u`,
  composer stash `mod+s`, steer queued `mod+shift+enter`, edit queued `alt+arrowup`,
  alternate send `mod+enter` (when running), send in background `mod+enter` (new
  thread) / `mod+alt+enter`, new thread `mod+n` / `mod+shift+o`, new local thread
  `mod+shift+n`, new no-project thread `mod+alt+n`, model picker `mod+shift+m`, host
  `mod+shift+h`, effort `mod+shift+e`, access mode `mod+shift+a`, workspace
  `mod+shift+x`, branch `mod+shift+g`, previous worktree `mod+shift+l`, previous/next
  thread `mod+shift+[` `]`, copy thread ref `mod+shift+c`, settle `mod+shift+s`, pin
  `mod+shift+p`, undo `mod+z` (not in editable), open in editor `mod+o`, theme select
  `mod+alt+a`. Thread jump `mod+1..9` on desktop.
- Design lesson: the central dispatcher lives in the route shell (`_chat.tsx`
  `resolveShortcutCommand(event, keybindings, {context})`); individual components only
  handle local keys. Composer send intent is resolved through the same table
  (`composerSubmissionIntentForKey`), so the user can rebind send variants.

### 13.3 Notifications (verified, `threadNotifications.ts`, `ThreadNotificationCoordinator.tsx`)

- Setting `notificationMode`: Off | Notifications only | Sound only | Notifications with
  sound; separate in-app toast option.
- Coordinator watches thread shells across *all* environments and fires on transitions
  to *needs you* (approval -> shield icon, question -> message icon, completed -> check,
  failed -> alert icon) with distinct sounds for completion vs input-needed. Uses the
  web `Notification` API with `tag = envId:threadId` (replacing previous), `silent: true`
  (sound played separately), click focuses window and navigates to the thread.
- Badge count: dock/taskbar badge via desktop IPC (`SET_NOTIFICATION_BADGE_CHANNEL`),
  canvas-drawn favicon badge in browsers and on Windows.
- Mobile push exists (`docs/user/mobile-notifications.md`).

---

## 14. Settings, onboarding, usage, scheduled tasks

### 14.1 Settings

Layering (verified `docs/user/project-settings.md`, `docs/internals/overview.md`):

- Client settings (device-local): appearance, fonts, notification mode, confirmations,
  diff prefs, browser prefs, send shortcut, follow-up behaviour, sidebar prefs.
- Environment settings (on the server): defaults for new threads (model, permissions,
  workspace), auto-settle rules, auto-pull, source control text generation, worktree
  naming/submodules, storage cleanup, providers, integrations.
- Project overrides: any environment setting can be overridden per project.
  Resolution order: project override -> environment -> repo `t3.json` -> built-in.
  A "layers" icon next to each row shows where the effective value came from; "Mixed"
  appears when editing several environments that disagree; "All environments" is an
  explicit bulk edit, not a stored global default.
- Pages: General, Providers (instances, auth, models, custom models, usage providers,
  updates, accent colors), Source Control (account detection, rescan, branch naming),
  Connections (environments, pairing, remote access, Tailscale, SSH), Integrations
  (browser/device access for agents), Keybindings, Appearance (themes incl. VS Code
  theme import, fonts, contrast, glass opacity, panel animation), Archived threads,
  Storage, Projects, Scheduled tasks, Diagnostics (traces, process list, resource
  telemetry), SnapShot, Open-source licenses.
- Settings is searchable and the project/environment target is URL state.

### 14.2 Onboarding

`welcome-wizard.md`: connect computers (local server / paired / T3 Connect), check
agents (detect installed/signed-in Claude Code and Codex, one-click open terminal with
the right command), import projects discovered from agent histories (git repos first,
preselect recently active ones with >=3 conversations). Gate also requires an empty
workspace so existing installs skip it.

### 14.3 Usage page

Aggregated token/cost/limits from local agent histories (Codex, Claude, Cursor,
OpenCode, Grok, Antigravity), estimated API-equivalent cost split by token type, per-
model breakdown and trends, rate-limit windows, per-environment filter, custom model
prices, keyboard (`c/t/l`, `mod+shift+1..4` for periods).

### 14.4 Scheduled tasks

Recurring prompts (`ScheduledTask`: title, prompt, schedule, project, optional thread,
workspace strategy, model, runtime/interaction mode), list with enable/pause/run-now/
delete; each run creates a thread (or posts to a bound one) and shows
`scheduledTaskId` on the user message. Mobile and web both manage them.

### 14.5 Remote access (brief)

Environments are first-class in the UI (machine icon per project, environment picker
in the composer strip, pairing links, Tailscale, SSH bootstrap, T3 Connect relay).
Multiple environments aggregate into one sidebar. For our project this is only
relevant as a reminder that "server may not be local" shapes the architecture.

---

## 15. Desktop shell aspects

Electron main (`apps/desktop/src`), Effect-based services:

- Spawns and supervises a local backend (`DesktopBackendManager`, pool), optional WSL
  backend, SSH-tunneled remote backends, server exposure modes (loopback / network /
  Tailscale serve).
- Preload bridge (`preload.ts`, `ipc/channels.ts`) exposes: folder picker, native
  context menus (with DOM fallback `contextMenuFallback.ts`), open external/in editor,
  theme set, notification badge, paste-as-text, update state machine (check/download/
  install, channels), client-settings persistence, connection catalog (secure storage
  via safeStorage / Linux secret storage), SSH host discovery and password prompts,
  quit-hold overlay, app activation (protocol URL handling), window fullscreen state,
  preview webview management, and SnapShot capture/permissions.
- Native application menu and quit shortcut handling (hold-to-quit 1.2 s or double
  press) to prevent accidental quits that kill running agents.
- Auto-update with release notes, and server update for remote environments.
- Linux specifics (global shortcut portal, desktop entry identity) and native modules
  isolated in child processes so a crash cannot take down the app.
- The web UI is the same bundle; capability checks (`window.desktopBridge`, `isDesktop`
  when-context) switch desktop-only features (in-app browser, jump keys, snapshot).
  Takeaway: keep the UI shell-agnostic and inject a platform trait.

---

## 16. Prioritized feature inventory

Legend: P0 = needed for a usable v1; P1 = strong follow-up; P2 = nice-to-have;
P3 = skip unless the product demands it.

### 16.1 Core data/state

| Pri | Feature | Notes |
|---|---|---|
| P0 | Thread/run/turn-item projection with entity-upsert events and a pure reducer | Copy the "event carries whole entity" pattern |
| P0 | Shell stream (thread list) separate from per-thread detail stream | Keeps sidebar cheap |
| P0 | Snapshot + sequence cursor resume; skip unknown event types | Forward compatibility |
| P0 | Idempotent commands with client-generated ids; ack != done | |
| P0 | Transport vs data-freshness status separation; jittered backoff reconnect | |
| P1 | Bounded initial snapshot + "load earlier" paging | Needed once threads get long |
| P1 | Offline cache of last projection | |
| P2 | Multiple environments / remote servers | Architecture should allow it, UI later |

### 16.2 Navigation / sidebar

| Pri | Feature |
|---|---|
| P0 | Thread list with project scope filter, per-thread status pill, running indicator |
| P0 | New thread (with project pick), rename, archive, delete |
| P0 | Unread/completed tracking synced server-side |
| P1 | Pin, settle (manual), snooze, undo toast, multi-select |
| P1 | Thread search (title + message full-text) |
| P1 | Command palette (threads, projects, actions) |
| P2 | Drag-and-drop reordering/sections; auto-settle; working shelf; hover cards |
| P2 | Scratch (no-project) threads |
| P3 | Fractional order keys across devices (needed only with drag reorder) |

### 16.3 Thread timeline

| Pri | Feature |
|---|---|
| P0 | User / assistant messages with markdown + code highlighting + copy |
| P0 | Streaming assistant text and reasoning; live "working" indicator + timer |
| P0 | Tool call rows: command (with exit code), file change, file/web search, generic tool |
| P0 | Turn fold ("Worked for ...") and per-group tool summaries |
| P0 | Error rows and thread-level error banner |
| P0 | Auto-follow scroll with "free scrolling" escape and virtualization |
| P1 | Todo list progress, proposed-plan card |
| P1 | Subagent cards and parent/child navigation |
| P1 | Changed-files summary per turn with open-diff |
| P1 | Compaction divider; token usage/context meter |
| P2 | Timeline minimap; attempt folds for steer-restarts; markdown incremental parse cache |
| P2 | Selection -> "cite in composer" |

### 16.4 Composer

| Pri | Feature |
|---|---|
| P0 | Multiline input, Enter send, Shift+Enter newline, Stop button while running |
| P0 | Provider/model picker, runtime (permission) mode, plan/default toggle |
| P0 | Pending approval takeover (approve / decline / always-this-session) |
| P0 | Pending question stepper (options, multi, custom text) |
| P0 | Draft persistence per thread |
| P1 | Queue vs steer with visible, editable, reorderable queue; configurable default |
| P1 | Reasoning effort / provider option descriptors rendered generically |
| P1 | Image + file attachments (paste, drop), upload progress |
| P1 | `@file` mentions and `/` commands, skills |
| P1 | Prompt history recall, stash |
| P1 | Banner stack (provider auth, usage limit, errors) |
| P2 | Rich inline chips (terminal excerpt, review comment, PR, thread) |
| P2 | Fan-out to multiple models; send-in-background; implement plan in new thread |
| P3 | Voice input, SnapShot, HEIC conversion |

### 16.5 Review / VCS

| Pri | Feature |
|---|---|
| P0 | Diff panel for current worktree changes (unified, file tree) |
| P1 | Per-turn diffs backed by checkpoints; "Edit from here" rewind with optional file revert |
| P1 | Line-comment review that feeds back into the composer |
| P1 | Worktree-per-thread creation with setup progress card; branch picker |
| P1 | Git actions split button (commit / push / create PR) with streamed progress |
| P2 | AI-generated commit/PR text; PR panel (summary, checks, comments) |
| P2 | Worktree cleanup policies; project scripts (`t3.json`-style) |
| P3 | Multi-host PR matrix, stacks, auto-merge, viewed marks |

### 16.6 Panels

| Pri | Feature |
|---|---|
| P0 | Terminal drawer (single terminal, PTY on server, scrollback cursor) |
| P1 | Tabbed right panel (diff, file, terminal) |
| P1 | File viewer with line reveal from links |
| P2 | Multiple/split terminals; terminal-excerpt-to-composer |
| P2 | Embedded preview browser (needs webview); agent-controlled browser |
| P3 | Device simulators, screen recording, SnapShot |

### 16.7 Cross-cutting

| Pri | Feature |
|---|---|
| P0 | Configurable keybindings file with `when` contexts and last-match-wins |
| P0 | Desktop notifications for approval / question / done / failed with click-to-thread |
| P1 | Settings with layering (client / server / project) |
| P1 | Onboarding that detects installed agent CLIs and their auth state |
| P1 | Provider status + setup (install/login command in a pre-filled terminal) |
| P2 | Usage dashboard; scheduled tasks; themes and font settings |
| P2 | Usage-limit recovery (resume at reset / snooze until reset) |
| P3 | Remote environments / Tailscale / SSH, mobile app, telemetry |

---

## 17. UX lessons

1. **Treat the thread list as an inbox.** One highest-priority status pill per row
   (approval > question > working > plan ready > unread done), dim what does not need
   you, and never reorder rows because of activity. Keep order stable and let
   *attention state* (color, dot, badge count) carry urgency. Provide "settle"
   (done-for-now) distinct from archive/delete.
2. **Separate "needs me" from "in progress".** Approvals and questions take over the
   composer in place, with a counter; they are also the top sidebar status and the main
   notification triggers. Do not bury them in the timeline.
3. **Fold history, show the tail.** Fold finished turns to "Worked for 2m", summarise
   tool runs in one sentence, show only the current tool live and keep the last tool in
   past tense after it ends (avoids flicker). Raw tool output is intentionally not in the
   timeline; inspect via diff or terminal. This keeps both DOM size and cognitive load
   low.
4. **Server-computed visibility.** `visibleTurnItems` is computed server-side
   (superseded attempts, interrupted steps) and only patched on the client. Avoid
   re-deriving complicated visibility rules in the UI.
5. **Make steer vs queue explicit and reversible.** Show what the send button will do,
   offer the opposite on a modifier, show the queue above the composer, allow edit/
   reorder/promote, and keep the queue durable across restarts (held until explicit
   Resume). The empty-composer button doubles as Stop; typing turns it into steer.
6. **Interrupt is asynchronous.** Show "stopping" until the run reports `interrupted`;
   do not assume. Allow stopping runs that are still preparing/starting.
7. **Capability-driven controls.** Hide or degrade rewind, fork, steer, compaction,
   `auto` permission mode, subagent messaging etc. by provider capability, with
   explanatory copy (`not_resumable`: "Provider process is gone - interrupt or
   restart"). Render provider-declared option descriptors generically so adding a
   provider does not require UI changes.
8. **Permission modes: few, named, per thread.** Four levels with one-line
   descriptions; the default for new threads comes from settings (project override
   possible); changing it mid-thread is allowed. Plan vs default mode is a separate
   axis.
9. **Composer context strip for the "where does this run" decision**: environment,
   checkout vs new worktree, branch. Collapse labels to icons when narrow.
10. **Context as inline chips, not hidden attachments.** Review comments, terminal
    excerpts, picked elements, PRs and threads appear at the cursor and travel with the
    text. Great for precision; costs a custom rich editor (Tiptap) and a chip contract.
    Consider starting with a plain textarea plus an attachment list, adding chips later.
11. **Close the review loop in the UI.** Diff -> select lines -> comment -> chip in
    composer -> send. This is the highest-value path for a coding-agent UI.
12. **Recoverable destructive actions.** 5-second Undo toasts for sidebar actions; a
    two-button revert dialog (keep changes vs revert files); refusing file restore when
    the directory is shared by other sessions.
13. **Errors are classified.** `usage_limit` is a warning with a recovery flow (resume
    at reset, snooze), distinct from errors; banners are dismissible per (thread,
    message); provider setup problems link to a fix action.
14. **Drafts are first-class and survive.** Per-thread draft persistence, a prompt
    stash, prompt history, blocking send during pending paste/compress work, prompt
    length validation with a precise over-limit message, large paste -> attachment.
15. **Avoid per-frame work.** Panel animation default 0; timers write to DOM nodes
    directly; rows memoised by id; markdown parse cache; virtualized timeline; lazy
    per-file diff loading; worker pool for syntax highlighting. They hit these problems
    at scale, we will too.
16. **Keyboard everything, with one dispatcher and context conditions.** Central
    shortcut table with `when` contexts, editable JSON, last-match-wins, `!terminalFocus`
    on most defaults so the terminal is never hijacked. Show shortcut labels in menus/
    tooltips from the same table.
17. **Notify on state transitions, not events.** Fire when a thread *becomes* blocked or
    done, replace by tag, click to focus the thread, and show a badge count; separate
    sounds for "needs input" and "done". Make notifications opt-in (default off).
18. **Transport state != data state.** Show "Reconnecting" only when a transport retry
    is actually pending; show stale-cached data labelled as such.
19. **Name things for the user, not the provider.** Provider instances (multiple
    accounts of the same driver), model favorites, and handoff markers read in model/
    account terms ("Handed off from Claude (work)").
20. **Keep rare giant features modular.** PR management, devices, usage, remote access
    are each large and live behind separate routes/panels with their own capability
    flags; the core thread experience never depends on them.

---

## 18. Recommendations for a Dioxus implementation

These are my suggestions, not t3code facts.

### 18.1 Layering (maps onto our existing crates)

- A **shared data crate** (like `crates/data`) holding the wire types: thread shell,
  projection, turn items, domain events, commands, stream items. Derives live there;
  RPC only consumes them. Mirror t3code's rule that every event carries the full
  entity so reducers are `upsert_by_id`.
- A **UI-agnostic client-state crate** (like `ui_core`): `ThreadStore` (snapshot +
  cursor + reducer), `ShellStore`, `Drafts`, `QueueState`, pure `derive_timeline_rows`
  and `derive_sidebar_status` functions. Keep all derivation functions pure and
  unit-tested (t3code has `*.logic.ts` + tests for nearly everything; that is the
  reason they can keep iterating on an 11k-line view).
- A **Dioxus UI crate** on top: components consume `ReadOnlySignal`s; rows keyed by
  stable row ids. A platform trait for desktop-only capabilities (notifications,
  badge, folder picker, open-in-editor, in-app webview).

### 18.2 Suggested minimal screen set (P0/P1)

1. Sidebar (projects scope, thread list with status pills, new thread, search).
2. Thread view: header (title, branch, git status), virtualized timeline, composer.
3. Right panel with tabs: Diff, Terminal, File.
4. Command palette.
5. Settings (providers + keybindings + notifications + defaults).
6. Onboarding that detects installed agent CLIs.

### 18.3 Timeline rendering plan

Port `deriveMessagesTimelineRows` in spirit, not line for line. Row enum:
`User`, `Assistant`, `TurnFold`, `WorkGroup{summary, entries}`, `WorkLive`, `Working`,
`Thinking`, `Event(item)`, `Plan`, `Compaction`, `SetupCard`. Start with: user,
assistant, turn-fold, work-group summary, live row, error event. Virtualization in
Dioxus is the riskiest part; options: windowing in Rust keyed by measured heights
(requires JS interop for measurement), or limit rendered history ("show earlier")
initially, which t3code also does server-side via bounded snapshots.

### 18.4 Streaming

Send whole-item `turn-item.updated` events at a throttled rate (t3code relies on
batching; I did not verify the server coalescing interval). Reduce re-render cost by
keeping the streaming item in its own signal and by caching parsed markdown prefixes.

### 18.5 Terminal and diff

- Terminal: server-owned PTY with cursor-based output deltas; client renders with a
  terminal widget (xterm.js via JS interop is the pragmatic route; t3code uses
  libghostty-vt WASM). Provide "send selection to composer".
- Diff: server computes patches; client renders unified diff with per-file lazy load and
  line selection for comments. Start with plain unified rendering and highlight via a
  Rust highlighter server-side or `syntect`/tree-sitter on the client.

### 18.6 Things to defer

Rich Tiptap-style chips, minimap, drag-and-drop sections, auto-settle, PR hosts matrix,
embedded browser, device simulators, SnapShot, voice, mobile.

### 18.7 Testing approach (from t3code)

Pure logic modules with table tests; replay-backed integration tests for the
orchestrator (only the provider transport is faked); fixtures for projections
(`orchestrationV2TestFixtures.ts`); SSR snapshot tests for components
(`MessagesTimeline.test.tsx`). Our `dxgraph` crate already has SSR tests, so the same
style transfers.

---

## 19. Open questions

- Event batching/coalescing for streaming deltas on the server was not verified; I only
  saw that clients treat text-only updates as a fast path.
- Exact stage list of worktree setup (`worktreeSetup.ts`) and the full list of settings
  were skimmed, not exhaustively read.
- Mobile app (`apps/mobile`) was not examined; docs imply parity of core flows with
  reduced diff/PR features.
- How "Auto" permission mode is implemented per provider is documented but not
  inspected in code.
- The legacy sidebar (project tree) is still present behind a setting; I did not
  compare its behaviours with the inbox-style sidebar beyond noting it exists.
