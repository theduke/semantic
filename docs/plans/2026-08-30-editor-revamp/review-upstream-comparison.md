# Editor revamp: upstream implementation comparison

Date: 2026-08-30  
Local implementation reviewed: `cb61a58e221f1b4cf328fe63fe30fd5df331fd46`  
Scope: `crates/dxeditor`, especially the Tiptap/ProseMirror browser engine and its Rust bridge. No production code was modified during this review.

## Executive conclusion

The implementation made the right foundational choice—ProseMirror owns the editable DOM—but it is not yet safe to ship as the Markdown editor described by the plan. Four defects are release blockers:

1. the Markdown-to-v2-to-Markdown path cannot represent several constructs the UI exposes and already accepts;
2. normal editing, paste, and table commands produce duplicate or unstable semantic node IDs;
3. a pending debounced change is discarded on component destruction;
4. external document replacement ignores the declared history policy and can leave stale content undoable.

The contextual UI is also substantially below the Notion-like interaction and accessibility contract. It is implemented as transaction polling plus manually positioned DOM rather than as selection-aware ProseMirror plugin state. That causes overlapping surfaces, work on every transaction (twice for selection-changing transactions), no robust scroll/resize positioning, incomplete IME handling, and unusable keyboard-only slash/mention menus.

The strongest upstream patterns to adopt are narrow rather than wholesale rewrites:

- Tiptap's `UniqueID` append-transaction strategy for identity;
- Tiptap Bubble/Floating Menu plugin views and Floating UI positioning for lifecycle, composition, focus, scroll, and selection awareness;
- Tiptap Suggestion's mapped plugin state and deterministic dismissal;
- `prosemirror-tables` `TableMap`, `CellSelection`, and cell-slice paste pipeline;
- BlockNote's hovered-block side-menu state and table-handle separation as interaction references only.

## Reproducible upstream baseline

The following repositories were shallow-cloned outside the workspace and inspected at these commits. Links below are pinned to these hashes.

| Project | Commit | Areas inspected |
|---|---|---|
| [Tiptap](https://github.com/ueberdosis/tiptap/tree/d02646bdaa6a1cdf88a450a67fb2596abf5305d9) | `d02646bdaa6a1cdf88a450a67fb2596abf5305d9` | core transaction/event lifecycle; `setContent`; Bubble/Floating Menu; Suggestion; UniqueID; UndoRedo; Table |
| [prosemirror-view](https://github.com/ProseMirror/prosemirror-view/tree/ca4c78e9b56f1b164c0b3758b59d8748f11b7534) | `ca4c78e9b56f1b164c0b3758b59d8748f11b7534` | clipboard, composition, view/plugin lifecycle, event precedence |
| [prosemirror-state](https://github.com/ProseMirror/prosemirror-state/tree/ffad5d9450a0b93438be53a801deee1a223a81bf) | `ffad5d9450a0b93438be53a801deee1a223a81bf` | transaction/selection/plugin-state invariants |
| [prosemirror-tables](https://github.com/ProseMirror/prosemirror-tables/tree/eb522f25959a1e4515a4ff5ce7e3a939f19c55e9) | `eb522f25959a1e4515a4ff5ce7e3a939f19c55e9` | table normalization, cell selections, copy/paste, commands, resizing |
| [BlockNote](https://github.com/TypeCellOS/BlockNote/tree/19b9b19d1681867a6a0a5ca71c07a864d4e812b1) | `19b9b19d1681867a6a0a5ca71c07a864d4e812b1` | side menu, suggestion menu, table handles, keyboard interaction |

BlockNote was used only as an interaction/architecture comparison. This report does not recommend copying its implementation or assets.

## Prioritized defects

### P0-1 — The primary Markdown round trip is structurally incomplete

Classification: concrete correctness defect; release blocker.

Evidence:

- `MarkdownEditor` deliberately decodes and emits the legacy `"markdown"` codec (`crates/dxeditor/src/component.rs:1037-1063`). Because Markdown is not registered in `DocumentFormatRegistry` (`crates/dxeditor/src/format.rs:225-229`), `decode_editor_payload` takes the legacy codec path and then calls `migrate_v1_to_v2` (`crates/dxeditor/src/component.rs:784-808`). Encoding performs the inverse migration (`crates/dxeditor/src/component.rs:811-833`).
- The Markdown parser/serializer supports hard breaks, images, raw HTML, task markers, strikethrough, and GFM tables (`crates/dxeditor/src/markdown.rs:735-813`, `644-693`).
- The v1→v2 migration accepts only text and mention as known inline nodes (`crates/dxeditor/src/migrate.rs:177-239`) and only bold, italic, code, and link as known marks (`crates/dxeditor/src/migrate.rs:243-270`). With the default `UnknownComponentPolicy::Reject` (`crates/dxeditor/src/migrate.rs:37-47`), existing Markdown containing image, hard break, raw HTML, or strikethrough can fail to open in `MarkdownEditor`.
- Task semantics are not migrated to `task_list`/`task_item`: legacy lists are reduced to ordered versus bullet (`crates/dxeditor/src/migrate.rs:88-165`). Thus a parsed Markdown task can become an ordinary list despite the browser engine exposing Tiptap task nodes.
- v2→v1 block migration does not accept task lists/items or opaque blocks (`crates/dxeditor/src/migrate.rs:368-415`); inline migration does not accept image, hard break, or opaque inline (`crates/dxeditor/src/migrate.rs:418-470`); mark migration does not accept strike (`crates/dxeditor/src/migrate.rs:472-489`).
- The browser maps Tiptap header cells to v2 `table_header` (`crates/dxeditor/web/src/index.ts:577-585`), but v2→v1 table migration accepts only `table_cell` (`crates/dxeditor/src/migrate.rs:491-539`). A table created from the slash menu has header cells (`crates/dxeditor/web/src/index.ts:939-952`) and therefore cannot be emitted as Markdown.
- Conversely, a Markdown table migrates legacy cells to v2 `table_cell`, with header status merely retained as an attribute (`crates/dxeditor/src/migrate.rs:272-321`), while `v2NodeToPm` always turns `table_cell` into a Tiptap `tableCell` (`crates/dxeditor/web/src/index.ts:508-511`). Its header row is not rendered as `th`.

Impact:

- Supported source can fail to open.
- UI-supported edits can set status to `Encoding failed` and never call the public Markdown `on_change`.
- Tasks and table headers can silently change semantics.
- Existing unit tests test the Markdown codec and migration pieces separately, but not the actual `MarkdownEditor` pipeline (`crates/dxeditor/src/markdown.rs:855-933`; `crates/dxeditor/web/src/index.test.ts:25-135`).

High-level fix:

1. Make Markdown a native v2 `DocumentFormat`, directly translating between Markdown and `ComponentDocumentV2`. Do not route the primary format through the incomplete v1 compatibility model.
2. Until that lands, complete both migration directions for every node and mark in the declared Markdown profile: tasks, strike, hard break, image, mention, table header/cell, and opaque Markdown.
3. Make the construct capability table executable: startup/catalog construction must fail when a component is offered in the browser but lacks decode and encode handlers for the active format.
4. Do not expose a command when its result cannot be represented by the output format; return a diagnostic instead.

Required tests:

- One integration parameter table that mounts or invokes the exact `MarkdownEditor` decode/encode path for every supported construct.
- For every slash/table/mark command: Markdown → engine JSON → command → v2 snapshot → Markdown → v2, with semantic equality assertions.
- Explicit regressions for image, hard break, strike, raw HTML opaque nodes, task items, table headers, and mentions.
- A test asserting that every command eligible in a Markdown session has a lossless encoder or is disabled with a stable reason.

### P0-2 — Semantic IDs become duplicate or unstable during ordinary edits

Classification: concrete data-integrity defect; release blocker.

Evidence:

- `SemanticId` only declares a nullable global attribute (`crates/dxeditor/web/src/index.ts:182-205`). No plugin fills missing IDs or resolves copied/split IDs.
- `pmNodeToV2` invents an ID while serializing a node whose attribute is null (`crates/dxeditor/web/src/index.ts:548-550`) but never writes that ID back to the ProseMirror document. The same node can therefore receive a different ID on every later snapshot.
- ProseMirror block splitting can copy node attributes. A direct Chromium reproduction against the checked-in fixture—place the caret in `block-1`, press Enter, and snapshot—produced two paragraphs both identified as `block-1`.
- Internal clipboard paste restores the serialized slice with all `semanticId` attributes intact (`crates/dxeditor/web/src/index.ts:736-743`, `768-779`), so copying content duplicates IDs. Only the custom `duplicateCurrentBlock` path explicitly regenerates them (`crates/dxeditor/web/src/index.ts:898-910`).
- Table commands create nodes through schema defaults; newly generated rows/cells can have null IDs and consequently snapshot-only IDs. The custom TSV path generates IDs, but native table commands do not (`crates/dxeditor/web/src/index.ts:843-872`).
- Rust validation treats duplicate IDs as invalid (`crates/dxeditor/src/document_v2.rs:421-476`; `crates/dxeditor/src/component_spec.rs:405-451`), and default encode normalization does not repair them (`crates/dxeditor/src/format.rs:305-318`). Thus the browser can create snapshots that Rust refuses to encode.

Upstream comparison:

- Tiptap's [UniqueID extension](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-unique-id/src/unique-id.ts#L69-L189) fills missing IDs outside history.
- Its [append-transaction logic](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-unique-id/src/unique-id.ts#L226-L371) confines scanning to changed ranges and repairs duplicates introduced by splits/copies.
- Its [clipboard transform](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-unique-id/src/unique-id.ts#L391-L467) removes IDs from pasted/copied content so append-transaction assigns new identities.

High-level fix:

1. Add a project-owned identity extension modeled on `UniqueID`, configured for all persisted semantic block/atom/table nodes.
2. Generate opaque IDs with `crypto.randomUUID()` (with a reviewed fallback), not timestamp plus a per-page counter.
3. In an append transaction, assign IDs to missing nodes and regenerate only duplicate occurrences introduced in changed ranges; mark maintenance transactions `addToHistory: false` and retain stored marks.
4. Strip IDs on copy-paste and copy-drag, preserve them on true moves, and continue regenerating them for explicit duplicate.
5. Validate uniqueness before every emitted engine snapshot and emit a structured engine error rather than passing an invalid document to Rust.

Required tests:

- Enter at beginning/middle/end of every splittable block.
- Backspace merge, list split/lift/sink, table row/column creation, block duplicate, internal copy/paste, HTML fallback paste, and copy-drag versus move-drag.
- Two successive snapshots without a document change have identical IDs.
- Property test: arbitrary supported command sequences always produce nonempty unique IDs on every identity-bearing node.

### P0-3 — Destroy drops pending input; blur does not actually flush the debounce

Classification: concrete durability defect; release blocker.

Evidence:

- Document changes are delayed for 180 ms (`crates/dxeditor/web/src/index.ts:753-760`).
- Blur emits a current snapshot but does not cancel/consume the pending timer and does not advance its revision (`crates/dxeditor/web/src/index.ts:763-765`). A later timer emits the same state as a second, higher revision.
- Destroy clears the pending timer and never emits or returns the final snapshot (`crates/dxeditor/web/src/index.ts:1230-1237`).
- Dioxus drop calls only `Destroy` (`crates/dxeditor/src/bridge/mod.rs:18-21`), so a component removed within the debounce window can lose its final edit. This contradicts the plan's explicit blur/component-drop flush gate.

Upstream comparison:

- Tiptap [`Editor.destroy`](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/core/src/Editor.ts#L828-L847) and ProseMirror [`EditorView.destroy`](https://github.com/ProseMirror/prosemirror-view/blob/ca4c78e9b56f1b164c0b3758b59d8748f11b7534/src/index.ts#L458-L473) are teardown operations, not persistence hooks. The host must flush before calling them.

High-level fix:

1. Replace the loose timer with one snapshot scheduler owning `dirty`, `lastEmittedDoc`, revision, and `flush(reason)`.
2. `flush` must clear the timer, increment revision exactly once if the document differs from the last emitted snapshot, and emit that revision/document.
3. Call it on blur, explicit save/submit, `visibilitychange`/`pagehide` where appropriate, and before teardown.
4. Add a typed `Flush`/`DestroyWithSnapshot` bridge command or a synchronous registry method so Dioxus can receive the last snapshot before destroying the editor. Define the unavoidable browser-unload limitation separately.

Required tests:

- Type and unmount at 0, 1, 179, and 181 ms.
- Type then blur before/after the debounce; assert one monotonic document revision and no duplicate callback.
- Rapid blur/refocus and destroy after a pending cut/table command.
- Parent acknowledgment arriving between blur flush and the former timer deadline.

### P0-4 — `HistoryPolicy` is ignored during external replacement

Classification: concrete protocol/history defect; release blocker.

Evidence:

- The Rust protocol declares `HistoryPolicy::{Preserve, Reset}` on `ReplaceDocument` (`crates/dxeditor/src/protocol.rs:49-71`).
- The bridge discards that field with `..` (`crates/dxeditor/src/bridge/mod.rs:125-147`).
- The TypeScript `replaceDocument` API does not accept a policy and always calls `setContent` (`crates/dxeditor/web/src/index.ts:1220-1227`).
- Tiptap [`setContent`](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/core/src/commands/setContent.ts#L51-L78) is a document replacement transaction; `emitUpdate: false` only sets update-event metadata. It neither clears history nor implements this protocol's reset semantics.
- Tiptap UndoRedo is ordinary ProseMirror history ([source](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extensions/src/undo-redo/undo-redo.ts#L46-L85)). Old local history can therefore survive a server document replacement, while the replacement itself may also enter history.

Impact: Undo after an external load can resurrect stale or conflicting content, violating revision isolation.

High-level fix:

1. Carry `history_policy` through the registry API.
2. For `Preserve`, replace with an explicitly non-history transaction and close the previous history group.
3. For `Reset`, create a fresh ProseMirror state/history plugin state (or deliberately remount the engine while preserving only approved UI state); do not assume `setContent` clears history.
4. Define selection/focus behavior for both policies and test the revision guard around the operation.

Required tests: local edit → acknowledge → external replace → undo/redo under each policy; replace during a closed/open history group; focus/selection mapping; stale command rejection.

## High-priority correctness and performance work

### P1-1 — Table TSV paste bypasses table maps, cell selection, and span invariants

Classification: concrete correctness deficit.

Evidence:

- `pasteTableGrid` locates row/cell indices through ancestor depth and clones/replaces the entire table JSON (`crates/dxeditor/web/src/index.ts:791-841`).
- It declines any table containing a rowspan or colspan, ignores a rectangular `CellSelection`, does not clip/repeat to the selection, and relies on transaction mapping to recover a usable selection after replacing the whole table.
- Replacing the full table creates a large transaction and invalidates node identity/decorations more broadly than necessary.

Upstream comparison:

- `tableEditing` guarantees cell selection, paste handling, and table repair in one plugin ([source](https://github.com/ProseMirror/prosemirror-tables/blob/eb522f25959a1e4515a4ff5ce7e3a939f19c55e9/src/index.ts#L83-L146)). Tiptap Table correctly installs resizing before `tableEditing` ([source](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-table/src/table/table.ts#L596-L615)); the local extension benefits from this part already.
- Upstream paste computes the target rectangle with `TableMap`, supports `CellSelection`, clips/repeats content, and inserts cell slices ([input](https://github.com/ProseMirror/prosemirror-tables/blob/eb522f25959a1e4515a4ff5ce7e3a939f19c55e9/src/input.ts#L129-L171), [copy/paste algorithms](https://github.com/ProseMirror/prosemirror-tables/blob/eb522f25959a1e4515a4ff5ce7e3a939f19c55e9/src/copypaste.ts#L32-L178), [insertion](https://github.com/ProseMirror/prosemirror-tables/blob/eb522f25959a1e4515a4ff5ce7e3a939f19c55e9/src/copypaste.ts#L318-L384)).

High-level fix: turn TSV into a valid rectangular cell slice/area, then use or closely adapt the upstream table insertion primitives. Keep configured limits, but apply them before allocating/growing. Preserve header types and cell attributes, set an explicit post-paste `CellSelection`, and let `fixTables` normalize.

Tests: single-cell and rectangular selections; smaller/larger source than selection; bottom/right growth; headers; row/col spans in typed mode; undo as one event; IDs; 100×100 limit; HTML/TSV parity.

### P1-2 — Contextual surfaces use the wrong selection/lifecycle abstraction

Classification: concrete UI correctness and performance defect.

Evidence:

- Both `onSelectionUpdate` and `onTransaction` call `updateSurfaces` (`crates/dxeditor/web/src/index.ts:761-762`). Tiptap emits `transaction` and then `selectionUpdate` for the same selection-changing transaction ([core dispatch](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/core/src/Editor.ts#L690-L720)), so position reads/writes and suggestion scans run twice.
- Every call performs several `coordsAtPos`/layout reads, wrapper bounds reads, DOM queries, inline styles, text scans, and async suggestion orchestration (`crates/dxeditor/web/src/index.ts:1148-1198`).
- It does not check `view.composing`, so floating UI and async queries update during composition.
- `bubble.hidden = empty || imageActive` treats any nonempty selection—including a table `CellSelection`—as text formatting. Table and text toolbars can overlap. Node selections other than images are similarly not classified.
- `blockControls.hidden = !empty` exposes whole top-level block actions for every collapsed caret, including inside nested lists/tables. Yet `currentTopLevelRange` always targets depth 1 (`crates/dxeditor/web/src/index.ts:887-930`), so a control visually next to a table cell can duplicate/delete/move the whole table.
- Positioning only runs on editor transactions/selections. There are no scroll, resize, `ResizeObserver`, or `VisualViewport` updates (`crates/dxeditor/web/src/index.ts:617-621`, `1148-1198`). `positionSurface` only clamps horizontally and above; it cannot flip, shift, or avoid clipping.

Upstream comparison:

- Tiptap Bubble Menu classifies focus, empty text, `NodeSelection`, and `CellSelection`, debounces updates, skips composition, and uses Floating UI middleware ([selection/anchor handling](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-bubble-menu/src/bubble-menu-plugin.ts#L195-L220), [cell/node anchors](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-bubble-menu/src/bubble-menu-plugin.ts#L303-L368), [update path](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-bubble-menu/src/bubble-menu-plugin.ts#L509-L587)).
- It also owns focus/blur/scroll/resize listeners and removes them in plugin-view destruction ([lifecycle](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-bubble-menu/src/bubble-menu-plugin.ts#L371-L469)).
- BlockNote's side menu is hover/block based rather than “every caret is a block handle”; it tracks the hovered block and updates only when the block/document changes ([source](https://github.com/TypeCellOS/BlockNote/blob/19b9b19d1681867a6a0a5ca71c07a864d4e812b1/packages/core/src/extensions/SideMenu/SideMenu.ts#L125-L275)).

High-level fix:

1. Introduce a single ProseMirror plugin that derives a compact `UiState`: selection kind, target block/node/table, mapped anchor, enabled/active command states, and one winning contextual surface.
2. Enforce arbitration: modal/popdown → node/table → link → text selection → suggestion → passive hovered/focused block gutter.
3. Update only when selection/doc/UI plugin state actually changes; skip geometry work while composing; coalesce positioning in `requestAnimationFrame`.
4. Use Tiptap menu plugins/Floating UI or equivalent reviewed primitives with scroll/resize/visual-viewport observers and cleanup.
5. Make the block gutter target an explicitly resolved semantic block, not implicitly depth 1 from any caret.

Tests: CellSelection never opens the text bubble; node selection opens one node surface; nested list/table targets; scroll/resize/zoom; editor blur into a popup; composition; multi-line and RTL selections; teardown listener leak test.

### P1-3 — Slash and mention menus are mouse-only regex polling, not mapped editor state

Classification: concrete keyboard/accessibility and IME defect.

Evidence:

- Slash and mention matching rescans text before the caret with regular expressions (`crates/dxeditor/web/src/index.ts:932-937`, `1078-1084`, `1184-1197`). Ranges are recomputed from the current selection instead of mapped across transactions.
- Neither listbox handles ArrowUp/ArrowDown, Home/End, Enter, or active option state. The typed `/` menu leaves focus in the editor; mention insertion is effectively pointer-only (`crates/dxeditor/web/src/index.ts:1053-1071`, `1117-1130`).
- `role=listbox` options lack `aria-selected`; the contenteditable lacks `aria-controls`, `aria-expanded`, and active-descendant association.
- Escape merely hides the surface. Because no dismissed range is stored, the same trigger can reopen on the next transaction.
- Matching is not excluded in code blocks or other ineligible contexts and does not explicitly protect Enter during composition.

Upstream comparison:

- Tiptap Suggestion stores active range/query/composition/dismissal in plugin state and maps dismissed ranges through changes ([state](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/suggestion/src/plugin/state.ts#L47-L180)); Escape dispatches plugin metadata for deterministic closure ([keyboard props](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/suggestion/src/plugin/props.ts#L36-L93)).
- BlockNote similarly stores a tracked trigger position and rejects code-block contexts ([source](https://github.com/TypeCellOS/BlockNote/blob/19b9b19d1681867a6a0a5ca71c07a864d4e812b1/packages/core/src/extensions/SuggestionMenu/SuggestionMenu.ts#L244-L325)). Its keyboard handler covers arrows, paging, Enter, and composition ([source](https://github.com/TypeCellOS/BlockNote/blob/19b9b19d1681867a6a0a5ca71c07a864d4e812b1/packages/react/src/components/SuggestionMenu/hooks/useSuggestionMenuKeyboardHandler.ts#L5-L71)).

High-level fix: use separate keyed Suggestion plugins for `/` and `@`, with eligibility predicates, mapped ranges, deterministic dismissal, async request IDs/abort, and an accessible active-descendant or managed-focus listbox controller. Insertion must delete the plugin's mapped range atomically.

Tests: complete keyboard matrix, no Enter during `isComposing`, Escape dismissal persistence, cursor relocation, document mutation while results are pending, code/table eligibility, zero/many results, stale result suppression, screen-reader announcements.

### P1-4 — Format-specific presentation controls can claim persistence that Markdown cannot provide

Classification: concrete product/data-fidelity defect.

Evidence:

- Tables are configured `resizable: true` (`crates/dxeditor/web/src/index.ts:720`), and `pmNodeToV2` persists `colwidth` (`crates/dxeditor/web/src/index.ts:577-585`). GFM Markdown serialization persists alignment only, not widths (`crates/dxeditor/src/markdown.rs:644-693`). A resize can appear saved in the live editor but disappear after reload.
- The generic command surface is not passed active-format capabilities; it cannot disable or label lossy operations (`crates/dxeditor/web/src/index.ts:843-885`).

This is explicitly contrary to the plan's rule that unsupported Markdown presentation attributes must be hidden, editor-only, or diagnosed.

High-level fix: pass a generated format capability manifest into the engine. For Markdown sessions, make column width explicitly ephemeral UI state (not document state), or disable resizing; for typed sessions, allow persisted `colwidth`. Never report “Saved” for an attribute the selected encoder discarded without a diagnostic.

Tests: resize/reload in Markdown and typed formats; capability-driven command visibility; encoder fidelity diagnostic assertions.

### P1-5 — The 468 KB engine bundle is parsed/evaluated for every mounted editor

Classification: concrete startup performance and CSP/maintainability defect.

Evidence:

- The 468,379-byte checked-in IIFE is embedded as `ENGINE_SCRIPT` (`crates/dxeditor/src/bridge/mod.rs:8`).
- Every `EditorBridge` mount interpolates the entire bundle into a new `document::eval` script (`crates/dxeditor/src/bridge/mod.rs:23-86`). `window.__semanticDxEditor ??=` prevents replacing the eventual registry, but it does not prevent JavaScript parsing/executing the preceding bundle for each mount.
- The plan called for a narrow typed boundary and specifically rejected arbitrary JS snippets/user content at that boundary; the current command bridge still constructs and evaluates JavaScript strings (`crates/dxeditor/src/bridge/mod.rs:97-209`). JSON quoting makes current interpolation materially safer, but the architecture remains difficult to audit and CSP-hostile.

High-level fix:

1. Load the hashed engine asset once as a module/script at application startup (or through a once-only loader promise).
2. Mount editors through a stable typed JS/WASM binding; keep data as structured values rather than source strings.
3. Split optional heavy features only after measuring; first eliminate repeated parse/eval.
4. Add performance marks for bundle load, editor construction, first editable paint, and second-editor mount.

Tests/benchmarks: assert bundle initialization once across N editors; strict CSP without unsafe-eval; mount/destroy leak loop; cold and warm startup budgets.

## Important maintainability and completeness work

### P2-1 — Rust catalog, wire model, and ProseMirror schema are manually duplicated

Classification: architecture deficit that blocks the promised future typed-component extensibility.

Evidence:

- Component kinds/attributes live in Rust (`crates/dxeditor/src/document_v2.rs`, `component_spec.rs`) and are independently re-declared as loose TypeScript records and switch statements (`crates/dxeditor/web/src/index.ts:28-58`, `452-603`).
- The browser extension array is static (`crates/dxeditor/web/src/index.ts:714-722`). Registering a Rust component does not create an editable ProseMirror node or node view; unknown components become generic opaque nodes (`crates/dxeditor/web/src/index.ts:466-474`, `512-514`).
- The internal clipboard envelope calls the Rust catalog fingerprint a schema fingerprint (`crates/dxeditor/web/src/index.ts:126-154`, `736-743`), although compatibility is actually determined by the JavaScript ProseMirror schema plus slice codec. A Rust renderer/catalog change can reject compatible slices; a JavaScript schema change not reflected in the Rust fingerprint can accept incompatible ones.

High-level fix:

1. Generate TypeScript wire types, component descriptors, and golden schema fixtures from one versioned manifest.
2. Keep behavior adapters explicit: each typed component registers its Tiptap node/mark extension, node view, commands, clipboard behavior, and format capabilities. Catalog build fails if any required adapter is absent.
3. Separate `document_catalog_fingerprint` from `pm_clipboard_schema_fingerprint`; derive the latter from ordered PM node/mark specs plus clipboard codec version.
4. Add one real custom typed atomic component fixture proving typed load/edit/copy/paste/read render/encode, rather than treating opaque fallback as extensibility.

### P2-2 — Toolbar/menu semantics and command state are incomplete

Classification: concrete accessibility deficit.

Evidence:

- All controls are ordinary buttons created by one helper (`crates/dxeditor/web/src/index.ts:605-615`). Formatting buttons never expose `aria-pressed`/mixed state; unavailable table commands are not disabled.
- `role=toolbar` surfaces have every button in the tab order and no arrow/Home/End roving focus (`crates/dxeditor/web/src/index.ts:626-672`, `969-980`, `1072-1076`).
- `role=menu` children are not assigned `menuitem`; listbox options omit selected state.
- The public `aria_label` prop is attached to the host (`crates/dxeditor/src/component.rs:456-468`, `656-660`), but the actual contenteditable receives a hard-coded “Document editor” (`crates/dxeditor/web/src/index.ts:729-731`).
- The existing axe test checks only the idle editor state (`crates/dxeditor/web/e2e/editor.spec.ts:48-53`), which cannot verify keyboard behavior or most hidden contextual surfaces.

High-level fix: include `canExecute`/active/mixed command state in `UiState`; implement APG toolbar roving focus and menu/listbox patterns; expose a documented editor-to-toolbar shortcut; pass the real accessible name to the contenteditable; test every open surface by keyboard and with an accessibility tree snapshot plus manual AT matrix.

### P2-3 — Table UI is a generic toolbar rather than contextual row/column/cell handles

Classification: product completeness deficit, not an engine defect.

Evidence: the local table surface offers add/delete-after, one header toggle, alignment, and delete table in one toolbar (`crates/dxeditor/web/src/index.ts:1072-1076`). It does not expose selecting a row/column/table, before/after placement, edge handles, or a narrow-screen overflow. Tiptap already exposes merge/split, header row/column/cell, navigation, repair, and programmatic cell selection ([commands](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-table/src/table/table.ts#L170-L257)); only the GFM-safe subset should be surfaced in Markdown mode. BlockNote demonstrates the useful separation of edge handles, add/remove controls, and cell menus ([TableHandles extension](https://github.com/TypeCellOS/BlockNote/blob/19b9b19d1681867a6a0a5ca71c07a864d4e812b1/packages/core/src/extensions/TableHandles/TableHandles.ts)).

High-level fix: build row/column edge handles from the table plugin state, with explicit target index and before/after commands. Add row/column/table selection actions and mobile overflow. Keep merge/split disabled in Markdown sessions because GFM cannot encode spans; enable only when a typed format declares it lossless.

### P2-4 — Notion-like block drag interaction is not implemented

Classification: scoped enhancement required by the accepted interaction plan.

The local gutter is shown beside every collapsed caret and offers buttons only (`crates/dxeditor/web/src/index.ts:1038-1052`, `1148-1164`). Keyboard move buttons are a good fallback, but there is no hovered-block drag handle, drag image, mapped drop target, or copy-versus-move identity behavior. BlockNote's side-menu plugin demonstrates block hit-testing, frozen-menu state, cross-editor drag cleanup, and listener teardown ([source](https://github.com/TypeCellOS/BlockNote/blob/19b9b19d1681867a6a0a5ca71c07a864d4e812b1/packages/core/src/extensions/SideMenu/SideMenu.ts#L125-L275), [cleanup](https://github.com/TypeCellOS/BlockNote/blob/19b9b19d1681867a6a0a5ca71c07a864d4e812b1/packages/core/src/extensions/SideMenu/SideMenu.ts#L599-L723)).

High-level fix: first resolve P0 identity and P1 contextual state. Then add a plugin-owned hovered semantic-block target and use ProseMirror's native drag slice/transaction mapping; preserve IDs for moves and regenerate for copies. Retain move-up/down keyboard controls.

### P2-5 — Test breadth does not match the implementation's claims

Classification: verification deficit.

Current browser tests cover a useful happy-path slice but not the invariants above (`crates/dxeditor/web/src/index.test.ts:25-174`; `crates/dxeditor/web/e2e/editor.spec.ts:9-114`). Missing release-gate coverage includes:

- exact MarkdownEditor integration, all declared constructs, and command-to-Markdown round trips;
- identity under split/paste/table/list/drag operations;
- history policy and external revision scenarios;
- blur/unmount durability;
- IME/composition events, RTL, cross-block deletion, browser HTML paste corpus;
- keyboard operation of every popup and toolbar;
- cell selections, header preservation, HTML/TSV rectangular paste, undo, and table normalization;
- scroll/resize/mobile viewport positioning;
- strict CSP and multiple-editor startup/leak measurements.

High-level fix: make these invariant suites prerequisites to feature breadth. Run Chromium, Firefox, and WebKit where host libraries exist, but retain real-device/IME/AT manual gates because browser automation cannot prove them.

## Recommended implementation order

1. **Codec gate:** make Markdown a complete v2 format (or complete migrations), add construct/command capability tests, and remove unsupported commands until lossless.
2. **Identity gate:** add the unique-ID plugin and copy/move identity policies; run command-sequence invariant tests.
3. **Durability/history gate:** centralize flush/revision scheduling and implement both history policies.
4. **Bridge gate:** load the engine once and replace source-string eval with a structured binding.
5. **Context-state gate:** introduce one PM UI-state plugin, composition-aware updates, explicit surface arbitration, and robust Floating UI positioning.
6. **Suggestion/a11y gate:** move slash/mention to keyed suggestion state and implement full keyboard/ARIA behavior.
7. **Table gate:** replace full-table TSV rebuilding with TableMap/cell-slice insertion; add handles and GFM-aware capabilities.
8. **Interaction completion:** hovered block gutter and drag/reorder with keyboard parity; mobile/touch adaptations.
9. **Release gate:** cross-browser suites, real IME/mobile/AT matrix, CSP, performance budgets, and unmount/reload durability.

Do not begin drag polish or additional component types before steps 1–3 pass. Those defects affect persisted user data and make later UI testing unreliable.

## Upstream practices already used correctly

These parts should be retained:

- ProseMirror/Tiptap exclusively owns the mounted editable subtree (`crates/dxeditor/web/src/index.ts:724-766`).
- The standard Tiptap Table extension already installs column resizing and `tableEditing`, so native table normalization, `CellSelection`, Tab behavior, and ordinary table clipboard support are present beneath the custom UI ([Tiptap plugin registration](https://github.com/ueberdosis/tiptap/blob/d02646bdaa6a1cdf88a450a67fb2596abf5305d9/packages/extension-table/src/table/table.ts#L575-L615)).
- Native copy serialization is used for interoperable HTML/plain fallbacks (`crates/dxeditor/web/src/index.ts:768-779`), matching ProseMirror's clipboard pattern ([source](https://github.com/ProseMirror/prosemirror-view/blob/ca4c78e9b56f1b164c0b3758b59d8748f11b7534/src/input.ts#L595-L612)).
- Internal clipboard input is treated as untrusted and bounded before `Slice.fromJSON` (`crates/dxeditor/web/src/index.ts:97-154`).
- Mention requests are debounced, cancellable, and stale-query guarded (`crates/dxeditor/web/src/index.ts:1086-1146`). Preserve those properties when moving to plugin state.
- The bridge serializes interpolated data through `serde_json` rather than hand-escaping it (`crates/dxeditor/src/bridge/mod.rs:29-37`, `106-114`, `128-135`).
- Opaque nodes render inert text rather than executable raw HTML (`crates/dxeditor/web/src/index.ts:231-278`).

## Final ship assessment

The implementation is a promising vertical spike, not the completed production editor described in the plan. The engine selection is sound and several low-level integrations are good, but persisted Markdown correctness, semantic identity, durability, and history isolation must be fixed before broader rollout. Once those gates pass, adopting upstream plugin-state patterns will reduce bespoke code while materially improving IME behavior, contextual correctness, performance, accessibility, and maintainability.
