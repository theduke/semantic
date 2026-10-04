# Step 14: AI UI workspace: inbox, new thread, providers, review panel

Wave W5. Depends on: 13 (and the API from 06; the review panel uses `diff`/`revert`).
Agent: opus. Runs in parallel with step 11. Develop against `MockAgentsSource`.

Read: research/t3code-ui.md §5 (layout), §6 (sidebar inbox, status model), §10
(checkpoints, diff panel, revert), §11 (worktrees, workspace modes), §13.3
(notifications), §14.1-14.2 (settings, onboarding), §16-17; steps/12, steps/13.

## Goal

The full coding-agent workspace, composed in one top-level component that host apps
mount:

```rust
#[component] pub fn AgentsWorkspace(source: Rc<dyn AgentsSource>, scope: Option<String>, selected_thread: Option<String>) -> Element
```

Layout is a CSS grid:

* left: the inbox;
* center: `ThreadPane` or the new-thread view;
* right: a collapsible review panel.

Below 860 px the inbox becomes a `dxcomp::Sheet` and the review panel an overlay.

## Inbox (`agents/inbox/`)

* Header:
  * "New thread" button;
  * search field (title filter locally plus the server `search` param, title full-text,
    after 2 chars; message search is not available because transcripts are
    provider-owned, K12);
  * workspace filter (`dxcomp::Select`, "All workspaces").
* Sections:
  * **Pinned**;
  * **Active** (unsettled, not archived);
  * **Settled** (collapsed, paged);
  * Archived is reachable via a filter toggle.
  Delegated child threads nest under their parent with a disclosure.
* Row: title, relative time, `StatusPill` from `Attention` (approval > question >
  plan ready > failed > limited > working > unread done), and a running pulse. Rows
  **do not reorder on activity** within a session: sort by `created_at` desc in
  Active, with new threads on top (UX lesson 1). Needs-you rows are visually
  prominent; working and read rows recede.
* Row menu: rename, pin/unpin, settle/unsettle, archive, delete (confirm via
  `ConfirmDangerDialog`-like dialog from dxcomp `AlertDialog`), copy id. Show an undo
  toast for settle, pin and archive (5 s), using the ui_core `ToastProvider` action.
* Live: `use_thread_list` with `watch_list`.
* Keyboard: `Mod+Shift+O` new thread; `Alt+Up/Down` previous/next thread (registered
  through a small keymap helper in this crate; a global registry is later).

## New thread view (`agents/new_thread.rs`)

* Workspace picker:
  * existing workspaces;
  * "Add workspace…" opens a dialog with a path text field (validated server-side;
    the desktop platform `pick_directory` fills it when available).
* Workspace mode for git workspaces: "Current checkout" or "New worktree" (branch
  name, base branch select from the `workspaces.branches` source call defined in
  step 06).
* Provider, model, access and plan controls reuse the step 13 pickers. Defaults come
  from the workspace, else the last used (local storage).
* A large composer creates the thread with `initial_message` and navigates to it.
* "Import existing session…" (secondary action): pick an instance and enter or
  select a native session id. A session list is a later extension; v1 takes the id
  as text. This calls `threads.import`. Possible because history is provider-owned.
* A provider not ready (not installed or unauthenticated) shows a banner with a link
  to the providers settings.

## Providers settings (`agents/providers/`)

* A list of instances with driver, display name, enabled toggle, status (installed
  version, auth state with account label, compatibility) and a refresh button.
* Edit dialog:
  * display name, binary path, home dir, extra launch args, env (secret entries
    show "provided by environment variable SEMANTIC_AGENTS_ENV_<NAME>");
  * ACP: agent preset select plus command;
  * MCP capability defaults (checkboxes for SemanticWrite, Tasks).
* Unauthenticated: show auth methods. For terminal methods show the command to copy
  ("Run `claude auth login` in a terminal"). The embedded terminal is later.
* Onboarding empty state: when no instance is enabled, show detected agents with
  "Enable".

## Review panel (`agents/review/`)

* Tabs:
  * **Changes**: the per-run diff for the selected run, with a run selector
    (defaults to the latest run with a checkpoint);
  * **All changes**: thread start to working tree;
  * **Links**: thread links (URL, PR, entity, task; entity links use the host's
    entity navigation callback).
* `DiffView` from step 12 with lazy loading of large diffs (the source `diff` call is
  per selection).
* **Revert**: "Revert to before this run…" opens a dialog explaining the effects,
  with two actions: "Revert conversation and files", disabled with an explanation
  when the workspace is not isolated (error code `invalid_state` message), and
  "Revert conversation only". It calls `revert`.
* The file-change rows' "view diff" action from the timeline opens this panel scoped
  to that run and file.

## Notifications

* Through the `AiUiPlatform` trait. A pure `NotificationCoordinator` decides when to
  notify, based on thread-list stream transitions:
  * notify when a thread's attention changes **into** approval, question, plan_ready,
    failed or unread_done, and the thread is not the visible, focused one;
  * deduplicate by `(thread, attention, run)`.
* Off by default. A setting toggle in the providers/settings view requests
  permission.
* Desktop implementation is provided by `semantic_ui` in step 15; web uses the
  browser Notification API via `document::eval` (optional; may be the Noop on web
  for v1; document this).

## Settings persistence

UI preferences (follow-up default steer/queue, send key, notifications, inbox
collapsed sections, last used provider/model/access) go in local storage via the
platform or drafts module. No server settings in v1.

## Tests

* Pure:
  * inbox sectioning and ordering, including nesting of delegated threads and
    stability under activity;
  * `NotificationCoordinator` transitions;
  * new-thread defaults resolution.
* SSR: inbox with fixture threads covering all attention states; providers list (all
  status kinds); review panel with a fixture diff; revert dialog states.
* Mock-source flows:
  * create a thread from the new-thread view (captured payload, navigation callback
    called);
  * a settle action with undo;
  * the revert dialog sends the correct payload.
* wasm check.

## Acceptance

* `AgentsWorkspace` renders all areas against the mock source. Tests and wasm check
  are green.
* One commit: "Add AI UI agents workspace: inbox, new thread, providers, review".
