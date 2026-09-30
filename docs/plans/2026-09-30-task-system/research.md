# Task system: initial research (2026-09-30)

## Baseline and constraints

Git status was clean before research. This document is the only research change. Root AGENTS.md applies; no nested AGENTS.md appeared in the initial file inventory. Never alter historical migrations, never aggressively change core types/behavior, use full `Result<T, E>`, and run checks/tests through Nix. Smartedit skill read at `/home/theduke/.agents/skills/smartedit/SKILL.md`; prefer bounded `smartedit ast-print --loc` and compact edits. Nix and smartedit are available.

## Existing architecture and exact footholds

- `crates/base/src/lib.rs`: exports `BasePackage`, currently implements `RuntimePackage` with `LabelContext`, returns base package schema plus label commands. New public `tasks` and `comments` modules fit existing `labels/{model,service,commands,tests}.rs` organization.
- `crates/base/src/bundle/mod.rs`: `root_module()` assembles attributes/classes; `package()` adds shared module and `migrations::all()`.
- `crates/base/src/migrations/mod.rs`: 11 ordered migrations, last `011_note_markdown_default`; notes_v1 and notes_v2 are isolated snapshots. Forward migrations must capture initial task/comment/content definitions in historical helpers rather than call mutable current schema functions indefinitely. Existing package tests assert migration counts and last migration, so update those expectations deliberately without altering old operations.
- `crates/base/src/schema/notes.rs`: Note has optional title, required note_format (text/markdown; markdown default), required note_content, created_at/updated_at defaults. Class is non-strict. Existing Note should remain unchanged.
- `crates/data/src/schema/lowered/{mod,lower,tests}.rs`: embedded `TypeKind::Class` already lowers to stored record with canonical attribute IDs. Do not invent a core content mechanism or change core types. Confirm exact embedded class/union conversion and validation with targeted tests before choosing final content representation.
- `crates/base/src/schema/labels.rs`: relation definition and forward migration exemplar (`UpsertRelationship`); relationship is external, indexed, source collection `DEFAULT_COLLECTION`. Relation definitions are migration/catalog objects, not a Module relation map.
- `crates/base/src/labels/service.rs`: reusable store trait (`select/get/commit`), atomic batches, escaped SQL helpers (`directory_query::{sql_ident,sql_string}`), explicit domain validation and shared async write gate. Label writes illustrate retention of existing timestamps/unrelated attributes. Prefer separate domain service validations rather than generic UI inserts for hierarchy/content/comment operations.
- `crates/base/src/labels/commands.rs`: typed SemanticType/IntoValue/FromValue payloads, RPC command adapters, scope_id support. Good template for task/comment commands.
- `crates/app/src/labels.rs`: request context resolves current db scope into store adapter. App builder in `crates/app/src/command.rs` registers BasePackage automatically (around line 333) for default scopes; nondefault opened scopes do not automatically register base schema. Optional feature initialization must respect this behavior.
- `semantic_data::attr::ATTR_PARENT` is `semantic:parent`, a string parent ID. External relations use ATTR_RELATION_FROM/TO/RELATION; retain collection identity with EntityRef rather than bare IDs for links across collections.

## Recommended domain approach for planner

1. Treat comments and tasks as optional runtime packages/features housed in base crate, with explicit registration/installation and catalog presence gating in UI. Keep shared main-content schema in a reusable base module and define dependency/installation order (base/shared content -> comments -> tasks). Alternative is always-installed schema with optional entrypoint; planner must explicitly decide what “optional” means and document it. Avoid changing BasePackage existing command bounds unless needed.
2. Flexible main content should be an embedded typed entity/object. Start with a Note-shaped content class (format + body and appropriate identity), and use a type/class discriminator so future Document content can be added by a forward migration and renderer dispatch. A string enum plus flat body is insufficient by itself. Decide whether embedding the existing Note class as initial variant causes required field/default issues; test canonical versus class field-name keys. Do not duplicate standalone Note persistence or force content into separate unrelated entity references.
3. Tasks: title, description/main content, status/workflow stage, priority, progress with explicit range, optional due date, optional parent ID for subtasks, creation/update times. Leave a growth path for assignable status entities/workflows, projects, cycles and dependencies; scope first implementation to a cohesive working task system. Use relations for ownership/assignment/dependencies/labels when they represent graph links, parent IDs for single intrinsic task hierarchy. Server validates parent is task, no self-parent/cycles, date/progress/status values and safe deletion semantics.
4. Comments: generic attachment relation from arbitrary entity reference to comment, optionally a distinct reply relation if useful beyond parent ID. Parent ID models single reply parent, relation attaches all comments to a root subject. Cross-thread replies must be rejected; include subject collection in identity. Preserve replies on delete with tombstone (recommended) or explicitly reject parent deletion; never leave orphan trees. Include content, author/creation/edit timestamps, threading toggle in reusable UI. Domain commands commit entity and relationship changes atomically and preserve unrelated fields.
5. Explicitly decide authorization/author source, task progress versus child rollup, completed status/progress interactions, day-only due dates vs DateTime, deletion and edit policy, pagination/filter strategy. Do not claim Linear completeness; deliver a foundation that can extend cleanly.

## UI integration

- `crates/ui_core/src/ui_catalog/notes.rs`: attribute/class/form renderer registration; MarkdownEditor with semantic entity links (`use_semantic_entity_links`), on_change/focus/blur; MarkdownView renderer respects markdown feature with text fallback. Extract/reuse content editor/view without disrupting Note renderer. Public reusable commenting components belong to ui_core; concrete task entrypoint belongs to ui.
- `crates/ui_core/src/ui_catalog/{renderer,render_registry,catalog,defaults}.rs`: custom class renderers and class forms; `ClassRenderContext` provides object/class/catalog context. Add task and comment class renderers plus content renderer dispatch.
- `crates/ui/src/views/mod.rs`: Dioxus Route enum; add /tasks and detail/create route if appropriate. `components/sidebar.rs`, `components/shell.rs`, `views/home.rs`: navigation integration. `views/jobs.rs` shows scoped RPC async loading/errors but is not a suitable visual ambition for tasks.
- `crates/ui/assets/core_styles.css` and ui_core components: existing visual language. Task workspace should have a deliberate page header, clear primary create action, compact status/priority/date/progress rows, useful filters/search/result counts, empty/loading/error states, keyboard accessible forms and restrained colors. Detail surface should keep editable properties alongside content/subtasks/comments. Comment tree requires readable indentation, author/time metadata, reply/cancel actions and no broken narrow layouts.
- ui_core default feature includes markdown, native/wasm dxeditor dependencies differ. UI must compile in web feature and retain no-markdown fallback.

## Verification and browser testing

Nix dev shells: default/ui include UI tooling and native dependencies; base shell is slimmer. Run appropriate commands via `nix develop -c ...` (or `nix develop .#ui -c ...`).

- `cargo check --quiet --message-format=short -p semantic_base -p semantic_app -p semantic_ui_core -p semantic_ui` (select sensible UI features if desktop deps constrain checking).
- `cargo check --quiet --message-format=short -p semantic_ui --no-default-features --features web --target wasm32-unknown-unknown`.
- `cargo test --quiet --message-format=short` scoped to changed crates/domain tests, migrations, RPC/scopes; then required workspace check if practical.
- `cargo fmt --all` after final changes; verify git diff/status.

Meaningful tests: package migration replay/frozen snapshot integrity; embedded content roundtrip/validation; task create/update/filter/progress/date/subtask cycles; comment cross-subject reply rejection; atomic relations and delete semantics; scoped RPC roundtrips. Validate UI DOM and browser behavior rather than mirror implementation unit tests.

`Makefile` documents dev startup. Through Nix, run server with isolated temporary `SEMANTIC_DATA_DIR`, `SEMANTIC_INTERFACE=127.0.0.1`, `SEMANTIC_PORT=8888`, `cargo run -p semantic_server`; run `dx serve --web --package semantic_ui --no-default-features --features web`. `Dioxus.toml` proxies /api to localhost:8888. Existing target/debug/semantic_server and semantic are present but must rebuild changes.

Collaborative browser tools are available through dynamic tool discovery (`ALL_TOOLS` filter name/description for browser): `mcp__t3_code__preview_open/navigate/snapshot/click/type/press/resize/evaluate`, plus recordings. Discover metadata for input shape. Existing Playwright config is dxeditor-specific (`crates/dxeditor/web/playwright.config.ts`), not app e2e.

Browser acceptance: fresh empty task screen; create rich description; status/progress/priority/due date updates persist after reload; filters and search; task detail/subtasks; generic entity comments and nested replies; comments edit/delete policy; unavailable package behavior; responsive desktop/mobile widths and keyboard focus; no console errors or overflow. Capture screenshots/recordings and evidence in scratchpad. Browser testing must actually exercise rebuilt UI/server, not merely serve static mockups.
