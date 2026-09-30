# Task and comment system implementation plan

Date: 2026-09-30. Inputs: user request, root AGENTS.md, `research.md`. This is the implementation handoff; routine details can be adapted to actual APIs, but all acceptance requirements below must be delivered.

## Product and architecture decisions

Deliver a cohesive first version with a durable extension path, not a claim of Linear feature parity. Task tracking includes title, rich description, status, priority, day-only due date, progress, nesting, labels, list filtering/sorting, creation, editing and archive. Comments are reusable on arbitrary entity targets and support composing, editing, deleting and optional threading.

Keep public domain types and behavior in new `semantic_base::tasks` and `semantic_base::comments` modules; place shared embedded-content types in `semantic_base::content`. Prefer model/schema/service/commands/tests files inside these modules, following labels. Do not change core `Value`, `TypeKind`, query semantics, or the existing Note entity.

Use independent schema/runtime packages housed in the base crate: `CommentsPackage` and `TasksPackage`. Shared content is a new forward base migration. Comments depend on base; tasks depend on base and comments. The main application enables both by default, with explicit builder/config/environment options to disable tasks independently and disable both. Tasks enabled implies comments enabled. This makes the feature available in the main app while libraries can opt in to the packages. Document the toggles, package names, and dependencies. Existing explicitly opened scopes must retain their schema initialization behavior; missing packages yield an intelligible unavailable state, not auto-migration of unrelated scopes.

Register packages through the existing runtime package machinery. Preserve `BasePackage`'s existing generic bounds where possible. Feature UI gating must check both available RPC handlers and the active scope catalog: catalog presence alone is insufficient when a previously enabled database retains its schema after the application feature is disabled. Existing command introspection may support this; otherwise add a small app capability command. Do not remove persisted schema/data when disabling a feature.

## Shared main content

Create `MainContent` as an explicitly tagged variant with a first `Note` payload that is an embedded entity/object using `TypeKind::Class` and the existing Note-shaped format/body schema. Use existing `TypeKind::Variant` with a newtype class payload rather than a flat format/body enum. The initial payload can embed the existing Note class, including its required format/body/timestamps; create those timestamps consistently. A later Document variant must be addable with a forward migration and renderer dispatch, without rewriting task/comment service code.

Expose constructors, body/format access, conversion to/from `Value`, validation, and `SemanticType`/RPC conversion. Store canonical attribute IDs inside the embedded class. Use explicit variant conversion if derives do not produce canonical persisted keys. Test actual database insert/load and RPC roundtrip, not only model conversion. Reuse existing lowering/validation behavior. Reject unsupported variants and malformed bodies with stable domain errors. Shared rendering/editor dispatch should handle unsupported variants with a readable fallback rather than panic.

Introduce a reusable main-content attribute in base (`semantic:base:main_content`, or an equally consistent canonical ID). Notes remain standalone entities and retain their existing renderer/forms. Tasks and comments own embedded content; there is no separate Note database insert for their body.

## Tasks

`Task` fields: id, class/type, title, main_content, status, priority, progress, optional due_date, optional existing `semantic:parent`, archived flag, created_at and updated_at. Suggested fixed initial statuses: backlog, todo, in_progress, blocked, done, canceled. Suggested priorities: none, low, medium, high, urgent. These are typed values with schema constraints, not arbitrary strings. Progress is an integer in 0..=100. Due dates use existing `Date`, displayed in the user's calendar without timestamp/timezone shifting. Title is nonblank and trimmed.

Reuse the existing **parent attribute** for subtasks. Tasks initially live in the default collection, so a task parent is another task in the same collection and scope. Never add a second task-parent field or duplicate hierarchy as an external relation. Validate parent type, existence, self-parent and all ancestor cycles server-side. Support nested task creation and parent changes. Reject malformed legacy chains safely. Show subtasks, completion count and links to parent/children in task detail.

Progress is explicit per task. Moving to done forces 100; reopening a done task defaults back to 0 unless a new explicit valid progress is supplied. Reject inconsistent done/progress input rather than silently storing it. Show child completion separately so users can distinguish it from explicit progress. Canceled tasks are excluded from overdue counts. Archive is reversible and does not delete subtasks/comments. Hide archived tasks by default; include archive/unarchive controls and filter.

Reuse existing entity-label relations/components on task detail. Do not store label ID arrays as the authoritative graph. Leave named workflow/status models, projects, cycles, assignees and dependency graphs for later migrations; keep DTOs and services separated so those extensions do not require core type changes.

Provide typed commands for list/search/filter/sort, get/detail including children or a separate child list, create, update, archive/unarchive. Include scope IDs consistently. Prefer patch updates with explicit unset support for due_date/parent; never clobber unknown attributes, original timestamps, or concurrently unrelated fields by reconstructing the entire entity from a stale DTO. Return useful totals and bounded pagination/load-more. Filter at least status, priority, due/overdue, archive and search; support due-date and updated-date ordering with deterministic ID tiebreaks. Escape all dynamic SQL identifiers/values with existing helpers; validate sort keys rather than interpolate caller SQL.

## Generic comments and relations

`Comment` fields: id/class/type, main_content, immutable author_id from the current request principal, existing optional `semantic:parent` for a reply, created_at, updated_at/edited metadata, and deleted/tombstone state. Comments initially live in the default collection. Subject targets have collection plus id; scopes remain explicit at the RPC boundary.

Define a new external indexed `entity_comment` relation. Each comment, including a nested reply, has one attachment relation from its root subject to the comment. Preserve the arbitrary subject collection using declared relation metadata, following existing label membership's collection-qualified identity convention. Two subjects with the same ID in different collections must have completely separate comments. Do not assume a bare relation endpoint string encodes collection identity; use a typed target DTO and collection metadata.

The existing **parent attribute** is the authoritative comment reply hierarchy. A separate reply relation is unnecessary for this version. Validate that reply parent is a comment attached to the exact same subject, is not deleted when creating a reply, and cannot create a cycle. Reply parent is fixed after creation. Root subject must exist. Creation atomically commits comment and attachment relation; never create an unattached comment. Use appropriate async write serialization around read/validate/commit, following labels, with atomic batches.

Edits and deletes require author ownership or existing privileged system principal behavior; source author from context, never a caller-supplied display name. Do not invent a separate user account system. Show author ID intelligibly. A delete clears the body and marks a tombstone while retaining identity and hierarchy, so replies remain readable. Repeat delete is safe. Edited/deleted timestamps and unrelated attributes survive updates. Comments on any valid entity are supported independently of tasks.

Commands: list for target (stable chronological ordering and bounded loading), create with optional parent, edit, delete. Return attachment context as needed by the UI. Flat display and threaded display use the same persisted parent data. Switching presentation must not rewrite hierarchy. Avoid arbitrary list caps that silently hide replies; use load-more and preserve/root-group ordering or initially load complete bounded threads. Defensively render orphan/cyclic legacy data without recursion crashes.

## Migrations

Add a new base forward migration after 011 for shared content. Each optional package has its own initial forward migration introducing all new attributes, classes and the comment relationship. Register declarations in the matching package modules. Never change existing migration operations or meaning.

New migrations must use frozen v1 snapshots/helpers, isolated from current runtime schema helpers. Do not make historical snapshots call mutable `content::schema`, `tasks::schema` or `comments::schema` indefinitely. Follow `notes_v1`/`notes_v2` and relationship `UpsertRelationship` patterns. Keep already persisted migration names and order unchanged; only append/update expected package migration tests. Test replay on a database with the previous base package installed, package dependency order, current schema/migration validation and embedded-content data validation.

## UI implementation and visual requirements

Put reusable `MainContentEditor`, `MainContentView`, `CommentComposer`, `CommentTree`, and a data-loading `EntityComments` integration in ui_core. Editor dispatch uses dxeditor MarkdownEditor with existing semantic entity links; plain text and no-markdown builds receive a usable textarea/view fallback. Share primitives with Note rendering where practical without changing Note behavior. Preserve drafts until successful save; retain them after failed requests. Reset composer after successful submit, show saving state, prevent duplicate submit, support cancel and visible errors. Keyboard shortcuts may supplement, never replace, visible controls.

Register custom class renderers and suitable forms for Task and Comment in the UI catalog. Task renderer gives title, clear status/priority/due/progress metadata and rich content. Comment renderer gives author/time/edited/deleted state and rich content. Ensure generic entity cards/details display these classes well. Route primary task editing through validated task commands instead of generic inserts. Hide specialized classes from generic creation if necessary to avoid creating comments without subject relations.

Integrate reusable EntityComments into the generic entity detail page for all subject types (including Notes), with correct active scope/collection/id. Put comments on task detail too without duplicate panels. Comment tree has restrained connectors/indentation, clear author/time typography, per-comment reply/edit/delete actions, inline reply composer and tombstones. A `threaded` prop controls threaded versus flat display; cap visual indentation on narrow screens while retaining reply context. Stable keyed rows, accessible action names, labeled composer, visible keyboard focus, appropriate buttons, and readable empty/loading/error states are mandatory.

Add a dedicated `/tasks` workspace and task detail/create entrypoints to main UI. Integrate navigation only when enabled and available in the scope. Deep links to unavailable features show a useful unavailable state. Workspace includes intentional page header, result count, primary create action, text search, status/priority/due/archive filters, clear-all action, sort and load-more. Rows show title plus compact aligned metadata, progress and subtask indicator; clicking a row opens task detail. Provide a crafted first-task empty state and separate no-filter-results state.

Detail uses a calm readable description column with a compact property sidebar or panel, parent breadcrumb, title editing, status/priority/due/progress controls, subtasks with add action, labels, archive action and comments. Create has an accessible form/dialog or page, real editor and sensible defaults. Maintain edited drafts while async data refreshes; confirm saving only when server write succeeds. Avoid raw JSON/IDs dominating normal task UX.

Use the app's established spacing, fonts, radius, icon/button system and CSS variables. Add deliberate scoped CSS to core_styles or relevant shared assets. Restrained status colors, strong typography hierarchy, sufficient contrast, consistent alignment, proper hover/focus/disabled states. Verify desktop around 1440px and mobile around 390px; no horizontal overflow, cramped multi-column form or endlessly indented reply tree. Loading, error, filtered-empty and first-use states must feel finished.

## Implementation sequence

1. Domain/schema/content conversions and immutable migrations. Run focused embedded persistence/migration checks before UI work; adapt encoding within the existing type system if necessary.
2. Services, validations, atomic writes, typed commands and app scoped adapters/feature config. Test task hierarchy and generic comment target invariants.
3. Shared content/comment UI, renderers and generic entity comments integration.
4. Task workspace/detail/create/navigation and visual polish.
5. Appropriate Nix checks/tests and formatting, then launch rebuilt isolated server/UI and complete browser acceptance. Save results/evidence paths in this scratchpad.
6. Astra medium independent review; implement every substantive correctness/UX finding with sol 6.1 high, rerun affected checks and browser flows, and update handoff/evidence. Do not end after a partial build or static UI mock.

## Validation and acceptance

Run through Nix when available, per root AGENTS.md:

```sh
nix develop -c cargo check --quiet --message-format=short -p semantic_base -p semantic_app -p semantic_ui_core -p semantic_ui
nix develop -c cargo check --quiet --message-format=short -p semantic_ui --no-default-features --features web --target wasm32-unknown-unknown
nix develop -c cargo check --quiet --message-format=short -p semantic_ui_core --no-default-features
nix develop -c cargo test --quiet --message-format=short -p semantic_base -p semantic_app -p semantic_ui_core
nix develop -c cargo fmt --all
```

Choose `.#ui` devshell where its browser/UI dependencies are needed. Check the workspace after focused checks if practical; record unrelated failures rather than altering unrelated core code. Final diff must contain no incidental historical migration changes or blanket formatting churn. Use full `Result<T, E>` throughout.

Meaningful automated cases: embedded note persistence/RPC roundtrip, malformed format/content rejection, previous base migration upgrade/frozen definitions, package enable/disable, task title/status/progress/date validation, self/cyclic/missing/non-task parent, parent clearing, timestamp/unknown-field preservation, filter/sort/pagination, task archiving; arbitrary collection subject comments, same ID different collection isolation, cross-thread reply rejection, atomic attachment creation, immutable author/parent, ownership checks, edit/tombstone/replies, stable complete tree rendering including malformed legacy input, scoped RPC isolation. Prefer real database tests for schema and query invariants; do not merely duplicate implementation in mock expectations.

Browser testing must run rebuilt actual UI and server against an isolated temporary database. Discover available browser preview tools; use Playwright only if appropriate tools are unavailable. Record commands/URLs and screenshots or recording paths in `browser-testing.md`.

Required browser flows:

- Fresh task workspace shows a polished empty state; create a task with rich Note content, priority, due date; open it and reload to verify persistence.
- Update title/status/progress/date including clearing date; complete/reopen; validate bad/empty input and inspect errors/draft retention.
- Create at least two subtasks, one nested; parent/child navigation, completion summary and progress remain understandable after reload.
- Search, status/priority/overdue/archive filters, sort, clear-all and no-results state; archive/unarchive and verify selection.
- Compose a root comment and nested reply, edit body, delete parent and see tombstone plus intact reply; cancel a reply and ensure draft handling.
- Open a standalone Note and create/edit/delete its comments to prove generic support. Verify flat/threaded presentation and scope/target changes do not leak prior comments/drafts.
- Desktop and mobile screenshots of populated task workspace, task detail and nested comments; inspect scroll/overflow, contrast, alignment and keyboard focus/tab flow.
- Disabled task package hides entrypoint and deep link is graceful; comments still work independently. Inspect browser console/network for errors throughout.

Any failures found during browser testing are implementation work, not merely documentation. Report completed functionality, exact checks, evidence, and only material remaining limitations after review/fixup.
