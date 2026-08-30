# Home — `/`

## Current implementation

`HomePage` reads the loaded UI catalog, counts collections/classes/attributes, and renders every collection as a link (`crates/ui/src/views/home.rs:7-46`). The route itself has no network state because the app currently withholds all routes until the catalog provider succeeds.

What works:

- Immediate overview of catalog size.
- Direct links to collection pages.
- No extra data request and negligible runtime cost for small catalogs.

## Findings

| Priority | Finding | Evidence / impact |
|---|---|---|
| P0 | No empty state | An empty catalog produces headings and an empty list, with no create/import/setup guidance. |
| P1 | Prototype presentation | `.semantic-home` and `.semantic-stats` have no CSS; layout falls back to browser defaults. |
| P1 | Weak dashboard value | “Workspace” exposes three inert counts but no scope, connection, recent work, health, or primary tasks. |
| P1 | Catalog scalability | All collection names are cloned and rendered eagerly (`home.rs:9-12, 25-42`). |
| P2 | Poor cross-feature discovery | Browse, Tree, Upload, Create, Player, and Query are reachable only through the overloaded global nav. |

## Target experience

Use the shared `PageHeader` and a compact workspace dashboard:

- Header: active scope/connection, “Search entities,” “Create entity,” and “Upload files.”
- Summary cards: entity/collection counts when cheaply available, catalog counts, last refresh, and health. Counts link to their destination.
- Recent section: recently opened/edited/uploaded entities, stored as a bounded local preference initially; move server-side only if cross-device history is desired.
- Collections: searchable compact list with description/count when available, Browse/Create actions, and progressive disclosure after 8–12 entries.
- Optional “Continue working” cards for saved Browse/Query/Player presets once those features exist.

States:

- Loading: retain the app frame; skeleton summary/list while catalog/scope loads.
- Empty: explain that the workspace has no catalog/collections and provide only valid setup or refresh actions.
- Catalog error/offline: shared connection error with retry; do not replace the entire application with a raw message.
- Ready/no recent items: omit the section or explain how items appear.

## Component and signal blueprint

Consume `AppFrame`, `PageHeader`, `AsyncState`, `SearchTrigger`, `QuickActions`, and `CollectionList`. Home introduces `WorkspaceSummary { metrics, health, on_metric_open }` and `RecentEntities { items: Rc<[EntitySummary]>, on_open, on_clear }`; the summary pattern is reused by Catalog/Settings and recent entities by global search.

- Shared contexts own `Rc<UiCatalog>`, active scope, connection status, and bounded recent-item service.
- Router owns no additional state on `/`; local signals own only collection search and optional disclosure.
- A `Memo` derives filtered/progressively disclosed collection rows from catalog/search; cheap counts come from catalog metadata.
- Use a keyed `Resource` only for real server metrics/recent data `(scope_id, revision)`, retaining prior ready data while refreshing.
- Effects are limited to document title/focus and local recent-history persistence. No effect copies catalog data into signals.
- Pass compact immutable rows; do not clone `UiCatalog` or render an unbounded collection list. Stable collection name/ID keys prevent row state transfer.

## Visual direction

Use a calm two-tier layout: concise page header, then a modest summary strip above a denser collections/recent-work grid. Avoid oversized analytics cards and decorative charts. Primary Create/Search actions receive accent emphasis; counts and health use neutral surfaces, clear typography, and restrained status color.

## Accessibility, responsive, and performance

- Use one route `h1`, labelled sections, real links for navigation, and avoid clickable cards containing nested interactive controls.
- At 320 px, stack actions and summary cards; keep collection names and IDs wrap-safe.
- Derive catalog counts with a memo/shared catalog metadata selector only if catalog updates are reactive; do not clone the complete catalog per card.
- Do not add charting or dashboard libraries. Simple CSS cards and lists are sufficient.

## Missing functionality

- Global search/command palette and recent entities.
- Current scope switcher and connection health.
- Saved/recent views and query history.
- Onboarding for a new or empty workspace.

## Acceptance criteria

- Empty, loading, error, offline, and populated states are explicit and keyboard/screen-reader understandable.
- Primary actions remain visible at 320 px and 200% zoom without horizontal page scrolling.
- A catalog with 1,000 collections does not eagerly create 1,000 prominent dashboard items; search/progressive disclosure is available.
- All count and collection navigation uses valid route links and preserves scope.

## Dependencies

Shared `AppFrame`, `PageHeader`, async/empty states, scope context, and optionally recent-item storage. This is a good early visual proving ground after the shared foundation lands.
