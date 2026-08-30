# Create entity — `/entities/create`

## Current implementation

The page reads catalog classes/collections, defaults to each first entry, provides two filterable comboboxes, synthesizes a class-specific object/ID, and renders `DynamicClassForm` (`crates/ui/src/views/form.rs:14-144`). The form engine provides typed fields, submit validation, reset, and submit errors (`crates/ui_core/src/form/util.rs:14-132`).

## Strengths

- Schema-driven forms and catalog registries are the correct abstraction; do not replace them with route-specific forms.
- Class/collection comboboxes already have accessible labels and empty options.
- Field-level validation and async submit plumbing exist.

## Findings

| Priority | Finding | Evidence / impact |
|---|---|---|
| P0 | Entity ID is created during render | `new_entity_id()` runs in the render branch (`form.rs:113-120`) using timestamp milliseconds (`:283-289`); rerenders can change identity and clients can collide. |
| P0 | Class/collection change discards draft | The keyed form remounts on `{class.id}:{collection}` (`form.rs:128-137`) with no warning or migration. |
| P0 | No success workflow | Submit resolves to unit; page shows no created link, toast, redirect, or next action. |
| P1 | Defaults are arbitrary | First catalog entries are selected (`form.rs:23-30`), not route/user/schema-aware defaults. |
| P1 | Form semantics are visual | Table header cells label rows but are not consistently associated with controls; required/help/error relationships are weak (`ui_core/form/class.rs:278-287`). |
| P1 | No dirty-exit protection/cancel | Back/navigation can lose substantial input; Reset is immediate. |
| P1 | Mobile layout risk | Form table keeps fixed label/control widths and visible overflow. |

## Target experience

- Route accepts optional `collection` and `class` presets; selection follows explicit route → remembered preference → schema default → first valid option.
- Generate a stable UUID/ULID or backend ID exactly once per draft, stored in form/page state. If backend-generated IDs are supported, prefer that contract.
- Page header provides breadcrumb, “Create entity,” selected class/collection summary, and Cancel.
- Changing class/collection with a dirty draft opens a confirmation offering Keep compatible fields, Discard and switch, or Cancel. Do not silently coerce incompatible values.
- Shared sticky actions: **Create entity**, **Create and add another**, **Cancel**; Reset becomes **Discard changes** with confirmation only when dirty.
- On success: toast, invalidate relevant lists, and navigate to created detail by default; retain “create another” as explicit intent.
- Field shell displays friendly label, required/optional/computed state, description/help, type hint on demand, inline error, and catalog ID tooltip/copy.

States: no catalog classes, no compatible collection, validation errors, submitting, submit error with preserved draft/retry, success/navigation.

## Component boundaries and reuse

Consume `FormPage`, `PageHeader`, `AsyncState`, `FormField`, `FormActions`, `UnsavedChangesGuard`, catalog `ClassPicker`/`CollectionPicker`, and `ToastCenter`. Create evolves `FormPage { mode, identity, header_meta, primary_label, on_submit_outcome, on_cancel }`; both edit variants reuse it. The pickers are reused by Browse/Player/Query helpers. `DynamicClassForm` remains the field renderer and emits narrow dirty/submitting/outcome events instead of exposing its writable root broadly.

## Signals and implementation

- Page signals: selected class/collection and perhaps switch dialog. Form engine owns draft/dirty/field state.
- The stable entity ID belongs to the keyed draft initialization, not to render.
- Derive class/collection options with `use_memo` only if catalog size warrants it. Pass `Rc<UiCatalog>` cheaply.
- Use an `UnsavedChangesGuard` integrated with Router navigation and window close where supported.
- Do not mirror every form field into page signals; retain `dxform` field-level subscriptions.
- Router/query owns optional class/collection presets; shared scope/catalog owns schema/services; local signals own selection, stable draft identity, and switch-confirm dialog. Use a memo for selected class/options, a form-owned async submit task with generation identity, and effects only for focus/title/navigation guard/success navigation. A coroutine is unnecessary.
- Keep form fields keyed by canonical field identity. Avoid rebuilding/cloning class/catalog data on every keystroke; the `Rc` catalog and field-level subscriptions localize rerenders.

## Visual direction

Use a focused form column with a slim context panel/header for class and collection, generous section rhythm, and dense but legible field rows. Required/help/error states use typography and icons in addition to color. The sticky action surface should be quiet until dirty/submitting, with one clear primary button.

## Accessibility/responsive

- Real label/control association, `aria-required`, `aria-invalid`, `aria-describedby`, and focused error summary after invalid submit.
- At compact width, labels stack above controls; sticky actions respect safe-area inset and never obscure the focused field.
- Class/collection switching confirmation restores focus to its trigger.

## Missing functionality

Collection/class presets, draft protection, success routing, duplicate-from-entity, optional templates/defaults, and—only if product policy allows—adding undeclared catalog attributes.

## Acceptance criteria

- Unrelated rerenders never change the draft ID.
- Dirty class/collection/navigation changes cannot silently lose input.
- All invalid fields are programmatically related to messages; first invalid field/error summary receives focus.
- Submit error preserves the draft and allows retry; success navigates to the correct default/named collection route and refreshes list caches.
- Complete creation works at 320 px, keyboard-only, and 200% zoom.
