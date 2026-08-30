# Named-collection entity edit — `/collections/:collection/:id/edit`

## Current implementation

This route passes `Some(collection)` into the same `EditEntityPageView` used by default entities (`crates/ui/src/views/form.rs:157-245`). The shared form findings and desired save/dirty/error behavior are described in [Default entity edit](default-edit-entity.md).

## Collection-specific findings

- Heading uses raw `{collection}/{id}`; metadata repeats this without navigation (`form.rs:182-210`).
- Resource keys must include both collection and ID; a scope or route change must never retain a prior entity/form.
- The collection determines the primary-ID field and submit destination (`form.rs:191-226`), so stale collection/catalog state is a data-integrity risk.
- Cancel and post-save routing must target the named-collection detail, not the default entity route.
- Router segments and generated links must encode collection/ID safely.

## Component and state blueprint

Consume `FormPage`, `PageHeader/Breadcrumbs`, `AsyncState`, `InlineNotice/Toast`, `UnsavedChangesGuard`, and the existing `DynamicClassForm`. Evolve `FormPage` here and on default edit/create; props should include mode, entity target, title/meta slots, submit labels, cancel target, and success event. It should expose `on_saved(EntityTarget)` and `on_dirty_change(bool)` rather than the whole writable form signal.

- Router owns `collection` and `id`.
- Shared scope/catalog contexts own scope and immutable schema.
- A keyed `Resource` owns loaded object state: `(scope_id, collection.clone(), id.clone())`.
- `dxform` owns fields/validation/dirty/submitting state.
- Local `Signal` only owns dirty-guard/dialog state and submit outcome.
- Use a mutation task/generation guard for submit; an `Effect` is only appropriate for focus, title, navigation guard, or success navigation—not deriving collection/class metadata.
- Pass cheap `Rc<UiCatalog>` and stable `EntityTarget`; avoid cloning complete objects/catalog on every keystroke.

## Visual direction

Match the default edit surface and add collection context only through the breadcrumb/badge. Eliminate the repeated collection/ID/class metadata table; show immutable identity compactly above a focused form, with a single sticky save surface and clear dirty/saving/error styling.

## Target states and interactions

Loading skeleton; actionable not found; error with retry; clean/dirty; saving; validation/RPC error with retained draft; saved confirmation; dirty navigation confirmation. Unknown class uses a deliberate read-only/advanced fallback policy.

## Accessibility/responsive

Breadcrumb links to the named collection/detail, proper label/error associations, focused error summary, sticky safe-area action bar, and wrap-safe identity.

## Missing functionality

Named-collection-aware presets/routing, dirty-exit protection, success/cache invalidation, retry/recovery states, conflict/version handling, and a policy for unknown-class objects.

## Acceptance criteria

- Resource/submission always use the current `(scope, collection, id)` and late stale results are ignored.
- Save updates the correct named collection and routes to `/collections/:collection/:id`.
- Cancel/dirty guard never falls through to default collection or arbitrary history.
- Shared form/page components are reused by both edit variants and Create without duplicating field state.
- Narrow/zoomed/keyboard/screen-reader requirements from [Default entity edit](default-edit-entity.md) pass.
