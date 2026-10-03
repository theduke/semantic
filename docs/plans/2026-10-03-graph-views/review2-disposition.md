# Graph views: review 2 disposition and verification

Historical record: [review 3](review3-disposition.md) supersedes the
cross-collection endpoint contract, move-coalescing policy, and shared reset
ownership described here. Current relation endpoints use the default `entities`
collection, and the canvas owns transient drag positions.

This records the changes following [review2.md](review2.md), whose baseline
was commit `fbd0157a`. The original review and plan remain unchanged. The
results here describe the implementation at review 2; the original browser and
Semantic desktop timing sections remain historical evidence.

## Disposition

| Review item | Decision and resulting behavior |
| --- | --- |
| H1: pointer-event bounds round trips | Addressed. Pan and drag moves use cached coordinates without per-move bounds requests. Native zoom-wheel, background world-coordinate input, and pinch start wait for fresh bounds; queued movement samples and compatible wheel events are coalesced while transitions retain their order. Query failure cancels gated gestures. Web refreshes before the first origin-dependent event in a refresh interval, rather than reading bounds on every move. Native measurements and their limits are recorded separately below. |
| H2: outgoing target dead ends | Addressed. The loader batches endpoint ids across the catalog's candidate collections using the same scoped `GraphSource`. Exactly one actual matching object supplies the collection and payload; zero matches or multiple collection matches produce typed, non-navigable ambiguity. Previously loaded nodes are no longer a resolution input. Objects from probes are reused instead of fetched a second time. RPC batches retain the existing 50-id request bound. |
| M1: unstable ambiguous identities | Addressed. Ambiguous ids are `ambiguous:{byte_length}:{raw_id}` and do not include candidate lists or `Debug` formatting. A loader result carries identities verified across candidate collections. Verified resolution replaces the placeholder and rewrites incident edge ids, expansion edge/node ownership, and layout parents, retaining pinned geometry. Loading an explicit entity in one collection alone cannot prove that an id-only endpoint is unique, so it intentionally does not erase a possible collision. |
| M2: duplicate expansion state | Addressed. Loading and expanded flags live only in `EntityNodeData`; explorer queries derive them from the payload. Object replacements preserve those flags, including an ancestor response racing with a prior load. Failure and collapse update that same payload. |
| M3: cloned controller rectangles | Addressed. The pipeline and controller share the rectangle map through `Rc`. Container changes have their own effect, and viewport-limit writes notify only when limits actually change. |
| L1: extra layout after remount | Addressed. Pipelines initialize with the controller's current layout and reset revisions. A remounted session does not replay old commands. |
| L2: dead measurement threshold | Addressed. The state measurement helper stores the size without deciding significance. The pipeline remains responsible for comparing Full-detail layout sizes and deciding whether to lay out. |
| L3: overloaded rectangle effect | Addressed. Container resizing emits `ContainerChanged`; geometry-only updates keep `RectsChanged`. Controller updates preserve the shared rectangle map. |
| L4: permanent pins | Addressed with an explicit **Reset positions** toolbar action. Ordinary Re-layout and layout switches keep pins. Reset positions clears the model's positions/pins and the canvas's moved/stable coordinates, then performs automatic layout without replacing the exploration session. A session-owned controller supports the same reset when callers omit a controller. |
| L5: mutable model identities | Addressed. `node_mut` and `edge_mut` were replaced by payload/style accessors, plus explicit position, pin, and layout-parent setters. Presentation code changes only edge style. Endpoints and map identities cannot be rewritten through these accessors. |
| L6: keyed-session workaround | Retained and documented with the exact upstream Dioxus 0.7.9 keyed-fragment reconciliation source. The one-element dynamic list is still necessary for session remounts; replacing it without changing upstream behavior would reintroduce stale-session loads. Actual view lifecycle tests are retained. |

No review item was dropped. Alternative suggestions that were not selected:

- H2 target-class filtering: the current `RelationType` has a source collection
  but no target collection or class constraint. Catalog probes avoid inventing
  such a restriction or changing persisted schemas.
- M1 merging every explicitly loaded entity: rejected because the same raw id
  can still refer to an entity in another collection. Only a successful complete
  candidate probe supplies the required uniqueness evidence.
- L4 making Re-layout implicitly unpin: rejected in favor of a separate action,
  so users can recompute surrounding layout while preserving their placements.
- L6 deleting the remount workaround: deferred until upstream reconciliation
  provides the required behavior. The source link describes the actual current
  behavior rather than claiming an unverified upstream issue or fix.

Native input retains a narrow limitation: discrete transitions can accumulate
during a pathological bounds-IPC stall because silently dropping press/release
ordering would corrupt gestures. Bounds refreshes have one active task; this
is not a claim that every possible transition queue has an absolute size bound.
Query failure clears gated input, and session unmount cancels its scoped tasks.

The review's missing test requests are covered by origin classification and
refresh-coalescing tests, loader tests for unique outgoing targets in default
and non-default collections, and explorer tests for stable ambiguity plus
verified replacement with shared collapse ownership. Tests also preserve
collisions when explicit entity loads lack uniqueness evidence, preserve
payload state during replacement, and reset pins without clearing expansion.

## Current verification

All Rust commands run through the Nix devshell using Rust 1.96.0 / Dioxus 0.7.9.

```sh
nix develop -c cargo test --quiet --message-format=short -p dxgraph
nix develop -c cargo test --quiet --message-format=short -p semantic_ui_core --no-default-features
nix develop -c cargo test --quiet --message-format=short -p semantic_ui --features desktop --no-default-features
nix develop -c cargo check --quiet --message-format=short -p dxgraph --features demo --examples
nix develop -c cargo check --quiet --message-format=short -p semantic_ui_core --no-default-features
nix develop -c cargo check --quiet --message-format=short -p semantic_ui --features desktop --no-default-features
nix develop -c cargo check --quiet --message-format=short -p semantic_ui --features web --no-default-features --target wasm32-unknown-unknown
nix develop -c cargo fmt
nix develop -c cargo fmt --all --check
nix develop -c dx fmt --file crates/ui/src/views/graph.rs --check
nix develop -c dx fmt --file crates/ui_core/src/graph/view.rs --check
nix develop -c dx fmt --file crates/ui_core/src/graph/view_tests.rs --check
```

Toolkit results: **68 unit tests and 3 SSR tests** passed. Semantic results:
**123 core unit tests and 22 integration tests**, and **120 native UI tests**
passed, for **336 tests** across the reported suites. The existing identity
reconciliation test was also rerun with an explicit default-collection alias;
all 10 explorer tests passed. Native UI/core checks, native example build/check,
the supported wasm checks, the final integrated workspace check, and Rust/RSX
formatting checks passed. The five actual-view lifecycle tests cover root, mode, and
scope remount/cancellation, layout continuity, and reset continuity with the
session's own controller. The existing unsupported native-target web/RPC
feature mismatch was not changed; the supported web check uses the wasm target.

Browser verification uses [browser.cjs](browser.cjs), a fresh web build, and a
fresh isolated server process with database
`/tmp/semantic-graph-review2-20261003` on port 8888. The fixture adds an untyped
collection through the existing public DDL command solely to create a real
cross-collection id collision; application schemas and migrations are unchanged.
The script checks outgoing-target object rendering, expand/open/focus,
collision nodes without navigation controls, preserved pins during Re-layout,
and Reset positions restoring automatic geometry. It retains hierarchy,
ancestor, edge update, LOD, shifted-origin zoom, root picker, layout, keyboard
fallback, and desktop/mobile overlap/overflow checks, and additionally checks
a wheel burst plus the detail panel's NoDrag/NoWheel boundaries.

The final fresh-build browser run passed all **27 boolean checks**, along with
node-count, fit, and pairwise-bound assertions, at 1440×1000 and 390×844, with
**zero page or console errors**. Desktop hierarchy/relation and mobile
screenshots were visually inspected. At the mobile fitted scale, Minimal nodes
remain intentionally compact; the loaded-entity list retains text/navigation.

Browser artifacts are in the ignored `target/graph-views-browser/review2/`: fixture and
result JSON plus desktop/mobile screenshots. Native artifacts and the
remeasurement procedure are documented in
[desktop-testing.md](desktop-testing.md), under the separate Review 2 section.
The new measurements exercise the shared 300-node native toolkit input path;
historical Semantic-native numbers are not presented as new measurements.

The final native debug run also verified first zoom after an 80px container
shift preserves its world anchor, plus pan/zoom/drag, incident-edge updates,
and the final displacement of a 30-move 60Hz pan burst. Its pan median/p95 was
**31.03/43.63ms**, drag **30.87/32.10ms**, burst **528.48ms**, and final input
to DOM **29.52ms**. These include X11 input-tool startup and inspector polling;
they do not isolate IPC latency or establish a speedup against historical runs.

The upstream keyed-list behavior is documented in
[Dioxus 0.7.9's fragment diff](https://github.com/DioxusLabs/dioxus/blob/v0.7.9/packages/core/src/diff/iterator.rs#L8-L29)
and its
[non-reused-key remount branch](https://github.com/DioxusLabs/dioxus/blob/v0.7.9/packages/core/src/diff/iterator.rs#L271-L282).
