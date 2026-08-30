# Named-collection entity detail — `/collections/:collection/:id`

## Current implementation

This route uses the same `EntityPageView` as default entities, passing `Some(collection)` to `semantic.db.get` and entity navigation (`crates/ui/src/views/entity.rs:19-75`). Its shared baseline and most recommendations are documented in [Default entity detail](default-entity.md).

## Collection-specific findings

- The heading is raw `{collection}/{id}` and neither segment links back to the collection (`entity.rs:41-48`).
- Route segments/hrefs require robust percent encoding for collection and ID; current string href builders manually interpolate values (`crates/ui/src/app.rs:126-139`).
- Delete’s `go_back()` is especially unreliable because the correct fallback is this named collection, not default Entities.
- References are rendered as links to the default collection only (`app.rs:222-237`); typed references need collection-aware targets to avoid cross-collection misnavigation.
- Collection schema/primary-ID identity should be displayed consistently without duplicating the form metadata table.

## Target experience

- Breadcrumb: Collections → named collection → entity title, with the collection crumb linking to its preserved Records view.
- Collection badge and copyable entity ID in the header.
- After delete, navigate to `/collections/:collection` (or captured origin URL) and announce success.
- Ensure every action and renderer receives `EntityTarget { collection: Some(...), id }` and uses encoded router navigation.
- Offer “Create another in this collection” and “Query collection” in appropriate secondary menus.

## Component and signal blueprint

Consume exactly the shared `EntityPageHeader`, `EntityDetail`, `AsyncState`, actions, copy, confirmation, and notice components from [Default entity detail](default-entity.md); introduce no route-specific renderer. Props/events differ only by `EntityTarget { collection: Some(collection), id }` and named fallback navigation.

- Router owns collection/ID; shared scope/catalog own services; a keyed `Resource` owns the entity; local signals own dialogs/pending mutations.
- A memo derives encoded breadcrumbs/title metadata; delete runs as a cancellable task and invalidates named-collection caches.
- Effects are limited to title/focus/success navigation. Stable target identity keys the detail subtree so changing either segment cannot retain child dialog state.
- Avoid collection/object/catalog cloning; reuse `Rc` data and narrow action events.

## Visual direction

Match the default detail layout exactly, adding only a clearly styled collection breadcrumb/badge. Collection context should not create another nested card or repeat the raw `{collection}/{id}` heading.

## States, accessibility, responsiveness, performance

Use the exact shared state model and performance requirements in [Default entity detail](default-entity.md). Collection and ID must both appear in the accessible page title/breadcrumb without forcing long single-line layout.

## Missing functionality

Collection-aware reference targets, encoded deep links, reliable return-to-collection, copy identity/link, relation/backlink context, raw/schema view, and file analysis where applicable.

## Acceptance criteria

- Collection/ID route segments round-trip safely for all router-supported characters.
- All open/edit/delete/reference actions preserve the named collection.
- Delete returns to the named collection or known origin, never an unrelated route.
- The shared detail component passes `Detail` action placement and does not create a second rendering implementation.
