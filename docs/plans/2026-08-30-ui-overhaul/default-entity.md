# Default entity detail — `/entities/:id`

## Current implementation

`DefaultEntityPage` delegates to `EntityPageView { collection: None }`, loads with `semantic.db.get`, and renders a schema-aware `EntityCard` with actions (`crates/ui/src/views/entity.rs:9-75`). Not found, RPC error, and loading are plain text. Delete calls `navigator().go_back()`.

## Findings

- **P0 — reactive load key:** the resource captures plain ID/scope values (`entity.rs:29-40`); explicitly key it by `(scope_id, default collection, id)` and guard stale completion.
- **P0 — unsafe post-delete destination:** `go_back()` can leave the application or land on unrelated content for a direct-opened URL (`entity.rs:60`). Navigate to the originating list when known, otherwise default Browse.
- **P1 — incorrect action placement:** full detail is rendered through `EntityCard`, which requests `EntityActionPlacement::Card`; detail-only registrations may never appear (`ui_core/components/entity/card.rs:84-91`).
- **P1 — weak page identity:** raw ID is the heading and the card repeats title/metadata; there are no breadcrumbs, copy ID/link, refresh, collection link, or clear primary action.
- **P1 — non-actionable states:** load error has no retry; not-found has no Browse/Search action; async semantics are not announced.
- **P2 — missing context:** no backlinks/relations, history/version, neighboring results, or raw/schema inspection toggle.

## Target experience

Use `EntityPageHeader`:

- Breadcrumb: Workspace/Browse → Entities → friendly entity title.
- Title from semantic title/filename/name, with class badge and copyable ID beneath.
- Primary Edit; secondary Copy link/ID, refresh, Open file/external when registered; destructive Delete in overflow.
- Detail content uses the registered class renderer with collapsible Raw fields/Schema information.
- Optional Relations section for outgoing and incoming links once a bounded query/API is defined.

States: skeleton preserving header geometry, refresh indicator over retained data, retry error, actionable not found, delete pending/error/success. Confirm delete must name the entity and explain permanence.

## Component boundaries and reuse

Consume `EntityPageHeader`, `Breadcrumbs`, `AsyncState`, `EntityDetail`, `EntityActions`, `CopyableCode`, `ConfirmDangerDialog`, and notices/toasts. This page evolves `EntityDetail { target, object: Rc<Object>, mode, action_placement, on_action }`; named-collection detail, Tree/Player dialogs, and result previews reuse its renderer body with appropriate mode. The full page, not `EntityCard`, owns page heading/layout.

## Signals/performance

- Key resource by scope and ID; one focused invalidation after edit/delete.
- `EntityCard`/detail should consume a cheap shared catalog reference, not clone `UiCatalog` multiple times.
- Split detail rendering from list-card layout so placement, semantics, and responsive styling are correct without duplicating renderer logic.
- Router owns ID; shared scope/catalog/data client own services; keyed `Resource` owns entity load; local signals own action/dialog pending state. `Memo` derives title/class/target metadata. Delete is a cancellable mutation task; effects only manage title/focus/toast/navigation. No network coroutine is needed.
- Pass stable `EntityTarget` and `Rc<Object>`; detail renderers read cheap catalog context. Avoid complete object/catalog clones per action and use focused invalidation after mutation.

## Visual direction

Separate page identity from entity content: a clean breadcrumb/title/action header followed by one neutral detail surface. Friendly values lead; raw IDs/types are secondary mono metadata. Custom note/media renderers may create visual emphasis inside the content, but actions and field tables retain consistent framing.

## Accessibility/responsive

- One `h1`, labelled metadata/field sections, copy confirmation through a polite live region, and stable focus after refresh/dialog close.
- Entity actions stay reachable at narrow widths through a named overflow menu; long IDs wrap/copy without horizontal page overflow.

## Missing functionality

Copy ID/link, reliable return-to-list, backlinks/relations, history/version/conflict context, raw/schema toggle, file analysis action, and duplicate-as-new.

## Acceptance criteria

- Same-route ID/scope changes fetch exactly once; stale results never display under a newer URL.
- Loading, not found, error/retry, refresh, delete pending/error/success are all explicit.
- Delete never unexpectedly exits the app and list caches are invalidated.
- Detail-only registered actions render in the correct placement.
- Long IDs and rich objects remain usable at 320 px and keyboard-only.
