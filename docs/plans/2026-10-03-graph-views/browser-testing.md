# Graph explorer browser verification

Current review4 verification uses entity IDs throughout the graph route and
loading API. The root picker uses its default `entities` collection, and graph
URLs contain only root, mode and layout parameters. Legacy `collection` input
is ignored and disappears when the picker or another graph control navigates.
See [review4-disposition.md](review4-disposition.md) for the complete disposition.
The fresh run completed all 39 recorded checks at 1440×1000 and 390×844 with
zero page or console errors. Desktop relation and mobile graph screenshots were
visually inspected. The added checks observe the picker's actual default entity
query and assert collection-free URLs after picker, mode, layout and focus actions.

```sh
nix develop -c bash -c 'SEMANTIC_DATA_DIR=/tmp/semantic-graph-browser-review4-20261003-425877 SEMANTIC_INTERFACE=127.0.0.1 SEMANTIC_PORT=8888 cargo run --quiet --package semantic_server'
nix develop -c dx serve --web --package semantic_ui --no-default-features --features web --port 8080 --open false --watch false --hot-reload false
nix develop -c node docs/plans/2026-10-03-graph-views/browser.cjs
```

The review4 run discovered the T3 preview tools, but status/open reported no
available automation host. Verification therefore used the retained Playwright
script, an owned local UI server, and a fresh isolated fixture database.

Current artifacts are under `target/graph-views-browser/review4/`. The script
uses live typed Label and EntityLabel fixtures for outgoing and incoming
entities. The database rejects dangling typed references, so the script adds
one missing-endpoint relationship row to the browser's RPC response. Its
batched `entities` object lookup remains a real query returning no object.
This covers the unresolved UI shape without weakening database integrity or
creating another collection. It also verifies drag/edge updates, Re-layout
retention, one-call Reset positions, root/mode/layout lifecycle, shifted wheel
anchors, controls, loaded-list keyboard access, and desktop/mobile rendering.

The observations below are historical and retain their original evidence.

These are historical observations from the original implementation, before
`review1.md` fixes. The scripts are one-off verification artifacts with local
ports and fixture assumptions. See [review-disposition.md](review-disposition.md)
for the review fixes and their new verification results. Timing observations
below were not remeasured as part of the review fixes.

Historical review 2 behavior and verification are recorded in
[review2-disposition.md](review2-disposition.md), including resolved outgoing
targets, actual cross-collection collisions, and Reset positions.

Verified on 2026-10-03 with Chromium through Nix, using an isolated server database at `/tmp/semantic-graph-browser-20261003`. Preview status/open both returned `PreviewAutomationNoAvailableHostError`, so the retained Playwright script uses the locally installed Chromium and Playwright module.

Original commands (historical configuration; use the review4 commands above):

```sh
nix develop -c bash -c 'SEMANTIC_DATA_DIR=/tmp/semantic-graph-browser-20261003 SEMANTIC_INTERFACE=127.0.0.1 SEMANTIC_PORT=8888 cargo run --quiet --package semantic_server'
nix develop -c dx serve --web --package semantic_ui --no-default-features --features web --port 8080 --open false --watch false --hot-reload false
nix develop -c node docs/plans/2026-10-03-graph-views/browser.cjs
```

The script inserts a unique typed Label fixture through existing RPC commands: root with four children, ten grandchildren, an ancestor, and incoming/outgoing EntityLabel relations. It only writes to the isolated server on port 8888. Existing migrations and schemas are unchanged.

The final suite passed at 1440×1000 and 390×844, with zero page or console errors. It exercises empty-route root search by ID, initial root plus four children, full initial neighborhood framing, wheel zoom, background pan, node drag with incident SVG path updates, EntityCard selection, ancestor loading, expansion, radial layout and changed node positions, relation loading, mode/layout control synchronization, keyboard access to the loaded-entity list, Focus here, refitting a changed root after panning, root-picker selection, empty state, and populated mobile rendering. Pairwise bounds checks found no overlaps in the initial, expanded, and relation views. Mobile pages had no horizontal overflow.

Screenshots were visually inspected. At small fitted scales, mobile and radial graphs intentionally use the Minimal node representation; the loaded-entity list retains text and navigation access. Evidence is written under the workspace's ignored `target/graph-views-browser/` directory:

- `fixture.json`: generated IDs and root URL.
- `result.json`: assertions, measured bounds, and browser errors.
- `hierarchy.png`, `radial.png`, `relations.png`: populated desktop graphs.
- `mobile-empty.png`, `mobile-graph.png`, `last.png`: mobile states and final screenshot.

Issues fixed during this verification:

- Initial fitting now waits for root neighborhood loading. A generation-aware readiness reducer prevents stale root/URL requests from revealing a newer graph prematurely.
- Canvas identity follows the root, so selecting or focusing another entity fits the new neighborhood after prior panning.
- Layout controls now follow URL/mode changes, including the default Force layout when switching to Relations.
- The graph root picker uses canonical `id` and `semantic:title` fields. Native verification found the shared default short `name` alias ambiguous; existing autocomplete defaults are preserved for other callers. Graph search retains ID matches even when the displayed title differs, respects the configured collection, and can display a selected ID outside the bounded search page.
- Relation styles have stable color classes, and graph nodes provide a keyboard-usable loaded-entity list.
- Canvas node resize propagation was corrected by the graph toolkit worker after native verification found child sizes replacing the viewport size.

Final Semantic checks passed through Nix: `semantic_ui_core` without default features, 111 unit tests plus 22 integration tests; `semantic_ui` with desktop features, 120 tests; supported web-target `cargo check` with `--target wasm32-unknown-unknown`; and `cargo fmt`. The toolkit's independent tests and native desktop/performance evidence are recorded by its worker.

```sh
nix develop -c cargo test --quiet --message-format=short -p semantic_ui_core --no-default-features
nix develop -c cargo test --quiet --message-format=short -p semantic_ui --features desktop --no-default-features
nix develop -c cargo check --quiet --message-format=short -p semantic_ui --features web --no-default-features --target wasm32-unknown-unknown
nix develop -c cargo fmt
```

A web-feature check against the native target fails in the existing RPC client's websocket feature configuration. This was reproduced with the same command on original commit `47169a97` in a detached temporary worktree, then the worktree was removed. The supported wasm target and desktop feature configuration pass; no RPC feature or core behavior was changed to address that baseline failure.
