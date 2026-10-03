# Graph views review 4 disposition

This records the changes following [review4.md](review4.md), against commits
`97d0666b` and `27281208`. Original review and plan inputs remain unchanged.

| Item | Disposition |
| --- | --- |
| M1: collection-aware graph APIs | Addressed by an entity-only graph boundary. Routes, view/explorer roots, expansion requests, and source/load queries use entity IDs. Source queries use `DEFAULT_COLLECTION` explicitly; collection grouping and nondefault guards are removed. Graph-created `EntityTarget` values use the default collection for navigation and EntityCard. The picker uses its default entity collection. |
| L1: needless struct update | Removed the redundant `..Default::default()` from the complete `ExpansionResult` test value. |

The accepted native FIFO and transient-position limits remain unchanged. No
core schema, migration, or database behavior changes are needed. The completed
changes were committed after the user's subsequent explicit request. No push
was performed.

## Verification

All Rust commands ran through the Nix devshell with
`--quiet --message-format=short` for checks and tests.

- Full `semantic_ui_core --no-default-features` suite: 131 unit and 22
  integration tests passed. The 26 focused graph tests are included in that
  coverage, including recorded RPC requests for default collection, scope,
  50-ID batching, shared hierarchy limits, and empty/zero-limit requests.
- Native UI graph route suite: 3 tests passed, including ignoring legacy
  collection parameters and serializing entity-only URLs. These and the core
  suite account for 156 Rust tests without counting focused reruns twice.
- Core, supported desktop UI, supported wasm UI, and the final default-feature
  workspace checks passed. Final `cargo fmt` and `git diff --check` passed.
- Targeted Clippy reported no graph warnings or `needless_update`; inherited
  warnings in unrelated modules remain in `target/review4-clippy-ui-core.log`.
- Fresh browser verification passed 39 recorded checks at 1440×1000 and
  390×844 with zero page or console errors. Screenshots were visually reviewed.
  Default picker requests and collection-free URLs were verified across legacy
  input, selection, focus, layout, and mode changes; existing endpoint,
  lifecycle, drag/reset, and mobile behavior remained covered.

T3 preview status/open reported no connected preview automation host and
directed using headless Playwright. The retained [browser script](browser.cjs)
was used with an isolated backend at
`/tmp/semantic-graph-browser-review4-20261003-425877`. Browser artifacts are in
`target/graph-views-browser/review4/`; see [browser-testing.md](browser-testing.md)
for the procedure. The missing-endpoint row remains a browser-only RPC fixture
because the live typed database rejects dangling references; incoming/outgoing
objects and source queries use live RPC. All owned preview/server processes
were stopped. Native input behavior was unchanged, so its earlier smoke was
not repeated.

## Remaining limits

During a pathological native bounds IPC stall, the input FIFO can grow until
the query returns; failure cancels gated gestures. Ordinary pan/drag remains
synchronous outside that window. Drag positions are transient across root,
mode, or scope remounts; durable persistence remains caller-owned through
`on_node_moved`.
