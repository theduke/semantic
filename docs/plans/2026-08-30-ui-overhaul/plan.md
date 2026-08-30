# Semantic Web UI overhaul plan

Date: 2026-08-30
Status: proposed
Scope: `crates/ui`, the user-facing parts of `crates/ui_core`, and shared styling in `crates/ui/assets/core_styles.css`

## Purpose

This plan turns the current functional Dioxus UI into a coherent, fast workspace for browsing, editing, organizing, querying, uploading, and playing semantic data. It preserves the existing schema-driven renderer/form architecture and avoids a heavy frontend dependency stack.

The review covered every `Routable` variant in `crates/ui/src/views/mod.rs`, the shared app shells, the catalog provider, generic entity/form components, directory browser, upload queue, media player, and all 1,655 lines of application CSS. Source inspection is the primary evidence. A web build was also attempted against the already-running local API; visual automation was unavailable and the current Nix shell has `wasm-bindgen-cli` 0.2.121 while the lockfile requires 0.2.122, so browser-rendered observations must be revalidated during implementation. No UI code is changed by this plan.

## Executive assessment

The product has a stronger functional foundation than its visual surface suggests. It already supports schema-driven CRUD, browsing, raw SQL, directory operations, uploads with progress, and a scalable media playlist. The largest problem is inconsistency: Browse, Tree, Upload, and Play are relatively developed, while Home, Catalog, Collection, Query, and the shell remain near-prototype quality.

The highest-value overhaul is therefore not a visual rewrite. It is a shared application frame, a small design-system layer, consistent async/empty/error states, and consolidation of repeated entity/query behavior. This gives every route a coherent baseline before adding higher-level capabilities such as global search, structured filters, scope switching, bulk actions, tagging, and maintenance jobs.

Key current risks:

- The header renders nine equal-weight links, has no current-page state, no mobile navigation pattern, no search, and no scope/connection identity (`components/shell.rs:31-111`).
- Home, Catalog, Collection, and Query have no page-specific CSS at all; `.semantic-error` also has no shared visual definition (`core_styles.css`).
- Several routes expose backend-oriented names and raw identifiers as the primary information architecture.
- Async pages mostly show plain text and do not consistently provide retry, cancellation, stale-result protection, or actionable empty states.
- `UiCatalogProvider` writes signals while rendering and hides the entire app until catalog load completes (`ui_catalog/provider.rs:123-151`).
- Catalog/collection/entity/edit/picker/player resources often capture cloned plain scope/route values instead of explicit reactive keys; same-component parameter or live scope changes can leave old data visible (`ui_catalog/provider.rs:109-116`, `views/collection.rs:12-18`, `views/entity.rs:29-40`, `views/form.rs:167-181`).
- The Collection page duplicates a weaker subset of Browse, hard-caps at 100 rows, interpolates an unquoted collection name into SQL, and has no empty state (`views/collection.rs:31-87, 109-140`).
- Query has no running state, stale-request protection, keyboard execution, table schema, history, export, or query safety guidance (`views/query.rs:9-111`).
- Create generates its initial entity ID during render and remounts the form when class/collection changes, which can silently discard entered values (`views/form.rs:113-139`).
- Browse stores custom SQL only as a browser-local hash, creating non-portable links and a hard failure after storage is cleared (`views/browse.rs:35-45, 460-500`).
- Upload serializes “Upload all” instead of using bounded concurrency and cannot cancel active work (`views/upload.rs:175-201`).
- Player clones the complete `PlayerState` every render and runs a permanent 100 ms loop, even when no image timer is active (`views/play/mod.rs:105-123, 166`).
- Tree is feature-rich but its single coroutine owns many independent states and serializes all commands, making busy feedback and request cancellation difficult (`directory_browser/view.rs:72-146, 148-735`).
- Tree list/grid VirtualList counts initialize from props only and can become stale when a page grows/shrinks (`directory_browser/view.rs:1199-1262`); entity and upload loops also need stable domain keys.
- `use_ui_catalog()` clones the complete catalog and is called repeatedly inside entity rows/actions (`ui_catalog/provider.rs:89-95`, `components/entity/card.rs:37,132,227,250`).
- Theme ownership conflicts: Semantic forces light color scheme while `dxcomp` follows system light/dark tokens. The application ships about 37.6 KB raw core CSS plus the entire 114.4 KB raw `dxcomp` stylesheet, much of which this app does not use.
- No route-level UI/integration/accessibility tests are present; existing tests are primarily pure data/query/state tests.

## Route inventory

Every declared page/route has its own audit:

| Route | Page document | Current role | Priority |
|---|---|---|---|
| `/` | [Home](home.md) | Workspace summary and collection entry points | P1 |
| `/catalog` | [Catalog](catalog.md) | Schema/catalog inspection | P1 |
| `/collections/:collection` | [Collection](collection.md) | Legacy/basic collection table | P0 |
| `/entities/create` | [Create entity](create-entity.md) | Schema-driven creation | P0 |
| `/entities/:id` | [Default entity detail](default-entity.md) | Entity detail in the default collection | P0 |
| `/collections/:collection/:id` | [Collection entity detail](collection-entity.md) | Entity detail in a named collection | P0 |
| `/entities/:id/edit` | [Default entity edit](default-edit-entity.md) | Edit entity in the default collection | P0 |
| `/collections/:collection/:id/edit` | [Collection entity edit](collection-edit-entity.md) | Edit entity in a named collection | P0 |
| `/browse?...` | [Browse](browse.md) | Flexible entity result browser | P0 |
| `/query` | [Query](query.md) | Raw SQL workbench | P1 |
| `/tree?root=...` | [Tree](tree.md) | Directory/file organization workspace | P0 |
| `/upload` | [Upload](upload.md) | Multi-file ingestion | P0 |
| `/play` | [Player](play.md) | Media queue and playback | P1 |

There is no not-found route. Add one as part of the shell/router work and treat it as a shared system page rather than a product route in the inventory above.

## Product and information architecture

### Primary navigation

Replace the flat nine-link header with four stable areas:

1. **Workspace** — Home, global search, recent items, saved views.
2. **Data** — Browse, Tree, Catalog.
3. **Create** — New entity, Upload.
4. **Tools** — Query, Player, later Settings/Jobs.

Desktop should use a compact sidebar or grouped top navigation with a visible active state. Mobile should use a single menu/sheet with the same semantic groups. Keep route transitions client-side and preserve query strings. Show current scope/connection in the app frame and expose switching when supported.

Collection is currently a weaker duplicate of Browse. Make `/collections/:collection` a collection-specific Browse preset or give it a distinct administration role (schema, counts, indexes, and records tabs). Do not maintain two unrelated list implementations.

### Global workflows to add

Ordered by impact and feasibility:

1. Global entity search/command palette with `/` or `Ctrl/Cmd+K`, title/ID search, keyboard navigation, recent items, and direct actions.
2. Scope/database chooser with current-scope indicator, initial connection recovery, catalog reload, and safe reset of scope-bound page state.
3. Structured Browse filters: search, class/type, tags, sort, and portable URL serialization, retaining raw SQL as an advanced mode.
4. Saved queries/views and recent query history, with copy, download CSV/JSON, explain, duration, affected row count, and safe read-only defaults.
5. Bulk selection/actions across Browse and Collection: tag, move/add to directory, export, and delete with explicit confirmation.
6. Purpose-built tagging: attach/detach on entities, create while tagging, tag administration, and later transactional merge.
7. Upload improvements: drag/drop, clipboard paste, bounded concurrency, cancel/resume policy, duplicate decisions, optional tags, and batch metadata.
8. File entity action for single-file analysis using the existing command; refresh the entity after success.
9. Settings/jobs hub for long-running analysis, blob audit/cleanup, import, and duplicate-image workflows. Build a job contract before bulk operations.
10. URL importer only after preview/import APIs and media policy exist; blob cleanup only after reference audit/revalidation APIs exist.

## Shared design-system foundation

Keep `dxcomp` as the primitive library and build a thin Semantic-specific layer in `ui_core` or `ui`, not a second component library.

### Tokens

Expand the existing CSS variables into intentional token groups:

- Color: canvas, surface levels, text, muted text, border, primary, primary-hover, focus, success, warning, danger, and media-stage colors.
- Typography: page title, section title, body, label, caption, and mono. Keep system fonts for zero download cost.
- Space: retain the 4 px scale and add named control/page/container spacing.
- Shape/elevation: radius variants, focus ring, and only two shadow levels.
- Layout: content widths, sidebar/header sizes, safe-area padding, and breakpoints.
- State: disabled opacity, hover/pressed colors, skeleton tone, selected background.

Support light mode first, but avoid hard-coded `#fff`, `#fbfcfd`, `#111418`, and `#edf7fa` in components so dark/high-contrast modes remain possible. Use `@media (prefers-reduced-motion)` for all transitions, not only Player scrolling.

### Shared components

Implement or consolidate these reusable components:

| Component | Responsibility | Route implementations / reuse |
|---|---|---|
| `AppFrame` / `PrimaryNav` | grouped navigation, active state, mobile sheet, scope, search trigger | every route; start with [Home](home.md) and [Player](play.md) compact variant |
| `PageHeader` | breadcrumb, title, description, primary/secondary actions | [Home](home.md), [Catalog](catalog.md), [Collection](collection.md), all other pages |
| `AsyncState<T>` helpers | loading skeleton, error with retry, empty CTA, retained refresh | [Collection](collection.md), [entity detail](default-entity.md), [edit](default-edit-entity.md), [Browse](browse.md), [Query](query.md), [Tree](tree.md), [Player](play.md) |
| `InlineNotice` / `ToastCenter` | semantic status with live-region policy and dismiss/action support | mutation flows in [Create](create-entity.md), [Tree](tree.md), [Upload](upload.md), and entity pages |
| `ConfirmAction` | consistent destructive confirmation and pending/error behavior | entity deletion, [Tree](tree.md), form discard, Upload clear/cancel |
| `EntityExplorer` / `EntityResults` | query/filter/result orchestration; cards/table, selection, stable keys | introduced by [Browse](browse.md), consumed by [Collection](collection.md) |
| `StructuredEntityFilter` | portable search/class/tag/sort model | introduced by [Browse](browse.md), consumed by [Player](play.md) and saved views |
| `EntityPicker` | searchable, collection-aware entity selection | [Upload](upload.md), form references/tagging, [Tree](tree.md) add-existing |
| `CollectionPicker` / `ClassPicker` | labels, IDs, empty/help states | [Create](create-entity.md), [Browse](browse.md), [Player](play.md), Query helpers |
| `QueryEditor` | mono editor, Run/Cancel, shortcut, validation, diagnostics | introduced by [Query](query.md), consumed by [Browse](browse.md) and [Player](play.md) |
| `ResponsiveDataTable` / `Pagination` | accessible rows, responsive mode, paging/focus restoration | [Query](query.md), [Browse](browse.md), [Collection](collection.md), [Tree](tree.md) |
| `FormPage` / `FormActions` / `UnsavedChangesGuard` | sticky actions, dirty guard, outcome and error summary | [Create](create-entity.md), [default edit](default-edit-entity.md), [named edit](collection-edit-entity.md) |
| `EntityPageHeader` / `EntityDetail` | identity/breadcrumb/actions plus shared renderer body | [default detail](default-entity.md), [named detail](collection-entity.md), Tree/Player detail dialogs |
| `CatalogExplorer` / `CatalogDetail` | indexed schema search/details | [Catalog](catalog.md), collection schema and form help |
| `DirectoryPicker` / `SplitPane` | accessible hierarchical selection and pane resizing | evolved in [Tree](tree.md); consumed by [Upload](upload.md); SplitPane also by [Player](play.md) |
| `DropZone` / `JobProgress` | file acquisition and queued/running/success/failure progress | introduced by [Upload](upload.md); progress reused by later maintenance jobs |
| `VisuallyHidden` / `IconButton` conventions | accessible labels/tooltips and consistent hit targets | shell, [Tree](tree.md), entity actions, [Player](play.md) |

Prefer composition over large prop surfaces. Domain components should own semantics; `dxcomp` should continue to own low-level behavior such as dialog focus trapping and combobox keyboard interaction.

### Cross-page dependency graph

```text
AppFrame + PageHeader + AsyncState + Notices
├── CatalogExplorer ─────────────── Catalog → Collection schema / form help
├── EntityExplorer
│   ├── StructuredEntityFilter ─── Browse → Player
│   ├── EntityResults ──────────── Browse → Collection
│   └── Pagination / Selection ─── Browse → Collection → Tree
├── FormPage + FormActions ─────── Create → both Edit routes
│   └── Class/Collection/Entity pickers → Browse / Player / Upload / Tree
├── EntityPageHeader + Detail ──── both Detail routes → Tree / Player dialogs
├── QueryEditor + DataTable ────── Query → Browse advanced → Player advanced
├── DirectoryPicker + SplitPane ── Tree → Upload / Player
└── DropZone + JobProgress ─────── Upload → future importer/settings jobs
```

Each page document specifies props/events, state owner, reactive primitive, and rerender boundary for the component it introduces or consumes. A shared component should land with its first route and at least one fixture/test for the next consumer so its API is proven by reuse rather than generalized speculatively.

## Interaction and state standards

Every data surface must explicitly support:

- **Initial loading:** stable skeleton matching final geometry; `aria-busy` on the region; no layout jump.
- **Refreshing:** keep current data visible, show a subtle progress indicator, and prevent stale responses from replacing newer state.
- **Empty:** explain why it is empty and offer the next valid action (create, clear filters, upload, or choose another scope).
- **Error:** plain-language summary, optional technical detail disclosure/copy, retry, and preserved user input.
- **Mutation pending:** disable only conflicting actions; show the affected row/item state.
- **Success:** toast or inline confirmation plus deterministic navigation/refresh.
- **Offline/connection loss:** persistent connection banner and retry; never reduce this to a raw RPC string.

Use optimistic UI only for reversible/local actions such as selection or view mode. Deletion, moves, form submit, and upload finalization should remain server-confirmed. Preserve drafts on recoverable errors.

## Dioxus 0.7 implementation architecture

### Ownership boundaries

- `AppRoot` owns stable services: `RpcClient`, active scope signal, catalog load state/reload callback, toast dispatcher, and user preferences.
- The router/URL owns navigable state: current collection/entity, Browse filters/view/page/page-size, Tree root, and shareable Query/Player presets. Back/forward must reproduce the view.
- A page owns ephemeral interaction state: dialog open, selection, unsaved form draft, active upload handles, and transient errors.
- Leaf components receive the smallest immutable props or read a deliberately narrow context. Do not pass the entire page state when a boolean or `Rc<[T]>` is enough.
- Server data belongs in `Resource`/a small query wrapper keyed by explicit scope and route/filter inputs. Mutation callbacks trigger focused invalidation instead of window reload.
- Context APIs must expose reactive read handles as well as one-shot clones. `use_active_scope_id()` and `use_rpc_client()` currently return cloned plain values (`context/scope.rs:39-40`, `context/rpc_client.rs:23-24`), which encourages resources that never observe live provider changes. Add read-only signal/context hooks and clone values only at an RPC call boundary.

### Signals and derived state

- Use `Signal<T>` for independently changing local values. Split unrelated hot state: Player progress must not invalidate playlist/filter rendering; Upload progress for one item should not clone/re-render the whole queue; Tree selection should not invalidate loaded page data.
- Compute cheap derived values inline from a single read snapshot. Use `use_memo` only for expensive filters/maps or when a stable derived value is a resource/effect dependency. Avoid memoizing trivial labels.
- For large collections, store `Rc<Vec<T>>`/`Arc<[T]>` or normalized maps plus stable ID order. Update only the changed entry. Always render stable domain keys (`entity target`, upload queue ID, playlist occurrence ID), never an index when identity can change.
- Avoid `state.read().clone()` on large aggregate state as in Player. Read narrow fields or split state into transport, queue, UI, and progress signals.
- Avoid repeated catalog and vector cloning in render paths (`use_ui_catalog()` currently clones the complete catalog). Provide a cheap shared `Rc<UiCatalog>`/read-only signal or selector-based access.
- Do not write signals during render. Move `UiCatalogProvider` status/catalog updates into the resource completion/effect path. Render must remain a pure projection.
- Prefer safe tracked reads; use `read_unchecked()` only with a documented need. Most current resource matches can use normal tracked reads.

### Resources, effects, and async work

- Construct resources with explicit reactive keys: `(scope_id, collection, page, filter)` rather than relying on closure capture. Reset page/selection only in response to actual key changes.
- Use a generation token or cancellable task for overlapping requests. Query, Browse, entity loads, directory loads, and search must ignore stale completion. Player already uses a generation token for playlist loads; standardize this pattern.
- Use `use_effect` only to synchronize with an external system: document title, focus, local storage, fullscreen, event listeners, or scroll position. Do not use effects to copy one signal into another when the value can be derived.
- Replace permanent polling/timer loops with lifecycle-scoped tasks that exist only while active. The Player image timer should sleep until the next meaningful tick and stop when paused/non-image/unmounted.
- Keep browser storage as enhancement, not canonical route data. Version and validate stored preferences; catch unavailable storage. Custom SQL links must remain usable without local storage.
- For coroutines, avoid one global serial command queue for unrelated I/O. Tree operations should have typed mutation tasks and per-operation busy state; Upload should use a bounded worker pool. Serial command reducers are appropriate for fast synchronous state transitions.

### Component/render performance

- Keep full objects out of list rows where summaries suffice. Browse/Collection should request or derive stable row summaries; Player already uses lightweight queue entries and virtualizes above 200 items.
- Virtualize Tree list/grid and large Catalog lists; measure row heights and keep accessible non-virtual fallbacks for small sets.
- Deduplicate RPC parsing/invocation in a UI data-access module with typed functions (`query_rows`, `get_entity`, `delete_entity`, catalog load). This reduces inconsistent error decoding and makes cancellation/caching testable.
- Avoid per-row closures cloning complete objects/catalog data. Put action identity in keyed row components and pass small targets.
- Keep CSS global and static; no runtime styling framework. Use CSS container/media queries for layout and `content-visibility` only where it does not harm keyboard/accessibility behavior.

## Accessibility baseline

Target WCAG 2.2 AA:

- Add a skip link, landmark labels, one `h1` per route, route/page titles, and programmatic focus on navigation.
- Provide visible `:focus-visible` for every interactive control; current application CSS only styles the Player resizer focus state.
- Maintain 44×44 CSS pixel touch targets for icon-only controls on coarse pointers.
- Mark active navigation with `aria-current="page"`; toggle groups need pressed state and group labels.
- Icon buttons need both accessible names and tooltips. Several Tree icons currently have `title` but no explicit accessible label.
- Tables need captions or associated headings, scoped headers, responsive handling, and meaningful column names (not lowercase backend keys).
- Async notices need deliberate `status`/`alert` behavior; never announce every progress tick.
- Forms need actual label association, required/optional text, descriptions, field-level `aria-invalid`/`aria-describedby`, and focus to the error summary after invalid submit.
- Dialogs must retain focus and restore it to their trigger. Destructive confirmations must name the target.
- Do not rely on color alone for selected/cut/error status. Respect reduced motion, zoom to 200%, high contrast, keyboard-only use, and screen-reader reading order.

## Responsive strategy

- Design from 320 px upward; current shell wraps a long nav into multiple rows rather than providing mobile navigation.
- Use page-level container queries for toolbars and two-pane content so embedded desktop windows behave like narrow browsers.
- At narrow widths, convert data tables to either horizontal scroll with sticky identity/actions columns or an explicit card/list mode; do not silently hide critical fields.
- Stack form labels above controls below the form container threshold; current fixed-width table headers still consume substantial mobile width.
- Keep bottom/sticky action bars above safe-area insets. Player controls should prioritize transport and now-playing, moving secondary controls to overflow on small screens.
- Verify 320×568, 390×844, 768×1024, 1280×800, 1440×900, and a small desktop WebView window.

## Delivery sequence

### Phase 0 — guardrails and baselines (P0)

1. Fix the reproducible web dev shell version mismatch and document the launch command.
2. Add route smoke tests, screenshot/visual fixtures with seeded data, axe-style accessibility checks, and viewport matrix.
3. Record baseline WASM/CSS sizes, route load/request counts, and render timing for 100/1,000/50,000-row scenarios where applicable.
4. Add a not-found route and a shared connection/catalog error screen.
5. Add reactive correctness tests, then fix catalog render-time writes/reactive keying, O(1) catalog context ownership, Tree VirtualList grow/shrink counts, and stable entity/upload row keys before visual restructuring.

Acceptance: all 13 route variants can be opened from a clean seeded scope in desktop and web targets; CI can test them without manual data preparation.

### Phase 1 — shared frame and state language (P0, quick wins)

1. Introduce tokens, global focus style, semantic status colors, typography/layout utilities, and reduced-motion coverage.
2. Implement `AppFrame`, grouped active navigation, mobile menu, skip link, `PageHeader`, `AsyncState`, `InlineNotice`, and toast infrastructure.
3. Refactor catalog loading so it does not mutate signals during render and retains the shell on error/retry.
4. Add document titles and route focus management.

Acceptance: every route has the same page hierarchy, loading/error/empty language, active navigation, keyboard focus, and responsive shell without materially increasing JS/WASM dependencies.

### Phase 2 — consolidate core data workflows (P0)

1. Create typed UI data functions and consistent request/error mapping.
2. Make Collection reuse `EntityResults`/Browse or become the collection administration page.
3. Add `EntityPageHeader`, mutation feedback, delete pending/error handling, and focused invalidation.
4. Add dirty-state guards, Save/Create naming, success navigation, and stable create IDs to the shared form page.
5. Give Browse portable filter state, query retry/refresh, actionable empty state, and stable result keys.

Acceptance: users can move from list → detail → edit/create → success without losing context; failed requests preserve inputs and offer retry; back/forward restores list state.

### Phase 3 — complex workspace refinement (P1)

1. Refactor Tree operations into independently busy/cancellable tasks; add accessible labels, keyboard navigation, undoable remove where safe, and URL-synced sort/view/page.
2. Refactor Upload into per-item keyed state with bounded concurrency, cancel, drag/drop, clipboard paste, and consolidated progress component.
3. Split Player hot signals, lifecycle-scope its timer, add compact/overflow controls, error recovery, and queue persistence policy.
4. Turn Query into a real workbench with Run/Cancel, shortcut, schema-aware results, history, export, duration, and safe limits.

Acceptance: large lists remain responsive, progress updates do not repaint unrelated panels, and every complex action is keyboard operable with clear pending/success/failure feedback.

### Phase 4 — high-value added functionality (P1/P2)

1. Global search and recent entities.
2. Scope/connection switcher.
3. Structured Browse filters and saved views.
4. Bulk actions and tagging.
5. File analysis action, then shared job infrastructure and maintenance Settings.
6. Importer/blob/duplicate-media tools only after backend contracts are defined.

## Quick wins versus larger changes

Quick wins (days, low architectural risk): active nav, skip link/focus rings, consistent page headers, styled errors/empty states, retry buttons, query running state and shortcut, table wrappers, upload drop-zone copy, clearer form action labels, create ID stability, entity copy-ID action, document titles, and `aria-current`/icon labels.

Larger changes (multi-step): app-frame/sidebar redesign, typed/cancellable data layer, Collection/Browse consolidation, portable structured filters, keyed granular Upload state, Tree command architecture, scope switching, global search, saved views/history, bulk tagging, job infrastructure, and maintenance/import tools.

## Test and quality gates

- Pure unit tests for route/filter serialization, query builders, reducers, stale-response handling, and mutation state transitions.
- Dioxus component render tests for every loading/empty/error/success variant and accessible name/role contract.
- Browser smoke tests for all routes, navigation, create/edit/delete, query, Tree dialogs and keyboard shortcuts, upload retry/cancel, and Player transport.
- Automated accessibility scan plus manual keyboard/screen-reader pass on shell, forms, tables, dialogs, Tree, and Player.
- Visual snapshots at the viewport matrix with representative long labels, deep nesting, no data, many rows, errors, and zoom.
- Performance gates: no new runtime framework; CSS remains modest; no list renders unbounded full objects; no continuous timer/polling while idle; large collections use pagination/virtualization; progress updates stay localized.

## Overall acceptance criteria

- All route documents’ page-level acceptance criteria pass.
- Navigation has clear hierarchy, active state, mobile behavior, search entry point, and current scope/connection context.
- Every async page implements initial loading, refresh, empty, error/retry, and success/mutation states as applicable.
- All interactive functionality is keyboard operable with visible focus and WCAG 2.2 AA contrast/name/role/value expectations.
- Back/forward and copied URLs restore meaningful route state without depending on local storage.
- No broad aggregate signal causes high-frequency state (upload/player progress) to rerender unrelated large regions.
- UI code uses explicit resource dependencies, stale-request protection, pure rendering, stable keys, and scoped effects.
- Desktop and web builds share the same component architecture and pass route smoke tests.
- Bundle and interaction performance do not regress beyond agreed measured budgets; new product features are code-split or deferred behind routes when feasible.

## Open product decisions

1. Is `/collections/:collection` a user-facing record list or an administration page? The answer determines consolidation with Browse.
2. Should custom/raw SQL be shareable in the URL, saved server-side, or referenced by a portable saved-view ID?
3. Is scope switching a normal user task, and what authentication/connection model should the shell expose?
4. Are open-schema extra attributes intentionally supported on create/edit, or should forms remain strictly class-defined?
5. What undo/retention semantics are available for entity and directory deletion?
6. What upload concurrency, resumability, and duplicate policies are safe across web and desktop transports?
7. Which maintenance capabilities have backend contracts now, versus requiring a separate backend roadmap?
