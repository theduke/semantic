# Graph views review 1: disposition and verification

Implemented the local toolkit and UI fixes from [review1.md](review1.md).
Existing schema migrations, database/core behavior, and unrelated plans were
left intact. The original `plan.md` and `review1.md` remain review inputs.

## Review dispositions

| Item | Disposition |
| --- | --- |
| H1 | Full-detail measurements own layout sizes. Compact/Minimal measurements are ignored, and resize bursts schedule one layout tick. Pure pipeline and browser LOD regressions cover this. |
| H2 | Memoized linear edge grouping uses unordered endpoint pairs and centered lanes, including opposite directions. Lane geometry and self-loop tests cover the cases. |
| H3 | Web input synchronously reads the current mounted container origin. Native input refreshes the origin in an ordered coroutine. The browser verifies wheel anchoring after a container shift without an intervening click. |
| H4 | A pure `CanvasPipeline` reducer owns measurement batches, layout, stored rectangles, revision, and initial fitting. The component applies effects through one helper. |
| H5 | Canvas handlers are named closures; toolkit and Semantic graph RSX were formatted with `dx fmt`. This change does not add repository-wide CI formatting policy. |
| M1 | Component packing uses the square root of summed rectangle areas; a many-component test covers broad row packing. |
| M2 | Edge geometry receives an explicit self-loop flag based on endpoint IDs. Coincident rectangles belonging to different nodes remain ordinary edges. |
| M3 | Explorer edges use presentation-neutral anchors. The view selects directional sides for tree layouts and floating anchors for other layouts, with a focused test. |
| M4 | Unknown target collections become typed ambiguous nodes instead of default-collection guesses. Source collections may come from relation metadata; unique known targets may resolve by identity. A mock records requests and verifies that even an existing default-collection ID is never fetched when its collection is unknown. Collection-aware relationship indexing remains a follow-up. |
| M5 | Documented v1 limitation: hierarchy children and ancestors stay in the parent/child collection. Cross-collection hierarchy refs need collection information or a deliberate cross-collection query design. No schema/index behavior was changed. |
| M6 | Both mode suppresses only `RelationMode::Embedded` relations backed by the exact `semantic:parent` attribute. User relations named `parent` are preserved; a test covers both. |
| M7 | Overflow nodes say `More…`; the unsupported numerical hint was removed. |
| M8 | Loading and expansion flags are node payload state. Render and accessible-label callbacks consume `NodeRenderContext` without reading the explorer. A test verifies an unaffected sibling's payload stays equal. |
| M9 | `LayoutConfig::Custom(Rc<dyn LayoutAlgorithm>)` supports caller engines with pointer identity comparison. README and reducer tests describe and exercise it. |
| M10 | Controller relayout is the single explicit relayout mechanism; activation uses one callback for Enter/double-click. Semantic uses these APIs. |
| M11 | Canvas state, keyboard, pipeline, input/platform adapters, and component packing are private implementation details. Root exports are explicit; `NodeDetail` lives in canvas. |
| M12 | The toolkit's content width cap was removed; Semantic's own node styling retains its width policy. |
| M13 | `EntityNodeData` distinguishes real entity targets, ambiguous endpoints, and overflow. Only real targets expose navigation/focus/expansion. Placeholder IDs cannot alias real entity IDs; collection-qualified graph IDs also avoid slash collisions. Unit and browser tests cover identity/navigation. |
| M14 | One keyed session initializes once with a cancellable future. Its identity includes root, mode, and active scope. A one-item keyed iterator is necessary because Dioxus retains a statically positioned child scope when only its VNode key changes. Reset duplication, generation counters, fallback focus initialization, and unused `set_mode` were removed. Browser mode/root transitions exercise the boundary. Four actual-view VirtualDom regressions verify mode/root/scope remounts, pending-request cancellation, the new scope payload, and layout-change preservation. |
| M15 | Explorer root/mode/limits are private with getters. A narrow `pin` method replaces unrestricted mutable model access. |
| M16 | Explicit follow-up below: repair shared autocomplete query aliases across all callers, then remove the graph-specific canonical-field override. Existing generic query behavior is preserved. |
| L1 | All valid Full-detail sizes update geometry; only relayout uses the size-change threshold, measured against the last completed layout. |
| L2 | Active scope participates in session identity, so switching scope creates a new explorer/source and cancels the previous session's tasks. |
| L3 | Edge labels live on `GraphEdge`, unused hit paths were removed, and the duplicate label stroke declaration was removed. |
| L4 | Strict rectangle overlap is centralized; component gap and fit padding are named constants. Radial focus is optional and root ordering avoids cloning the entire input. Mind-map uses one children copy and indexed node lookup; force computes components without a spanning forest. |
| L5 | Neighbor lookup walks incident edges. Edge access/membership helpers avoid explorer edge scans, and canvas state derives `Default`. |
| L6 | Graph navigation has a distinct Network icon, page/toolbar styling uses CSS, detail controls use `NoDrag`/`NoWheel`, loaded objects with unknown classes are described accurately, hues are explicit, and a context struct bundles load handles. |
| L7 | Test-only JSON dependency moved to dev dependencies; duplicate web feature forwarding removed. Review-scoped clippy cleanups include moved autocomplete tests and needless dereference/conditional cleanup. Unrelated existing workspace warnings are outside this change. |

## Tests and process items

The redundant integration culling file was removed and its bounded 300-node
assertion retained in private state unit coverage. Hard-threshold ignored timing
tests were removed; historical timing measurements remain observations in
[desktop-testing.md](desktop-testing.md), not reproducible benchmark claims.
The tautological query-predicate checks were removed. AST/value round trips
remain, and the KV execution test now exercises the actual relationships query,
including incoming/outgoing edges, the built-in parent relationship, direct
depth filtering, unrelated rows, and the limit. The database query test stays
local to this feature's loader until a broader database test consolidation.

Pipeline tests cover LOD filtering, resize coalescing, small-size geometry
updates, fit ordering, stale timeout/flush tokens, incremental positions,
custom engines, controller relayout, and invalid measurements. Layout and edge
regressions cover packing and lane/self-loop identity. Layout-signature coverage was split into focused payload, drag-position, and
structure tests. Existing collapse scenarios remain; splitting those already
covered explorer scenarios further was a low-priority suggestion and is deferred.
The mind-map test's left/right variable names were corrected.

`browser.cjs` and `desktop.mjs` are explicitly marked as one-off local
verification artifacts. They retain fixture IDs/ports and are not CI or a new
shipping inspection API. The earlier testing documents are labeled historical.
The toolkit README now accurately explains custom engines, measurement policy,
context labels, relayout, and serialization.

## Follow-up work and deliberate limits

1. Extend the relationship index/API to retain endpoint collections, then replace
   ambiguous endpoints with safe schema-aware targets. V1 avoids guessing.
2. Define collection-aware parent refs or an explicit cross-collection hierarchy
   query strategy. V1 hierarchy traversal can omit cross-collection parents and
   children; it does not scan every collection.
3. Fix canonical aliases in the generic autocomplete query for all consumers,
   preserving caller constraints and shared eligibility/filter behavior. After
   that repair, remove `search_fields` and the canonical-filter override added
   for the graph picker. This review does not change generic autocomplete policy.

## New verification after review fixes

Run through the Nix devshell on 2026-10-03:

```sh
nix develop -c cargo test --quiet --message-format=short -p semantic_ui_core --no-default-features
nix develop -c cargo test --quiet --message-format=short -p semantic_ui --features desktop --no-default-features
nix develop -c cargo check --quiet --message-format=short -p semantic_ui --features web --no-default-features --target wasm32-unknown-unknown
nix develop -c cargo check --quiet --message-format=short -p semantic_ui --features desktop --no-default-features
nix develop -c cargo fmt
```

Final Semantic UI core verification passed 118 unit tests and 22 integration
tests, including the four actual-view lifecycle regressions. Desktop UI passed
120 tests. Both supported UI checks passed again after the keyed-session fix.
The native-target web configuration's existing RPC feature mismatch was
previously reproduced on the original baseline and was not changed.

The final Chromium suite used a rebuilt web bundle after the canvas fixes and
keyed-iterator correction, plus the isolated RPC database at
`/tmp/semantic-graph-browser-20261003`. It passed with zero page/console errors
at 1440×1000 and 390×844. In addition to picker, pan, drag/SVG updates,
EntityCard, ancestor/expansion, layout and list interactions, it verifies:

- Full → Compact → Minimal → Full zoom keeps every node's world position.
- A shifted container anchors wheel zoom without a prior click or resize.
- A mode change reloads relations, and focusing a different root fits its graph.
- Ambiguous relation endpoints expose neither Open nor Focus here.
- Empty/populated mobile pages have no horizontal overflow.

Fresh browser evidence is under ignored `target/graph-views-browser/`, including
`result.json`, `fixture.json`, and screenshots. Hierarchy, relation, and mobile
screenshots were visually inspected. Small fitted scales intentionally use the
Minimal representation; the loaded-entity list supplies accessible text and
valid navigation.

The toolkit passed 56 unit tests and 3 SSR regressions, its wasm check, and the
native demo check. Changed graph RSX also passed `dx fmt --check`. A fresh native
300-node demo smoke passed pan, wheel zoom, drag/SVG updates, and a burst of
input events. Evidence is under `target/graph-views-desktop/results.json` and
`demo-300-focused.png`, `demo-300-panned.png`, and `demo-300-dragged.png`.
Historical latency distributions were not remeasured. The scope-switch regression
uses the actual view with pending mock RPC requests in VirtualDom; it does not
claim a browser scope-picker or a multi-scope database fixture was exercised.

Final coordinator validation also passed `cargo fmt`, the workspace
`cargo check --quiet --message-format=short`. Scoped toolkit/UI-core Clippy
also passed during the review; a final combined rerun is recorded by the
coordinator, with unrelated existing workspace warnings left intact.
