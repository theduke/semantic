# Step 13: AI UI thread view: timeline, composer, requests

Wave W4. Depends on: 12. Agent: opus. Runs in parallel with step 10. Develop against
`MockAgentsSource`. A live backend is not required (step 15 does the end-to-end).

Read: research/t3code-ui.md §7 (timeline pipeline, folding and grouping rules), §8
(composer), §9 (approvals, questions, plans, subagents), §16.3-16.4 (priorities), §17
(UX lessons 1-8, 13-15); steps/12; steps/06 (ThreadView, SendMode, RequestResponse);
steps/01 (Item and Request types).

## Goal

The thread pane, split into two layers:

* `chat/`: **generic** chat components that only need `semantic_agent` types and a
  `ChatController` trait. Any project with an AI chat can reuse them.
* `agents/thread/`: the coding-agent thread pane. It composes `chat/` with
  `use_thread`, the composer pickers, the queue, requests, banners and the header.

## chat/ (generic)

```rust
pub trait ChatController: 'static {   // implemented by the agents layer (and by other projects)
    fn send(&self, input: ComposerSubmit) -> LocalBoxFuture<'_, Result<(), String>>;
    fn interrupt(&self) -> LocalBoxFuture<'_, Result<(), String>>;
    fn respond(&self, request: RequestKeyStr, response: RequestResponse) -> LocalBoxFuture<'_, Result<(), String>>;
}
```

Components:

* `Timeline { rows: ReadSignal<Vec<TimelineRow>>, item_signal: impl Fn(&RowKey) -> ReadSignal<ItemView> }`:
  keyed rows inside `ChatScroll`, with `role="log"`.
* `derive_timeline_rows(view: &ThreadView, ui: &TimelineUiState) -> Vec<TimelineRow>`: a
  **pure** function, the heart of this step. It is table-tested. Row kinds:
  * `UserMessage` (content parts, intent badge for steer/queued);
  * `AgentMessage` (StreamingMarkdown, copy button disabled while streaming,
    duration);
  * `TurnFold{run, label: "Worked for 2m 14s", expanded}`: wraps the work between the
    user message and the final agent message of a completed run. Collapsed by default
    for completed runs; open for the active run and for failed or interrupted runs;
  * `WorkGroup{summary, entries, failed}`: consecutive tool, command, file and search
    items summarized into one sentence by category ("Ran 3 commands, edited 2 files
    and read 5 files"). Specific categories take priority; at most two named plus a
    remainder count;
  * `WorkLive`: the currently running tool. After it ends it stays in past tense, so
    the row does not flicker;
  * `Reasoning` ("Thinking…" shimmer while streaming; "Thought for Ns" collapsed);
  * `TodoList` (checklist with statuses);
  * `PlanProposal` card (markdown, expand, copy; actions come from the request panel);
  * `Subagent` card (title, status, summary; link to the child thread via the
    navigation callback);
  * `Error` (classified: usage limit or rate limit as a warning tone with `reset_at`;
    others danger, with an expandable message);
  * `Notice` (centered divider style);
  * `Compaction` divider;
  * `Working` indicator with `ElapsedTimer` when the run is active and no row is
    streaming.
* Item detail when a row is expanded:
  * command: command text, exit code, duration, output in `AnsiText`. Long output is
    collapsed to the last 200 lines with "show all"; output is whatever the live
    session or provider history delivered, nothing is fetched separately;
  * file change: paths with +/- counts and a "view diff" action (calls back to the
    host, which opens the review panel in step 14);
  * tool call: name, server, compact JSON input, output preview;
  * web search: the query and results.

  Raw output is never shown inline by default (t3code UX lesson 3).
* `Composer` (generic):
  * auto-growing textarea;
  * Enter sends, Shift+Enter inserts a newline; configurable via the prop
    `send_on: Enter|ModEnter`;
  * attachments list (images and files via an upload callback prop);
  * slots for leading and trailing controls (the agents layer adds pickers);
  * a primary action button whose state comes from `ComposerAction`:
    `Send | Steer | Queue | Stop | Disabled{reason}`. The empty composer shows Stop
    while running; typing switches to the default follow-up action; Mod reverses
    steer and queue;
  * draft value bound to a signal provided by the host.
* `RequestPanel { request: RequestView, on_respond }` **replaces the composer input**
  while requests are pending (UX lesson 2). It has a counter "1 of 3" and supports:
  * Approval: subject rendering (command in a code block, file paths, tool plus input
    preview, reason). There is one button per provider option, ordered as delivered,
    with the decision tone (deny = destructive). Keyboard: 1..9 select. AllowAlways
    options show the warning text.
  * Questions: a stepper with one question per step, options as radio or checkbox,
    custom text input when allowed, and back/next/submit.
  * PlanApproval: "Implement plan" (approve) or "Refine" (feedback textarea → reject
    with feedback).
  * Elicitation: render a JSON-schema form for simple object schemas (string,
    number, bool, enum). Otherwise show "Open in browser" for URL mode or decline.
  * Non-live requests (expired or not resumable) show an explanation and a
    "dismiss" action.
* Accessibility: the timeline uses `role="log"` + `aria-live="polite"` on the live tail
  only; buttons are labelled; focus moves to the request panel when it appears.

## agents/thread/

* History states (K12):
  * "Load earlier" at the top when `view.before` is set, calling `prepend_history`;
  * a subtle banner when the history is `Partial`/`Unavailable` ("Conversation history
    is stored by <provider> and could not be loaded: <reason>"), while the run list
    (from metadata: `input_summary`, status, `preview`) still renders as a compact
    fallback timeline;
  * turns without an aligned run (continued outside Semantic) render normally with
    an "external" marker.
* `ThreadPane { thread_id }`:
  * header: title (inline rename), workspace and branch, provider and model badge,
    connection indicator;
  * menu: archive, delete, settle, pin, copy id;
  * `Timeline`; banners (thread error, usage limit with `reset_at`, provider
    unavailable, session not resumable);
  * queue strip: queued runs with edit, cancel, reorder (buttons up and down; drag
    later), "promote to steer";
  * `RequestPanel` or `Composer`.
* Composer controls (agents layer):
  * `ModelPicker`: instances grouped by provider, models with search; option
    descriptors rendered generically, with `Select` → dxcomp Select and `Toggle` →
    Switch;
  * `AccessPicker`: four levels with one-line descriptions, and modes the instance
    does not support disabled with a tooltip;
  * `PlanToggle` (InteractionMode);
  * a context meter when the usage has a context window.
  Changes apply via `update_thread` (between runs) or as `overrides` on send. The UI
  states when a change applies on the next turn (`ConfigSwitch::NextTurn`).
* Send semantics:
  * `SendMode::Auto` when idle;
  * the default follow-up action while running comes from a setting (default
    `steer`, falling back to `queue` when capabilities say steering is unsupported);
  * the button label reflects the effective action.
* Interrupt shows "Stopping…" until the run reports interrupted (UX lesson 6).
* Visiting: call `visit` when the pane mounts and when new events arrive while
  visible.
* Draft persistence per thread (store/drafts). The navigation guard is not needed;
  drafts persist.

## Tests

* `derive_timeline_rows` table tests:
  * simple Q&A;
  * a completed run with tools folded;
  * active run expanded with a live row;
  * failed run expanded;
  * steer message inside a run;
  * subagent;
  * plan plus todo;
  * error classes;
  * a grouping sentence for every category mix;
  * row key stability across streaming updates (the same keys and order when only
    text grows).
* `ComposerAction` resolution table (running, empty or not, Mod, capability).
* RequestPanel logic: answer validation mirrors `RequestBody::validate_response`;
  stepper state machine.
* SSR snapshots of each row kind and each request kind with fixture data.
* Mock-source integration: scripted `/approve` flow; the composer is replaced by the
  request panel, responding restores the composer, and the respond payload is
  captured.
* wasm check.

## Acceptance

* Tests green; wasm check green. `chat/` has no imports from `agents/`,
  `semantic_agent_domain` or `source/` (check with grep in a test or review).
* One commit: "Add AI UI chat components and agent thread pane".
