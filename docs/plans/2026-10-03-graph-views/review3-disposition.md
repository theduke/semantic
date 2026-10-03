# Graph views review 3 disposition

Historical record: [review 4](review4-disposition.md) makes graph roots and
query APIs entity-only. The nondefault root routes and identity guards described
below have been removed; these verification results remain historical evidence.

This records the review2 and review3 changes since review1 commit `fbd0157a`. The
project owner's endpoint decision supersedes the collection assumptions in
review1 M4/M5/M13 and review2 H2/M1. Historical verification remains in the
older disposition and testing documents.

| ID | Disposition and evidence |
| --- | --- |
| M1 | Relation endpoints always use `semantic_data::builtin::DEFAULT_COLLECTION`. The loader issues one batched entity request for the deduplicated endpoints, retains catalog metadata for labels and embedded-parent filtering, and keeps active-scope and bounded RPC behavior. Missing rows become `Entity { object: None }` with the same stable ID as a loaded entity. The loader test covers outgoing, incoming and missing endpoints, exact request targets, one batch, expandability, and later `set_object` without changing edges. Hierarchy children and ancestors also use default entities. A nondefault root cannot inherit hierarchy or relation edges for another collection's entity with the same raw ID; targeted regressions cover that boundary. |
| M2 | Native input keeps a FIFO queue during a bounds-dependent IPC request. Pointer-move coalescing and pointer anchors are removed; adjacent monotonic wheels at the same anchor still merge. A queued wheel gates following presses, preserving zoom before pan begins. `OriginRefresh` owns the refresh loop beside `OriginCache`; the component wires callbacks. Queue tests cover mount-response freshness, wheel/press ordering, ordinary pan/drag, reverse movement and cancellation; the native desktop smoke covers shift-first-wheel, immediate wheel then press/pan, drag/edges and final viewport state. |
| M3 | Canvas owns transient drag positions. Explorer pinning and reset methods, the view's drag-to-model callback, and `reset_positions_revision` are removed. The toolbar calls `controller.reset_positions()` once; Re-layout preserves transient drag positions. The optional-controller session supplies one fallback controller and passes that same controller to its canvas. Actual-view tests cover pending-load continuity, loaded fallback layout/root lifecycle and loaded supplied-controller reset; browser checks cover drag, incident edge updates, Re-layout retention and one-call Reset restoration. Explicit durable model pins remain caller-owned and respected. |
| M4 | Deleted `Ambiguous`, candidate merging, `resolved_endpoints` and `reconcile_endpoint`, including node re-keying and provenance/edge repair. Stable default entity IDs need no reconciliation. Removed the obsolete ambiguity tests and cross-collection collision browser fixture. |
| L1 | One `entity_edge_id(kind, source, target)` formats parent and relation IDs for expansion and ancestor insertion. Overflow keeps its separate sentinel ID. |
| L2 | Removed the dead canvas container `onscroll` handler. Web ancestor scrolling still refreshes through the capture-phase window listener; native origin-sensitive input refreshes through the IPC path. |
| L3 | Completed after the user's explicit commit request. The coupled dxgraph API/input and Semantic consumers are committed together to keep their dependencies consistent; validation documentation and smoke harnesses form a separate commit. Original untracked plan/review inputs and unrelated directories remain untouched. No push was performed. |

## Verification

Rust checks and tests use the Nix devshell and the requested quiet short-message
format. The final production sources passed the full workspace default-feature
check; the supported web configuration passed the wasm target check. The
coordinator completed final `cargo fmt`, reviewed the integrated scope, and
verified `git diff --check`.

- `nix develop -c cargo test --quiet --message-format=short -p semantic_ui_core
  --no-default-features`: 122 unit and 22 integration tests passed before the
  final two test-only view regressions.
- `nix develop -c cargo test --quiet --message-format=short -p semantic_ui_core
  --no-default-features graph::view_tests`: all 7 final lifecycle tests passed,
  including loaded fallback-controller layout/root changes and loaded
  supplied-controller reset. These overlap the full suite and are not added
  to its count.
- `semantic_ui --no-default-features --features desktop views::graph::tests`:
  2 route/layout tests passed.
- `dxgraph`: 70 unit and 3 SSR tests passed; supported web-target and native demo
  checks/build passed (canvas worker).
- `nix develop .#ui -c cargo check --quiet --message-format=short`: full workspace
  default-feature check passed (coordinator).
- `nix develop -c cargo check --quiet --message-format=short -p semantic_ui
  --no-default-features --features web --target wasm32-unknown-unknown`: passed.
- The fresh Semantic web preview rebuilt from the final production sources.
- The fresh browser run completed all 33 recorded checks at 1440×1000 and
  390×844 with no page or console errors. Relation and mobile screenshots were
  visually inspected. Output is under `target/graph-views-browser/review3/`,
  using only the owned isolated backend at
  `/tmp/semantic-graph-browser-review3-20261003`. The owned backend and preview
  were stopped after validation; fixture data and artifacts remain isolated.
- Native runtime evidence is under `target/graph-views-desktop/review3/`; see
  [desktop-testing.md](desktop-testing.md) for the commands and actual scope.

## Remaining limits

During a pathological native IPC stall, the FIFO queue can retain every move
sample and every discrete transition until the bounds request returns. This
supersedes review2's bounded move-sample claim. Ordinary pan and drag remain
synchronous outside the bounds-dependent window. Failed bounds reads discard
queued input and cancel active captured gestures. No queue bound or timeout
policy was added in this review.

The browser missing-endpoint relationship row is a browser-only RPC response
fixture: the live typed database correctly rejects dangling references. Its
outgoing/incoming fixtures and endpoint object loads use live RPC. See
[browser-testing.md](browser-testing.md) for the commands and fixture boundary.

Semantic drag positions remain transient; persistence is a future consumer
operation through the public `on_node_moved` notification. Callers with durable
model pins must clear those model pins themselves if they want to discard them.
The canvas reset clears only positions owned by the canvas.

No core schema, migration, relationship index, or RPC feature configuration was
changed. The documented native-target web-feature baseline failure remains
outside this review; the supported web configuration uses the wasm target.
