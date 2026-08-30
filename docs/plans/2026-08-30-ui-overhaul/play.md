# Media player — `/play`

## Current implementation

`PlayPage` is a full media workspace: initial/default playlist load; structured media filter and guarded raw SQL; replace/add; image/audio/video stage; transport, mute/cycle/shuffle/fullscreen, image interval, seek; global shortcuts; resizable persisted playlist; virtualization above 200 entries; failed-item recovery; entity detail/delete dialog (`crates/ui/src/views/play/*`).

## Strengths

- Pure `PlayerState` has good coverage for boundaries, stale session events, shuffle, duplicate occurrences, failure loops, and 50k queues (`views/play/state.rs:89-572`).
- Queue is `Rc<Vec<QueueEntry>>`, objects load only for the active item, and playlist virtualizes.
- Playback session/occurrence IDs protect against stale renderer events.
- Resize handle supports pointer and keyboard with ARIA values (`playlist.rs:94-231`).
- Many control pressed/expanded labels and shortcuts are already present.

## Findings

| Priority | Finding | Evidence / impact |
|---|---|---|
| P0 | Initial loading/error is hidden | Stage says “No playable items” during initial load; initial error lives in closed Filter (`play/mod.rs:42-77`, `stage.rs:23-27`). |
| P0 | Aggregate hot state rerenders | Full `PlayerState` is cloned each render (`mod.rs:166-180`); 100 ms image ticks can invalidate controls, stage, playlist, and dialog. |
| P1 | Permanent timer loop | A 100 ms future runs forever even while paused/non-image (`mod.rs:105-123`). |
| P1 | Active-object error recovery is incomplete | Stage shows error but no direct retry; dialog says “still loading” even on an error (`stage.rs:32-36`, `entity_dialog.rs:28-30`). |
| P1 | Seek updates on commit | Range uses `onchange`, reducing scrubbing feedback (`controls.rs:75-83`). |
| P1 | Small-screen hierarchy | Full nine-link app header consumes player space; toolbar exposes all secondary controls inline. |
| P1 | Queue management gaps | No non-destructive remove/reorder/clear/save/search; remove-active currently means delete entity through dialog flow. |
| P1 | Filter quality | Collection is free text; advanced SQL uses prefix-style validation rather than parsed SELECT. |
| P2 | Playback feature gaps | No volume, speed, repeat-one, captions, picture-in-picture, shortcut help, or slideshow countdown. |

## Target experience

- Player-specific compact frame: back/workspace, current title, queue/filter, transport, fullscreen; full global nav moves to a menu on compact/narrow layouts.
- Initial stage explicitly shows Loading playlist, load error with Retry/Edit filter, or real empty state.
- Primary controls prioritize Previous/Play/Next, progress, volume; secondary cycle/shuffle/slides/fullscreen live in responsive groups/overflow.
- Queue supports search, non-destructive remove, clear, reorder (accessible controls as well as drag), and optional saved playlist/preset. Deleting the underlying entity remains a distinct destructive action.
- Filter uses shared catalog-backed collection/structured entity filter and shared `QueryEditor` advanced mode; preview matched count when feasible.
- Entity dialog distinguishes loading/error/loaded and offers Retry/Open full detail/Edit.

States: initial loading/error/empty, filter draft/loading/error/result warning, active object loading/error, media preparing/ready/playing/paused/buffering/finished/failed/play rejected, queue empty/failed, fullscreen unavailable, dialog states.

## Component boundaries and reuse

Consume `InlineNotice/Toast`, `AsyncState`, shared `StructuredEntityFilter`, `QueryEditor`, `EntityPageHeader/EntityDetail`, `SplitPane`, and `IconButton`. Player evolves the reusable `SplitPane { orientation, size, min/max, on_size_change }` for Tree and `MediaTransport`/`JobProgress` patterns only where semantics truly overlap.

Suggested boundaries:

- `PlayerQueueController` owns queue/order/active/session and exposes typed actions.
- `PlaybackController` owns intent, observed media state, progress, mute/volume/rate, active handle.
- `PlayerUiState` owns panels/fullscreen/dialog/follow/persisted split size.
- Child props are narrow: `PlayerControls` reads transport/summary, `PlayerPlaylist` reads `Rc<[QueueEntry]>` plus active/failed sets, `PlayerStage` reads active target/object/playback options.
- Events remain typed (`select`, `toggle`, `seek`, `media_event`) rather than sharing writable aggregate signal.

## Dioxus state architecture

- Split high-frequency `PlaybackProgress` from low-frequency queue and UI signals; no `state.read().clone()` snapshot of the full aggregate on each tick.
- `Memo` derives active queue entry/request key and disabled-control flags. The active-object `Resource` is keyed by `(scope, target, playback session)` and generation-guarded.
- Initial playlist `Resource` explicitly keys scope/filter; same-scope refresh retains queue until replacement succeeds.
- Start a lifecycle-scoped deadline task only while an image is ready and playing; cancel/recreate on session/pause/interval. Effects synchronize browser shortcuts, fullscreen, scroll-follow, media handles, and stored preferences with cleanup.
- Keep `Rc` queue and virtual list; ensure child props are stable and player progress cannot invalidate the playlist subtree.
- Query/filter load uses a cancel/generation task (existing request generation is a good pattern). Parse raw SELECT through shared typed query logic.

## Visual direction

Let media dominate: a dark tokenized stage, compact high-contrast transport, and neutral queue/filter surfaces. Reduce the current wall of same-weight text buttons with familiar icons plus accessible labels/tooltips, prominent primary transport, and grouped overflow. Fullscreen removes non-player chrome; audio receives intentional artwork/metadata treatment rather than a floating native control.

## Accessibility/responsive/performance

- Transport has named controls, pressed state, shortcut help, live status that does not announce every time update, accessible range values, captions/subtitle controls when available.
- At 320 px, preserve stage and primary transport; queue/filter become sheets and secondary controls overflow. Safe areas apply in fullscreen.
- Verify 50k queue with bounded DOM, no idle 100 ms wakeups, and progress updates localized to transport/stage.

## Missing functionality

Visible initial states, queue edit/save/search, catalog-backed filters, volume/speed/repeat-one, captions, PiP, shortcut help, play-from-Browse/Tree, slideshow countdown, and robust active-object retry.

## Acceptance criteria

- Initial load never appears as a false empty state; errors are visible without opening Filter.
- Progress/timer changes do not rerender playlist/filter/dialog; there is no continuous timer while idle.
- Active-object/media errors offer Retry/Skip and dialog displays the true state.
- Queue operations never delete entities unless explicitly chosen in a named destructive action.
- All controls work keyboard-only at narrow widths and queue stays responsive with 50k entries.
- Shared filter/query/split/entity/notice components are reused with Browse/Query/Tree rather than duplicated.
