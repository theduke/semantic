# Catalog — `/catalog`

## Current implementation

`CatalogPage` clones all collection names and every `(id, name)` pair for classes and attributes, then renders three flat unordered lists (`crates/ui/src/views/catalog.rs:5-57`). Collections are plain text even though Home makes them navigable.

## Findings

| Priority | Finding | Evidence / impact |
|---|---|---|
| P0 | No useful empty/error model | Empty groups silently render empty lists; catalog load failure occurs outside the route with no page context. |
| P1 | Not an explorer | No search, deep link, detail, inheritance, constraints, usage, copy ID, Browse, or Create actions. |
| P1 | Inconsistent navigation | Collections are not links (`catalog.rs:24-30`). |
| P1 | No visual system | `.semantic-catalog` and `.semantic-catalog__grid` have no CSS. Long IDs and large lists are unmanaged. |
| P1 | Unbounded DOM/cloning | Every entry is cloned and rendered on every page render (`catalog.rs:7-18, 27-51`). |
| P2 | Backend-first labels | Raw IDs sit beside names without distinction, context, or copy affordance. |

## Target experience

Build a searchable schema explorer rather than three lists:

- Tabs or a segmented control for Collections, Classes, and Attributes; counts remain visible.
- Shared search with matched text, filters (inherited/custom/computed where applicable), and URL-backed selected tab/query.
- Rows/cards show a friendly title, canonical ID in `CopyableCode`, short description, and key metadata.
- A detail pane/page shows collection fields/indexes, class inheritance/extends/attributes, attribute type/constraints, and usage links.
- Contextual actions: Browse collection, Create this class, copy ID/deep link, inspect raw schema.
- Optional inheritance visualization should be CSS/HTML or a later lightweight view, not a graph library in the initial overhaul.

States: skeleton list, per-tab empty state, no search matches with Clear, catalog error with retry, and a non-blocking refreshing indicator.

## Component and signal blueprint

Consume `PageHeader`, `AsyncState`, `SearchField`, `Tabs`, `VirtualList/ResponsiveDataTable`, and `CopyableCode`. Catalog introduces `CatalogExplorer { catalog: Rc<UiCatalog>, route_state, on_select }`, `CatalogRow`, and `CatalogDetail`; Create reuses catalog class/collection row labels, while form/debug views reuse `CatalogDetail`/copy primitives.

- Router owns selected catalog kind, search/filter, and selected entry if deep linking/back-forward matters.
- Shared catalog context owns immutable `Rc<UiCatalog>` and reload state; local signals own only transient detail-sheet open/focus if not route-backed.
- `Memo<Rc<[CatalogRow]>>` performs expensive indexing/filtering once per catalog revision/query. Virtual count derives from that memo, not an initialization-only signal.
- Catalog refresh is the provider’s keyed `Resource`; the page should not clone it into local signals or write during render.
- Effects handle title/focus and no ordinary derived data. Stable canonical IDs key virtual rows.
- Detail receives one selected ID and looks up through the shared catalog; avoid passing cloned schemas/maps through every row.

## Visual direction

Treat the catalog like a technical explorer: compact search/navigation column, high-density rows, mono IDs as secondary metadata, and a readable detail surface with grouped definitions. Use badges sparingly for type/required/computed state; hierarchy comes from spacing/type, not multiple card shadows.

## Accessibility, responsive, and performance

- Tabs must have correct tab semantics and work with arrows; a simple set of route links is also acceptable and more robust.
- Detail pane becomes a full-width region/sheet on narrow screens. Focus moves to detail heading and returns on close.
- Virtualize or paginate large groups. Keep catalog as `Rc<UiCatalog>` and derive compact `Rc<[CatalogRow]>` once per catalog revision.
- Search uses a memo for the filtered projection; no effect should mirror search results into another signal.

## Missing functionality

- Deep-linkable schema details.
- Inheritance and attribute usage discovery.
- Raw schema copy/export.
- Catalog refresh/status and change/version visibility.

## Acceptance criteria

- Users can find a class/attribute/collection by title or ID and navigate to relevant Browse/Create workflows.
- 10,000 catalog entries keep a bounded DOM and responsive filtering.
- Empty/no-match/error states have clear recovery actions.
- Names, IDs, constraints, and relationships remain legible at narrow widths and 200% zoom.

## Dependencies

Shared page/header, search, copyable code, tabs, virtual list/table, catalog `Rc` ownership, and route/query serialization.
