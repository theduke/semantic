# Directory tree — `/tree?:root`

## Current implementation

`TreePage` is a thin route wrapper around `semantic_ui_core::DirectoryBrowser` (`crates/ui/src/views/tree.rs:4-8`). The browser implements breadcrumbs, tree/list/icon views, sorting, page size, virtualization, selection, cut/copy/paste, drag-to-move, create/rename/add existing, remove versus hard delete, context menus, keyboard shortcuts, dialogs, and detail (`crates/ui_core/src/components/directory_browser/view.rs:738-1942`).

## Strengths

- Broad, genuinely useful file-manager workflow.
- Separate tree/content load states and explicit destructive distinctions.
- Virtual list/grid and paged content are sound performance foundations.
- Root is URL-backed and dialog primitives provide a base for focus handling.

## Findings

| Priority | Finding | Evidence / impact |
|---|---|---|
| P0 | Virtual counts can become stale | `DirectoryList`/`DirectoryGrid` initialize VirtualList count from props once; page length changes can render missing/out-of-bounds rows (`view.rs:1199-1262`). |
| P0 | Network I/O blocks command processing | One coroutine awaits loads/mutations inline, queueing Escape, selection, close, and navigation behind RPC (`view.rs:148-735`). |
| P0 | Obsolete-load risk | Long tree/content loads need keyed generation/cancellation; current serialized sync does not model retained refresh and per-request identity clearly. |
| P1 | Operation state is unclear | Controls remain active without per-operation busy state; errors may appear in the selection toolbar behind a dialog (`:972-1040`). |
| P1 | Stale dialog drafts | New/rename/add signals can retain earlier values; add query errors collapse into empty results (`:1765-1942`). |
| P1 | Incomplete keyboard semantics | `focused_item` is not used; no roving focus, arrow/range selection, tree/treeitem semantics, or keyboard move alternative. |
| P1 | Pointer/touch gaps | Mouse-style drag lifecycle and limited drop feedback do not form a robust touch/cancel/keyboard workflow. |
| P1 | Accessibility labels | Icon toolbar relies on `title`; expanders/view toggles lack complete labels/pressed/expanded relationships (`:863-969, 1384-1454`). |
| P1 | Scaling limits are silent | Root loads broad entity/link sets; tree children are capped at 200 without a “more” affordance (`data.rs:22, 81-103, 230-269`). |
| P2 | Mobile hides context | Type/ID/order/date are simply hidden (`core_styles.css:1614-1620`). |

## Target experience

- Page header/breadcrumbs with current directory, Search, New directory, Upload here, Add existing, and overflow actions.
- Responsive two-pane file manager: tree collapsible/persisted; content toolbar groups sort/view/page; narrow layout exposes details through a secondary line/sheet instead of removing them.
- Conventional selection: click, Ctrl/Cmd toggle, Shift range, Select all visible, Escape clear; roving focus and arrow/Home/End navigation.
- Drag-and-drop has explicit draggable/drop states and announcements; **Move to…** dialog is the full keyboard/touch alternative.
- Mutations show item-level busy state, disable only conflicts, return toast/inline result, and offer Undo for reversible move/remove when backend semantics allow it.
- Search/filter, inline rename, upload to current directory, breadcrumbs overflow for depth, and direct Open/Edit from detail.

States: initial/refresh tree and content independently, empty root/directory, no search matches, cycle/cap warning, mutation pending/success/error, stale refresh, clipboard state, dialog-specific load/error/empty.

## Component boundaries and reuse

Consume `PageHeader`, `Breadcrumbs`, `DataToolbar`, `Pagination`, `AsyncState`, `SelectionToolbar`, `InlineNotice/Toast`, `ConfirmDangerDialog`, `EntityPicker`, `ResponsiveDataRow`, and `SplitPane`. Tree evolves reusable `FileTree`, `DirectoryPicker`, `MoveToDialog`, and selection/clipboard controller; Upload reuses `DirectoryPicker`, entity forms can use `EntityPicker`, Player/Browse can use file/directory presets.

Suggested boundaries:

- `DirectoryBrowserController`: methods/events, read-only narrow state handles; no UI rendering.
- `DirectoryTree { rows, expanded, active, selection, busy, on_event }`.
- `DirectoryContent { page: Rc<DirectoryPage>, view, selection, clipboard, on_event }`.
- `DirectoryDialogs { dialog, mutation_state, on_submit, on_close }`.
- `DirectoryItemEvent` typed enum for open/select/context/move; avoid exposing all writable signals.

## Dioxus state architecture

- Router owns `root`; add URL-owned sort/view/page/page-size/search when back/share behavior is valuable. Stored preference may supply defaults only.
- `Resource`s separately own current content and expanded tree data, keyed by `(scope, root, page, page_size, sort, search)` and `(scope, expanded IDs/tree revision)`. Preserve ready content during refresh; generation/cancel guard responses.
- Local signals own selection, focus, expanded set, clipboard, drag, current dialog, and per-mutation status. Split by invalidation frequency.
- Replace awaited network work in the coroutine with direct spawned mutation tasks; a lightweight reducer/coroutine may process synchronous typed UI events only. Closing a dialog and Escape must never wait for RPC.
- `Memo`s derive visible IDs, cut IDs, virtual counts, and filtered local rows. Fix counts using a `Memo`/synchronized read signal as Player does; add grow/shrink tests.
- Use stable item IDs, `Rc<[DirectoryBrowseItem]>`, and avoid cloning all page items/selected sets for each row. Effects only handle focus restoration, announcements, persistence, and pointer/DOM integration cleanup.

## Visual direction

Retain a compact professional file-manager density. Tree and content panes use one border/elevation level; selected, focused, cut, dragged, and drop-target states are visually distinct but tokenized. Toolbar icons sit in named groups with tooltips, while destructive actions stay out of the primary cluster. List and grid share the same identity typography.

## Accessibility/responsive/performance

- Correct tree/treeitem or accessible listbox/grid semantics, `aria-expanded/selected/current`, labelled icon buttons, roving tabindex, operation announcements, and focus restoration.
- At 320 px, single-pane navigation with a tree sheet; all metadata remains discoverable. At 200% zoom, no inaccessible off-screen action strip.
- Paginate/lazy-load each tree node, disclose caps, virtualize large content, and measure 10k directory nodes/1k visible items.

## Missing functionality

Search, Upload here, persisted/shareable view state, range selection, keyboard move, inline rename, undo, operation progress, lazy child paging, direct detail Edit/Open, and richer previews.

## Acceptance criteria

- Virtual list/grid grow and shrink correctly across pages.
- Network latency never blocks Escape, selection, focus movement, or dialog close; stale loads never replace the current root.
- All operations have pending/success/error states in their relevant surface and are keyboard/touch accessible.
- Tree caps/pagination are visible; 10k-node fixtures remain responsive with bounded DOM.
- Shared DirectoryPicker/EntityPicker/selection/notice components are reused by Upload and related routes.
