# Native desktop smoke and performance

Verified on 2026-10-03 with native Dioxus desktop/WebKitGTK, an Xvfb display at 1440×1000, and the existing isolated RPC server on port 8888. No user database was opened. The desktop bundle was built with global assets; running the bare Cargo binary alone does not bundle those assets.

## Commands

```sh
nix develop --command cargo build --quiet --message-format=short -p semantic_ui --features desktop
nix develop --command dx build --desktop --package semantic_ui --no-default-features --features desktop
nix develop --command nix shell nixpkgs#xorg-server --command Xvfb :93 -screen 0 1440x1000x24 -nolisten tcp
nix develop --command env DISPLAY=:93 WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:9223 SEMANTIC_RPC_URL=http://127.0.0.1:8888/api/v1/rpc target/dx/semantic_ui/debug/linux/app/semantic_ui
nix develop --command cargo build --quiet --message-format=short -p dxgraph --example demo --features demo
nix develop --command env DISPLAY=:93 WEBKIT_INSPECTOR_HTTP_SERVER=127.0.0.1:9224 target/debug/examples/demo
nix develop --command nix shell nixpkgs#imagemagick --command env DISPLAY=:93 INSPECTOR_PORT=9224 node docs/plans/2026-10-03-graph-views/desktop.mjs
nix develop --command nix shell nixpkgs#imagemagick --command env DISPLAY=:93 INSPECTOR_PORT=9223 node docs/plans/2026-10-03-graph-views/desktop.mjs
```

The two inspector ports identify the standalone demo and Semantic application. `desktop.mjs` sends real X11 input through xdotool and observes rendered state through WebKitGTK's test inspector. It selects the largest window belonging to the process, avoiding GTK's 1×1 helper windows. `DESKTOP_PID` and `MAGICK` can override process selection and screenshot executable. Native DPR was approximately 1.041667, so input coordinates include device scaling and the menu-bar offset. The inspector is restricted to loopback and enabled only by the test launch environment. Its environment variable is documented by [WebKit](https://trac.webkit.org/wiki/EnvironmentVariables). The application and example contain no JavaScript inspection or evaluation code.

The Semantic scenario uses isolated fixture Notes `graph-desktop-note-root`, `graph-desktop-note-child-1`, and `graph-desktop-note-child-2`. Their titles are `Desktop graph note`, `Desktop child 1`, and `Desktop child 2`; each has existing `semantic:base:note` type, `note_format="text"`, and `note_content="fixture"`. Both children refer to the root with `semantic:parent`. No schema changes are involved. The script searches the root by title if a graph is not already open.

## Results

Both native scenarios passed background pan, wheel zoom, node drag with incident SVG path updates, and a 30-move burst scheduled at 60 Hz. Semantic also passed title autocomplete, measured initial fit containing all three nodes and both edges, and selecting the root to render its EntityCard with `fixture` content. The Semantic canvas measured 1116×480 CSS pixels, and all three measured node bounds were inside it after the initial fit.

The standalone example contains 300 nodes and 299 hierarchy edges. At 1× zoom after measurement and centering, the actual native DOM contained **5 node wrappers and 12 edge groups**, with no hidden measurement wrappers. The controller centered the root correctly in the measured 1228.8×838.1 CSS pixel canvas. The independent Rust culling test also verifies selected and dragged offscreen nodes remain exposed, and panning changes the visible set.

| Native scenario | Pan median / p95 | Drag median / p95 | 30-event burst | Last input → DOM |
| --- | --- | --- | --- | --- |
| 300-node demo | 30.4 / 31.0 ms | 32.1 / 33.0 ms | 519 ms | 19.7 ms |
| Semantic graph | 31.1 / 32.3 ms | 31.5 / 32.9 ms | 531 ms | 32.5 ms |

Each latency distribution has 20 moves. Timings include xdotool process startup and inspector polling and therefore give an upper bound on Rust/IPC handling, rather than isolated IPC duration. The 60 Hz test sends events without waiting for each render and checks the final viewport displacement; it verifies throughput and eventual state, not presentation of every frame. These runs showed no stuck pan/drag or missing final update, so no event-coalescing or JS fallback was introduced.

The 300-node layout tests passed in both intended profiles: debug tree 0.722 ms / force 441 ms; release tree 0.172 ms / force 63.138 ms. Release performance therefore meets the plan's tree <50 ms and force <500 ms goals, and the observed debug run also meets them.

## Regressions found and corrected

Node resize events bubbled to the canvas and replaced its container dimensions with a node's size. This made controller centering incorrect on desktop. `NodeView` now stops resize propagation, with a regression checking that canvas resize handlers cannot receive node measurement events. Native centering and actual DOM culling passed with the fix.

Native title autocomplete also exposed an existing ambiguous `name` field alias in the generic entity query. The graph picker now uses explicit canonical title/ID fields, with the shared component's default query preserved and AST regression coverage. A valid typed Note fixture confirmed this was a query failure, rather than fixture eligibility. The final native title picker and browser ID/title picker both passed.

## Durable evidence

Generated evidence is kept in the ignored workspace directory `target/graph-views-desktop/`:

- `native-build.log`, `semantic-ui.log`, `demo.log`: build and application output.
- `results.json`, `demo-results.log`: actual 300-node DOM counts, bounds, gesture timings, and assertions.
- `semantic-results.json`, `semantic-results.log`: actual Semantic graph bounds, gesture timings, and selected EntityCard content.
- `demo-300-focused.png`, `demo-300-panned.png`, `demo-300-dragged.png`: native 300-node screenshots.
- `semantic-graph-focused.png`, `semantic-graph-panned.png`, `semantic-graph-dragged.png`, `semantic-graph-selected.png`: native Semantic screenshots.

Final verification also includes `cargo check --quiet --message-format=short -p dxgraph --features demo --examples`, `cargo test --quiet --message-format=short -p dxgraph`, and formatting the changed Rust files, all through the Nix devshell. The desktop example is behind the `demo` feature; inspector scripts are test-only tooling outside the shipping code.
