# Step 12: AI UI foundation (`semantic_ai_ui`)

Wave W3. Depends on: 01, 06. Agent: opus. Runs in parallel with steps 03, 04, 05, 07 and 09.
Uses a mock source only; no running backend is needed.

Read: plan.md (crate map, the UI-related K6 and K8 items), steps/06 (API, ThreadView,
status), research/t3code-ui.md §4 (client data flow), §7.1 (streaming and markdown
caching), §17 (UX lessons 15, 18), §18; research/semantic-app-ui-layers.md §5 (UI
layers, component inventory, gaps, styling, patterns), `crates/ui_core/src/graph/`
(the reusable-view-with-source-trait precedent), `crates/ui_core/src/components/comments.rs`,
`crates/dxeditor/src/{markdown,component}.rs` (`DocumentView`, `parse_markdown`).

## Goal

The reusable base of the AI UI crate:

1. **data access**: the `AgentsSource` trait, its RPC implementation and a mock;
2. **client stores and hooks**: thread view (snapshot plus cursor, reconnect),
   thread list, providers;
3. **widgets**: generic, unit-tested building blocks that the workspace does not have
   yet (streaming markdown, diff view, ANSI text, chat scroll container, code block,
   elapsed timer);
4. **crate conventions**: styling, navigation and platform abstraction, a test harness.

Steps 13 and 14 build the actual screens on top of this.

## Structure

```
crates/ai_ui/src/
  lib.rs                 pub mod chat, agents, widgets, source, store, platform, nav; Stylesheet component
  source/
    mod.rs               AgentsSource trait (LocalBoxFuture / LocalBoxStream; wasm-friendly, not Send)
    rpc.rs               RpcAgentsSource over semantic_rpc::RpcClient (invoke::<C>, invoke_stream::<C>)
    mock.rs              MockAgentsSource: in-memory scripted backend implementing the ThreadView semantics (for tests + demos)
  store/
    thread.rs            use_thread(source, scope, thread_id) -> ThreadHandle
    list.rs              use_thread_list(source, scope, filter) -> ThreadListHandle
    providers.rs         use_providers(source, scope) -> ProvidersHandle
    connection.rs        ConnState {Live, Connecting{attempt}, Offline{since}, Resyncing}; jittered backoff (500 ms .. 15 s)
    drafts.rs            per-thread draft persistence (localStorage on web, in-memory on desktop; behind platform trait)
  widgets/
    markdown.rs          StreamingMarkdown { text, streaming } : block-level cache (closed blocks parsed once), dxeditor::DocumentView render
    code.rs              CodeBlock (monospace, copy button, language label; highlighting later)
    diff.rs              DiffView { files: Vec<semantic_agent::diff::DiffFile>, mode: Unified } : file list + collapsible files + hunks with line numbers
    ansi.rs              parse_ansi(&str) -> Vec<StyledSpan> (SGR colors/bold/underline; strips other escapes) + AnsiText component
    scroll.rs            ChatScroll: bottom-anchored container with follow/free-scroll detection and "jump to latest" button (minimal document::eval glue)
    timer.rs             ElapsedTimer { since } : updates once per second via dioxus_sdk_time without re-rendering parents
    status.rs            StatusPill { attention } : maps semantic_agent_domain::status::Attention to label/tone
  platform.rs            AiUiPlatform trait: notify(title, body, thread), request_notification_permission, open_path_in_editor, pick_directory;
                         NoopPlatform; context provider + hook
  nav.rs                 AgentsNavigation context: open_thread(id), open_new_thread(workspace?), href_for_thread(id) (host app supplies; ui crate maps to routes)
  styles/ai_ui.css       all styles, `semantic-ai-*` BEM classes using semantic tokens (--semantic-color-*, --semantic-space-*, ...);
                         light/dark via existing tokens only; no hard-coded colors
```

## AgentsSource

It mirrors the API from step 06 one to one, with typed DTOs:

```rust
pub trait AgentsSource: 'static {
    fn list_threads(&self, q: ThreadListPayload) -> LocalBoxFuture<'_, Result<ThreadPage, SourceError>>;
    fn get_thread(&self, q: ThreadGetPayload) -> LocalBoxFuture<'_, Result<ThreadSnapshot, SourceError>>;
    fn history(&self, q: ThreadHistoryPayload) -> LocalBoxFuture<'_, Result<TranscriptPage, SourceError>>;   // older provider history (K12)
    fn watch_thread(&self, q: WatchThreadPayload) -> LocalBoxFuture<'_, Result<LocalBoxStream<'static, Result<ThreadStreamItem, SourceError>>, SourceError>>;
    fn watch_list(&self, q: WatchListPayload) -> LocalBoxFuture<'_, Result<LocalBoxStream<'static, Result<ThreadListStreamItem, SourceError>>, SourceError>>;
    fn create_thread(..); fn update_thread(..); fn delete_thread(..); fn visit(..);
    fn send(..); fn interrupt(..); fn queue(..); fn respond(..); fn diff(..); fn revert(..);
    fn providers(..); fn refresh_provider(..); fn upsert_provider(..); fn workspaces(..); fn create_workspace(..);
    fn branches(..); fn links(..); fn runs(..); fn import_thread(..);
}
```

`SourceError { code: Option<String>, message: String, transient: bool }` is mapped from
`RpcClientError`. The `transient` flag drives reconnect.

## use_thread (normative)

1. Load `get_thread(max_turns = 20)` and build `ThreadView::from_snapshot`. The
   snapshot is composed server-side from provider history plus the live overlay
   (K12) and carries the live cursor, if any.
2. Start `watch_thread(cursor = view.cursor)` and apply events.
3. On stream error, or end without `WatchEnd`: set `ConnState::Connecting`, back off,
   and resubscribe with the current cursor. On `Reset`, or a `ViewError`
   (epoch mismatch or gap): reload the snapshot (step 1) while keeping scroll position
   and drafts.
4. The handle exposes:
   * `view: ReadSignal<ThreadView>` (coarse);
   * fine-grained signals per item key for streaming rows. `ViewChange` drives a
     `Store`-like map of `Signal<ItemView>`, so only the streaming row re-renders;
   * `conn: ReadSignal<ConnState>`;
   * `load_earlier()`, which calls `history(before)` and `ThreadView::prepend_history`;
   * `history: ReadSignal<HistoryAvailability>`.
5. Dropping the hook cancels the stream (drop of the stream future).

Keep the reducer logic in `semantic_agent_domain::view` and only add Dioxus glue here.
Any non-trivial client-only logic goes in pure functions with unit tests.

## Widgets details

* `StreamingMarkdown`:
  * split text into top-level blocks with a lightweight scanner (fences, blank-line
    separated paragraphs);
  * cache parsed `EditorDocument`s for closed blocks keyed by `(block index, hash)`;
  * re-parse only the trailing open block while streaming;
  * escape raw HTML: confirm that `parse_markdown` renders `raw_html` inertly, and
    sanitize if needed;
  * links open externally or via the platform.
* `DiffView`:
  * file header with status icon, path (rename arrow) and +/- counts;
  * collapsed by default above 500 changed lines;
  * hunks rendered as a table with old/new line numbers;
  * no syntax highlighting in v1;
  * "load more" for files beyond the first 50;
  * the `on_line_select` callback prop is reserved for the review-comment feature
    (later); render a data attribute only.
* `parse_ansi`: SGR 0-1, 3-4, 22-24, 30-37, 39-47, 49, 90-97, 100-107, 38/48;5;n and
  38/48;2;r;g;b are mapped to a small palette class set. Other CSI/OSC sequences are
  stripped. Exhaustive unit tests.
* `ChatScroll` auto-follows when within 48 px of the bottom. User scroll-up sets free
  mode. "Jump to latest" appears in free mode. Measurement uses `onscroll` events and
  `document::eval` only for `scrollTo`. Unit-test the pure mode state machine.

## Tests

* Pure tests: ANSI parser, markdown block splitter and cache invalidation, the scroll
  mode machine, the backoff schedule, connection-state transitions.
* `MockAgentsSource` tests:
  * scripted events reconstruct the expected `ThreadView`;
  * disconnect mid-stream followed by resubscribe with a cursor yields no duplicate
    text (offset idempotency);
  * `Reset` triggers a snapshot reload;
  * "load earlier" prepends history without duplicates;
  * the history-unavailable state.
* SSR tests (`VirtualDom` + `dioxus_ssr`) for `DiffView` (sample patch),
  `StreamingMarkdown` (code fence plus list), `AnsiText` and `StatusPill`.
* `use_thread` loader test with the mock source (pattern: `crates/ui/src/views/labels/mod.rs` tests).
* wasm check:
  `nix develop -c cargo check -p semantic_ai_ui --target wasm32-unknown-unknown --features web`.

## Acceptance

* Tests green, wasm check green. Widgets have no dependency on agent types, except
  `DiffView` (on `semantic_agent::diff`) and `StatusPill`.
* One commit: "Add semantic_ai_ui foundation: sources, stores, widgets".
