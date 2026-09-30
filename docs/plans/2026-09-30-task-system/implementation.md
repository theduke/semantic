# Implementation decisions and validation

The base crate exposes `content`, `tasks`, and `comments`. Shared content is an explicitly tagged embedded Note (`kind: note`, canonical Note attributes in `data`) introduced by base migration 012. Standalone Notes retain their schema and forms. Future content variants can extend the variant schema and shared editor/view dispatch through a forward migration.

Optional runtime packages are `semantic.comments` and `semantic.tasks`. The main app registers base, comments, and tasks in that order. Tasks imply comments. Library callers opt into CommentsPackage/TasksPackage themselves and must install base first. Optional packages declare a frozen base attribute dependency module because existing package replay validation requires foreign attribute declarations. Existing historical migration definitions remain unchanged.

Configuration defaults enable both features. `AppConfig::with_tasks(false)` and `SemanticAppBuilder::with_tasks(false)` disable tasks independently; `with_comments(false)` together with tasks disabled disables both. Environment equivalents are `SEMANTIC_TASKS=false` and `SEMANTIC_COMMENTS=false`. Disabling commands does not remove persisted schema or data. Explicitly opened scopes retain their initialization behavior; packages must be installed explicitly there. `semantic.app.capabilities` combines scope catalog and registered handler presence, preventing stale schema from exposing disabled features.

Task and reply hierarchy use the existing `semantic:parent` attribute. Task commands validate every ancestor's existence/type and reject cycles. Replies attach to a collection-qualified root subject through indexed `entity_comment` relations; attachment and comment creation are atomic. Reply parent and request-principal author are immutable. Only the author or system principal can edit/tombstone a comment. Tombstones retain hierarchy and attachment while clearing content.

Task mutations use validated typed commands and preserve unknown fields, original created timestamps, and unrelated fields. Due dates use a local DueDate wrapper over existing Date, avoiding changes to core conversions. Explicit progress is separate from child completion. Done forces 100; reopening defaults to 0; inconsistent explicit input is rejected. Archive is reversible.

Main UI routes are `/tasks`, `/tasks/create`, `/tasks/:id`. Generic task detail/edit routes use validated task forms; comments cannot be created or edited through generic forms. Content and comment components are reusable in ui_core and support no-markdown builds. Task lists and comment panels fetch consecutive bounded pages on load-more, avoiding the server page-size cap silently hiding later records.

## Validation

Initial combined native cargo check passed through Nix for semantic_base, semantic_app, semantic_ui_core and semantic_ui. Web wasm check passed. ui_core no-default-features check passed. Base full suite: 41 unit plus 4 integration tests passed. New ui_core malformed hierarchy tests: 2 passed. New app scope/config/principal integration tests passed; existing package-count assertions are being updated deliberately for the two new enabled packages before final full suite.

Browser verification uses rebuilt isolated server/UI with data under `/tmp/semantic-task-browser-20260930`. Preview tools reported no automation host; Playwright with installed Chromium is the fallback. Detailed flows/evidence are recorded in browser-testing.md after execution.

## Existing limitations retained

Existing low-level database writes do not enforce all attribute constraints or embedded content validation. Task/comment commands enforce these invariants, and specialized generic creation/editing is disabled or redirected. No core query/type behavior was changed to address this pre-existing limitation. Lists currently select class records then perform domain filtering and stable pagination in memory; larger datasets will need query-level optimization. User display names are not invented: comment authors use request principal IDs.

Final app suite passes: 66 unit tests plus 6/11/4 integration tests, with one existing ignored test. The existing import cancellation fixture now initializes its default database before the test starts timing method entry, keeping migration setup outside the cancellation deadline. ui_core full tests: 99 unit and 22 integration tests passed (doc test completion pending at this checkpoint). Final combined native check and no-markdown ui_core check passed after browser polish.

Browser completed: rich task create/reload; empty title error and retained draft; done/reopen progress; explicit progress and due-date clear/reload; two subtasks and one nested child; comments root/reply/cancel/edit/tombstone with intact reply; flat/threaded switch; search/status/priority/sort/clear/no-results; archive/restore; standalone Note comment create/edit/delete/reload; desktop/mobile screenshots and no horizontal overflow. Browser found accessible select name ambiguity and excess readonly editor whitespace, both corrected. No page/console errors during the main acceptance flows.

Final outcomes: ui_core 121 tests passed including all integration and doc-test commands; disabled task navigation/deep link and independent generic comments browser verification passed. Final Nix native check, web wasm check, no-markdown check, formatting and git diff --check passed. Browser evidence and actual scripts are linked/described in browser-testing.md. Independent Astra review and its requested high-effort fix pass remain the parent agent’s next phase.
