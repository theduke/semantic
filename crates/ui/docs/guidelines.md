# Semantic UI guidelines

This document defines the UX, component, and Dioxus conventions for future work in
`semantic_ui` and the user-facing parts of `semantic_ui_core`.

The UI is a workspace for exploring, editing, organizing, querying, uploading, and
playing semantic data. It should feel coherent and dependable without adding a heavy
frontend stack or sacrificing desktop/web parity.

## Product and UX principles

### Preserve context

- Use typed `Route` navigation. Back/forward and copied URLs should reconstruct the
  meaningful view whenever the route schema supports it.
- Collection-aware actions must preserve `Some(collection)`, including an explicit
  named route for the default collection.
- After mutations, navigate to a deterministic in-app destination. Never use browser
  history as the only delete or cancel fallback.
- Preserve drafts and the last successful data during recoverable failures.

### Make system state explicit

Every asynchronous surface must deliberately represent the states that apply:

1. Initial loading: stable skeleton geometry and `aria-busy`.
2. Refreshing: retain usable data and show a quiet refresh indicator.
3. Empty: explain why the surface is empty and offer a valid next action.
4. Error: plain-language summary, optional technical detail, and retry.
5. Mutation pending: disable only conflicting actions.
6. Success or partial success: show the outcome and any remaining recovery action.

Do not show an empty state while an initial request is still loading. Do not hide an
error in a closed panel or behind a dialog.

### Prefer honest capability over decorative controls

- A visible control must perform real work with the current backend/platform API.
- Defer features that require missing contracts, such as resumable upload, backend
  request cancellation, cross-collection reference resolution, or unsupported media
  controls.
- Use clear labels such as **Save changes**, **Create entity**, and **Remove from
  queue**. Avoid generic **Submit**, ambiguous **Rows**, or destructive labels for
  non-destructive actions.

### Use restrained visual hierarchy

- One route-level `h1`, supplied through `PageHeader` or an equivalent accessible
  player heading.
- Prefer one neutral surface level and typography/spacing hierarchy over nested cards
  and escalating shadows.
- Friendly names lead. Canonical IDs and backend types are secondary monospace
  metadata with copy affordances.
- Status color always has accompanying text, iconography, or semantics.

## Application structure

### Shared frame

- `AppFrame`, `AppShell`, `PlayerShell`, and `PrimaryNav` own application navigation
  and the sole `<main>` landmark.
- Route components must not render another `<main>`.
- Use `PageHeader` for breadcrumbs, route title, description, and route-level actions.
- Player may use the immersive frame variant, but it must retain an accessible heading
  and a route back to the workspace.

### Component ownership

Use the narrowest appropriate layer:

- `dxcomp`: low-level behavior such as buttons, dialogs, comboboxes, and focus traps.
- `semantic_ui_core`: domain renderers and components reusable outside the main app,
  including entity rendering, directory browsing, forms, async feedback, and toasts.
- `semantic_ui::components`: application composition such as page headers, explorer
  toolbars, form pages, confirmation conventions, and upload primitives.
- `views`: route state, server requests, navigation outcomes, and route-specific
  composition.

Do not add a second primitive component library. Extend the Semantic layer through
composition when `dxcomp` already owns the required interaction behavior.

### Existing shared components

Prefer these before creating route-local equivalents:

| Need | Components |
|---|---|
| Page structure | `AppFrame`, `PageHeader` |
| Async feedback | `LoadingSkeleton`, `RefreshingIndicator`, `EmptyState`, `ErrorState`, `InlineNotice` |
| Global feedback | `ToastProvider`, `ToastDispatcher`, `ToastViewport` |
| Safe actions | `IconButton`, `ConfirmAction`, `ConfirmDangerDialog`, `CopyableCode` |
| Entity results | `EntityExplorer`, `DataToolbar`, `EntityResults`, `Pagination` |
| Entity detail | `EntityPageHeader`, `EntityCard` with detail-mode action hooks |
| Query input | `QueryEditor` |
| Form layout | `FormPage`, `FormActions`, `UnsavedChangesPrompt` |
| Upload/job UI | `DropZone`, `JobProgress` |
| Directory selection | `DirectoryBrowser`, `FileTreePicker` |

A shared component should be introduced by a real route and proven by a second
consumer. Avoid speculative components with large prop surfaces.

## Dioxus state model

### Decide ownership before choosing a hook

- Router/URL: entity and collection identity, shareable filters, paging, view modes,
  and presets when the route schema supports them.
- Application context: stable services, active scope, catalog, toast dispatcher, and
  durable preferences.
- Route component: drafts, selection, dialogs, pending mutations, and request state.
- Leaf component: only interaction state that has no meaning outside that component.

Do not copy router or context state into a local signal unless it is intentionally an
editable draft. Applied state and draft state must remain distinct.

### Signals

Use `Signal<T>` for independently changing values.

- Split high-frequency state from large, low-frequency state. Playback progress and
  upload item progress must not invalidate an entire queue or page.
- Prefer keyed per-item state for queues. Keep stable order separately when useful.
- Do not bundle unrelated writable signals into a broad state object passed through
  the whole subtree.
- Never write a signal while rendering. Render is a pure projection.
- Callbacks should communicate typed intent or outcomes; children should not receive
  a parent's writable aggregate state.

### Memos

Use `use_memo` for expensive projections or stable dependencies, not trivial labels.

Important: a memo only reacts to signals read inside it. Capturing a plain cloned
`UiCatalog`, scope, prop, or vector produces stale derived data. For catalog-derived
options, read `use_ui_catalog_context().catalog_signal()` inside the memo.

Return `Rc<[T]>`, `Rc<Vec<T>>`, or another cheap immutable representation for large
derived collections. Key rendered rows by domain identity, not by the current index.

### Resources and request identity

Every server load needs an explicit immutable key containing all values that change
the response, commonly:

```text
(scope_id, collection, entity_id, filter, page, page_size, request_generation)
```

Recommended pattern:

1. Build a comparable request-key value from tracked route/context state.
2. Capture that complete key in the `Resource` or spawned task.
3. Return the key with the response.
4. Render a response only when its key still equals the current key.
5. Retain the previous successful value during same-key refresh or a recoverable
   failure when that does not mislabel old data as a new route.

Resource cancellation is local future/task cancellation. Do not describe it as server
cancellation unless the transport has an explicit cancellation contract.

### Tasks, workers, and coroutines

- Use spawned tasks for network work that must not block Escape, dialog close, or
  unrelated commands.
- Use generation/session/occurrence IDs to reject stale completion and stale media
  events.
- Use a bounded worker pool for independent uploads. Capture immutable options when
  work starts.
- A coroutine is appropriate for fast synchronous event reduction. Do not await
  unrelated long RPC operations in one global command receiver.
- Timers must exist only while needed. Prefer a lifecycle-keyed one-shot deadline to
  permanent polling.

### Effects

Use effects only to synchronize with systems outside normal render derivation:

- focus and focus restoration;
- document title;
- browser listeners/fullscreen;
- local-storage preferences;
- scroll-follow behavior;
- timer/listener cleanup.

Do not use an effect to mirror one signal into another when a memo or direct derived
value is sufficient.

## Forms

- Keep `DynamicClassForm` and `dxform` as the schema-driven field engine.
- Use the additive dirty, submitting, success, and failure callbacks. Do not mirror
  every field into route signals.
- A draft identity is created once and stored in state. It must not be generated in a
  conditional render branch.
- Changing class or collection while dirty requires confirmation.
- Submit failure preserves the draft. Submit success uses a toast and deterministic
  typed navigation.
- Use `SemanticFormActionLabels::create_entity()` or `save_changes()`.
- `UnsavedChangesPrompt` currently protects explicit in-UI transitions. Do not claim
  protection for browser Back/forward until a reliable router blocker is implemented.

Custom form renderers currently return opaque elements. Field groups expose names,
help, validation, and required/optional state, but native input-level `label for` and
`aria-required` require a future renderer-context accessibility contract.

## Lists, tables, and queues

- Clamp all route-provided page and page-size values at the boundary.
- Fetch a sentinel row (`page_size + 1`) when that is the only honest way to compute
  `has_more`; do not present a silent hard cap as a complete result.
- Bound or virtualize large DOMs. Current route-specific bounds must be disclosed in
  the UI.
- A virtual-list count must update when its source grows or shrinks.
- Filtering creates a new index space. Scroll/follow calculations must use filtered
  row positions, not underlying queue indices.
- Never flatten structured query rows into one comma-separated cell. Derive stable
  columns and render typed cells with scoped headers.

## Accessibility

Target WCAG 2.2 AA.

- One `<main>` application landmark and one route `h1`.
- Use native buttons and links for their real semantics.
- Active navigation uses `aria-current`; toggles use `aria-pressed`; expandable tree
  items use `aria-expanded` and `aria-controls`.
- Icon buttons require accessible labels and a visible tooltip/title convention.
- Async status uses deliberate `status` or `alert` behavior. Do not announce every
  progress tick.
- Dialog errors belong inside the active dialog. Closing restores focus to the
  trigger when the primitive supports it.
- Support keyboard-only selection, navigation, confirmation, and reordering. Provide
  a keyboard/touch alternative to pointer drag behavior.
- Preserve critical metadata on narrow screens; move it into a secondary line or
  details surface instead of silently hiding it.
- Respect `prefers-reduced-motion`, visible `:focus-visible`, 200% zoom, and coarse
  pointer hit targets.

## Styling and responsive behavior

- Use the Semantic CSS tokens in `assets/core_styles.css`; do not add hard-coded
  route colors when a semantic token exists.
- Keep runtime styling dependencies out of the application. Prefer static CSS,
  container/media queries, and system fonts.
- Reuse existing BEM-style class families and ensure Rust class hooks and CSS selectors
  match exactly.
- Design from 320 px upward. Test narrow phone, tablet, standard desktop, and small
  desktop WebView layouts.
- Sticky actions must respect safe-area insets and must not cover the focused control.
- Dark theme support is opt-in through Semantic tokens. Do not independently force a
  theme that conflicts with `dxcomp`.

Treat CSS transfer size as a budget. After meaningful styling work, record raw and
gzip sizes and consolidate repeated rules. Do not optimize by removing accessibility
or responsive states.

## Adding or changing a route

1. Read the matching plan document under `docs/plans/2026-08-30-ui-overhaul/`.
2. Identify which existing shared components the route consumes.
3. Define router, shared, route-local, and leaf state ownership.
4. Define complete request keys and stale-response behavior.
5. List loading, refresh, empty, error, mutation, and success states before coding.
6. Implement typed navigation and deterministic cancel/delete/success destinations.
7. Add stable keys and explicit rendering bounds.
8. Verify heading/landmark, keyboard, accessible name/role/value, and 320 px behavior.
9. Run the targeted UI check and formatting/diff hygiene.
10. Document any deferred feature that requires a new route, persistence, transport,
    or backend contract.

## Development workflow

Use the Nix UI devshell and check from the UI crate:

```bash
nix develop .#ui
cd crates/ui
cargo check --quiet --message-format=short --no-default-features
```

`--no-default-features` is appropriate while editor work is changing concurrently.
Run default-feature and broader workspace validation only when those dependencies are
stable and in scope. Use targeted formatting so unrelated concurrent files are not
modified, then run `git diff --check`.

Repository conventions still apply:

- Use full `Result<T, E>` types rather than local convenience aliases.
- Prefer `smartedit ast-print` for focused Rust exploration and narrow edits.
- Do not reformat, revert, or repair unrelated concurrent work.

## Review checklist

- [ ] Existing shared components are reused rather than forked.
- [ ] Route state ownership and typed destinations are explicit.
- [ ] Resource/task keys include scope and every response-changing input.
- [ ] Stale completions cannot render under a newer route.
- [ ] Hot progress/timer state cannot invalidate a large list subtree.
- [ ] Lists are keyed, bounded, paged, or virtualized.
- [ ] Loading, refresh, empty, error, mutation, and success states are honest.
- [ ] One main landmark, one route heading, visible focus, and complete control names.
- [ ] Narrow layouts preserve critical content and actions.
- [ ] No unsupported feature is represented as functional.
- [ ] UI-crate cargo check and diff hygiene pass, or an unrelated blocker is recorded.

## Known contract boundaries

The following require broader product/backend work and should not be reimplemented as
route-local hacks:

- Catalog deep-link state and Collection explorer query parameters.
- Create route class/collection presets.
- Portable saved queries and long-SQL persistence.
- Server-side request cancellation and paged tree children.
- Cross-collection references that carry only an ID.
- Resumable uploads and backend duplicate/conflict policy.
- Media volume, playback rate, repeat-one, captions, and picture-in-picture until
  `PlaybackMediaHandle` exposes those capabilities.
