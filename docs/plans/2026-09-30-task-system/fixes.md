# Review fixes and final validation

Completed 2026-09-30. All four required findings in review.md are fixed. This pass changes UI state/integration and a test fixture; it does not change core behavior, types, or migration definitions.

## Changes

- TaskForm now holds its controls in a complete TaskDraft. Save and archive/restore replace the entire draft and baseline from the server response, so unrelated remote changes cannot become accidental local patches. Controlled rich description content refreshes too. A shared payload builder keeps patch generation testable.
- The centrally registered generic delete action excludes tasks and comments using both catalog class metadata and the entity type. This covers cards, browse tables, details, and nested card/dialog rendering. No separate bulk delete exists in these browse flows. Task archive and comment tombstones remain the domain lifecycle. Other entities, including Notes, retain generic deletion.
- The parent selector includes archived candidates, labels them `(archived)`, and explicitly selects the saved parent when asynchronous options arrive. Its mobile field spans the full row so the archived label is visible. Hierarchy continues to use the existing `semantic:parent` attribute.
- Comment reloads follow consecutive chronological pages until the returned new root/reply ID is included, while retaining thread ordering and preventing duplicates. The panel displays a refresh state during loading. Load more uses the number actually displayed, including any expanded post window. Existing CommentTree callers retain their change callback behavior; an optional creation callback allows EntityComments to reveal new replies.
- EntityComments documents its host requirement for a scoped `semantic.app.capabilities` command; CommentsPackage alone does not provide that app integration.
- The existing import cancellation test now warms the plugin runtime alongside the database before timing invocation entry. Plugin startup validates/installs its schema; that fixture setup previously remained inside the timed call. Both five-second cancellation-test deadlines are unchanged. No production import code changed.

## Checks and regressions

All check/test/format commands ran through `nix develop .#ui -c`.

```sh
cargo check --quiet --message-format=short -p semantic_base -p semantic_app -p semantic_ui_core -p semantic_ui
cargo check --quiet --message-format=short -p semantic_ui --no-default-features --features web --target wasm32-unknown-unknown
cargo check --quiet --message-format=short -p semantic_ui_core --no-default-features
cargo test --quiet --message-format=short -p semantic_base -p semantic_ui_core -p semantic_ui
cargo test --quiet --message-format=short -p semantic_app -p semantic_ui -p semantic_ui_core
cargo fmt --all
cargo fmt --all -- --check
git diff --check
```

Passed: base 41 unit + 4 integration; UI 105 unit; ui_core 101 unit + 22 integration; app 66 unit + 6/11/4 integration, with one pre-existing ignored test. Doc-test commands also completed successfully. The three new regressions exercise complete task reconciliation and subsequent title-only patches, delete availability for both domain classes in all three placements (also without metadata, while preserving Note deletion), and root/reply visibility beyond 100 with thread order and finite termination when a reveal target is missing.

An initial combined test run and a serial import-suite retry failed the existing cancellation entry gate during schema/plugin initialization. The fixture warmup above resolved this: the focused cancellation regression passed across all six method/disconnect combinations, and the subsequent full app suite passed. This was not addressed by loosening deadlines or altering runtime cancellation behavior.

After the final mobile field-width presentation change, native and web checks, formatting, and a rebuilt-browser screenshot/persisted-state acceptance passed. The earlier full suites were not repeated for that CSS/class-only adjustment.

## Browser acceptance and evidence

Fresh isolated database: `/tmp/semantic-task-fixes-20260930`; backend 127.0.0.1:8888; rebuilt web UI localhost:8080. The old development fixtures with migration drift were not reused. The fresh fixture was reopened successfully by a restarted server without migration drift. Chromium/Playwright ran through Nix against actual pages; domain RPC calls only seeded the large comment fixture.

Reproduction scripts are saved beside this document:

```sh
nix develop .#ui -c node docs/plans/2026-09-30-task-system/fix-browser.cjs
nix develop .#ui -c node docs/plans/2026-09-30-task-system/fix-generic-browser.cjs
nix develop .#ui -c node docs/plans/2026-09-30-task-system/fix-polish-browser.cjs
```

Passed flows:

- Two tabs: another client changes description, status, priority, progress, due date, and parent; a title-only save becomes clean and reflects all remote fields. A second title-only save preserves them after reload.
- Two tabs: remote description/priority/progress changes are reflected by archive, then preserved by a subsequent title-only save and restore.
- Archive parent → reload child → inspect correct selected `(archived)` option → choose another value and reselect archived parent → save an unrelated title → reload with relationship intact.
- Seed 105 comments → first page shows 100 → post root through UI → all 106 records and new root appear without Load more. Reload → first page shows 100 → reply to a visible parent → all 107 records appear, with the reply immediately beneath its parent in threaded mode and at the end in flat mode. Tombstoning the parent retains its identity and visible reply.
- Generic custom and fallback renderers, card and table views, task and comment details: no hard-delete action. Normal Note detail still opens the delete confirmation, and its browse-table action remains available.
- Final rebuilt UI: archived parent is visibly selected on mobile; loading the remaining seven comments preserves the tombstone and reply; desktop/mobile screenshots refreshed. No horizontal overflow at 390px; page and console error lists empty.

Evidence: `/tmp/semantic-task-browser-evidence/fixes/` contains `result.json` (actual parent/child URLs), `detail-desktop.png`, `detail-mobile.png`, `comments-desktop.png`, `comments-mobile.png`, `workspace-mobile.png`, and `generic-comment-table.png`. Screenshots were visually inspected. The mobile parent label is now fully visible. Browser scripts create their own task fixture; the polish script uses the result from the main acceptance script.

## Retained limitations and extension requirements

- Task filtering still runs in memory, and comment listing still loads attached records individually. These are existing scale limitations.
- Domain write locks still serialize unrelated scopes. Generic low-level database writes remain outside domain invariant enforcement; ordinary UI lifecycle bypasses are now closed.
- Overdue currently uses the UTC calendar day on both server and client. A local/user timezone policy and midnight-boundary tests remain a follow-up; core Date behavior was not changed.
- Library hosts using EntityComments must supply the documented scoped capability integration in addition to CommentsPackage.
- A future Document/content-variant migration must coordinate optional-package dependency declarations with newer base schema/migration order, so later optional-package installation cannot restore an older shared content definition. Frozen historical migrations and helpers must remain unchanged.
