# Independent implementation review

Reviewed 2026-09-30 against plan.md, research.md, implementation.md, browser-testing.md, root AGENTS.md, tracked diff and new implementation files. No implementation changes made. Recommendation: fix the two P1 findings before completion, then address the bounded P2 UI issues below.

## Required fixes

### P1 — Successful task saves can turn unrelated concurrent updates into destructive local patches

`crates/ui/src/views/tasks/detail.rs:260` updates `title` and `baseline` from the returned task, but leaves content, status, priority, progress, date and parent controls unchanged. The next patch compares these stale controls to the new baseline. Consequently, a field changed by another client becomes an apparent local change and is overwritten on the next save. The archive success path has the same reconciliation problem.

**Independently reproduced in the running browser:** two tabs opened the same task with priority `none`. Tab B saved `urgent`. Tab A saved only a title change. A then displayed priority `none` and “Unsaved changes”, although its successful response contained `urgent`. A's next title save restored the database priority to `none`. Output: `AFTER_SAVE {"displayed":"none","server":"urgent","dirty":1}` / `AFTER_SECOND_SAVE none`.

Reconcile all controls with the returned task after a successful save/archive, or maintain a precise local dirty-field model. Controls are disabled during writes, so complete reconciliation is practical. Add a two-client regression covering save and archive; assert a successful save becomes clean and a subsequent title-only save preserves unrelated remote changes. Reusable controlled editor state must reflect reconciled content too.

### P1 — Generic entity actions still offer hard deletion for tasks and comments

`crates/ui/src/views/entity.rs:158` excludes delete only on the comment detail surface. `crates/ui_core/src/ui_catalog/defaults.rs:168` still enables the global delete action for every nonempty entity ID. `crates/ui_core/src/components/entity/card.rs:294` and `:316` render those actions in browse rows and cards. The button uses `semantic.db.delete` (`components/entity/actions.rs:129`), rather than task archive/comment tombstone commands.

This exposes an ordinary UI path that removes task/comment identity and bypasses the new domain lifecycle. Hard-deleting a task parent breaks child hierarchy; hard-deleting a comment removes the intended tombstone context. This finding is established by the action registration/call chain; destructive browser execution was unnecessary.

Disable or replace generic destructive actions for these classes centrally, across card/table/detail/dialog placements. Keep task archive and comment tombstoning in their domain flows. This is a focused UI integration correction, not a request to redesign low-level database authorization or deletion. Add action availability coverage for both classes and a browser check in generic browsing.

### P2 — Archived parents disappear from the parent selector

`crates/ui/src/views/tasks/detail.rs:163` loads parent options with default `include_archived: false`; the select at `:298` uses the persisted parent ID as its value. Archiving a parent intentionally retains its children, so opening a child then has a selected value with no matching option. The selector cannot correctly display the existing relationship and cannot explicitly reselect an archived parent after another choice.

Ensure the current parent is always represented, including an archived indication. Either include archived candidates with clear labeling or fetch/pin the selected parent separately. Test archive-parent → reload-child → inspect parent → edit an unrelated field, preserving the relationship. Static code finding, not yet independently browser-reproduced.

### P2 — Posting after the first comment page clears the draft without showing the new comment

`crates/ui_core/src/components/comments.rs:54` initializes a 100-comment visible limit. The loader starts again at offset zero; successful create (`:146`) increments revision/composer key without changing that window. For a subject already containing 100 comments, posting a root comment or a new reply succeeds, clears the composer, and reloads the same first 100 records. The submitted comment remains invisible until Load more is clicked, potentially repeatedly. Reply insertion also fails to appear beside its visible parent.

On success, reveal the returned comment and its thread, or expand/load the required window with a clear loading state. Preserve chronological/thread ordering and avoid duplicates. Add a >100-comment integration/browser regression for both root posting and replying to a visible parent. Existing two-record pagination tests do not exercise this behavior. Static code finding.

## Verification and assessment

- Existing base migrations are unchanged in meaning in the reviewed diff: the migration list only appends 012 and adds frozen content helpers. New task/comment definitions have separate v1 snapshots; current task status derives are not used to construct their historical enum lists. No core Value/TypeKind/query change was introduced.
- Existing `semantic:parent` is authoritative for tasks and comment replies. Ancestor walks reject self/cyclic/missing/wrong-class task parents and cross-subject or malformed reply chains. Comment entity plus attachment is committed in one batch. Collection-qualified subject identity, explicit scope routing, principal-derived authors and domain edit ownership have meaningful tests.
- Embedded content uses an adjacent tag with an embedded Note class, canonical attribute keys and timestamps. UI fallback exists for unsupported values. Service code mostly operates on the shared content abstraction, providing the intended later variant extension point.
- Feature capabilities combine registered handler and scoped catalog presence. Tasks imply comments; disabling retains schema/data. Scoped tests cover explicit installation and retained-schema disabling. Specialized detail/edit routes are integrated, subject to the generic action gap above.
- Domain tests use real embedded storage and cover progress transitions, cycles, content conversion/persistence, preservation of unknown attributes, dates, comments, isolation, upgrades and pagination. App tests cover request principals and scopes. UI hierarchy tests cover cycles/orphans. Full check/test counts in implementation.md are prior implementation evidence; this review did not rerun those suites. The server was rebuilt/run through Nix and `git diff --check` passed.
- Inspected supplied mobile screenshot: task metadata, description, child summary, tombstone and reply are legible, with no apparent horizontal overflow. Existing recorded flows cover substantial happy-path desktop/mobile behavior. The independent two-tab test adds a realistic missing state-management case. Visual evidence predates the very last label/group polish, as browser-testing.md already states.

## Limitations and follow-up boundaries

The in-memory task filtering and N+1 comment loading are known scale limitations, not blockers for this first version. Global per-domain write locks are conservative but serialize unrelated scopes. Generic low-level database mutations remain outside domain invariant enforcement; no aggressive core change is requested.

Due dates retain their calendar date when displayed, but overdue calculation uses UTC today on server and client (`tasks/service.rs:145`, `views/tasks/mod.rs:186`). Around local midnight users can see overdue classifications a day early/late. Clarify a timezone policy and add boundary tests as a follow-up or focused improvement; avoid changing core Date behavior.

Reusable EntityComments currently requires the application-specific `semantic.app.capabilities` command. A library host registering only CommentsPackage will need that integration as well; document it or expose an availability override/provider if standalone reuse is intended. This is an integration limitation, not evidence the main app gating is broken.

The optional package dependency declarations currently re-upsert frozen shared content/Note attributes. A future content variant migration must coordinate those declarations and migration order, so installing an optional package after a newer base does not restore an older shared definition. No current variant extension was requested; record this requirement when adding Document.

## Browser environment and reproducibility

The previously documented `/tmp/semantic-task-browser-20260930` fixture could not be reopened by either existing or freshly rebuilt server: `applied migration 'tasks::001_tasks' for package 'semantic.tasks' differs from the stored definition`. This is drift in a new development fixture migration; it does not demonstrate modification of pre-existing historical base migrations. It does mean the old fixture cannot currently substantiate a fresh run of the final source. Recreate a fresh fixture and rerun final acceptance after fixes rather than modifying persisted migrations.

For independent reproduction, started a fresh isolated server with `SEMANTIC_DATA_DIR=/tmp/semantic-task-review-20260930`, interface 127.0.0.1, port 8888. Server session: **29341**. Existing UI remains on **8080**. Review fixture contains a task named “Review concurrency”; no real user database was touched.

Independent script: `/tmp/semantic-task-browser-evidence/review.cjs`; screenshot: `/tmp/semantic-task-browser-evidence/review-detail.png`. Run with `nix develop .#ui -c node /tmp/semantic-task-browser-evidence/review.cjs`. It creates its own task, so it does not depend on old fixture IDs. It prints the observed lost-update state and restores the task title. The supplied evidence screenshots remain in that same directory.

## Final focused re-review — approved

Reviewed the fixes on 2026-09-30 against `fixes.md`, final source, regression assertions and `/tmp/semantic-task-browser-evidence/fixes/result.json`. All four required P1/P2 findings above are resolved:

- Save and archive/restore now replace the complete `TaskDraft` and baseline from the returned task. Subsequent patch construction preserves unrelated returned fields, including rich content.
- Central generic delete eligibility excludes tasks/comments using both resolved class and raw entity type, covering every registered placement while preserving other classes' actions.
- Parent options include archived tasks, label them and explicitly select the persisted parent after asynchronous loading.
- Root/reply creation propagates the returned ID into comment loading, which continues until that ID is visible or the available records are exhausted. Load more advances from the actual visible count; existing CommentTree callback behavior remains supported.

The import cancellation fixture warmup is appropriate: it initializes database/plugin schema before the timed controlled invocation, leaving production behavior and both cancellation deadlines unchanged. The reported focused six-case and full-suite passes substantiate the adjustment.

No material remaining defect was found in this focused pass. Full suites and browser flows were not repeated here; the fix agent's recorded checks, regression tests and rebuilt-browser evidence cover the four scenarios, including fresh-fixture restart. `git diff --check` also passed during re-review. The earlier fixture/server details are historical; final evidence uses `/tmp/semantic-task-fixes-20260930` as documented in `fixes.md`.

Retained nonblocking limits remain accurately documented: in-memory filtering/N+1 loading, per-domain write serialization, low-level mutation boundaries, UTC overdue policy, the host capability requirement and coordination needed for future content migrations. Approval is for the completed first-version scope, with those explicit limits.
