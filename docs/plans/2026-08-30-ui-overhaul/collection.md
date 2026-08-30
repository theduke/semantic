# Collection — `/collections/:collection`

## Current implementation

`CollectionPage` requests `select * from {collection} limit 100`, then renders a raw table with ID, type, object field count, and Edit (`crates/ui/src/views/collection.rs:9-140`). Create opens the generic create route without preselecting this collection.

## Findings

| Priority | Finding | Evidence / impact |
|---|---|---|
| P0 | Incorrect/untrusted SQL identifier handling | Collection is interpolated without quoting (`collection.rs:118-121`), unlike Browse’s `sql_ident`. Names with punctuation fail. |
| P0 | Resource keys can become stale | `use_resource` captures plain collection/scope values (`collection.rs:12-18`); same-component route/scope changes are not an explicit reactive key. |
| P0 | Silent truncation | Hard limit 100 has no pagination, total, or “truncated” message. |
| P1 | Duplicate, weaker workflow | Browse already provides styled cards/table, actions, paging, and renderer selection. Two list systems drift. |
| P1 | Missing states and styling | Zero rows is a blank table; errors cannot retry; `.semantic-collection` has no CSS; table lacks wrapper/caption. |
| P1 | Create loses context | Generic Create does not preselect collection (`collection.rs:23-29`). |
| P1 | Low-information columns | “fields” is object length; no title, updated date, sort, search, or configurable schema columns. |
| P1 | Missing stable keys | Row components are not keyed by collection/ID; stateful descendants may move after deletion/reordering. |

## Product decision and target experience

Preferred: make this route the canonical collection-scoped entity explorer by reusing Browse’s `EntityResults`, filters, pagination, and URL model. Either redirect to `/browse?collection=...` or render the same feature component under the collection-friendly URL.

If collection administration is important, use tabs:

- **Records:** shared Browse result experience.
- **Schema:** fields, IDs, indexes, constraints, and copyable collection ID.
- **Activity/health:** only when real APIs exist; do not invent synthetic stats.

Header actions: Create in this collection, Upload, Query collection, Copy link. Empty records state offers Create/Upload. Errors preserve filter/page and show Retry.

## Component boundaries and reuse

Consume `EntityExplorer`, `EntityResults`, `DataToolbar`, `Pagination`, `SelectionToolbar`, `AsyncState`, and `PageHeader`. Collection should introduce no second table. If administration tabs are chosen, add `CollectionOverview { collection_schema, counts, on_action }`, reused by Catalog collection detail. Events are typed around route/filter change, entity open, selection, and entity action; do not expose writable result signals.

## Interaction states

- Loading: row/card skeleton with page header still visible.
- Refreshing: retain rows and show progress.
- Empty collection: action-oriented message.
- No filter matches: Clear filters, distinct from empty collection.
- Error: retry and copy/show technical details.
- Mutation success: focused result invalidation; never reload the whole window.

## Dioxus/performance

- Resource key: `(scope_id, collection, page, page_size, filter, sort)`.
- Clamp page size; quote identifiers through one typed query helper.
- Use `Rc<[EntitySummary]>`, stable `(collection,id)` keys, pagination and virtualization for large pages.
- Reuse a single `EntityResults` component rather than clone full `Object`s through parallel page implementations.
- Router owns collection and applied explorer state; shared scope/catalog own services/schema; local signals own only selection/dialog drafts. A `Memo<QuerySpec>` derives the safe query, the keyed `Resource` owns rows, and mutation tasks invalidate it. Effects are limited to focus/title/preference sync; no coroutine is required.

## Visual direction

Use the same result canvas as Browse: collection identity and count in a restrained header, one aligned toolbar, then either dense rows or consistent entity cards. Keep schema/admin tabs visually subordinate. Avoid the current raw HTML-table look and avoid a second collection-specific card style.

## Accessibility/responsive

- Table caption/associated heading, scoped headers, responsive scroll or card switch, visible focus, and meaningful column labels.
- On mobile, keep entity identity and primary action; move secondary actions into an accessible menu rather than hide data silently.

## Missing functionality

Search, structured filtering, sorting, total/count, column choice, selection/bulk actions, export, saved views, and collection-aware Create/Upload.

## Acceptance criteria

- Any valid collection name is safely queryable.
- Pagination never silently omits rows; empty and no-match states differ.
- Create preselects the route collection, and returning after create/edit restores list state.
- Collection and Browse use one result/state/component contract.
- Same-route collection/scope changes refetch exactly once and late previous results cannot overwrite the current route.
