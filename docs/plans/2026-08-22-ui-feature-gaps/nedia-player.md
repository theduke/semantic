# Media player (`/play`) implementation plan

## Goal

Reimplement the old Semantic `/play` page in the current Dioxus UI, preserving its useful behavior while correcting the incomplete media-control contract in the new rendering system.

The page must support:

- Image, audio, and video playback.
- Play/pause, previous/next, mute, cycle, shuffle, fullscreen, and a configurable image-slide interval.
- Progress and duration for timed media.
- Loading a queue from structured filters or SQL, with explicit **Replace** and **Add** actions.
- Optional recursive expansion of container entities to media descendants.
- Opening the active entity and its normal entity actions.
- Keyboard controls.
- A toggleable playlist that remains usable with tens of thousands of entries. It must use virtual scrolling and must not render the whole queue into the DOM.

The implementation should use the current generic `semantic:filestore:file` plus `mime_type` model. It must not assume the old dedicated `semantic/image`, `semantic/audio`, and `semantic/video` classes still exist.

## What the old page did

The old implementation is in:

- `../semantic-2/js/ui/src/component/play/PlayPage.tsx`
- `../semantic-2/js/ui/src/component/play/Player.tsx`
- `../semantic-2/js/ui/src/component/play/PlayerBar.tsx`
- `../semantic-2/js/ui/src/semantic/base/image.tsx`
- `../semantic-2/js/ui/src/semantic/base/audio.tsx`
- `../semantic-2/js/ui/src/semantic/base/video.tsx`
- `../semantic-2/js/ui/src/component/entity/filter/*`

Its effective feature set was:

- The initial filter selected image, audio, and video entities, with a nominal limit of 100,000.
- A filter panel supported a structured search/type/tag/sort form and raw SQL.
- Results could replace the queue or be appended to it.
- “Expand to media items” recursively followed `semantic/parent` from non-media containers and deduplicated media IDs.
- The player rendered media through the old UI registry and rendered a generic entity preview for unsupported objects.
- Audio and video advanced on `ended`; images and generic previews advanced on a five-second timer.
- The top bar exposed filter, mute, cycle, shuffle, fullscreen, previous, play/pause, next, position/count, and the active title.
- Clicking the title opened an editable entity modal. Playback paused while it was open and resumed afterward if it had previously been playing.
- Keyboard controls were Left, Right, Space, `f`, and Escape.

The plan should preserve the intended behavior, not its bugs. In particular:

- The old mute flag was not propagated to the active media handle.
- The old audio/video renderers did not emit pause, resume, duration, or progress events even though the API declared them.
- The old `showControls` option was ignored.
- Pressing play on a selected image skipped immediately instead of timing the displayed image.
- Non-cycling navigation could calculate an out-of-range item.
- Recursive expansion did not track visited container IDs, so relation cycles could recurse forever.
- Failed media was silently skipped.
- A 100,000-object result was retained and rendered without a scalable playlist UI.

## Current implementation constraints

The relevant new code is:

- Routes and shell: `crates/ui/src/views/mod.rs`, `crates/ui/src/components/shell.rs`
- Existing query pattern: `crates/ui/src/views/browse.rs`
- Catalog setup: `crates/ui/src/app.rs`
- Media lookup/contracts: `crates/ui_core/src/ui_catalog/media.rs`
- Generic media component: `crates/ui_core/src/components/media.rs`
- Entity rendering/actions: `crates/ui_core/src/components/entity/card.rs`
- File rendering and URLs: `crates/ui_core/src/ui_catalog/defaults.rs`
- File schema: `crates/data/src/filestore.rs`
- Directory relationship semantics: `crates/data/src/bundles/directory.rs` and `crates/ui_core/src/components/directory_browser/queries.rs`
- Query RPC: `semantic.db.query` in `crates/app/src/command.rs`
- Virtual list: `crates/dxcomp/src/components/virtual_list/component.rs`

The current catalog has promising but incomplete media abstractions:

- `MediaKind`, `MediaRenderOptions`, `MediaRenderEvent`, `MediaHandle`, and `MediaRendererRegistration` exist.
- `MediaView` can find a registered renderer.
- `MediaRendererRegistration` only returns an `Element`; it cannot publish a handle or events to a player.
- `MediaRenderEvent` lacks duration and progress variants and is not wired to a renderer callback.
- No production media renderers are currently registered.
- `MediaHandle` has no production implementation.
- `media_kind_for_object` checks `mime` and `content_type`, while new file entities normally use `mime_type` or its qualified attribute ID.
- Current default file rendering only embeds images; audio and video are links.

The `/play` work must therefore include a small, reusable playback layer in `ui_core`. It should not put raw audio/video DOM manipulation directly into the page reducer.

## Recommended architecture

Split the implementation into four layers:

1. **Catalog media playback contract (`ui_core`)**
   - Resolve an entity to a media kind and a playback renderer.
   - Render the active object.
   - Register/unregister its `MediaHandle`.
   - Emit typed lifecycle/playback events carrying a session token.

2. **Playlist data source (`ui`)**
   - Turn a filter/query into stable, lightweight queue entries.
   - Page large results and optionally expand containers.
   - Load the full object only when an entry becomes active or is opened.

3. **Pure player state machine (`ui`)**
   - Own playback intent, active position, queue order, progress, timer state, and errors.
   - Convert UI actions and media events into deterministic state transitions.
   - Reject stale events from previously rendered items.

4. **Dioxus page/components (`ui`)**
   - Render the stage, controls, filter drawer, entity dialog, and virtual playlist.
   - Bridge global keyboard/fullscreen events.
   - Forward user actions and renderer events to the state machine.

Keep query/data code and the reducer independent of Dioxus components where practical. This is important because most queue, cycling, stale-event, and failure behavior can then be unit-tested without a browser.

## Proposed modules and files

Add:

```text
crates/ui/src/views/play/
  mod.rs                 # PlayPage and route-facing composition
  controller.rs          # reducer/actions/effects orchestration
  state.rs               # pure state types and transitions
  data.rs                # RPC query/page/get and expansion client
  query.rs               # structured filter -> stable query/page specification
  stage.rs               # active media/entity stage
  controls.rs            # transport/status toolbar
  playlist.rs            # toggleable virtual playlist
  filters.rs             # filter/query drawer
  entity_dialog.rs       # active entity detail/actions dialog
  browser_bridge.rs      # keyboard/fullscreen/scroll bridge
```

Add or extend:

```text
crates/ui_core/src/ui_catalog/media.rs
crates/ui_core/src/components/media.rs
crates/ui_core/src/components/media/
  image.rs
  audio.rs
  video.rs
crates/ui/src/views/mod.rs
crates/ui/src/components/shell.rs
crates/ui/src/app.rs
crates/ui/assets/core_styles.css
```

If paged query support is added as recommended below:

```text
crates/app/src/command.rs              # register typed page command
crates/app/src/query_page.rs           # parse, validate, page, and count SELECTs
```

Do not duplicate generic SQL invocation helpers indefinitely. After the player works, the identical `semantic.db.query` decoding in Browse, Query, directory browsing, and Play should be consolidated into a small `ui_core` RPC helper in a separate cleanup change.

## Media playback contract

### Preserve the existing catalog API additively

Avoid changing `MediaRendererRegistration.renderer` to return a different type. It is a public struct and an aggressive replacement would force all render-only integrations into player lifecycle semantics.

Add a parallel registration type, for example:

```rust
pub struct MediaPlaybackRendererRegistration {
    pub name: String,
    pub class_id: Option<String>,
    pub media_kind: MediaKind,
    pub renderer: Rc<dyn Fn(Object, MediaPlaybackRenderOptions) -> Element>,
}
```

`MediaPlaybackRenderOptions` should contain:

- `session_id: PlaybackSessionId`
- `playing: bool`
- `muted: bool`
- `controls: bool`
- `on_handle: EventHandler<MediaHandleRegistration>`
- `on_event: EventHandler<MediaPlaybackEvent>`

Use a new `MediaPlaybackEvent` rather than silently repurposing the currently unused and underspecified `MediaRenderEvent`. Events should include:

- `Mounted`
- `Loaded { duration_seconds: Option<f64> }`
- `Playing`
- `Paused`
- `Progress { current_seconds: f64, duration_seconds: Option<f64> }`
- `Finished`
- `Failed { message: String, fatal: bool }`
- `PlayRejected { message: String }` for autoplay/user-gesture failures

Every event and handle registration must carry the session ID. A renderer must emit an unregister event or its handle token must be invalidated on unmount.

Use `Rc<dyn MediaHandle>` only for the currently mounted active item. A handle should support at least:

- `play()`
- `pause()`
- `set_muted(bool)`
- `seek(seconds)` if the progress control is seekable

The existing `MediaHandle` can be retained for compatibility and extended only if a repository-wide search confirms no external construction constraints. Otherwise introduce `PlaybackMediaHandle` and adapt old handles. Playback command failures must come back through an event; a fire-and-forget `play()` must not leave the UI claiming playback started when the browser rejected it.

### Renderer lookup

Playback lookup should use the same precedence as current media lookup:

1. Exact class registration.
2. Inherited class registration.
3. MIME-derived fallback registration.
4. Static generic entity rendering.

Fix MIME extraction to recognize, in order:

- `mime_type`
- `semantic:filestore:file:mime_type`
- `mime`
- `content_type`

Recognize `image/*`, `audio/*`, and `video/*`. Treat other files as `File`, not as playable timed media.

Resolve local file URLs using the active `RpcClient::file_url(id)` when available, falling back to `RenderSettings.file_api_prefix/{id}`. This keeps both the HTTP/web target and the desktop `semantic-file://localhost` scheme working. Remote URLs may be supported only when the object contains a validated `http://` or `https://` URL; do not infer a local filesystem path into an HTML `src`.

### Standard renderers

Register standard image/audio/video playback renderers in `configure_ui_catalog` or in a dedicated default-registration function called from it.

Image renderer:

- Render a single `<img>` with `object-fit: contain`, bounded by the stage.
- Emit `Loaded` only from the image load event; the slideshow timer starts after this.
- Emit a visible failure on image error.
- Use title/filename as alt text; fall back to an informative `"Media item <id>"` string.
- It has no timed-media handle.

Audio renderer:

- Render `<audio>` with native controls according to `controls`.
- Emit `Loaded`/duration from `loadedmetadata` or `durationchange`.
- Emit progress from `timeupdate`, throttled to roughly 4–10 updates/second if browser frequency is excessive.
- Emit playing/paused/ended/error events.
- Apply `muted` before trying autoplay.
- Provide a handle backed by a stable DOM element ID.
- Show a poster/metadata treatment in the stage rather than leaving a tiny audio control floating without context.

Video renderer:

- Same lifecycle semantics as audio.
- Render the video with `max-width/max-height: 100%` and `object-fit: contain`.
- Use an available preview image as `poster` when the schema gains or exposes one; otherwise omit it.

The renderer should be keyed by `session_id` so switching items creates a fresh DOM element. It must unregister the old handle during cleanup.

### DOM control implementation

Both Dioxus web and desktop render into a browser/WebView DOM. A small internal `DomMediaHandle` may invoke `play()`, `pause()`, seek, and mute through `document::eval` using a generated element ID. Keep the script fixed and pass IDs/values through the eval channel; do not interpolate entity data into JavaScript.

An alternative is a child command channel whose component reacts declaratively to option changes. Either implementation is acceptable, but it must satisfy these rules:

- The controller never retains a handle after the renderer unmounts.
- Pause the old handle before invalidating/removing it.
- A play promise rejection emits `PlayRejected`.
- The native media events, not the command call itself, are authoritative for `Playing` and `Paused`.
- Changing `muted` updates the mounted element immediately and is also applied to the next item before playback.

## Playlist data model and scaling

### Lightweight entries

Do not keep tens of thousands of full `Object` values solely to render playlist rows. Use a compact queue summary:

```rust
struct QueueEntry {
    occurrence_id: QueueOccurrenceId,
    target: EntityTarget,
    title: String,
    class_id: Option<String>,
    media_kind: MediaKind,
    mime_type: Option<String>,
    known_duration_seconds: Option<f64>,
}
```

`occurrence_id` must be unique per queue insertion, even if the same entity appears twice after **Add**. Playback identity is the occurrence, while object loading uses `target`.

Store full active objects in a small LRU cache keyed by `EntityTarget` (for example, current plus the previous and next few items). Load the selected object with `semantic.db.get`. Do not retain rendered `Element`s in state.

### Paged source

The preferred scalable design is a paged queue source rather than the old single 100,000-object query.

Add a read-only paged SELECT command or equivalent typed service with this behavior:

```text
input:
  scope_id
  query + format (or a typed SelectQuery)
  offset
  limit
  include_total

output:
  rows
  total (when computable)
  has_more
```

The command must:

- Parse and require a SELECT; reject mutation/DDL statements.
- Apply page offset/limit to the parsed query AST, not by string concatenation.
- Preserve any limit/offset in the original filter as source bounds.
- Return `limit + 1` internally when exact total is unavailable so `has_more` is reliable.
- Compute an exact total for the simple, ungrouped structured queries generated by this page.
- Reject or clearly mark grouped/distinct/raw SQL forms for which a scalable exact count cannot be produced. Do not silently materialize an arbitrary huge raw query just to count it.
- Require deterministic ordering for large paging. Generated queries should end with `id ASC` as a tie-breaker. Raw SQL without deterministic ordering should show a validation error or explicit warning before load.

Suggested page size is 500 summaries. Cap concurrent page requests (for example, two) and deduplicate requests for the same source generation/page.

If adding an RPC command is deferred, an interim version may issue generated `semantic.db.query` requests with explicit `LIMIT/OFFSET` plus a count query. Raw SQL should then be limited to user-supplied paginated SQL and documented as non-scalable. Do not call the fallback complete parity.

### Queue source segments

Represent each **Replace**/**Add** result as a source segment:

```rust
struct QueueSegment {
    id: SegmentId,
    source: PlaylistQuery,
    total: Option<usize>,
    pages: BTreeMap<usize, PageState<Vec<QueueEntry>>>,
    generation: u64,
}
```

Benefits:

- **Add** does not have to merge SQL strings.
- A failed appended source can be retried or removed without destroying the existing queue.
- Global queue index maps through prefix sums to `(segment, local_index)`.
- Replacement can atomically swap generations and make stale results harmless.

When exact totals are known, expose that full count to the virtual list and render unloaded rows as lightweight placeholders that request their containing page on mount. This allows the scrollbar to represent all 50,000 items immediately. When total is unknown, expose loaded rows plus a final loading sentinel until `has_more == false`.

Prefetch:

- The page containing the active item.
- The previous and next queue pages.
- The pages containing the next two playback items, especially in shuffled order.

### Materialized ordering and shuffle

Keep normal order implicit through segments and local indexes. For shuffle, create an explicit `Vec<QueueAddress>` only after queue summaries are materialized. A vector of addresses for tens of thousands of entries is small and avoids copying objects.

When the user requests shuffle:

1. Keep playback and current order usable.
2. Load any missing summary pages with bounded concurrency and visible `"Preparing shuffle…"` progress.
3. Fisher–Yates shuffle queue addresses.
4. Keep the currently active occurrence active; place it at the current logical position or reset to the first item according to the product decision below.
5. Atomically install the new ordering only after materialization succeeds.

Do not shuffle only the loaded prefix and claim the queue was shuffled.

For tens of thousands of entries this O(n) operation and memory cost are acceptable; the expensive part to avoid is full object loading and DOM construction.

### Virtual playlist

Use `dxcomp::VirtualList`, as already used by `DirectoryList` and `DirectoryGrid`.

- Give rows a fixed height (target 56 px) so the estimate is exact and jumping is stable.
- Pass the total logical queue count through a signal.
- Render only title, ordinal, media-kind icon/text, duration, load/error indicator, and active state.
- Key the row content by `occurrence_id`, not merely entity ID.
- Keep the primitive’s `role=list`, `aria-setsize`, and `aria-posinset`; make row selection a real button or keyboard-focusable option.
- Use `aria-current="true"` on the active row and expose `"Play <title>"` as its accessible action.

The current `dxcomp::VirtualList` has no public scroll-to-index controller. Because playlist rows are fixed-height, add a narrowly scoped browser bridge that sets the list container’s `scrollTop` to `index * ROW_HEIGHT`. Give the list an explicit generated DOM ID and send ID/index as eval values. Longer term, promote this into a reusable `VirtualList` controller in `dxcomp`.

Do not forcibly yank the user back to the active row while they are manually browsing the playlist. Track a “follow active item” toggle or suspend automatic following after manual scroll; provide a **Current** button to jump back.

Playlist visibility:

- Add a toolbar toggle with `aria-expanded` and `aria-controls`.
- Persist the preference in local storage under a versioned key.
- Desktop: a resizable or fixed-width side panel within the player layout.
- Narrow screens: a bottom panel/sheet that leaves controls reachable.
- Fullscreen: keep the playlist inside the fullscreen root so it can still be toggled.
- Toggling the panel must not remount or pause the active media element.

Playlist v1 needs selection and active highlighting. Clear/remove/reorder controls are useful follow-ups, but should not complicate the first parity implementation.

## Filter, load, append, and replace behavior

### Filter UI

The `/play` page should not wait for the full structured Browse-filter gap to be solved. Build a small reusable `PlaylistFilter` model that can later be shared:

- Collection, defaulting to the active/default entity collection.
- Search term over title/name/id.
- Media kinds: image/audio/video, all selected by default.
- Optional class filters derived from `UiCatalog`.
- Sort attribute/order, defaulting to title then ID or ID alone.
- Advanced SQL mode.
- `expand_to_media` boolean.

Keep the draft filter separate from the applied source. Editing controls must not mutate the playing queue until **Replace** or **Add** is pressed.

Use explicit buttons:

- **Replace**: query the first page, then atomically replace the queue if it succeeds.
- **Add**: append a new segment if its first page succeeds.
- **Cancel**: close the filter drawer without changing the applied queue.

Disable duplicate submissions while a first-page request is in flight. Show the request error inside the filter panel and retain the draft for correction/retry.

On successful Replace:

- Increment the queue generation.
- Pause and invalidate the old active renderer.
- Cancel/ignore old page and object requests.
- Select index zero when non-empty.
- Preserve the prior playback intent. If it was playing, start the first new item only after its object/renderer is ready.
- Enter the empty state when there are no results.

On successful Add:

- Leave the current item, progress, and playback intent unchanged.
- Append the segment and update the virtual count.
- If the old queue was empty, select the first appended item and honor the current playback intent.

### Relation expansion

The new repository models directory membership using `semantic:base:directory_node` relation entities, not the old page’s direct-only `semantic:parent` assumption. Implement expansion against an explicit convention rather than guessing every ref attribute.

For v1, define “expand to media” as:

- Direct media/file results remain queue entries.
- A result with a playable MIME type is not expanded further.
- Directory/container results are traversed through `DIRECTORY_NODE_RELATION_ID` in `directory_node.order`, title, and ID order.
- Traverse recursively, breadth-first, with paged child queries.
- Track visited container targets to terminate cycles.
- Track emitted media targets to deduplicate the same media reachable through multiple paths, matching the old page’s intent.
- Enforce a configurable maximum depth and maximum expanded result count. Reaching either limit returns a visible partial-result warning rather than hanging.
- Propagate cancellation/generation checks through every expansion request.

The existing `child_query` and relation constants in the directory browser are useful references, but player expansion logic should be extracted into a reusable public query/data helper instead of importing private directory-browser functions.

Decide before implementation whether to also support the legacy `semantic:parent` attribute. If it is needed for migrated data, add it as an explicit second expansion provider with tests; do not mix two conventions implicitly and produce duplicates or unstable order.

For very large recursive trees, a backend `semantic.media.expand_page`/playlist-source command is preferable to thousands of client-side RPC calls. Start with client BFS only if representative datasets prove request counts acceptable.

## Player state model

Use a reducer or controller coroutine with explicit commands. Avoid independent signals that can transiently disagree.

Core state:

```rust
struct PlayerState {
    queue: Playlist,
    queue_generation: u64,
    order: QueueOrder,
    active_index: Option<usize>,
    active_occurrence: Option<QueueOccurrenceId>,
    playback_session: u64,
    playback_intent: PlaybackIntent, // Playing | Paused
    observed_media_state: ObservedMediaState,
    muted: bool,
    cycle: bool,
    image_interval: Option<Duration>,
    progress: PlaybackProgress,
    slide_timer: SlideTimerState,
    active_handle: Option<RegisteredHandle>,
    active_load: ActiveObjectLoadState,
    playlist_open: bool,
    filter_open: bool,
    entity_dialog: EntityDialogState,
    fullscreen: FullscreenState,
    transient_error: Option<PlayerError>,
    consecutive_failures: usize,
}
```

Commands should include:

- `LoadReplace(filter)` / `LoadAppend(filter)` / page load results
- `Select(index)`
- `Play`, `Pause`, `TogglePlay`
- `Next { reason }`, `Previous`, `SetCycle`, `SetMuted`
- `Shuffle`
- `SetImageInterval`
- `HandleRegistered`, `HandleUnregistered`
- `MediaEvent { session_id, occurrence_id, event }`
- `ImageTimerElapsed { session_id }`
- `OpenEntityDialog`, `CloseEntityDialog`
- `TogglePlaylist`, `ToggleFilter`
- `RequestFullscreen`, `FullscreenChanged`
- `RetryActive`, `SkipFailed`

### Item switching protocol

Switching to another item must be ordered:

1. Cancel the image timer.
2. Pause the current handle.
3. Increment `playback_session` immediately.
4. Clear the old handle/progress/duration/error.
5. Set the target occurrence and active-object state to loading.
6. Load the object if absent from cache.
7. Render the object keyed by the new session.
8. Accept a handle only if its session and occurrence still match.
9. Apply mute.
10. If playback intent is Playing, call play after `Mounted/Loaded`; for an image/static slide, start its timer after load.

All async results and renderer events must contain the queue generation/session they were started for. Ignore mismatches. This prevents late `pause`, `ended`, `timeupdate`, page-load, or object-load events from changing the new item.

### Play/pause semantics

Separate **intent** from observed browser state:

- Pressing Play sets intent to Playing.
- Audio/video becomes observed Playing only after its native `playing`/`play` event.
- Pressing Pause sets intent to Paused, cancels slide timers, and pauses the current handle.
- A native pause caused by the user updates intent to Paused.
- A stale pause caused by unmount/switch is ignored by session ID.
- A browser autoplay rejection sets intent to Paused and presents a clear “Press play to continue” message; it is not treated as a corrupt file and must not auto-skip.

For images/static slides:

- Play starts or resumes the countdown for the currently displayed item; it never skips that item immediately.
- Pause stores remaining time, and resume continues from it.
- Changing the interval recomputes remaining time conservatively (cap remaining to the new interval).
- `None` means manual advancement.
- Timer completion sends a session-tagged event and advances only if intent is still Playing.

For audio/video:

- Advance on `Finished` only.
- A displayed image duration must not overwrite known audio/video duration metadata.
- Progress is current seconds plus optional duration; derive percent only when duration is finite and positive.

### Boundaries and cycle

- Previous is disabled at index zero when cycle is off; Next is disabled at the last index when cycle is off.
- With cycle on, Previous at zero wraps to the final item and Next/Finished at the final item wraps to zero.
- With cycle off, finishing the last item changes intent to Paused and leaves the final item selected.
- Empty queue actions are no-ops.
- A one-item cyclic queue should not remount forever on immediate failure. Failure-loop protection applies.

### Failures

Show active-item failures on the stage and mark the playlist row.

- Provide **Retry** and **Skip**.
- Network/decode failures may auto-skip only while playback intent is Playing and only if the user-enabled/default policy permits it.
- Stop automatic advancement after one full queue traversal or a small consecutive-failure threshold to avoid an infinite failure loop.
- Do not clear a queue-level filter/page error when an unrelated media event occurs.
- Log diagnostic details with entity ID, MIME, session, and error, but do not expose local paths or sensitive RPC payloads in the UI.

## Stage and fallback rendering

`PlayerStage` receives only the active object and current playback options.

- If a playback renderer exists, use it.
- If the item is an image, audio, or video that lacks a usable source, render an actionable missing-source error.
- For a non-playable object intentionally supplied by raw SQL, render `EntityCard`/`ClassView` in preview/detail style as a static slide and use the image interval.
- Show a spinner/skeleton while the active object loads.
- Keep the previous image/media off the DOM once switching begins; do not overlap two audio elements.
- Use a stable black/dark neutral stage surface independent of the normal card background, while controls continue to use repository theme variables.
- Constrain all media to the stage; no intrinsic media dimension may overflow the player.

The stage should expose a live but non-chattering status message for screen readers: loading, playing title, paused title, finished, or failed. Do not announce every `timeupdate`.

## Controls and status

The primary transport toolbar should contain:

- Filter drawer toggle.
- Playlist toggle.
- Mute/unmute.
- Cycle on/off.
- Shuffle.
- Fullscreen.
- Previous.
- Play/pause.
- Next.
- Current position and total (`1 / 42`, or loaded/unknown total while counting).
- Current title button.
- Elapsed/duration display and a progress bar for timed media.
- Image interval selector.

Use `dxcomp::Button`, `Toolbar`, `Tooltip`, and the existing Lucide icon dependency where suitable. Icon-only buttons need `title` and `aria-label`; toggles need `aria-pressed`.

If progress is seekable:

- Use an accessible range input with current/duration labels.
- Seek only on committed input or throttle change events.
- Disable it until duration is finite and the active handle supports seeking.
- Keep keyboard Left/Right reserved for item navigation unless the range input itself has focus.

The title button opens the entity dialog and truncates visually without losing its accessible full label.

## Entity dialog and actions

Use `dxcomp::Dialog` and the current catalog rendering system.

- Pause on open and remember whether playback intent was Playing.
- Load/use the current full object.
- Render an `EntityCard` with `preview: false`, `actions: true`, or extract a reusable detail body that preserves registered Open/Edit/Delete/External actions.
- If deletion succeeds, remove/invalidate the queue occurrence and select a valid neighbor.
- If the object was edited through a route/action, refresh it when the player regains focus or when an explicit refresh event is available.
- On close, resume only if playback was playing before open, the user did not explicitly pause in the meantime, the active occurrence has not changed, and no fatal error occurred.
- Suppress global player keyboard shortcuts while the dialog is open.

The old modal embedded an editable renderer. The current UI uses edit routes. For v1, preserving normal registered entity actions in the dialog is preferable to duplicating form submission inside `/play`. Inline editing can be added later by extracting the existing form page body.

## Keyboard and fullscreen bridge

Install document-level listeners in a `use_effect` and always remove them on cleanup. Use `document::eval` with a receive loop if Dioxus does not expose the required global event directly.

Shortcuts:

- Left: previous item.
- Right: next item.
- Space: play/pause.
- `m`: mute/unmute (addition, discoverable in tooltip/help).
- `f`: enter/exit fullscreen.
- Escape: let an open dialog/drawer close first; otherwise exit fullscreen.

Rules:

- Ignore shortcuts when the event target is an input, textarea, select, button with its own Space behavior, contenteditable region, or range control.
- Ignore transport shortcuts while the filter or entity modal has focus/open.
- Call `preventDefault` for handled Space/arrow events to prevent page scrolling.
- Do not trigger when Ctrl/Alt/Meta modifiers are held.
- Buttons remain fully usable without shortcuts.

Fullscreen behavior:

- Request fullscreen on the player root, not the media tag, so controls, errors, drawers, and playlist remain available.
- Treat `fullscreenchange` as authoritative; do not optimistically set fullscreen state.
- Handle browser-initiated Escape and request rejection.
- Hide/disable the control if the Fullscreen API is unavailable.
- Pause only if the page itself is unmounted/hidden according to the visibility policy, not merely when entering fullscreen.

## Page layout and CSS

Add a `semantic-player` block to `crates/ui/assets/core_styles.css` using the existing semantic variables and BEM naming.

Suggested structure:

```text
.semantic-player
  .semantic-player__toolbar
  .semantic-player__body
    .semantic-player__stage
    .semantic-player__playlist
  .semantic-player__filter-panel
  dialogs
```

Requirements:

- The page is a bounded flex/grid column with `min-height: 0` and `overflow: hidden`; the stage and virtual playlist must receive a real height.
- The playlist owns scrolling. The entire document must not grow to the virtual canvas height.
- Add `min-width: 0` to stage/title/flex children to make ellipsis and media containment work.
- Fullscreen root fills `100vw`/`100vh` and uses safe-area padding where appropriate.
- The shell currently caps `.semantic-ui__main` at 1180 px and adds padding. Either add a route-aware wide-player modifier in `AppShell` or make the player intentionally fit inside that frame; avoid negative-margin viewport hacks.
- On mobile, stack stage and playlist and keep transport controls wrapping in a predictable order.
- Honor `prefers-reduced-motion`; no required functionality should depend on animation.
- Maintain visible focus indicators from `dxcomp`.

## Routing and navigation

- Add `mod play` and export `PlayPage` in `crates/ui/src/views/mod.rs`.
- Add `#[route("/play")] PlayPage` under `AppShell`.
- Add a Play link in `crates/ui/src/components/shell.rs`.
- Pause and clean up listeners/timers/handles when navigating away.

Keep `/play` usable without route query parameters. Once the filter model stabilizes, optional shareable parameters can be added. Do not copy Browse’s local-storage-only SQL hash as the sole source of truth without documenting that URLs are browser-local.

## Accessibility

- All transport actions must be real buttons with names and disabled states.
- Toggle buttons expose `aria-pressed`; panel toggles also expose `aria-expanded`/`aria-controls`.
- The playlist exposes total position through the virtual-list ARIA attributes and active state with `aria-current`.
- Active-row selection must work by keyboard.
- Media has meaningful labels; images have alt text.
- Native audio/video controls remain enabled by default. A future custom-control-only mode must reach feature parity before hiding them.
- Status/error messages use appropriate `role=status` or `role=alert`, but progress updates are not live-announced continuously.
- Dialog focus is trapped/restored by `dxcomp::Dialog`; verify focus returns to the title button.
- Test 200% zoom and narrow layouts without losing transport controls.

## Loading, empty, and error states

Distinct states should be visible and recoverable:

- Initial queue query loading: stage skeleton plus cancel if supported.
- Empty filter result: “No playable items matched” with an Edit filter action.
- Queue contains non-playable raw-query entities: render them as static slides and label them as such.
- Active object loading: preserve controls but disable play until ready.
- Page placeholder loading: playlist skeleton row; retry only that page on failure.
- Query/expansion failure: retain the old queue on failed Replace; retain existing segments on failed Add.
- Missing object/deleted entity: mark occurrence unavailable and offer Skip/Remove/Retry.
- Missing media source/unsupported MIME: visible error plus entity action.
- Browser play rejection: paused state with a user-gesture prompt.
- Fullscreen rejection: nonfatal toast/message.

## Tests

### Pure state unit tests

Cover at least:

- Empty queue Play/Next/Previous are no-ops.
- Replace selects the first item and preserves intended play/pause policy.
- Append preserves active occurrence and progress.
- Next/Previous at every boundary with cycle on and off.
- Finished advances exactly once.
- Finishing the final non-cycling item pauses and stays selected.
- Image play starts the timer rather than skipping.
- Image pause/resume preserves remaining time.
- Timer event from an old session is ignored.
- Paused/Finished/Progress/Failed events from an old session are ignored.
- Handle registration from an old session is rejected.
- Mute is applied to current and future handles.
- Modal open/close resume policy, including explicit pause while open.
- Failure-loop guard stops repeated auto-skip.
- Delete active occurrence chooses the correct next/previous/empty state.
- Duplicate entity IDs remain distinct occurrences.
- Shuffle is a permutation of all occurrences, not only loaded entries.

### Query and expansion tests

- Default query selects current file entities by all three playable MIME prefixes.
- Generated order has a stable ID tie-breaker.
- SQL identifiers/strings are escaped through existing helpers.
- Page offsets and limits do not overlap or omit rows.
- Replace/append stale generations cannot install results.
- Directory expansion preserves defined order.
- Cycles terminate.
- The same media reached by multiple paths is emitted once.
- Depth/result caps return a partial warning.
- Legacy parent expansion is tested separately if enabled.

Use the in-memory/KV DB test patterns already present in directory-browser query tests.

### Renderer/component tests

- MIME lookup accepts `mime_type` and the qualified file MIME attribute.
- Exact/inherited renderer precedence remains intact.
- Audio/video DOM event mapping produces the correct typed events.
- Unmount unregisters the handle.
- Play rejection becomes `PlayRejected`.
- Changing muted options updates the media tag.
- Image load/error maps correctly.
- Fallback entity rendering works for non-playable objects.

Where DOM behavior cannot be exercised in ordinary Rust component tests, isolate the JS bridge behind a trait and test the controller with a fake handle. Add a focused browser smoke test for actual HTML media events.

### Virtual-list and scale tests

- Construct at least 50,000 lightweight entries and assert reducer/order operations stay linear and do not clone full objects.
- Verify only the visible playlist row window is mounted in a browser test.
- Jump to a far index (for example 40,000), load its page, select it, and scroll it into view.
- Append a second large segment and verify prefix/index mapping at the segment boundary.
- Resize/toggle the playlist without remounting the stage.
- Scroll rapidly while pages return out of order; rows must land at the correct logical indexes.

### Manual matrix

Test both web and desktop builds with:

- Local image/audio/video files through their respective file URL mechanisms.
- Remote media URL if supported.
- Native control pause/resume and toolbar pause/resume.
- Ended event and cycling.
- Mute across item changes.
- Filter Replace/Add while paused and playing.
- Fullscreen enter/exit via button and Escape.
- Keyboard exclusions in filter inputs/dialog.
- Playlist closed/open on desktop and mobile width.
- Broken source, deleted object, and autoplay rejection.
- A 10,000–50,000 item queue.

## Implementation phases

### Phase 0: confirm contracts and fixtures

- Confirm the canonical playable entity shape in a real catalog (`FILE_CLASS_ID`, `id`, `mime_type`, locator, title, duration).
- Confirm `RpcClient::file_url` behavior for web HTTP and desktop custom protocol.
- Decide the relation expansion convention and whether legacy `semantic:parent` is required.
- Add small image/audio/video fixtures legal for repository tests or document a manual fixture procedure.

Exit criterion: the team can write one deterministic query that returns playable file summaries and can resolve each summary ID to a browser-loadable URL.

### Phase 1: complete reusable playback rendering

- Add the additive playback renderer registration/event/handle contract.
- Fix MIME-kind lookup.
- Implement and register image/audio/video playback components.
- Implement handle cleanup and play rejection reporting.
- Add catalog/renderer tests.

Exit criterion: a small harness can render one object, play/pause/mute audio/video, display an image, and receive load/progress/duration/finish/failure events.

### Phase 2: pure player controller

- Implement queue occurrences, state, commands, boundary/cycle logic, sessions, progress, failure handling, and image timers.
- Test with fake media handles and a fake clock.

Exit criterion: controller tests cover all transitions and stale-event cases without Dioxus/DOM.

### Phase 3: basic `/play` parity

- Add route/nav/page.
- Implement stage, toolbar, initial media query, Replace/Add filter panel, keyboard, fullscreen, and entity dialog/actions.
- Start with a modest queue page size and normal order.

Exit criterion: all old useful behaviors work on a small mixed image/audio/video set, including pause/resume across renderer and dialog transitions.

### Phase 4: scalable playlist

- Add paged summary/count support.
- Implement segment/page cache and active object LRU.
- Add toggleable `dxcomp::VirtualList` playlist, far-index loading, follow/current behavior, and responsive/fullscreen layout.
- Add complete-queue shuffle materialization.
- Exercise 10,000–50,000 entries.

Exit criterion: the DOM contains only a small visible row window, far-index selection works, memory is dominated by summaries/addresses rather than full objects, and stage playback is unaffected by list scrolling/toggling.

### Phase 5: recursive expansion and hardening

- Implement cycle-safe, bounded, ordered expansion.
- Add cancellation, partial-result warnings, page retries, failure-loop protection, and visibility cleanup.
- Complete accessibility/browser/manual test matrix.

Exit criterion: expansion handles real directory graphs, failures are recoverable, and both web/desktop targets pass checks.

After every implementation phase, run through the Nix devshell when available:

```text
cargo check --quiet --message-format=short
cargo test --quiet --message-format=short
cargo fmt
```

Use targeted package tests during development, then workspace-level validation for the completed change.

## Prerequisites

Required before full completion:

- A production playback renderer registration and event/handle contract in `ui_core`.
- Standard image/audio/video playback renderers.
- Correct MIME lookup for current filestore objects.
- A stable paged/count query path for truly large queues.
- A documented relation convention for container expansion.

Not required to begin:

- The full global structured Browse filter implementation.
- A generic background-job subsystem.
- Bulk media analysis. Known duration can be used when present and learned from media metadata otherwise.
- Persistent user playlists. This plan concerns the current playback queue; saved playlist entities are a separate feature.

## Decisions to resolve before coding

1. **Paged query API:** add a generic read-only `semantic.db.query_page` command, or a player-specific media query command? Generic is more reusable; player-specific can enforce playable summary shape and expansion more cleanly.
2. **Raw SQL scalability:** reject unsupported grouped/distinct/unstable queries, accept them with unknown totals, or materialize them? The recommended choice is validate/reject for scalable mode and clearly offer a bounded compatibility mode.
3. **Expansion semantics:** directory relationships only, legacy `semantic:parent` too, or catalog-registered expander providers?
4. **Expansion order:** breadth-first direct media first (recommended and closest to the old recursion’s output) or depth-first directory order?
5. **Shuffle and active item:** preserve the current occurrence/position (recommended) or reset to the first shuffled item as the old code did?
6. **Native controls:** keep them visible in addition to the player bar (recommended for v1) or implement complete custom controls?
7. **Failure auto-skip:** automatically skip fatal media errors while playing (old behavior) or wait for explicit Skip? Recommended: auto-skip with a visible error history and a strict loop guard.
8. **Visibility policy:** pause on document hidden and optionally resume, or continue audio in background? Make this explicit; do not let incidental component rerenders decide it.
9. **Deletion semantics:** remove only the active occurrence, all occurrences of that entity, or leave unavailable rows? Recommended: remove all occurrences after confirmed entity deletion.
10. **Queue persistence:** reset on navigation (parity) or persist filter/order/position locally? Defer persistence until playback correctness and source versioning are established.

## Acceptance criteria

The feature is complete when:

- `/play` is reachable from the shell and initially loads playable current-schema files.
- Image, audio, and video render correctly on web and desktop.
- Toolbar and native media state remain synchronized for play, pause, mute, progress, duration, finish, and failure.
- Switching items never lets stale renderer events affect the new item.
- Images honor a pausable/resumable slideshow interval.
- Previous/next/cycle/shuffle boundaries behave deterministically.
- Filter Replace/Add and cycle-safe container expansion work without destroying a valid queue on failure.
- The active entity can be inspected and its normal actions used; modal pause/resume behavior is correct.
- Keyboard and fullscreen behavior are cleaned up on unmount and do not interfere with inputs/dialogs.
- The playlist can be toggled, selects/jumps to items, highlights the active occurrence, and uses `dxcomp::VirtualList` or an equivalent virtualizer.
- A queue with at least 50,000 entries does not create 50,000 DOM rows or retain 50,000 full entity objects.
- Loading, empty, unsupported, missing, autoplay-blocked, query-error, page-error, and media-error states are visible and recoverable.
- Unit, query, renderer, scale, and manual web/desktop checks pass.
