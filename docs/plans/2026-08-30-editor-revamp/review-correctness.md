# Editor revamp correctness review

Review target: commit `cb61a58e` and the editor implementation at `HEAD` on 2026-08-30.

This was a read-only production-code review. The checked-in browser bundle was rebuilt to a temporary file and is byte-for-byte identical to `web/dist/editor.iife.js`. The existing Rust, TypeScript, Vitest, and Chromium Playwright checks pass, but the current test layers do not exercise the most important Rust/JavaScript/Markdown integration path.

## Executive assessment

The ProseMirror editing island is a sound direction, but the implementation is not yet safe to ship as a Markdown editor. The primary Markdown path still goes through the legacy v1 document model, while the browser emits the richer v2 model. Several controls therefore create content that cannot be encoded, and several Markdown constructs cannot be opened. There is also a confirmed debounce race that can silently replace a local keystroke with an incoming controlled value.

The first release gate should be: every enabled editor command and every accepted Markdown construct must survive `Markdown -> v2 -> ProseMirror -> v2 -> Markdown`, and no local transaction may exist only inside a disposable browser timer.

## Confirmed defects

### 1. Critical — The primary Markdown editor cannot load or save several advertised node types

Evidence:

- Markdown is absent from the v2 `DocumentFormatRegistry`; only typed v2, legacy v1, and plain text are registered (`crates/dxeditor/src/format.rs:225-229`). Consequently, `Editor` decodes Markdown through `MarkdownCodec -> EditorDocument v1 -> migrate_v1_to_v2`, and encodes in the reverse direction (`crates/dxeditor/src/component.rs:784-834`).
- The v1-to-v2 migration recognizes only paragraph, heading, quote, code, ordinary/ordered lists, list items, divider, and table blocks (`crates/dxeditor/src/migrate.rs:88-103`). Its inline mapping accepts only text and mention (`crates/dxeditor/src/migrate.rs:177-238`), and its mark mapping omits strikethrough (`crates/dxeditor/src/migrate.rs:243-264`).
- The reverse migration omits `task_list`, `task_item`, images, hard breaks, opaque Markdown nodes, and strikethrough; table conversion also rejects `table_header` (`crates/dxeditor/src/migrate.rs:368-413`, `crates/dxeditor/src/migrate.rs:418-483`, `crates/dxeditor/src/migrate.rs:491-515`).
- Fenced-code attributes disagree across layers: the Markdown parser writes `language` (`crates/dxeditor/src/markdown.rs:327-339`), the v2 spec declares `info` (`crates/dxeditor/src/component_spec.rs:903-919`), and the JS v2 adapter uses `info` (`crates/dxeditor/web/src/index.ts:497-501`, `crates/dxeditor/web/src/index.ts:565-568`).
- Tables parsed from Markdown carry legacy `alignments`, row `header`, and cell `header` attributes, while the corresponding v2 specs do not declare those attributes (`crates/dxeditor/src/markdown.rs:411-485`, `crates/dxeditor/src/component_spec.rs:930-977`).
- Despite those conversion limits, the browser enables Strike, HardBreak, Image, TaskList/TaskItem, tables with headers, and opaque nodes (`crates/dxeditor/web/src/index.ts:714-722`) and exposes commands to create them (`crates/dxeditor/web/src/index.ts:843-882`, `crates/dxeditor/web/src/index.ts:1053-1076`).

Impact/reproduction:

- Initial Markdown containing a GFM task list, strikethrough, image, hard break, table, raw HTML, or fenced code can render the editor error state rather than the editor.
- In a successfully opened document, clicking Strikethrough, inserting an image/task list/table, or creating a hard break produces a valid ProseMirror document but `encode_editor_payload` fails. The Rust component only changes its status to `Encoding failed`; it does not call `on_change` (`crates/dxeditor/src/component.rs:683-703`). Blur silently ignores the same encode failure (`crates/dxeditor/src/component.rs:705-720`). The unsavable content remains visible until it is discarded.

High-level fix:

- Implement and register a native v2 `MarkdownDocumentFormat`; do not route the primary format through `EditorDocument` v1.
- Parse directly into standard v2 kinds and encode directly from v2. Preserve unsupported Markdown as the existing opaque block/inline nodes with source slices and diagnostics.
- Align code-block, table-header/alignment, image, task, hard-break, and strike attribute names in one shared contract.
- Until a kind has a lossless/declared Markdown encoder, capability-gate its command and insertion item for Markdown output rather than allowing unsavable edits.
- Treat encode failure as a blocking document error and retain a recoverable typed snapshot; never merely update a status string.

Suggested tests:

- A table-driven component-boundary test for every enabled standard kind and mark: decode Markdown through `Editor`, convert through JS, encode through `Editor`, and compare semantic structure plus a second-round canonical encoding.
- Explicit fixtures for tasks, strike, images, hard/soft breaks, fenced code with multiword info, raw HTML, tables/alignment/header cells, nested lists, mentions, and opaque constructs.
- A browser integration assertion that each visible insertion/format action results in a Markdown `on_change`, not only a v2 browser event.

### 2. Critical — A local edit can be silently overwritten during the 180 ms browser debounce

Evidence:

- `onUpdate` does not notify Rust immediately; it waits 180 ms before incrementing the local revision and emitting the document (`crates/dxeditor/web/src/index.ts:753-760`).
- Rust marks the session dirty only after that delayed `DocumentChange` arrives (`crates/dxeditor/src/component.rs:683-700`). Before then, the external-value effect sees `dirty == false` and applies a newer prop with `ReplaceDocument` (`crates/dxeditor/src/component.rs:557-587`).
- `replaceDocument` neither clears the outstanding local timer nor records/conflicts the pending local transaction (`crates/dxeditor/web/src/index.ts:1220-1228`). The old timer later serializes whichever document is currently installed and misreports it as a new local revision.

Impact/reproduction:

1. Type one character.
2. Within 180 ms, deliver a newer controlled value/external revision (for example, a remote update or form re-render).
3. Rust believes there is no local dirtiness and replaces the ProseMirror document, erasing the character. The pending timer then emits the replacement document as though it were a local change, obscuring the loss.

High-level fix:

- Make transaction ownership synchronous. Prefer emitting a revisioned v2 change to Rust on every document-changing transaction and debounce only storage/network work above the editor.
- If full-document serialization must remain debounced, emit an immediate lightweight `localDirty { revision }` event and refuse/queue external replacement until the matching snapshot has been delivered and resolved.
- Cancel or explicitly flush pending work before `replaceDocument`; include the expected local revision in replacement conflict checks.
- Model local, acknowledged, and external revisions as one state machine rather than independent Rust and JS booleans/timers.

Suggested tests:

- Fake-timer browser test: type, send external replacement at 0/50/179/180 ms, and assert no local text is lost or mislabeled.
- Dioxus integration test with controlled prop echoes and genuinely remote values, both with and without an explicit `external_revision`.
- Property/state-machine tests that generate local edits, acknowledgements, replacements, blur, and teardown in arbitrary order.

### 3. High — Blur and teardown do not provide an exactly-once final flush

Evidence:

- Blur emits the latest document with the old `revision`, does not clear the pending timer, and does not increment or otherwise identify the flushed state (`crates/dxeditor/web/src/index.ts:753-765`). The timer can therefore emit the same content again under a later revision.
- The Rust blur branch accepts an equal revision and emits it, but does not set `dirty`; encode errors on blur are ignored (`crates/dxeditor/src/component.rs:705-720`).
- `destroy()` clears `updateTimer` and destroys the editor without emitting a final snapshot (`crates/dxeditor/web/src/index.ts:1230-1237`). Dioxus `use_drop` sends only `Destroy` (`crates/dxeditor/src/bridge/mod.rs:18-21`). Removing a focused DOM node is not a reliable source of a preceding blur event.

Impact/reproduction:

- Type and immediately navigate away/unmount before 180 ms. If no usable blur reaches Rust, the only copy of the edit is canceled by `destroy()`.
- Type and blur: consumers can receive the same state twice with inconsistent revision meaning, increasing save churn and making acknowledgement races harder to reason about.

High-level fix:

- Add one `flushPending(reason)` function in the browser engine. It should cancel the timer, increment the revision once if the document changed, emit one snapshot, and remember the emitted document/revision.
- Call it from blur, explicit flush, external replacement conflict handling, and teardown. Do not depend on DOM blur for durability.
- Make destruction return/emit a final snapshot before the event channel is torn down, or remove the browser-side document debounce as recommended above.
- Propagate encode failures as explicit events and keep the session dirty/recoverable.

Suggested tests:

- Fake-timer tests for edit -> blur -> timer and edit -> destroy, asserting one final revision and no dropped text.
- Route-unmount and conditional-render Dioxus tests while the contenteditable retains focus.
- Encode-failure-on-blur test asserting a visible blocking error and retained recovery snapshot.

### 4. High — Stable node IDs are neither assigned persistently nor remapped on paste

Evidence:

- The global ProseMirror attribute defaults `semanticId` to `null` (`crates/dxeditor/web/src/index.ts:183-201`). Native editing operations such as Enter and Tiptap's `insertTable` therefore create nodes without IDs.
- `pmNodeToV2` invents an ID during each serialization but does not write it back into the ProseMirror document (`crates/dxeditor/web/src/index.ts:548-595`). Consecutive snapshots of the same newly created node can have different IDs.
- Internal clipboard paste inserts the serialized slice unchanged (`crates/dxeditor/web/src/index.ts:736-742`). Copied nodes retain their IDs, so paste can duplicate IDs in the same document. Block duplication has a dedicated ID-regeneration pass (`crates/dxeditor/web/src/index.ts:898-911`), showing that paste is missing equivalent handling.

Impact/reproduction:

- Press Enter, call `snapshot()` twice, and compare the new paragraph ID: it can change without a document transaction.
- Copy and paste a whole block using the internal MIME type: both instances retain the same stable ID. Typed-document validation then rejects the output; identity-based comments, components, collaboration, and incremental persistence cannot target nodes reliably.

High-level fix:

- Add a ProseMirror plugin/`appendTransaction` that assigns IDs to every newly created identity-bearing node in the transaction itself. Serialization must be pure and must fail/report if a required ID is absent.
- Remap all identity-bearing IDs on copy-paste and duplication, preserving references according to an explicit clipboard identity policy. Preserve IDs only for true moves.
- Validate uniqueness at the JS boundary before emitting and again in Rust before accepting an event.

Suggested tests:

- Two snapshots without intervening transactions are byte-for-byte ID stable.
- Enter, split, lift/sink list, table row/column insertion, TSV growth, drag/move, duplicate, and internal paste all produce present and unique IDs.
- Paste a subtree containing opaque/custom nodes and verify nested IDs and internal references are remapped consistently.

### 5. High — `HistoryPolicy::Reset` is discarded, so undo can cross an external replacement

Evidence:

- Rust constructs external replacements with `HistoryPolicy::Reset` (`crates/dxeditor/src/component.rs:581-585`).
- The bridge pattern explicitly ignores the field and calls `replaceDocument` with only the document (`crates/dxeditor/src/bridge/mod.rs:125-146`).
- The JS interface has no history-policy parameter and implements replacement as `setContent` on the existing editor state (`crates/dxeditor/web/src/index.ts:69-74`, `crates/dxeditor/web/src/index.ts:1220-1228`). No history reset is performed.

Impact/reproduction:

- Edit locally, accept an external/server document, then press Undo. Old local content can be resurrected across the external replacement, violating revision ownership and potentially overwriting the accepted server state.

High-level fix:

- Carry `historyPolicy` across the bridge.
- For Reset, install the replacement into a fresh editor state/history (while preserving required plugins/view state) or use a tested history-reset mechanism. For Preserve, define and implement position mapping explicitly; do not let default `setContent` behavior decide policy.
- Update undo/redo availability after replacement and expose it to the optional history buttons.

Suggested tests:

- Local edit -> Reset replacement -> Undo must not reveal pre-replacement content.
- Preserve replacement behavior should have an explicit test contract.
- Replacement while a command/menu selection is active should restore a valid selection and surface state.

### 6. High — The runtime catalog does not actually configure the browser schema, commands, or menus

Evidence:

- The bridge sends only a schema fingerprint, document, readonly flag, mention callback, and event callback (`crates/dxeditor/src/bridge/mod.rs:54-75`). It does not send component specs, format capabilities, action definitions, input rules, or render descriptors.
- The browser extension list, command switch, and slash insertion list are hard-coded (`crates/dxeditor/web/src/index.ts:714-722`, `crates/dxeditor/web/src/index.ts:843-885`, `crates/dxeditor/web/src/index.ts:1053-1076`).
- Any v2 kind not in that switch becomes an inert opaque node (`crates/dxeditor/web/src/index.ts:476-514`), regardless of a registered Rust `ComponentSpec`.

Impact/reproduction:

- Registering a typed component in `EditorCatalog` changes the fingerprint but does not make the component renderable, insertable, or editable in the browser. Even format capabilities cannot hide commands unsupported by the selected output format. The architecture therefore does not yet meet the stated future typed-component requirement.

High-level fix:

- Define a versioned browser engine manifest derived from the Rust catalog: node/mark name, content expression, attributes/defaults/validation, DOM adapter key, commands, menu metadata, clipboard policy, and per-format capability.
- Pair it with an explicit JS adapter registry for components that require custom NodeViews or commands. Fail mount with a precise missing-adapter diagnostic rather than silently treating a supposedly installed component as unknown.
- Build contextual menus from command/capability metadata; keep presentation ordering overridable but not duplicated in two languages.

Suggested tests:

- Register a fixture typed atom/block/mark and prove schema creation, rendering, insertion, copy/paste, snapshot, and typed round trip.
- Change a catalog attribute/default and assert the fingerprint and browser schema both change.
- Assert every visible command is supported by the current output format.

### 7. Medium — Read-only rendering downgrades valid v2 documents through the incomplete v1 model

Evidence:

- Readonly `Editor` first calls `migrate_v2_to_v1`; on any unsupported node it replaces the entire document with one plain-text document (`crates/dxeditor/src/component.rs:650-654`).
- As noted above, v1 migration cannot represent standard v2 tasks, table headers, hard breaks, images, opaque nodes, or future components (`crates/dxeditor/src/migrate.rs:368-483`).
- The v1 read renderer renders all table cells as `td` inside only `tbody` (`crates/dxeditor/src/component.rs:911-933`) and does not have first-class image/hard-break/opaque-inline rendering (`crates/dxeditor/src/component.rs:963-1007`).

Impact/reproduction:

- A valid typed document containing one image or task can cause every block to collapse to undifferentiated plain text in readonly mode. Table header semantics and accessible media are lost even where migration succeeds.

High-level fix:

- Render `ComponentDocumentV2` directly through a safe v2 renderer/catalog. Provide explicit renderers and accessible fallbacks for every standard kind and opaque/custom atoms.
- Degrade only the unsupported node, never the entire document.

Suggested tests:

- SSR/DOM tests for each standard v2 kind, mixed known/unknown documents, table `thead`/`th` semantics, image alt text, task state, and safe links.

### 8. Medium — The table validator does not validate a ProseMirror-compatible table map

Evidence:

- `validate_table_shapes` compares only each row's number of child cells (`crates/dxeditor/src/component_spec.rs:715-733`). It ignores `colspan` and `rowspan`, even though both are declared and accepted up to 1000 (`crates/dxeditor/src/component_spec.rs:957-976`). It also does not validate `colwidth` length/values or header placement.

Impact/reproduction:

- A row with one `colspan=2` cell and a row with two ordinary cells is structurally rectangular but is rejected. Conversely, two rows with equal child counts but incompatible spans are accepted and can fail or normalize unexpectedly when Tiptap constructs the ProseMirror table.

High-level fix:

- Validate an occupancy grid equivalent to ProseMirror's `TableMap`: positive spans, no overlaps/holes, equal effective width, rowspan bounds, `colwidth.len() == colspan`, and positive widths.
- Decide and validate header-row/header-column rules separately from geometry.

Suggested tests:

- Valid and invalid colspan/rowspan fixtures, overlap/hole cases, mixed headers, colwidth mismatch, and maximum-size boundaries.
- Cross-check accepted Rust tables by constructing them with the actual ProseMirror schema.

### 9. Medium — URL policy differs across Rust validation, browser editing, and readonly rendering

Evidence:

- Rust component validation allows `http`, `https`, `mailto`, and `semantic`, but not `tel`; it applies that generic policy to both links and image `src` (`crates/dxeditor/src/component_spec.rs:761-777`, `crates/dxeditor/src/component_spec.rs:998-1013`, `crates/dxeditor/src/component_spec.rs:1080-1089`).
- Browser links allow `http`, `https`, `mailto`, and `tel`, while browser images allow only `http`/`https` (`crates/dxeditor/web/src/index.ts:76-91`).
- The readonly renderer allows `tel` but not `semantic` (`crates/dxeditor/src/component.rs:1010-1026`).

Impact/reproduction:

- A `tel:` link created by the browser is rejected by typed Rust encoding; a Rust-valid `semantic:` link is dropped at the browser boundary and loses link semantics. Rust accepts nonsensical `mailto:`/`semantic:` image sources that the browser converts to opaque content.

High-level fix:

- Define separate canonical policies for hyperlink, image/media, and internal-entity URLs. Share a conformance corpus across Rust and TypeScript, and distinguish permitted persistence schemes from permitted navigation behavior.
- Keep mentions typed rather than relying on a generally valid `semantic:` hyperlink.

Suggested tests:

- The same corpus of absolute, relative, fragment, protocol-relative, mixed-case, whitespace/control-character, `tel`, `mailto`, `semantic`, `data`, `blob`, and `javascript` URLs against Rust decode/encode, JS adapters, clipboard, and readonly rendering.

### 10. Medium — Contextual menus are not keyboard-complete and the public aria label is ignored

Evidence:

- The Dioxus `aria_label` prop is placed on the non-focusable host (`crates/dxeditor/src/component.rs:656-660`), while the actual contenteditable always receives the hard-coded `Document editor` label (`crates/dxeditor/web/src/index.ts:724-731`).
- Slash and mention containers use `role=listbox`, and their buttons are changed to `role=option` (`crates/dxeditor/web/src/index.ts:634-638`, `crates/dxeditor/web/src/index.ts:697-701`, `crates/dxeditor/web/src/index.ts:1058-1070`, `crates/dxeditor/web/src/index.ts:1117-1130`), but there is no active option, `aria-selected`, `aria-activedescendant`, Arrow/Home/End/Enter handling, or focus-return state machine. The only editor key handling is Escape (`crates/dxeditor/web/src/index.ts:1211-1217`).
- Formatting buttons expose no pressed/active state (`crates/dxeditor/web/src/index.ts:605-615`, `crates/dxeditor/web/src/index.ts:969-980`).

Impact/reproduction:

- Type `/tab` or `@ali` without a pointer: the menu opens, but Arrow keys and Enter cannot select a result in the expected combobox/listbox interaction. Screen-reader users are not told which mark is active. Two editors cannot be given distinct accessible names through the Rust API.

High-level fix:

- Pass the public label into the browser mount and apply it to the contenteditable.
- Implement the APG combobox/listbox pattern (or use Tiptap's suggestion utilities with a tested menu controller): active option, arrows, Home/End, Enter, Escape, focus/selection preservation, announcements, and stable IDs.
- Add `aria-pressed` and visual active state to mark controls; disable unavailable table/history actions.

Suggested tests:

- Playwright tests that complete slash insertion and mention selection using only the keyboard.
- Accessibility-name test with two custom-labeled editors; active-format state tests; axe scans with every surface open.
- NVDA/JAWS/VoiceOver manual scripts and forced-colors/reduced-motion coverage.

### 11. Medium — Overlay positioning and transaction handling force synchronous layout work and go stale on viewport changes

Evidence:

- Every transaction calls `updateSurfaces`, and selection updates call it again (`crates/dxeditor/web/src/index.ts:761-762`).
- Positioning synchronously reads editor coordinates, wrapper bounds, surface dimensions, and then writes layout styles (`crates/dxeditor/web/src/index.ts:617-621`, `crates/dxeditor/web/src/index.ts:1148-1197`). There is no scroll, resize, visual-viewport, or ancestor-layout observer.

Impact/reproduction:

- Typing and selection changes can trigger repeated forced layout, particularly with table controls and mentions. Scroll a container or resize/zoom without another editor transaction: the floating UI can remain detached from its anchor or clipped.

High-level fix:

- Use one popup/surface state machine and a positioning library or equivalent middleware supporting flip, shift, clipping ancestors, and visual viewport.
- Batch geometry work in one `requestAnimationFrame`, update only the active surface, and subscribe while open to scroll/resize/`ResizeObserver` events. Avoid duplicate transaction/selection callbacks.

Suggested tests:

- Nested scrolling container, page scroll, resize, 200% zoom, mobile visual keyboard, RTL, long selection, table near edges, and transformed ancestor cases.
- Performance trace/budget for typing and selection in 10k/50k-word documents.

### 12. Medium — Browser events are trusted as v2 documents before Rust validation

Evidence:

- `SessionRevisionGuard` checks protocol/session/revision only (`crates/dxeditor/src/protocol.rs:175-260`).
- The `DocumentChange` handler stores and encodes the browser document without first calling `validate_component_document` (`crates/dxeditor/src/component.rs:683-703`).
- The legacy/Markdown encode fallback migrates without validation (`crates/dxeditor/src/component.rs:811-834`). Typed-format encoding happens to validate in its codec, but behavior depends on output format.
- Internal clipboard validation allowlists node names but preserves IDs and accepts arbitrary object-valued node attributes before `Slice.fromJSON` (`crates/dxeditor/web/src/index.ts:111-154`, `crates/dxeditor/web/src/index.ts:736-742`).

Impact/reproduction:

- Duplicate IDs from internal paste and malformed/oversized attributes can enter Rust state. Markdown may hide identity errors because IDs are discarded during serialization, while typed output rejects the same browser state. The protocol contract is therefore format-dependent.

High-level fix:

- Validate and normalize every browser snapshot at the Rust trust boundary before updating `current_document` or invoking any codec. Return structured validation errors to the engine/UI and keep the last valid document recoverable.
- Tighten clipboard attribute schemas per node/mark and impose recursive byte/depth/count budgets, then remap identities before insertion.

Suggested tests:

- Inject protocol events with duplicate/missing IDs, invalid attrs, unsafe URLs, bad tables, excessive nesting/text/attrs, unknown nodes, and wrong child shapes for every output format.
- Fuzz clipboard JSON and v2 event JSON; assert no panic, bounded work, and identical validation decisions across formats.

### 13. Low/Medium — The engine bundle and full-document pipeline are unnecessarily expensive per editor

Evidence:

- The minified checked-in engine is 468,379 bytes. It is embedded into `ENGINE_SCRIPT` and interpolated into every mount eval (`crates/dxeditor/src/bridge/mod.rs:8`, `crates/dxeditor/src/bridge/mod.rs:39-42`), so pages with multiple editors repeatedly transfer/parse/execute the same bundle text even though the global registry is singleton.
- Each emitted update builds the complete ProseMirror JSON, converts the complete tree to v2, serializes it through the Dioxus eval channel, and then serializes the complete Markdown document (`crates/dxeditor/web/src/index.ts:753-759`, `crates/dxeditor/src/component.rs:683-700`). CSS is also injected once per Rust editor instance (`crates/dxeditor/src/component.rs:30-410`, `crates/dxeditor/src/component.rs:593-595`).

Impact/reproduction:

- Multi-editor screens pay repeated bundle parsing and style duplication. Large documents allocate and traverse several full trees per burst of typing, on top of synchronous overlay geometry work.

High-level fix:

- Load/install the engine asset once per page/application and call the global registry with small commands; install editor styles once.
- Measure before adopting transaction deltas, but at minimum avoid duplicate conversion/serialization and keep persistence debounce outside the correctness protocol.
- Add explicit editor-count/document-size budgets and instrumentation for mount, transaction-to-event, encode, and heap use.

Suggested tests:

- Benchmark 1/10/50 editor mounts and 1k/10k/50k-word documents; track bundle eval count, long tasks, allocations, and p95 input latency.

## Risks and coverage gaps (not confirmed defects)

### Composition and mobile input

There are no tests for IME composition, Android/iOS selection, dictation, dead keys, RTL/bidi, or virtual-keyboard viewport changes. `updateSurfaces` and mention/slash detection run during every transaction without an explicit composition policy. Add Playwright coverage where browser support is reliable and a real-device/manual matrix for Safari/iOS, Chrome/Android, Japanese/Chinese/Korean IMEs, RTL, and dictation.

### Browser matrix and accessibility depth

`playwright.config.ts` declares Chromium, Firefox, and WebKit, but the verified run for this review was Chromium only. The axe test scans only the base editor state (`crates/dxeditor/web/e2e/editor.spec.ts:48-53`), not open menus/dialogs, tables, invalid input, keyboard navigation, or readonly output. Run all engines in CI and add manual assistive-technology acceptance criteria.

### Markdown fidelity corpus

The Markdown tests prove canonical idempotence for a small isolated v1 codec sample (`crates/dxeditor/src/markdown.rs:855-932`), not source fidelity through the real component boundary. There is no fixture corpus for comments, reference links, autolinks, nested/loose lists, delimiter edge cases, code fence info, entities, HTML boundaries, Unicode, large input, malformed input, or opaque-source preservation. Add golden fixtures plus semantic and source-preservation assertions, and fuzz decode/encode with bounded resources.

### Protocol and Dioxus lifecycle integration

Bridge tests deserialize a few events but do not mount a Dioxus component against the real browser engine (`crates/dxeditor/src/bridge/mod.rs:212-257`). Browser e2e mounts JS directly, bypassing Rust Markdown conversion and controlled-prop behavior (`crates/dxeditor/web/e2e/editor.html:19-50`). Add a WASM/browser integration harness that exercises the actual `MarkdownEditor` component, prop echoes, external revisions, focus/blur, encoding failures, and unmount.

### Build artifact enforcement

The rebuilt source currently matches `dist/editor.iife.js` byte-for-byte, but the repository has only documentation telling contributors to commit both (`crates/dxeditor/web/README.md:14-29`). Add a CI target that builds to a temporary path and fails on a bundle diff, then runs TypeScript, unit, and browser tests.

## Recommended fix order

1. Replace the Markdown-v1 detour with a direct v2 Markdown format and add the complete enabled-feature round-trip matrix.
2. Remove the browser debounce from the correctness boundary (or add immediate dirty ownership), implement exactly-once flush, and fix external replacement/history semantics.
3. Assign stable IDs transactionally, remap paste identities, and validate every browser snapshot in Rust.
4. Make readonly rendering native v2 and align URL/table validation.
5. Introduce the catalog-derived browser manifest/adapter registry and capability-driven menus.
6. Complete keyboard/accessibility/popup behavior, then optimize bundle loading, geometry, and large-document serialization against measured budgets.

## Validation performed

- `nix develop --command bash -lc 'cargo test --quiet --message-format=short -p dxeditor'`: passed (16 + 26 + 11 tests).
- `nix develop --command bash -lc 'cargo check --quiet --message-format=short -p dxeditor'`: passed.
- `npm run check && npm test`: passed (11 Vitest tests).
- `npm run test:e2e:chromium`: passed (8 Playwright tests).
- Temporary esbuild output compared byte-for-byte with `crates/dxeditor/web/dist/editor.iife.js`: identical.
