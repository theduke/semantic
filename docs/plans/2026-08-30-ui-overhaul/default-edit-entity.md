# Default entity edit — `/entities/:id/edit`

## Current implementation

`DefaultEditEntityPage` delegates to `EditEntityPageView { collection: None }`. It loads the entity, resolves its catalog class, renders collection/ID/class metadata, and mounts `DynamicClassForm` in edit mode (`crates/ui/src/views/form.rs:147-245`).

## Findings

| Priority | Finding | Impact |
|---|---|---|
| P0 | Resource dependency is implicit | Plain ID/scope captures can remain stale on same-component route/scope change (`form.rs:167-181`). |
| P0 | No post-save outcome | Successful submit gives no confirmation, detail navigation, or cache refresh. |
| P0 | No unsaved-change guard | Back, nav, or route change can silently discard edits. |
| P1 | Unknown class is a dead end | Object with no registered class form cannot be edited; no guarded raw fallback (`form.rs:228-230`). |
| P1 | Generic action language | “Submit”/“Reset” is less clear than “Save changes”/“Discard changes.” |
| P1 | Error semantics | Errors are not consistently live/focused; loading/not found/RPC error have no retry/recovery. |
| P2 | Metadata duplication | Page heading, metadata table, and form header repeat identity. |

## Target experience

- Breadcrumb/header shared with detail; clear **Save changes**, **Cancel**, and optional **Save and return** behavior.
- Sticky action bar reports Unsaved/Saving/Saved/Failed, but avoids noisy “unchanged” text when not useful.
- On save: refresh/invalidate entity and lists, toast success, and return to detail by default.
- On navigation with dirty fields: confirmation dialog. Cancel returns to detail without relying on arbitrary history.
- Load/not found/error use shared state with Retry and Browse/Search actions.
- Unknown-class object: offer read-only detail and, only if product policy permits, an advanced raw-object editor with schema/validation warning. Do not silently enable unsafe generic editing.
- Conflict/version detection when backend support exists; otherwise clearly document last-write-wins.

## Component boundaries and reuse

Consume `FormPage`, `PageHeader/Breadcrumbs`, `AsyncState`, `DynamicClassForm`, `FormActions`, `UnsavedChangesGuard`, and notices/toasts. Default and named edit co-evolve `FormPage`; Create reuses its field/action/state contract. Props should carry `mode`, target, loaded class/object, action labels, and cancel destination; events should be `on_dirty_change`, `on_saved(EntityTarget)`, and `on_cancel`, not a shared writable page state.

## Signals and forms

- Key resource by `(scope_id, default collection, id)` with generation/cancellation protection.
- Form engine remains owner of field state. Page consumes only dirty/submitting/outcome signals through a small form-page contract.
- Do not rebuild the form from a resource refresh while dirty without a user decision.
- Mutation invalidates focused entity/list resources; do not reload the browser window.
- Router owns ID; shared scope/catalog own services; keyed entity `Resource` owns loaded data; `dxform` owns draft/field state; local signals own dirty dialog and mutation outcome. Use memo for identity/class metadata, a cancellable submit task, and effects only for focus/title/guard/success navigation. No coroutine is required.
- Keep the loaded object in `Rc`, use stable field keys, and prevent progress/status changes from rerendering unrelated field rows.

## Visual direction

Use the same identity/header language as detail, then a focused editable surface. Differentiate immutable identity metadata from editable fields without a redundant metadata table; reserve danger color for errors/destructive actions and show dirty/saving status in the shared action bar.

## Accessibility/responsive

- Programmatic labels/help/errors, `aria-busy` on form during save, focus to first error on failure, and focus restoration after dirty guard.
- Stack labels/actions at compact widths; keep Save reachable without covering fields.

## Missing functionality

Predictable Cancel/success routing, dirty-exit guard, retained retry, conflict/version handling, raw fallback policy for unknown classes, and reusable post-save invalidation.

## Acceptance criteria

- Dirty edits cannot be lost without confirmation.
- Successful save is visible, updates cached views, and navigates predictably.
- RPC/validation failure preserves every entered value and is announced once.
- Same-route ID/scope changes never show the old entity.
- Keyboard-only, screen-reader, narrow-width, and 200% zoom flows pass.
