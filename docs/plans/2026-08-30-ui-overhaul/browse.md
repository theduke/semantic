# Browse — `/browse?:collection&:view&:renderer&:page&:page_size&:sql`

## Current implementation

`BrowsePage` is the strongest conventional data page. It has cards/table modes, 1–3 card columns, raw SQL editing, page size and previous/next pagination, schema-aware entity actions, and URL-backed applied state (`crates/ui/src/views/browse.rs:18-342`). It correctly uses `use_resource(use_reactive(...))` for query/scope (`:46-57`) and quotes default-query collection identifiers (`:412-430`).

## Findings

| Priority | Finding | Evidence / impact |
|---|---|---|
| P0 | Custom links are not portable | URL stores only a local-storage hash; missing/cleared/other-browser storage fails (`browse.rs:35-45, 460-500`). |
| P0 | Query safety is unclear | Custom SQL is sent without a parser-backed SELECT/read-only guard. |
| P0 | Route inputs are unbounded | `page_size` may be zero or huge; URL manipulation can create broken paging or excessive DOM (`:33-34`). |
| P1 | Draft can become stale | `sql_input` initializes once and does not sync when route collection/hash changes (`:35-39`). |
| P1 | Misleading custom-SQL controls | Custom SQL disables pagination but page/page-size controls remain; Apply preserves current page (`:63-65, 153-168, 214-220`). |
| P1 | Incomplete visible state | `renderer` is URL-parsed but has no control; grid columns are ephemeral; zero rows has no empty CTA. |
| P1 | No user-friendly filtering | No search, class/type/tag filters, sorting, total, selection, export, saved views, or collection picker. |
| P1 | Result cloning/keying | All rows are cloned into `EntityList` (`:284-297`), whose loops lack stable entity keys (`ui_core/components/entity/card.rs:178-220`). |

## Target experience

Separate two intentional modes:

1. **Filters:** collection picker, search, class/type, tag, sort/direction; compile a typed model to SQL. Serialize the model directly in portable URL parameters.
2. **Advanced SQL:** shared `QueryEditor`, validated read-only SELECT, explicit Execute/Cancel, non-portable state never masquerading as a shareable URL. Prefer URL-safe compressed query only within a bounded length or a server-backed saved-view ID.

Header: “Browse entities,” current collection, Create/Upload, Save view, Export. Toolbar: search/filter button, view toggle, column/grid density, selection/bulk actions, result count. Pagination must distinguish known total from unknown `has_more` and reset page when applied query/page size changes.

States: initial skeleton, retained-results refresh, empty collection, no filter matches with Clear, invalid filter/query, RPC error with Retry/Edit query, selection/mutation pending, export progress/success.

## Component boundaries and reuse

Consume:

- `PageHeader`, `AsyncState`, `InlineNotice/Toast`.
- `DataToolbar { query, active_filter_count, view, on_* }` shared with Collection/Catalog/Tree where applicable.
- `StructuredEntityFilter` reused by Player queue loading and global saved views.
- `QueryEditor { draft, running, read_only, on_run, on_cancel, on_change }` reused by Query and Player advanced SQL.
- `EntityResults { rows: Rc<[EntitySummary]>, mode, renderer, selection, on_open, on_action }` reused by Collection.
- `Pagination { page, page_size, total/has_more, on_change }` shared with Collection/Tree.
- `SelectionToolbar`/`ExportMenu` reused by Tree/Collection.

Browse introduces/evolves the canonical `EntityExplorer` composition: route model + filter/query + result list. Collection should consume it as a preset rather than fork it.

## Dioxus state architecture

- Router is source of truth for applied `BrowseRouteState`: collection, structured filters/query reference, page, page size, mode, renderer, grid/density when shareability matters.
- Local `Signal<String>` owns only the editable, unapplied SQL/search draft; synchronize/reset it in an `Effect` keyed by applied query identity, or key the editor component by that identity. The effect exists because the draft intentionally mirrors external applied state.
- A `Memo<QuerySpec>` performs typed filter → query derivation and validated input clamping.
- A keyed `Resource` owns remote result state with `(scope_id, QuerySpec, page)`; keep previous `Rc<[EntitySummary]>` during refresh and generation/cancel guard late results.
- Local selection is `Signal<BTreeSet<EntityTarget>>`, cleared deliberately on query identity change, not on every refresh.
- No effect derives row count/next state; compute cheap values from the resource snapshot or memoize expensive projections.
- Stable row key is `(collection,id)`; move `UiCatalog` to `Rc` context; avoid cloning full `Object` rows and per-row catalog/action registries.

## Visual direction

Keep Browse’s existing restrained surface/card language, but simplify the stacked borders and duplicate pagination. Use one strong page title, one compact neutral toolbar, visible active-filter chips, and a spacious result canvas. Cards and table share typography/action placement; selected/filter-active states use tokenized color plus icons/text, not shadow escalation.

## Accessibility/responsive/performance

- Toggle groups expose pressed/current state; filters have names/descriptions; query errors link to editor; results region uses `aria-busy` without hiding existing rows.
- Table has caption/headers and sticky identity/actions or card fallback. At 320 px toolbars collapse to labelled menus, never an unlabeled icon row.
- Clamp page size (for example 25/50/100/200), virtualize when result policy allows larger sets, and request summary fields/total separately where possible.
- Structured filters should debounce only the local suggestion/search preview; explicit applied URL changes prevent a request on every keystroke.

## Missing functionality

Portable structured filters, collection/class/tag pickers, saved/shared views, search/sort, total/direct page, column chooser, selection/bulk actions, export, query history, and renderer control.

## Acceptance criteria

- Copied URLs reproduce the applied view without browser-local storage.
- Invalid/mutating SQL is rejected before RPC; page size and page are clamped/reset correctly.
- External/back-forward route changes synchronize the editor and refetch once; late results cannot overwrite newer state.
- Empty/no-match/error/refresh states preserve clear user action and input.
- 1,000 displayed summaries keep a bounded DOM; entity deletion/reordering cannot transfer row-local dialog state.
- Collection and Player reuse the filter/result/editor pieces rather than reimplement them.
