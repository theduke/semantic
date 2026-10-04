# Graph Views (Mind Maps / Connection Graphs) Implementation Plan

Date: 2026-10-03

Status: proposed

Scope: a generic, content-agnostic Dioxus graph canvas crate (`dxgraph`) with
automatic layout, plus a first Semantic integration: an entity graph explorer
for hierarchy (`semantic:parent`) and relationship edges at a new `/graph`
route.

## 0. Decisions (confirmed)

| Topic | Decision |
|---|---|
| Rendering | DOM-based. Nodes are absolutely-positioned HTML elements rendered by Dioxus; edges are an SVG layer rendered by Dioxus; both live inside one CSS-transformed "world" element. No `<canvas>`/WebGL. |
| Content | `dxgraph` is **content-agnostic**: it renders whatever `Element` the caller returns for a node. It knows nothing about entities, classes, media, or the editor. |
| Interaction | **No raw JS.** Gestures (pan, zoom, pinch, node drag, click-vs-drag disambiguation) use Dioxus's own cross-platform events (`onpointer*`, `onwheel`, `onresize`, `onmounted`) and feed a pure-Rust gesture state machine. web-sys is used only for wasm-only extras (pointer capture) behind a `web` feature. Custom JS through `wasm-bindgen` is allowed only as a last resort and must be justified in review. See §4.6. |
| Layout | Rust-native, in `dxgraph::layout`, with no Dioxus dependency. The modules are pure and deterministic, and they are unit-tested natively. The `LayoutAlgorithm` trait leaves room for an external engine (e.g. elkjs) later. |
| Data | Loaded client-side through a `GraphSource` trait in `semantic_ui_core`, implemented on top of the existing `semantic.db.query` RPC. No backend, RPC, or schema changes. **No migrations.** |
| v1 node content | A compact node shows a class accent, the title, the class name and the id. Clicking it opens a side panel with the existing `EntityCard`. |
| v1 entry point | Dedicated `/graph` route only. |
| Persistence | None in v1. View state is ephemeral and partially encoded in the URL. The in-memory model can be serialized and is shaped to map onto JSON Canvas later (see §9). |

## 1. Goals and non-goals

### Goals (this effort)

1. A reusable `crates/dxgraph` crate, in the same spirit as `dxeditor`/`dxcomp`:
   - a generic graph model (`GraphModel<N, E>`);
   - a `GraphCanvas` component with pan, zoom, fit-to-view, selection, node drag, culling and level-of-detail;
   - rich arbitrary HTML node content, sized automatically by measurement;
   - SVG edges with straight/bezier/step paths, arrowheads and labels;
   - pure-Rust automatic layouts: tidy tree, mind map (left/right), force-directed and radial.
2. A Semantic entity graph explorer:
   - **Hierarchy mode**: starting from a root, follow `semantic:parent` downward (children) and upward (ancestors).
   - **Relations mode**: show direct relationship edges (`__semantic.relationship_edges`, `depth = 1`) around a focus entity.
   - **Exploration**: expand or collapse a node on demand. Already placed nodes keep their positions (mental-map preservation).
   - Clicking a node shows the `EntityCard` in a side panel with "Open" navigation.
3. Tests at every layer: unit tests for geometry/layout/state, SSR tests for components, and a browser smoke test.

### Non-goals (explicitly deferred, but designed for)

- Creating or editing mind maps, connecting nodes by dragging handles, and persisted canvases. A schema migration in `crates/base` will be needed later; see §9.
- Rich media/editor content *inside* graph nodes. The canvas supports it; the v1 entity node just doesn't use it.
- Minimap, compound/group nodes, a layered (Sugiyama) layout, an ELK backend, and SVG/PNG export.
- A server-side neighborhood RPC command.
- Graph views inside `/browse`, the entity page or the tree page.

## 2. Research summary: how similar systems do it

### 2.1 Rendering approaches

| System | Node rendering | Edges | Notes |
|---|---|---|---|
| **React Flow / Svelte Flow (xyflow)** | HTML `div`s, absolutely positioned inside a viewport `div` with `transform: translate() scale()` | One SVG layer in the same transformed space | This is the de facto standard for rich-node graphs. During panning the viewport transform is written straight to the DOM (via d3-zoom), so React does no work per pan frame. Each node is measured with a `ResizeObserver` after mount, and its dimensions are stored in state. The pipeline is "initialize → measure → layout → render". `nodrag`, `nowheel` and `nopan` class markers opt node sub-elements out of gestures. |
| **Obsidian Canvas / JSON Canvas** | HTML nodes (markdown, files, embeds, links, groups) | SVG | JSON Canvas is an open format: nodes have `id,type,x,y,width,height,color`; edges have `fromNode,toNode,fromSide,toSide,fromEnd,toEnd,label,color`; array order is z-order. It is a good target shape for future persisted mind maps. |
| **tldraw (Logseq/Heptabase whiteboards)** | HTML/SVG shapes in a transformed layer | SVG | Uses the same DOM approach, because rich embeds (video, iframes, editors) need it. |
| **mind-elixir, jsMind** | HTML nodes | SVG/canvas lines | Mind map libraries. Their layout is a left/right balanced tree. |
| **markmap** | SVG `foreignObject` | SVG | Uses **d3-flextree** for variable-size nodes. `foreignObject` has many browser quirks; avoid it. |
| **Excalidraw, Cytoscape.js, Sigma.js, G6 canvas mode** | Canvas / WebGL | Canvas / WebGL | They scale to very large graphs but cannot host real interactive HTML content (video, editor, Dioxus components). This is ruled out by requirement. |
| **dioxus-flow (crates.io 0.1.x)** | Dioxus DOM nodes + SVG edges | SVG | It confirms the approach works in Dioxus. It is very young (v0.1.3, about 7 stars), web-first and not under our control, so we use it as a reference only, not as a dependency. |

Conclusion: **HTML nodes plus an SVG edge layer in a single CSS-transformed world container**, with **gesture handling outside the framework render loop**. That is the proven architecture for rich-content graphs.

### 2.2 Layout algorithms

| Use case | Standard algorithm | References |
|---|---|---|
| Hierarchy / tree with variable node sizes | Non-layered tidy tree: van der Ploeg 2014, "Drawing non-layered tidy trees in linear time", which extends Reingold–Tilford and Walker/Buchheim | `d3-flextree` (used by markmap) |
| Mind map (root in centre, branches left/right) | Split root children into two balanced sets by subtree extent, then lay out each side as a horizontal tidy tree (one mirrored) | XMind, mind-elixir, `@antv/hierarchy` |
| General relation graph, exploration | Force-directed (Fruchterman–Reingold / velocity Verlet), with rectangle collision for non-point nodes and warm start from previous positions | d3-force, cola.js (constraints), ForceAtlas2 |
| Neighborhood around a focus | Radial / concentric rings by hop distance | Cytoscape `concentric`, Gephi radial |
| Directed acyclic flows | Layered (Sugiyama): rank assignment → crossing minimization (barycenter) → coordinate assignment (Brandes–Köpf) → edge routing | Graphviz dot, dagre (Rust port `dagre-rs`/supramark), ELK Layered |

### 2.3 Best practices adopted

1. **Measure, then lay out.** Real HTML content has unknown size. Render nodes hidden, measure their unscaled border-box size with a `ResizeObserver` (Dioxus 0.7 `onresize`), and only then run layout and reveal. A `size_hint` avoids a blank first frame. Note: `getBoundingClientRect` returns *scaled* sizes under the CSS transform, while ResizeObserver border-box sizes are unscaled. Always use the latter.
2. **Keep pan/zoom frames cheap.** xyflow writes the transform outside React. We get the same effect within Dioxus: the viewport signal is read only by the world wrapper's `style`, and node and edge components are memoized with props that don't depend on the viewport. A pan frame then diffs a single attribute. Culling and LOD are recomputed with hysteresis, not on every frame.
3. **Cull off-screen nodes** using the world-space visible rect plus a margin. Edges render when either endpoint is visible.
4. **Level of detail.** When zoomed out, renderers get a `NodeDetail::{Full, Compact, Minimal}` hint. Below a threshold, the canvas renders plain placeholder boxes instead of calling the content renderer.
5. **Preserve the mental map.** When the graph grows incrementally (expanding a node), existing nodes keep their positions. Force layouts warm-start from previous positions and keep user-dragged nodes pinned. New nodes are seeded near the node they were expanded from.
6. **Determinism.** Layouts must produce identical output for identical input: no wall-clock randomness, a seeded PRNG, and stable ordering via `IndexMap` or sorted ids. This keeps tests reliable and prevents nodes from jumping on re-layout.
7. **Bound the data.** Cap total nodes and per-expansion fan-out. Show a synthetic "+N more" overflow node rather than loading unbounded neighborhoods.
8. **Interactive content inside nodes.** xyflow uses `nodrag`/`nowheel` class markers to opt node sub-elements out of gestures. Without JS `closest()` lookups, we provide the same thing as components: `NoDrag` and `NoWheel` wrappers that stop event propagation (§4.6).
9. **Accessibility.** The container gets `role="application"` and an `aria-label`. Nodes get `tabindex=0`, `role="button"` and an `aria-label` supplied by the caller. Enter activates, Escape clears the selection, `+`/`-`/`0` zoom and fit. The explorer also keeps a non-graph "list" fallback path through existing entity pages.
10. **Separate geometry from rendering.** Viewport math, edge anchors and paths, bounds, and hit-tests are pure functions with unit tests. The Dioxus components stay thin.

## 3. Architecture overview

```text
crates/dxgraph              (generic, content-agnostic; depends on dioxus, serde, indexmap)
  model      GraphModel<N,E>, NodeId, EdgeId, GraphNode, GraphEdge
  geometry   Point, Size, Rect, Viewport math, bounds, culling
  edge       anchors (floating / sided), path builders (straight, bezier, step), labels
  layout     LayoutAlgorithm trait + tree / mindmap / force / radial      (NO dioxus imports)
  interaction  pure-Rust gesture state machine (pan/zoom/pinch/drag/click) + web-only pointer capture
  canvas     GraphCanvas, GraphControls, GraphBackground components, CSS
       ^
       | used by
crates/ui_core::graph       (Semantic-specific, no routing)
  source     GraphSource trait + RpcGraphSource (semantic.db.query)
  explorer   EntityGraphExplorer state machine (pure Rust, unit-tested)
  view       EntityGraphView component: GraphCanvas + EntityGraphNode + detail side panel
       ^
       | used by
crates/ui                   /graph?:root&:collection&:mode&:layout  -> GraphPage + nav item
```

Layering rules:

- `dxgraph` must not depend on any `semantic_*` crate. It must compile and test on its own.
- `dxgraph::layout`, `dxgraph::geometry`, `dxgraph::edge` and `dxgraph::model` must not import `dioxus`. Enforce this with a code review checklist item. Optionally, a later refactor can move them into a `dxgraph_layout` crate.
- `semantic_ui_core::graph` owns every entity-specific decision: which nodes and edges exist, how they look, and what clicking does.
- `semantic_ui` only adds the route, the page chrome and URL state.

## 4. `dxgraph` crate design

### 4.1 Crate layout

```text
crates/dxgraph/
  Cargo.toml
  README.md                 short usage + the ownership rules from §4.6
  src/
    lib.rs                  re-exports
    model.rs                ids, GraphModel, GraphNode, GraphEdge, mutations
    geometry.rs             Point, Size, Rect, Viewport, transforms, fit, culling
    edge.rs                 EdgeAnchor, EdgeSide, EdgePathStyle, EdgeGeometry, path builders
    layout/
      mod.rs                LayoutAlgorithm, LayoutInput, LayoutOutput, LayoutConfig, run_layout
      tree.rs               TreeLayout (variable-size tidy tree, van der Ploeg)
      mindmap.rs            MindMapLayout (left/right balanced)
      force.rs              ForceLayout (deterministic, rect collision, warm start, pins)
      radial.rs             RadialLayout (concentric rings by hop distance)
      spanning.rs           spanning forest extraction (BFS from roots) shared by tree/mindmap/radial
      rng.rs                tiny seeded PRNG (SplitMix64); no `rand` dependency
    interaction/
      mod.rs                re-exports
      gesture.rs            GestureState reducer: pointer/wheel inputs -> GestureEffect outputs (pure, no dioxus)
      input.rs              conversion from Dioxus PointerData/WheelData into gesture inputs
      platform.rs           wasm-only extras via web-sys (pointer capture); no-ops elsewhere
    canvas/
      mod.rs                GraphCanvas component + props
      node.rs               node wrapper (measurement, selection, a11y attrs)
      edges.rs              SVG edge layer + markers
      controls.rs           GraphControls (zoom in/out/fit/relayout)
      background.rs         dotted grid driven by CSS variables
      state.rs              CanvasState: measured sizes, viewport snapshot, selection, phase
    dxgraph.css             all styles, prefixed `dxgraph-`, using dxcomp CSS variables where possible
  tests/
    ssr.rs                  dioxus-ssr render tests
```

Cargo: copy the metadata block from `crates/dxeditor/Cargo.toml`. Dependencies: `dioxus` (workspace), `serde` + `serde_json` (workspace), `indexmap` (with `serde`), `thiserror`. Dev-dependencies: `dioxus-ssr = "0.7.9"`. Features: `default = []`, `web = ["dep:web-sys"]` (web-sys features `Element`; add more only as needed). Gate web-sys code with `#[cfg(all(feature = "web", target_arch = "wasm32"))]` so desktop builds compile it out. **Do not use `document::eval` or any raw JS.** `semantic_ui` and `ui_core` enable `dxgraph/web` from their own `web` features.

### 4.2 Model (`model.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub String);       // impl From<&str>, From<String>, Display
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EdgeId(pub String);

#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode<N> {
    pub id: NodeId,
    pub data: N,
    /// World-space top-left. `None` = not yet placed by layout or user.
    pub position: Option<Point>,
    /// Size used before measurement (avoid zero-size layout). Measured size wins.
    pub size_hint: Size,
    /// User-fixed position: layouts must not move it.
    pub pinned: bool,
    /// Optional layout parent (tree/mindmap); independent of edges.
    pub layout_parent: Option<NodeId>,
    /// Stable ordering key among siblings (e.g. title); falls back to insertion order.
    pub order_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphEdge<E> {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    pub data: E,
    pub style: EdgeStyle,          // path style, dash, markers, css class, label
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct GraphModel<N, E> {
    nodes: IndexMap<NodeId, GraphNode<N>>,
    edges: IndexMap<EdgeId, GraphEdge<E>>,
}
```

API (all O(1)/O(degree) where possible; keep a lazily rebuilt adjacency index or recompute on demand, whichever is simpler):
`insert_node`, `upsert_node` (keeps the existing position and pin), `remove_node` (also removes incident edges), `insert_edge` (rejects unknown endpoints with `GraphError::UnknownNode`), `remove_edge`, `node`, `node_mut`, `nodes()`, `edges()`, `incident_edges(id)`, `neighbors(id)`, `set_position`, `set_pinned`, `retain_nodes(pred)`, `bounds(sizes)`.

`N` and `E` must be `Clone + PartialEq + 'static`, which Dioxus props require.

### 4.3 Geometry and viewport (`geometry.rs`)

- `Point { x: f64, y: f64 }`, `Size { width, height }`, `Rect { origin, size }` with `center`, `contains`, `intersects`, `union`, `inflate`.
- `Viewport { x: f64, y: f64, zoom: f64 }`, where screen = world × zoom + (x, y).
  - `world_to_screen`, `screen_to_world`.
  - `zoom_at(screen_point, factor, min, max)` keeps the point under the cursor fixed.
  - `fit(bounds, container: Size, padding: f64, min_zoom, max_zoom) -> Viewport`.
  - `visible_world_rect(container) -> Rect`.
- `ViewportLimits { min_zoom: 0.1, max_zoom: 2.0 }` (defaults).
- `NodeDetail::{Full, Compact, Minimal}` comes from `zoom` via thresholds in `CanvasConfig`. Defaults: Full ≥ 0.6, Compact ≥ 0.3, otherwise Minimal.

Unit tests: round-trip transforms, `zoom_at` invariant, `fit` centering and clamping, culling at edges.

### 4.4 Edges (`edge.rs`)

- `EdgeSide::{Top, Right, Bottom, Left}`, matching JSON Canvas `fromSide`/`toSide`.
- `EdgeAnchor::{Floating, Side(EdgeSide)}`. Floating anchors are the intersection of the center-to-center line with each node's rectangle, best for force/radial. Sided anchors suit tree layouts: TB uses Bottom→Top and LR uses Right→Left.
- `EdgePathStyle::{Straight, Bezier, Step}`. Bezier control points are offset along the anchor side normal, as React Flow does; Step is orthogonal with one midpoint bend.
- `EdgeMarker::{None, Arrow}` per end. Defaults are `from_end = None` and `to_end = Arrow`, as in JSON Canvas.
- `EdgeStyle { path, source_anchor, target_anchor, from_end, to_end, dashed: bool, class: Option<String>, label: Option<String> }`.
- `fn edge_geometry(source: Rect, target: Rect, style: &EdgeStyle) -> EdgeGeometry { path_d: String, label_pos: Point, start: Point, end: Point }`.
  - Self-loops get a small fixed loop on the top-right corner.
  - Parallel edges between the same pair: callers pass an `offset_index`, and the bezier curvature is offset so the edges stay distinguishable.
- Number formatting: round to 0.1 to keep the DOM small and SSR snapshots stable.

Unit tests cover rectangle intersection for all quadrants, each path style's `d` string for fixed inputs, self-loops, and overlapping rectangles (degenerate case: fall back to the centers).

### 4.5 Layout (`layout/`)

```rust
pub struct LayoutNode { pub id: NodeId, pub size: Size, pub fixed: Option<Point>, pub previous: Option<Point>,
                        pub layout_parent: Option<NodeId>, pub order_key: Option<String> }
pub struct LayoutEdge { pub source: NodeId, pub target: NodeId, pub weight: f64 }
pub struct LayoutInput { pub nodes: Vec<LayoutNode>, pub edges: Vec<LayoutEdge>, pub roots: Vec<NodeId> }
pub struct LayoutOutput { pub positions: IndexMap<NodeId, Point> }   // top-left corners

pub trait LayoutAlgorithm {
    fn layout(&self, input: &LayoutInput) -> LayoutOutput;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayoutConfig {
    Tree(TreeLayoutOptions), MindMap(MindMapOptions), Force(ForceOptions), Radial(RadialOptions), Manual,
}
pub fn run_layout(config: &LayoutConfig, input: &LayoutInput) -> LayoutOutput;
```

General rules for every algorithm:

- Output must contain every input node.
- Nodes with `fixed` must be output exactly at `fixed`.
- Results must be deterministic: sort children by `(order_key, id)`, use a seeded PRNG, and never iterate a `HashMap`.
- Disconnected components: lay each out separately, then pack them left-to-right in rows using bounding boxes (shared helper `pack_components`).
- Cycles: tree, mindmap and radial build a **spanning forest** via BFS from `roots`, or from `layout_parent` when present. Non-tree edges are still drawn, just ignored by the layout.

Algorithms:

1. **`TreeLayout`** (`direction: TopDown | LeftRight`, `sibling_gap`, `level_gap`).
   - Implement the non-layered tidy tree of van der Ploeg (2014). Port it from the d3-flextree source; its structure is clear: first walk, contour threads, `mod`/`prelim`/`shift`/`change`.
   - In TopDown mode, node `y` = parent `y` + parent height + `level_gap`, which makes it "non-layered": a tall node pushes only its own subtree down. In LeftRight mode, swap axes before and after.
   - Must be O(n). Recommended approach for juniors: first implement a simple correct version (post-order subtree width packing, O(n²) worst case) behind the same API, test it, then replace it with the linear algorithm while keeping the tests.
2. **`MindMapLayout`** (`h_gap`, `v_gap`, `balance: Auto | AllRight`).
   - The root is centered at (0,0).
   - Root children are assigned greedily, in order, to the side with the smaller accumulated subtree height.
   - Each side is laid out with `TreeLayout` LeftRight. The left side is mirrored on x, and both sides are vertically centered on the root.
3. **`ForceLayout`** (`iterations: 300`, `link_distance`, `charge`, `collision_padding`, `gravity`, `seed`).
   - Velocity-Verlet with alpha decay, as in d3-force: link springs, all-pairs repulsion (O(n²); acceptable up to the v1 cap of 300 nodes; Barnes–Hut later), weak gravity toward the centroid, and a final **rectangle overlap removal** pass (iterative separation along the axis of least overlap).
   - Warm start uses `previous`. New nodes without `previous` are seeded around their already-placed neighbors' centroid with a seeded jitter, otherwise on a seeded spiral.
   - `fixed` nodes never move.
4. **`RadialLayout`** (`focus: NodeId`, `ring_gap`).
   - BFS hop distance from the focus. Ring k radius = max(ring k−1 radius + max node extent + gap, the circumference needed for the ring's nodes).
   - Angular order follows the parent's angle, so subtrees stay together.
5. **Layered (Sugiyama)**: deferred (Phase 7, optional). Keep the trait stable so that this, or an ELK adapter, can be added later.

Tests (per algorithm):

- golden positions for small fixtures (single node; chain; star; binary tree; mixed sizes);
- no overlap between node rects (property-style loop over generated trees with seeded sizes);
- determinism (run twice, compare);
- fixed nodes respected;
- disconnected components packed without overlap;
- cycles don't hang;
- a 300-node performance smoke test (< 50 ms in debug for tree, < 500 ms for force), marked `#[ignore]` if it's slow in CI.

### 4.6 Interaction and gestures (`interaction/`)

There is **no custom JavaScript**. All input comes from Dioxus's own event system, which works on both web (wasm) and desktop (webview). The relevant behavior was verified in the Dioxus 0.7.9 sources:

- **Desktop event dispatch is synchronous**: the interpreter waits for the Rust handler's reply. `evt.prevent_default()` therefore works on desktop as well as web.
- **Dioxus registers listeners on the app root element**, not on `window`/`document`/`body`. Browsers only make wheel listeners passive by default on those three targets, so `onwheel` + `prevent_default()` reliably blocks page scrolling while zooming.
- **`onresize`** is backed by a ResizeObserver and gives unscaled border-box sizes (`get_border_box_size()`).
- **`onmounted`** gives `MountedData` with `get_client_rect()` and `set_focus()`.

web-sys only exists on the wasm target. Desktop Rust runs natively and never touches the DOM directly. web-sys is therefore limited to **optional, web-only enhancements** in `interaction/platform.rs`, gated by `#[cfg(all(feature = "web", target_arch = "wasm32"))]`. On wasm, Dioxus `MountedData` downcasts to `web_sys::Element`:

- `set_pointer_capture` / `release_pointer_capture` on gesture start/end, so drags continue when the pointer leaves the canvas.
- On other targets these functions are no-ops, and the gesture state machine ends a drag on `onpointerleave` of the container instead.

Custom JS via `wasm-bindgen` is a last resort. Use it only if a needed browser API is missing from web-sys, add a comment explaining why, and get it reviewed. Nothing in v1 needs it.

Ownership rule (document it in the README):

> Dioxus/Rust owns everything: which nodes and edges exist, their content, world
> positions, selection, and the viewport. The viewport is a signal read **only** by the
> world wrapper's `style`, so a pan/zoom frame re-renders one element. Node and edge
> components must be memoized with viewport-independent props.

DOM contract (rendered by `GraphCanvas`):

```html
<div class="dxgraph" role="application" aria-label="…" tabindex="0"
     onpointerdown/move/up/cancel/leave, onwheel, onkeydown, onresize, onmounted>
  <div class="dxgraph-background" style="--dxg-x:…;--dxg-y:…;--dxg-zoom:…"></div>
  <div class="dxgraph-world" style="transform-origin:0 0; transform:translate(Xpx,Ypx) scale(Z)">
    <svg class="dxgraph-edges" …> <defs>markers</defs> <path…/> … </svg>
    <div class="dxgraph-node" data-dxgraph-node="{node id}" style="transform:translate(Xpx,Ypx)" tabindex="0"
         onpointerdown, onresize>…content…</div>
  </div>
  <div class="dxgraph-overlay"> controls / panels (screen space) </div>
</div>
```

**Gesture state machine** (`gesture.rs`, pure Rust, no Dioxus imports, fully unit-tested):

```rust
pub enum GestureTarget { Background, Node(NodeId) }
pub enum GestureInput {
    PointerDown { pointer: i32, client: Point, target: GestureTarget, shift: bool, primary: bool },
    PointerMove { pointer: i32, client: Point },
    PointerUp   { pointer: i32, client: Point },
    PointerCancel { pointer: i32 },          // also used for pointerleave without capture
    Wheel { client: Point, delta: Point, ctrl: bool },   // delta normalized to pixels
}
pub enum GestureEffect {
    SetViewport(Viewport),
    NodeDragStart(NodeId), NodeDragMove { node: NodeId, position: Point }, NodeDragEnd { node: NodeId, position: Point },
    NodeClick { node: NodeId, shift: bool }, NodeDoubleClick(NodeId),
    BackgroundClick { world: Point },
    CapturePointer(i32), ReleasePointer(i32),   // executed by platform.rs; no-op off-web
}
pub struct GestureState { /* active pointers, mode: Idle | Pending | Panning | Dragging | Pinching, last click */ }
impl GestureState {
    pub fn handle(&mut self, input: GestureInput, ctx: &GestureContext) -> Vec<GestureEffect>;
}
pub struct GestureContext<'a> {
    pub viewport: Viewport, pub container_origin: Point, pub limits: ViewportLimits,
    pub wheel_mode: WheelMode, pub node_position: &'a dyn Fn(&NodeId) -> Option<Point>,
}
```

Behavior:

- A pointer down enters `Pending`. Moving beyond a 4 px threshold becomes `Panning` (on background) or `Dragging` (on a node). Releasing before the threshold produces a click. Two clicks on the same target within 350 ms and 4 px produce a double click; timestamps come from the input, so tests stay deterministic.
- Pan and drag use only **client deltas**. Drag position = start position + delta / zoom.
- Zoom-at-cursor needs the cursor in container coordinates: `client − container_origin`. `container_origin` is cached from `get_client_rect()` on mount and on every container `onresize`, and refreshed asynchronously on each pointer down to handle page scroll.
- Wheel:
  - `ctrl` (also set by trackpad pinch) → zoom at the cursor, with factor `exp(-delta.y * 0.002)`;
  - otherwise, in `WheelMode::Pan`, pan by the delta;
  - in `WheelMode::Zoom`, zoom.
  - Always `prevent_default()` in the Dioxus handler.
- Pinch: two active pointers become `Pinching`. Zoom by the distance ratio around the midpoint and pan by the midpoint delta.
- Non-primary buttons are ignored (left for future context menus).

`input.rs` converts Dioxus `PointerData` (`client_coordinates()`, `pointer_id()`, `modifiers()`, `trigger_button()`) and `WheelData` (`delta()`: Pixels, Lines × 16, Pages × container height) into `GestureInput`s. It is the only module that touches Dioxus event types.

**Event targeting without `closest()`:**

- The node wrapper's `onpointerdown` calls `evt.stop_propagation()` and feeds `PointerDown { target: Node(id) }`. The container's `onpointerdown` therefore only sees background presses.
- Move/up/cancel are handled on the container (bubbled).
- Opt-outs for interactive node content are provided as components instead of DOM attribute markers:
  - `NoDrag { children }`: a `div` with `onpointerdown: |e| e.stop_propagation()`, so buttons, links, inputs, video controls and editors inside a node never start a gesture.
  - `NoWheel { children }`: stops `onwheel` propagation, so scrollable node content keeps the wheel.
  - Node renderers wrap interactive regions in these components. Document this prominently.

**Per-frame cost:**

- During a drag, the model update re-renders the moved node's wrapper and the edges incident to it. Edges are memoized per edge with their two endpoint rects as props, so only those edges diff.
- On desktop, each pointermove is one synchronous IPC round-trip plus a small DOM edit. That is the standard cost of any interactive Dioxus desktop app and acceptable at v1 graph sizes. Step 6.2 measures it.

Testing:

- `gesture.rs` gets exhaustive unit tests: click vs drag threshold, double click, pan, node drag with zoom ≠ 1, wheel zoom invariant (the point under the cursor stays fixed), pinch, cancel mid-drag, second pointer during drag, and non-primary buttons ignored.
- `input.rs` gets conversion tests where Dioxus event data can be constructed; otherwise keep it trivial.
- Real-browser behavior is covered by the Playwright check in Phase 6.

### 4.7 Canvas component (`canvas/`)

```rust
#[derive(Props, Clone, PartialEq)]
pub struct GraphCanvasProps<N: Clone + PartialEq + 'static, E: Clone + PartialEq + 'static> {
    pub model: ReadSignal<GraphModel<N, E>>,
    pub render_node: Callback<NodeRenderContext<N>, Element>,
    /// Optional: placeholder for `NodeDetail::Minimal` (default: plain box with `aria_label`).
    #[props(default)] pub render_minimal: Option<Callback<NodeRenderContext<N>, Element>>,
    pub node_label: Callback<NodeId, String>,                 // aria-label
    #[props(default)] pub layout: LayoutConfig,
    /// Bump to force a full relayout (e.g. "Re-layout" button).
    #[props(default)] pub layout_revision: u64,
    #[props(default)] pub config: CanvasConfig,              // limits, LOD thresholds, cull margin, wheel mode
    #[props(default)] pub controller: Option<GraphController>, // imperative handle (fit, zoom, center_on)
    #[props(default)] pub on_node_click: Option<EventHandler<NodeEvent>>,
    #[props(default)] pub on_node_double_click: Option<EventHandler<NodeEvent>>,
    #[props(default)] pub on_node_moved: Option<EventHandler<(NodeId, Point)>>,
    #[props(default)] pub on_selection_change: Option<EventHandler<Vec<NodeId>>>,
    #[props(default)] pub on_background_click: Option<EventHandler<Point>>,
    #[props(default)] pub class: Option<String>,
    #[props(default)] pub children: Element,                 // overlay content (screen space)
}

pub struct NodeRenderContext<N> { pub id: NodeId, pub data: N, pub selected: bool, pub detail: NodeDetail }
```

Note: if Dioxus 0.7 `#[component]` generic props turn out awkward, use a manual `#[derive(Props)]` struct plus `fn GraphCanvas<N, E>(props: GraphCanvasProps<N, E>) -> Element`. Spike this first (Step 2.1).

Internal `CanvasState` (signals, owned by the component):

- `measured: HashMap<NodeId, Size>`, updated from each node wrapper's `onresize` via `get_border_box_size()`.
- `positions: IndexMap<NodeId, Point>`, the layout result merged with the model: model positions of pinned nodes and nodes the user moved win.
- `phase: Measuring | Ready`.
- `viewport: Signal<Viewport>`, read only by the world wrapper and background styles. `container: Size` and `container_origin: Point` come from container `onresize`/`onmounted`.
- `gesture: GestureState`, plus `culled_rect: Rect`, the inflated world rect the current culled set was computed for.
- `selection: IndexSet<NodeId>`.

Layout pipeline:

1. When the model's node set changes (compare sorted ids plus a hash of `layout_parent`s and edges), mark `needs_layout`.
2. Newly added nodes render with `visibility:hidden` at their seed position until measured. Measurement is collected in a batch: layout runs once every pending node has reported a size or after 100 ms, whichever comes first, using `size_hint` for stragglers.
3. Run `run_layout` synchronously. It is fast enough for v1 caps. Large graphs can later move to a spawned task or a web worker.
4. Write positions, reveal nodes, and on the **first** layout set `viewport = Viewport::fit(bounds, container, …)`.
5. If a measured size later changes by more than 8 px (e.g. an image loaded), re-run layout with all current positions as `previous`. For tree and mindmap this is a full layout; for force it is a warm start with existing nodes held fixed except the changed node's neighborhood.

Rendering:

- Edges: one `svg` with `overflow: visible`, at 1×1 px at the world origin, with `pointer-events: none` except on paths (`pointer-events: stroke` with a wider transparent hit path for future edge clicks). Markers live in `defs`, with ids namespaced by session.
- Nodes: `div.dxgraph-node` with `transform: translate(x, y)` (not left/top; compositor friendly) and `data-dxgraph-node`. Add `data-selected` when selected. The `render_node` output is placed inside.
- Culling: render only nodes whose rect intersects `visible_world_rect.inflate(cull_margin)`. Selected and dragged nodes always render. Edges render when either endpoint renders, using the last known rect for the culled endpoint. Re-cull with **hysteresis** so it doesn't run every frame, and without timers: compute the visible set for `visible.inflate(cull_margin)` and store that rect as `culled_rect`. Recompute only when the current visible rect is no longer contained in `culled_rect`, or when the zoom changed by more than 15% since the last cull. Implement this as a pure `fn needs_recull(culled_rect, visible, last_zoom, zoom) -> bool` with unit tests.
- LOD: `detail` is computed from the zoom. At `Minimal`, `render_minimal` (or the built-in box) is used.

Controls and background:

- `GraphControls` renders zoom in, zoom out, fit and an optional "re-layout" callback, using dxcomp `Button` and icons.
- `GraphBackground` is pure CSS: `radial-gradient` dots sized via `calc(var(--dxg-zoom) * 20px)` and offset via `--dxg-x/y`.

`GraphController` is a `Copy` handle created by `use_graph_controller()`. It exposes `fit_view()`, `zoom_by(f)`, `center_on(NodeId)` and `set_viewport(v)`, implemented by writing the canvas's viewport signal. It holds `Signal`s, so it stays `Copy`. `center_on` and `fit_view` need container size and node rects, which the canvas registers into the controller on mount.

Keyboard (container focused):

- Tab/Shift+Tab moves through nodes (DOM order follows model insertion order).
- Enter triggers `on_node_double_click`-equivalent "activate". Make it a separate `on_node_activate` callback.
- Space selects.
- Escape clears the selection.
- `+`/`-`/`0` zoom in, zoom out and fit.
- Arrow keys pan by 50 px.

CSS: `dxgraph.css` is injected once with `style { {include_str!("dxgraph.css")} }` inside `GraphCanvas`, following `ui_core` `comments.css`, or exposed as a `dxgraph::CSS` constant plus a `Stylesheet` component like `dxcomp`. Prefer the latter, rendered once from the app root next to `dxcomp::Stylesheet`. Use the dxcomp theme variables for colors.

## 5. Semantic integration (`crates/ui_core/src/graph/`)

### 5.1 Data access: `GraphSource`

```rust
pub struct EntityRef { pub collection: Option<String>, pub id: String }  // reuse `EntityTarget` instead if equivalent
pub struct RelationEdgeRow { pub relation: String, pub source: String, pub target: String }

#[async_trait(?Send)]  // or a boxed-future trait object matching existing ui_core patterns
pub trait GraphSource {
    async fn entities(&self, ids: &[EntityTarget]) -> Result<Vec<Object>, String>;
    async fn children(&self, parents: &[EntityTarget], limit: usize) -> Result<Vec<Object>, String>;
    async fn relation_edges(&self, ids: &[String], limit: usize) -> Result<Vec<RelationEdgeRow>, String>;
}
```

Check how `ui_core` already abstracts async RPC calls before choosing between `async_trait` and boxed futures, and match that pattern.

`RpcGraphSource` uses `semantic.db.query` with the AST helpers in `crates/ui_core/src/query_ast.rs`. Use the query shapes already proven in `crates/ui_core/src/components/entity/associations.rs`:

- **children**: `select child from <collection> where child."semantic:parent" IN $ids order by title, id limit $limit`. Use `BinaryOp::In` with a list parameter. Verify `In` with a list parameter works on KV/redb; if not, fall back to `any([eq …])`, as `associations.rs` does with `any`.
- **parent/ancestors**: read the `semantic:parent` value from already-loaded objects, then batch-load with `entities`.
- **relation_edges**: `select edge from "__semantic.relationship_edges" where edge.depth = 1 and (edge.source IN $ids or edge.target IN $ids) order by relation, source, target limit $limit`.
- **entities**: `select e from <collection> where e.id IN $ids`, grouped by collection.
  - Endpoint collection resolution: relationship edge rows only carry ids. For v1, look up endpoints in the `RelationType.source_collection` (for `source`) and the default collection. Render unresolved endpoints as an "unresolved" node, marked as such, instead of dropping them.
  - Before implementing, the step's agent must confirm in `crates/db_core/src/embedded/db.rs` (around `RELATION_EDGES_COLLECTION`) which fields an edge row contains, and how target collections can be resolved.
- Relation display names come from the UI catalog's relation types (`RelationType.meta.title`, else `name`). Hide the `semantic:parent` relation in Relations mode when Hierarchy edges are also shown, to avoid duplicates.

Make `const RELATION_EDGES_COLLECTION` a shared `pub(crate)` constant in `ui_core` rather than duplicating it.

A `MockGraphSource` (in-memory) lives in a `#[cfg(test)]` module for explorer tests.

### 5.2 Explorer state machine (`explorer.rs`, pure Rust)

```rust
pub enum GraphMode { Hierarchy, Relations, Both }
pub struct ExplorerLimits { pub max_nodes: usize /*300*/, pub fan_out: usize /*50*/ }

pub struct EntityNodeData { pub target: EntityTarget, pub object: Option<Object>, pub kind: EntityNodeKind }
pub enum EntityNodeKind { Entity, Unresolved, Overflow { parent: NodeId, remaining_hint: usize } }
pub struct EntityEdgeData { pub kind: EntityEdgeKind }   // Parent | Relation { relation_id, label }

pub struct EntityGraphExplorer {
    model: GraphModel<EntityNodeData, EntityEdgeData>,
    mode: GraphMode, limits: ExplorerLimits,
    root: EntityTarget, expanded: IndexSet<NodeId>, loading: IndexSet<NodeId>,
}
```

Operations (all synchronous; async loading happens in the view, and results are applied here):

- `new(root, mode, limits)` seeds the root node.
- `begin_expand(node) -> Option<ExpansionRequest>` returns `None` if the node is already expanded or loading, or if the node budget is exhausted. Otherwise it returns the request describing what to load.
- `apply_expansion(node, ExpansionResult)` adds nodes and edges, deduplicating by id. Over-budget items become one `Overflow` node. Each new node's `layout_parent` = the expanded node, so the tree and mindmap layouts follow the exploration order.
- `collapse(node)` removes nodes that are only reachable from the root through `node`, using BFS reachability excluding `node`'s outward expansion edges. The root is never removed.
- `set_mode(mode)` re-seeds from the root and keeps the root's object.
- `ancestors_request(node)` / `apply_ancestors(...)` handle hierarchy upward expansion. The new parent becomes the `layout_parent` *of the current root side*: re-root the tree so the topmost ancestor is the layout root.
- Edge ids: `parent:{child}->{parent}` and `rel:{relation}:{source}->{target}`.
- Node id: `{collection}/{id}`.

Unit tests (with fixtures):

- seed;
- expand children;
- expand relations, both directions;
- dedup when two expansions reach the same entity;
- fan-out overflow node;
- global cap;
- collapse keeps shared nodes;
- mode switch;
- cycles in relations;
- unresolved endpoints.

### 5.3 View (`view.rs`)

`EntityGraphView { root: EntityTarget, mode: GraphMode, layout: LayoutConfig }`:

- It holds `Signal<EntityGraphExplorer>` and derives `ReadSignal<GraphModel>` with `use_memo` for the canvas.
- On mount it loads the root object and auto-expands one level, children and/or relations depending on the mode.
- It uses `GraphCanvas` with:
  - `render_node` → `EntityGraphNode`;
  - `on_node_click` → select and open the side panel;
  - `on_node_activate`/double-click → expand or collapse.
- Side panel: a dxcomp `sheet` (or a right-docked panel inside the overlay) showing `EntityCard { object, options: EntityRenderOptions { preview: true, actions: true, .. } }` and buttons for Expand/Collapse, "Focus here" (re-root) and "Open" (navigate via the existing entity navigation helpers in `ui_catalog/entity_navigation.rs`).
- Loading and error states: a per-node spinner badge while `loading`, and a toast via `use_toast_dispatcher` on failure.

`EntityGraphNode` (compact, v1):

- A left accent bar colored from the class. Reuse the label/class color helpers if they exist; otherwise hash the class id into a dxcomp palette.
- The title via `catalog.entity_title(object)`, the class name (muted) and the id (muted, truncated).
- An expand toggle button (`+`/`−`, wrapped in `NoDrag`) showing the loading state.
- At `NodeDetail::Compact` only the title is shown; at Minimal, the canvas placeholder.
- Fixed `max-width: 240px`. Height comes from measurement.
- Overflow nodes render "+N more" and expand to the next page when activated. Paging support in `GraphSource` uses an offset; this is optional in v1 and acceptable to show as non-interactive.

Default layouts per mode: Hierarchy → `Tree(TopDown)`; Relations/Both → `Force`. MindMap and Radial are available in the layout menu.

Edge styles: Parent → solid, `Side` anchors, bezier, arrow toward the child. Relation → floating anchors, bezier, arrow from source to target, the relation title as label, and a CSS class per relation for subtle color variation.

## 6. App integration (`crates/ui`)

- Route in `crates/ui/src/views/mod.rs`: `#[route("/graph?:root&:collection&:mode&:layout")] GraphPage { root: Option<String>, collection: Option<String>, mode: Option<String>, layout: Option<String> }`.
- `crates/ui/src/views/graph.rs`: page header and toolbar:
  - root picker: reuse `components/entity_autocomplete.rs` from `ui_core`;
  - mode toggle: dxcomp `toggle_group`;
  - layout select: dxcomp `select`;
  - re-layout button.
  - The canvas fills the remaining height.
  - Without `root`, show an empty state that explains how to pick an entity.
  - Changing a control updates the URL via `navigator().replace(...)`, following `browse.rs` patterns.
- Navigation: add `NavItem::Graph` in `crates/ui/src/components/shell.rs` (look at how `NavItem::Tree` is wired around lines 185, 324 and 385) and a home tile if the home page lists views (`views/home.rs:31`).
- Render `dxgraph::Stylesheet` once at the app root, next to the existing dxcomp stylesheet.
- Cargo: add `dxgraph = { path = "../dxgraph" }` to `ui_core` (and to `ui` only if it uses dxgraph types directly).

## 7. Phases and subagent step plan

Conventions for every step (put these in each agent brief):

- Use Opus subagents (`model: "opus"`). One logical group = one step = **exactly one commit**. Stage only the step's own paths (never `git add -A`) and check `git status` afterwards. Leave unrelated dirty files alone.
- Run every command through the Nix devshell when available: `cargo check --quiet --message-format=short`, `cargo test --quiet --message-format=short -p <crate>`, `cargo fmt`.
- No `Result<T>` aliases; use `Result<T, E>`. No `unwrap()` in non-test code paths that handle external input.
- Don't touch migrations or schema. This effort needs none.
- Don't change core types in `semantic_data`, `semantic_db_core` or `semantic_rpc`. If a step seems to need that, **stop and report back** instead.
- Each brief includes the relevant section numbers of this plan, the files to create or modify, and the acceptance criteria below. Agents read only those sections and files.

Parallelism: Steps marked ∥ can run in parallel once their dependencies are committed. Run parallel agents with `isolation: "worktree"` and merge sequentially, or run them serially if merge conflicts in `lib.rs` are likely. The simplest option is to run them serially.

### Phase 1: Core foundations (pure Rust, no UI)

**Step 1.1: Crate scaffold, model, geometry.** Depends on: none.
- Create `crates/dxgraph` (Cargo.toml, README skeleton, `lib.rs`), `model.rs` and `geometry.rs` per §4.2–4.3.
- Acceptance: unit tests for every public geometry function and model mutation (including `remove_node` cascading edges and `insert_edge` rejecting unknown endpoints). `cargo check` for the workspace passes.
- Commit: `Add dxgraph crate with graph model and viewport geometry`.

**Step 1.2 ∥: Edge geometry.** Depends on: 1.1.
- `edge.rs` per §4.4.
- Acceptance: tests from §4.4, with exact `d` strings for fixed inputs.
- Commit: `Add dxgraph edge anchoring and path builders`.

**Step 1.3 ∥: Layout framework + tree layout.** Depends on: 1.1.
- `layout/mod.rs` (trait, input/output, `LayoutConfig`, `run_layout`, `pack_components`), `layout/spanning.rs`, `layout/rng.rs`, `layout/tree.rs`.
- Implement TreeLayout as a simple correct version first, then the linear van der Ploeg version (§4.5.1). Keep both behind `#[cfg(test)]` comparisons only if useful; ship one.
- Acceptance: all generic layout tests from §4.5 for TreeLayout, both directions.
- Commit: `Add dxgraph layout framework and tidy tree layout`.

**Step 1.4: Mind map, force, radial layouts.** Depends on: 1.3.
- `layout/mindmap.rs`, `layout/force.rs`, `layout/radial.rs` per §4.5.
- Acceptance: generic layout tests per algorithm, plus:
  - mindmap: the sides are balanced within one subtree;
  - force: warm start moves pre-positioned nodes less than 1 node width when one node is added; no rect overlaps after overlap removal;
  - radial: ring ordering.
- Commit: `Add mind map, force-directed and radial layouts to dxgraph`.

### Phase 2: Canvas rendering (static, no gestures)

**Step 2.1: GraphCanvas static render + measurement + layout pipeline.** Depends on: 1.2, 1.4.
- Spike generic props first (§4.7 note). Write `canvas/mod.rs`, `canvas/node.rs`, `canvas/edges.rs`, `canvas/state.rs`, `dxgraph.css`, `dxgraph::Stylesheet`.
- The viewport is a Rust signal from the start (§4.6 ownership rule). In this step it is only set by the initial fit; gestures come in Step 3.2.
- Acceptance:
  - `tests/ssr.rs` renders a 3-node graph and asserts node wrappers, edge paths and markers are present;
  - unit tests for the "positions merge" logic (pinned and user-moved nodes win) and the "needs relayout" detection;
  - a small example `examples/basic.rs` (desktop feature off; it just needs to compile) or a doc example.
- Commit: `Add dxgraph canvas rendering with measurement-driven layout`.

**Step 2.2: Culling + LOD + controls + background.** Depends on: 2.1.
- Culling and LOD in `state.rs`, `controls.rs`, `background.rs`.
- Acceptance: unit tests for the visible-set computation (selected nodes always included; edges with one culled endpoint) and for the detail thresholds; an SSR test for `GraphControls`.
- Commit: `Add culling, level-of-detail and controls to dxgraph canvas`.

### Phase 3: Interaction

**Step 3.1 ∥: Gesture state machine.** Depends on: 1.1 (can run alongside Phase 2).
- `interaction/gesture.rs` per §4.6: `GestureInput`, `GestureEffect`, `GestureState`, `GestureContext`, `WheelMode`. Pure Rust with no Dioxus imports.
- Acceptance: every test listed under "Testing" in §4.6.
- Commit: `Add dxgraph gesture state machine`.

**Step 3.2: Wire gestures into the canvas.** Depends on: 2.2, 3.1.
- `interaction/input.rs` (Dioxus event → `GestureInput`), `interaction/platform.rs` (web-sys pointer capture behind `web` + wasm32 cfg; no-op otherwise), the `NoDrag`/`NoWheel` components, and handlers on the container and node wrappers.
- Apply `GestureEffect`s to `CanvasState`. Add `GraphController` via `use_graph_controller`.
- No `document::eval`, no JS files.
- Acceptance:
  - unit tests for applying effects to canvas state;
  - an SSR test that the world wrapper's transform reflects the viewport signal;
  - `cargo check -p dxgraph` and `cargo check -p dxgraph --features web --target wasm32-unknown-unknown` both pass;
  - grep confirms there is no `eval(` in `crates/dxgraph`.
  - Manual checks happen in Phase 6. Optionally add `crates/dxgraph/examples/demo.rs` behind a `demo` feature using `dioxus/desktop`.
- Commit: `Wire pan, zoom and node dragging into dxgraph canvas`.

**Step 3.3: Selection, keyboard, a11y, drag persistence.** Depends on: 3.2.
- Selection model (click, shift-click, Escape, background click), the keyboard map (§4.7), `aria` attributes, and `on_node_moved`, which pins the node (sets `pinned`) via the callback. The *caller* decides whether to write it to the model; document this.
- Acceptance: unit tests for the selection reducer (make it a pure function: `fn reduce_selection(current, event) -> IndexSet<NodeId>`); an SSR test for the a11y attributes.
- Commit: `Add selection, keyboard navigation and accessibility to dxgraph`.

### Phase 4: Semantic data layer

**Step 4.1 ∥: GraphSource + RPC implementation.** Depends on: none (can run alongside Phases 2–3).
- `crates/ui_core/src/graph/{mod.rs,source.rs}` per §5.1. First verify the edge row fields, endpoint collection resolution, and `IN` list-parameter support. Report the findings in the commit body.
- Acceptance: unit tests asserting the generated `SelectQuery` ASTs and params for each method; row → `RelationEdgeRow` parsing tests, including malformed rows. If the RPC client can't easily be mocked, test only query construction and parsing.
- Commit: `Add entity graph data source over query RPC`.

**Step 4.2: Explorer state machine.** Depends on: 1.1, 4.1.
- `crates/ui_core/src/graph/explorer.rs` per §5.2, with `MockGraphSource`-driven fixtures.
- Acceptance: all tests listed in §5.2.
- Commit: `Add entity graph explorer state machine`.

### Phase 5: Semantic view + app route

**Step 5.1: EntityGraphView + EntityGraphNode + side panel.** Depends on: 3.3, 4.2.
- `crates/ui_core/src/graph/view.rs` per §5.3. Add the `dxgraph` dependency to `ui_core`.
- Acceptance:
  - SSR test rendering `EntityGraphNode` for an entity, an unresolved node and an overflow node;
  - `cargo check` for the web and desktop feature sets of `semantic_ui`: `--features web --no-default-features` and `--features desktop`.
- Commit: `Add entity graph view to ui_core`.

**Step 5.2: /graph route, toolbar, nav.** Depends on: 5.1.
- Per §6.
- Acceptance: the route parses and builds URLs (unit test of the param → `GraphMode`/`LayoutConfig` parsing helpers); the app compiles for web and desktop.
- Commit: `Add graph explorer page`.

### Phase 6: Verification and polish

**Step 6.1: Browser verification.** Depends on: 5.2.
- Follow `docs/plans/2026-09-30-task-system/browser-testing.md` for running the server plus `dx serve --web` and Playwright through Nix. Seed a fixture: a parent hierarchy of 3 levels with 2 to 6 children each, plus a few relations. Script `docs/plans/2026-10-03-graph-views/browser.cjs`:
  - open `/graph?root=…`;
  - assert nodes are visible and not overlapping (read the `getBoundingClientRect`s);
  - wheel-zoom changes the world transform;
  - drag the background pans;
  - drag a node moves it and its edge `d` changes;
  - click opens the side panel with the entity card;
  - expand adds nodes;
  - switch the layout;
  - take screenshots.
- Write the findings to `docs/plans/2026-10-03-graph-views/browser-testing.md`. Fix small issues in the same step. Report larger issues as follow-ups.
- Commit: `Verify graph explorer in the browser` (script, notes, fixes).

**Step 6.2: Desktop smoke + performance pass.** Depends on: 6.1.
- Run the desktop build and check pan/zoom smoothness and drag latency over IPC.
- Generate 300 synthetic nodes, measure layout time, and check culling keeps the DOM node count bounded when zoomed in.
- If drag or pan over desktop IPC is laggy:
  - first check memoization: only the world wrapper, the dragged node and its incident edges should diff per frame;
  - then coalesce: apply at most one `SetViewport`/`NodeDragMove` per frame by keeping the latest pending effect.
  - Do not introduce JS. If it's still insufficient, stop and report with measurements.
- Commit: `Tune dxgraph performance for desktop` (only if changes were made).

**Step 6.3: Docs.** Depends on: 6.1.
- Finish `crates/dxgraph/README.md` (usage example, ownership rules, `NoDrag`/`NoWheel` gesture opt-outs, layout catalog) and add a `dxgraph` paragraph to `docs/ARCHITECTURE.md` under the UI Layer.
- Commit: `Document dxgraph`.

### Phase 7 (optional, later): Layered layout

- `layout/layered.rs`: simplified Sugiyama with longest-path ranking, dummy nodes for long edges, barycenter crossing reduction (4 sweeps), and simple coordinate assignment (median alignment + compaction). Or evaluate depending on the supramark `dagre` crate if it's published, maintained and compatible with wasm32.

### Dependency graph

```text
1.1 ─┬─ 1.2 ───────────────┐
     ├─ 1.3 ── 1.4 ────────┴─ 2.1 ── 2.2 ─┐
     └─ 3.1 ──────────────────────────────┴─ 3.2 ── 3.3 ─┐
4.1 ── 4.2 (needs 1.1) ──────────────────────────────────┴─ 5.1 ── 5.2 ── 6.1 ─┬─ 6.2
                                                                               └─ 6.3
```

## 8. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Generic `#[component]` props with `Callback<…, Element>` fight Dioxus macros | Spike at the start of 2.1. Fallback: a manual `Props` derive, or type-erased `Rc<dyn Fn(NodeRenderContext<N>) -> Element>` wrapped in a `PartialEq`-by-pointer newtype. |
| Pan/zoom frames re-render the whole graph | The viewport signal is read only in the world wrapper and background; node and edge components are memoized with viewport-independent props; culling uses hysteresis. A test in 3.2 asserts node props don't contain viewport data. |
| Measurement churn (images loading) causes repeated relayout and jumping nodes | 8 px hysteresis, batched measurement, warm-start/fixed-node relayout, and `size_hint` defaults. Future media nodes should declare an aspect-ratio box. |
| Desktop IPC latency during drag or pan | Each event is one synchronous IPC round-trip with a minimal DOM diff. Measure in 6.2, and coalesce effects per frame if needed. No JS fallback. |
| Drag ends when the pointer leaves the canvas on desktop (no pointer capture without JS) | Acceptable in v1: treat `pointerleave` as drag end. On web, web-sys pointer capture gives full behavior. |
| A needed browser API is missing from Dioxus and web-sys | Use `wasm-bindgen` custom JS only as a justified, reviewed last resort. It works on web only, so desktop needs a graceful fallback. |
| Force layout O(n²) | The v1 cap of 300 nodes. Barnes–Hut quadtree later. |
| Relation endpoint collections are ambiguous | Show an unresolved node instead of failing. The server-side neighborhood command is a later improvement. |
| `IN` with a list parameter is unsupported or slow on some backends | Verify in 4.1. Fall back to an OR of equalities, chunked to 50 ids per query. |

## 9. Forward compatibility: mind maps and rich content (not in this effort)

- **Rich node content**: `render_node` already returns arbitrary Elements. A future `MediaGraphNode` or editor node needs to:
  - wrap interactive regions in `NoDrag`/`NoWheel`;
  - provide `size_hint`s (aspect-ratio boxes) so layout doesn't jump when media loads;
  - honor `NodeDetail`, e.g. no video autoplay below `Full`.
- **Editing**: add connection handles (`EdgeSide` anchors already exist), "create node at point" (`screen_to_world`), a model change log for undo/redo, and `on_edge_connect`. All of it is additive to `GraphCanvasProps`.
- **Persistence**: a future "canvas"/"mind map" class in `crates/base`, added **via a new migration**. Node and edge fields mirror JSON Canvas (`id,x,y,width,height,color,type`; edges `fromNode,toNode,fromSide,toSide,fromEnd,toEnd,label`), with nodes referencing entities by id or embedding text/markdown. `GraphModel` + `EdgeStyle` were shaped so this mapping is mechanical. Consider JSON Canvas import/export as a bonus interoperability feature.
- **Server-side graph**: a `semantic.graph.neighborhood` app command, or SQL/PGQ `GRAPH_TABLE` once the graph query plan lands, can replace `RpcGraphSource` behind the same `GraphSource` trait.

## 10. References

- xyflow / React Flow: architecture (DOM nodes, SVG edges, direct viewport transform, ResizeObserver measurement): https://reactflow.dev, https://github.com/xyflow/xyflow/discussions/2973; performance guide: https://reactflow.dev/learn/advanced-use/performance
- JSON Canvas spec 1.0: https://jsoncanvas.org/spec/1.0/
- d3-flextree (variable-size tidy tree; used by markmap): https://github.com/Klortho/d3-flextree
- A. van der Ploeg, "Drawing non-layered tidy trees in linear time", Software: Practice and Experience, 2014.
- Mind map layout overview (d3-tree, flextree, @antv/hierarchy): https://infinitecanvas.cc/guide/lesson-023
- dagre Rust port (Sugiyama reference): https://github.com/kookyleo/dagre-rs (moved to Actrium/supramark)
- rust-sugiyama: https://github.com/Nerixyz/rust-sugiyama
- dioxus-flow (reference implementation in Dioxus): https://github.com/XiangpengHao/dioxus-flow
- d3-force (velocity Verlet force simulation): https://d3js.org/d3-force
