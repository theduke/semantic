# Step 15: Semantic UI integration, end-to-end tests, docs

Wave W6. Depends on: all previous steps. Precondition: the user's in-progress
graph-views changes in `crates/ui` are committed (see plan.md, execution model).
Agent: opus.

Read: research/semantic-app-ui-layers.md §5.2-5.7 (shell, routing, patterns), §6
(testing and Playwright flow); `docs/plans/2026-09-30-task-system/browser-testing.md`
and `acceptance.cjs`; `docs/plans/2026-10-03-graph-views/browser.cjs` and `desktop.mjs`;
`crates/ui/src/views/tasks/mod.rs` (capability gate), `crates/ui/src/components/shell.rs`,
`crates/ui/src/views/mod.rs`; steps/11, steps/14.

## Goal

1. Mount `semantic_ai_ui::AgentsWorkspace` in `semantic_ui`.
2. Verify the whole system end to end with the fake driver in the browser and desktop.
3. Run a live smoke check with real CLIs where available.
4. Update the architecture docs.

## 1. semantic_ui integration

* Routes: `/agents` and `/agents/:thread_id` (thread ids may contain only
  `[a-z0-9-]`; no b64 encoding needed; assert this in the domain id helpers). Use an
  immersive layout variant (full height, no document scroll) like `PlayerShell`, or a
  new `AppFrameVariant` if the immersive one lacks the standard nav. Keep the primary
  nav visible.
* Nav:
  * add `NavItem::Agents` plus `PrimaryNavLink` (lucide `bot` or similar icon), gated
    by `use_agents_available()` (modelled on `use_tasks_available`, reading
    `capabilities.agents`);
  * add `nav_item_is_active` arms and tests;
  * optional `NewMenu` entry "New agent thread".
* Provide `AgentsNavigation` (maps to `Route`), `RpcAgentsSource::new(rpc client)`,
  the scope from `use_active_scope_id`, and an `AiUiPlatform` implementation:
  * desktop: native notifications via the existing desktop stack if available, else
    Noop; `pick_directory` via `rfd` only if it is already a dependency, else Noop;
  * web: Noop or the browser Notification API.
* Include `semantic_ai_ui::Stylesheet {}` in `AppRoot`.
* Entity integration: the ui_core `UiCatalog` entity actions get "Start agent thread
  about this" for entities, which opens the new-thread view with
  `subject = entity` and the initial message prefilled with an `Entity` mention.
  Optional: if the catalog action API makes this awkward, skip it and note it.

## 2. End-to-end verification (fake driver)

Write `docs/plans/2026-10-03-ai-integration/e2e/agents.cjs`, following the conventions
of the existing `.cjs` scripts, including their `rpc()`/`decode()` helpers. Start the
server with:

```
nix develop .#ui -c bash -c 'SEMANTIC_DATA_DIR=/tmp/semantic-agents-e2e SEMANTIC_PORT=8888 SEMANTIC_AGENTS=1 SEMANTIC_AGENTS_FAKE=1 SEMANTIC_AGENTS_ROOTS=/tmp cargo run --quiet --package semantic_server'
```

Then serve the web UI with `dx serve --web --package semantic_ui --no-default-features --features web --port 8080`.

Scenarios, asserted through role and label locators:

1. The nav shows Agents. Its empty state offers the fake provider.
2. Create a workspace at `/tmp/semantic-agents-e2e-ws`, a `git init` repo seeded by
   the script.
3. Create a new thread with "hello". The streamed answer appears, the run folds after
   completion, and the inbox pill goes from working to done.
4. `/approve`: the request panel replaces the composer. Approve, and the run
   completes.
5. `/ask`: answer two questions via the stepper.
6. `/plan`: the plan card appears and "Implement plan" works.
7. `/slow` then Stop: "Stopping…" changes to the interrupted state.
8. Queue: send while `/slow` runs with queue mode. The queued strip shows, and the
   queued run runs next.
9. Reload the page mid-stream: the timeline restores (snapshot = fake history plus
   live overlay) with no duplicated text, and the live stream resumes.
10. Restart the server mid-run:
    * after restart, earlier turns load from the fake driver's history files;
    * the interrupted run shows "Interrupted because the app restarted";
    * the queue is held with a Resume control;
    * the DB contains no message content (check via `semantic api` query: only
      metadata classes and `input_summary`).
11. A fake run that writes a file (fake script `write_file`): the review panel shows
    the per-run diff. Revert files restores them.
12. At 390 px width there is no horizontal overflow, the inbox sheet works, and there
    are no console errors.

Desktop: adapt `desktop.mjs` for one smoke flow (scenario 3) on the standalone
desktop build (`--features desktop,standalone`). This verifies the embedded client
streaming path and that shutdown kills fake sessions on window close.

Save screenshots next to the script, under `e2e/screenshots/`, git-ignored if large.
Document how to run everything in `e2e/README.md`.

## 3. Live smoke checklist (manual, opt-in)

For each CLI on PATH (claude, codex, gemini, opencode), run one thread in a scratch
repo:

* streaming answer;
* one approval in Supervised mode;
* interrupt;
* resume after closing the session (idle release forced with a small timeout);
* after a server restart, the thread's history loads from the CLI's own session
  storage and matches what was shown live;
* `threads.import` of a session started directly in the CLI;
* the MCP tools are listed by the agent (ask it to call `thread_info`);
* the `ask_user` tool round trip.

Record the results (CLI versions, pass/fail, issues) in
`docs/plans/2026-10-03-ai-integration/live-smoke.md`. Fix small issues directly.
Report larger ones as follow-ups.

## 4. Docs

* `docs/ARCHITECTURE.md`: a new "Agent layer" section describing the crates,
  layering rules and data flow; a pointer from the UI section to `semantic_ai_ui`.
* `docs/agents.md` (new): user and operator guide covering:
  * enabling (`SEMANTIC_AGENTS`), roots, providers, access modes and their meaning
    per provider;
  * MCP tools and capabilities;
  * the security model (K8);
  * where data lives (K4, K12): metadata in Semantic, transcripts in each agent's own
    storage, and what happens when that storage is deleted;
  * data model overview;
  * troubleshooting (stderr tails, raw logs toggle).
* Update `AGENTS.md` project overview to mention the agent crates. Add one line for
  the `semantic.agents` package migration rule: it falls under the same discipline as
  core and base.

## Final review

Afterwards, the coordinator runs a full review with an opus agent using
`/code-review high` over the branch range for this plan. It checks:

* layering rules from plan.md (dependency graph via `cargo tree`);
* no provider-name branching in orchestrator or UI;
* migrations untouched after commit;
* no secrets logged or persisted;
* that tests cover the acceptance lists.

## Acceptance

* E2E script passes locally against the fake driver; desktop smoke passes; live
  smoke results are recorded.
* Docs updated.
* Commits:
  1. "Mount agents workspace in semantic UI";
  2. "Add agents end-to-end scripts and live smoke results";
  3. "Document agent architecture and usage".
