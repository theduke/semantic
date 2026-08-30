# Editor revamp remediation plan

Date: 2026-08-30

Implementation baseline: `cb61a58e221f1b4cf328fe63fe30fd5df331fd46`

Inputs: [correctness review](./review-correctness.md) and [upstream comparison](./review-upstream-comparison.md)

## 1. Outcome and release decision

The Tiptap/ProseMirror editing island should be retained. ProseMirror must continue to own the editable DOM, and the Rust side must continue to own the persisted document contract, format codecs, validation, and application callbacks. The current implementation is nevertheless a vertical spike, not a safe Markdown release.

Four defects block any user-data-bearing release:

1. Markdown is routed through the incomplete v1 model, so supported input and UI-created content can fail to load or save.
2. local changes exist only in a 180 ms browser timer and can be overwritten or lost on replacement, blur, or teardown;
3. semantic IDs are unstable or duplicated by ordinary editing and paste, while Rust requires them to be unique;
4. external replacement discards `HistoryPolicy`, allowing undo to cross revision boundaries.

In addition, browser snapshots are accepted before validation. That trust-boundary defect is part of the P0 identity/data-integrity work, not optional hardening.

The first releasable milestone is a deliberately bounded Markdown editor in which every visible command is losslessly representable, every local transaction is immediately owned by the host, every emitted document is valid, and undo cannot resurrect content from an older external revision. Notion-like polish, richer table controls, and general typed-component extensibility follow those gates.

## 2. Priority and gate definitions

| Priority | Meaning | Release treatment |
|---|---|---|
| P0 | Confirmed data loss, unsavable content, invalid identity, or revision isolation defect | Blocks all editor rollout |
| P1 | Correctness, security, accessibility, or core product-quality work required for a broad release | May follow a tightly controlled internal canary only if the affected capability is disabled |
| P2 | Architecture expansion, performance hardening, and interaction completeness | Does not block the bounded Markdown release unless measurements or product acceptance make it a blocker |

Every phase below must be independently mergeable. A phase may temporarily hide a command or disable a lossy behavior; it may not expose a state that the active output format cannot persist.

## 3. Cross-cutting contracts

These invariants apply to every work item and should be encoded as tests early.

### 3.1 Document and format invariants

- `ComponentDocumentV2` is the sole editable and browser wire model.
- Markdown decodes directly to v2 and encodes directly from v2. `EditorDocument` v1 remains a compatibility API only.
- A command shown in a Markdown session must produce a v2 document that the Markdown format can encode without silently dropping semantics.
- Canonical Markdown need not preserve insignificant whitespace, but `decode -> encode -> decode` must be semantically stable. Opaque constructs must retain their source slice byte-for-byte unless the user explicitly replaces them.
- Canonical standard attributes are shared across layers: code blocks use `info`; tables use `table_header`/`table_cell` and per-cell `alignment`; task items use `checked`; links use `href`/`title`; images use `src`/`alt`/`title`.
- Markdown table widths and spans are not persistable. `colwidth`, `colspan > 1`, and `rowspan > 1` are typed-format capabilities, not Markdown document state.

### 3.2 Identity invariants

- Every identity-bearing node has one nonempty `semanticId` in ProseMirror state, not only in a serialized snapshot.
- IDs are unique within a document and stable across snapshots when no document transaction occurs.
- True moves preserve identity. Copy, duplicate, paste, and copy-drag create new identities for the inserted copy, including nested rows, cells, blocks, atoms, and custom components.
- ID-maintenance transactions are excluded from undo history and do not become separate public change revisions.
- Serialization is pure: it never invents or mutates IDs.

### 3.3 Revision and durability invariants

- A document-changing ProseMirror transaction synchronously advances local ownership before an external replacement can be accepted.
- Editor protocol events are not debounced. Persistence/network consumers may debounce after `on_change`.
- Each distinct document state is emitted at most once per local revision. Blur and explicit flush do not duplicate an already emitted state.
- Teardown explicitly flushes before destroying the editor. DOM blur is not a durability mechanism.
- External acknowledgement does not clear a newer local revision. External replacement while local work is unresolved is rejected or queued with a visible conflict; it never silently overwrites local input.
- `HistoryPolicy::Reset` makes pre-replacement content unreachable by undo. `Preserve` has explicit, tested mapping semantics.

### 3.4 Trust, error, and recovery invariants

- Every document entering Rust from the browser is validated against the active catalog and resource limits before it updates `current_document` or reaches any format encoder.
- Browser-side validation catches missing/duplicate IDs and schema violations early, but Rust is the authoritative trust boundary.
- Invalid snapshots, encoding failures, and unsupported commands yield structured errors. The last valid v2 snapshot and last successfully encoded payload remain recoverable.
- “Saved” is never shown for a state or attribute that the selected format discarded.

### 3.5 Contextual UI invariants

- At most one primary contextual surface wins: modal/popdown, node/table, link, text selection, suggestion, then passive block gutter.
- A `CellSelection` never opens the text-formatting bubble. A node selection opens its node controls. A nested caret does not accidentally target the top-level table or list.
- Popup state is derived from mapped ProseMirror plugin state; geometry is scheduled separately and updated for scroll, resize, zoom, and visual viewport changes.
- Composition does not trigger commands, suggestions, or disruptive surface repositioning.
- All commands expose `canExecute`, active/mixed state, format availability, and a stable disabled reason.

## 4. Staged implementation

## Phase 0 — Freeze unsafe capabilities and establish integration gates

Priority: P0

Release status: blocker groundwork

Dependencies: none

### P0.0 Build the real-boundary test harness first

Evidence addressed: both reviews found that Rust codec tests and direct-JS Playwright tests pass while bypassing the actual `MarkdownEditor -> bridge -> engine -> on_change` path ([correctness coverage gaps](./review-correctness.md#protocol-and-dioxus-lifecycle-integration), [upstream P2-5](./review-upstream-comparison.md#p2-5--test-breadth-does-not-match-the-implementations-claims)).

Targets:

- `crates/dxeditor/src/component.rs`: make format decode/encode entry points testable without rendering where useful.
- `crates/dxeditor/src/bridge/mod.rs` and `crates/dxeditor/src/protocol.rs`: protocol state-machine tests.
- `crates/dxeditor/web/src/index.test.ts`: fake-timer, schema, ID, history, and surface unit tests.
- `crates/dxeditor/web/e2e/editor.html` and `editor.spec.ts`: retain direct-engine tests.
- Add a WASM/browser integration fixture under `crates/dxeditor/web/e2e/` or a dedicated `crates/dxeditor/tests/web/` harness that mounts the actual Dioxus `MarkdownEditor`.

Implementation:

1. Add fixture helpers for Markdown payloads, v2 documents, engine snapshots, and semantic normalization that intentionally ignores generated IDs only when testing format semantics.
2. Add a controlled-parent fixture that can echo `on_change`, send a genuinely newer external revision, blur, conditionally unmount, and expose received callbacks/revisions.
3. Add a table-driven inventory of every standard browser node, mark, and visible command. The inventory becomes the source of assertions that the active format either supports the construct or hides/disables it.
4. Add a checked-in bundle verification command that builds to a temporary output and fails when it differs from `web/dist/editor.iife.js`.
5. Preserve the current Chromium suite, but mark Firefox/WebKit and real-device checks separately so unavailable local host libraries do not hide CI expectations.

Temporary safety gate:

- Until Phase 1 is complete, hide or disable Markdown actions for strike, image, hard break insertion, task list, table header creation, and any other construct shown by the inventory to fail the current component-boundary round trip.
- Make the disabled reason explicit in command metadata; do not silently return `false` from the command switch.

Acceptance:

- A failing regression exists for each P0 defect before its fix.
- The test harness observes public Markdown `on_change`, not only an engine event.
- No visible Markdown command lacks a capability assertion.
- The bundle reproducibility check passes in CI.

## Phase 1 — Native v2 Markdown and executable format capabilities

Priority: P0

Release status: blocker

Dependencies: Phase 0 harness

### P0.1 Implement `MarkdownDocumentFormat`

Evidence addressed: [correctness defect 1](./review-correctness.md#1-critical--the-primary-markdown-editor-cannot-load-or-save-several-advertised-node-types) and [upstream P0-1](./review-upstream-comparison.md#p0-1--the-primary-markdown-round-trip-is-structurally-incomplete).

Targets and interfaces:

- Add `crates/dxeditor/src/markdown_v2.rs` with direct v2 parse/serialize functions.
- Add `MarkdownDocumentFormat` implementing `DocumentFormat` in `crates/dxeditor/src/format.rs` or re-export it there.
- Register it in `register_standard_document_formats` under format ID `markdown` and the appropriate Markdown media types.
- Route `decode_editor_payload` and `encode_editor_payload` in `component.rs` through `DocumentFormatRegistry`; the legacy codec fallback must no longer handle `MarkdownEditor`.
- Keep `crates/dxeditor/src/markdown.rs::MarkdownCodec` and `migrate.rs` available for v1 callers. Do not expand v1 into the primary editing model.
- Update `lib.rs` exports only where a public format type or fixture API is intentionally supported.

Required v2 profile:

- block: paragraph, heading 1–6, blockquote, fenced/indented code block, bullet list, ordered list with start, task list/item, thematic break, GFM table, opaque Markdown block;
- inline: text, hard break, image, mention, opaque Markdown inline;
- marks: bold, italic, strike, inline code, link;
- metadata and opaque source state according to the existing `DecodedDocument`, `FormatSourceState`, `FormatDiagnostic`, and `Fidelity` types.

Implementation details:

1. Use `pulldown_cmark::Parser::into_offset_iter` so opaque nodes and diagnostics can refer to exact UTF-8 source ranges. Do not reconstruct opaque raw HTML from rendered text.
2. Build `ComponentNode` values directly using the constants and typed attribute conventions in `document_v2.rs`/`component_spec.rs`.
3. Generate initial IDs once during decode with a deterministic document-local allocator or opaque UUIDs; identity stability after mount is the browser identity plugin's responsibility.
4. Emit `code_block.attrs.info` from fenced code and consume the same attribute on encode. Preserve the full info string; syntax highlighting may interpret only its first token.
5. Represent GFM header cells as `table_header`; body cells as `table_cell`; place alignment on each affected cell. Encode one GFM delimiter row and reject or diagnose spans/widths rather than dropping them.
6. Represent task markers structurally with `task_list`, `task_item`, and `checked`, not list attributes inherited from v1.
7. Keep raw HTML inert. Decode it as opaque Markdown with `{source, fallback, construct}` and encode the untouched `source`. Never render it through `dangerous_inner_html`.
8. Define the existing mention syntax as a documented Markdown extension. If the current syntax is a `semantic:` link, parse it into `mention` only when it matches the exact extension grammar; ordinary links remain links.
9. Validate decoded v2 before returning it and validate before serialization. Encoding unsupported known components returns `FormatError` with node path and capability diagnostics.
10. Preserve existing canonical output choices where compatible. A new canonicalization change needs a fixture and migration note.

Backward compatibility:

- Existing `EditorPayload { format: "markdown" }` remains unchanged.
- Legacy `EditorDocument`/`MarkdownCodec` APIs remain available and behaviorally stable; they are explicitly not used by `MarkdownEditor`.
- Source formatting may canonicalize on the first edit. Opaque source slices must not canonicalize.
- No stored data migration is required because Markdown remains the persisted payload.

Tests and acceptance:

- Golden fixtures for nested/loose lists, tasks, strike, images, hard and soft breaks, fenced code with multiword info, raw/inline HTML, tables with all alignments and headers, mentions, Unicode/entities, reference links, autolinks, comments, malformed input, and empty input.
- For every supported construct: `Markdown -> v2 -> Markdown -> v2` is semantically equal and the second encoding is byte-identical.
- Every command enabled in a Markdown session produces a successful public Markdown `on_change`.
- Opaque source survives edit of a neighboring block byte-for-byte.
- Invalid or oversized input returns a bounded structured error without panic.
- The temporary Phase 0 command gates are removed only as their individual round-trip test passes.

### P0.2 Make format capabilities executable

Evidence addressed: hard-coded browser actions can create unsavable content, Markdown table resizing falsely appears persistent, and the Rust catalog does not configure menus ([correctness defect 6](./review-correctness.md#6-high--the-runtime-catalog-does-not-actually-configure-the-browser-schema-commands-or-menus), [upstream P1-4](./review-upstream-comparison.md#p1-4--format-specific-presentation-controls-can-claim-persistence-that-markdown-cannot-provide)).

Targets and interfaces:

- Extend `component_spec.rs::FormatCapability` usage so every standard node/mark has an explicit Markdown capability rather than relying on defaults.
- Introduce a serializable `EditorEngineManifest`/`FormatCapabilityManifest` in a focused module such as `src/engine_manifest.rs`.
- Add the active manifest and `aria_label` to `protocol.rs::EngineCommand::Mount` and `web/src/index.ts::MountOptions`.
- Replace the independent slash/action arrays and unconditional command switch exposure in `web/src/index.ts` with manifest-driven visibility and disabled reasons. Execution adapters may remain explicit TypeScript functions.

Manifest minimum fields:

- protocol/manifest version and document catalog fingerprint;
- active format ID;
- component kind, identity policy, attributes/defaults, adapter key, clipboard policy;
- command ID, surface, label/order, required component/mark, active-format capability, and optional disabled reason;
- feature flags for persistent table alignment, width, spans, headers, media, tasks, and opaque content;
- accessible editor label.

Behavior:

- Catalog construction fails if a visible action references a missing component, command adapter, or unsupported active-format result.
- Markdown uses `resizable: false`, or stores resize widths in clearly ephemeral view state that is never reported as saved. The simpler first-release choice is `resizable: false`.
- Merge/split and spans are hidden in Markdown. Header rows, row/column changes, and alignment remain available after direct Markdown round-trip coverage passes.
- A missing JS behavior adapter causes a precise mount error, not an inert “known” component.

Tests and acceptance:

- A manifest golden test proves Rust and TypeScript agree on standard component names, attributes, and command IDs.
- Each visible command has a supported capability and registered JS adapter.
- Markdown resize/span actions are absent; typed-format fixtures may expose them.
- A custom fixture component with a missing adapter fails catalog/mount validation with an actionable message.

## Phase 2 — Transaction identity and the Rust trust boundary

Priority: P0

Release status: blocker

Dependencies: Phase 0; can run in parallel with Phase 1 until final integration

### P0.3 Add a project-owned semantic identity extension

Evidence addressed: [correctness defect 4](./review-correctness.md#4-high--stable-node-ids-are-neither-assigned-persistently-nor-remapped-on-paste) and [upstream P0-2](./review-upstream-comparison.md#p0-2--semantic-ids-become-duplicate-or-unstable-during-ordinary-edits).

Targets and interfaces:

- Extract `SemanticId` from `web/src/index.ts` into `web/src/extensions/semantic-id.ts`.
- Add a stable `generateId(): string` using `crypto.randomUUID()` with a collision-checked fallback for test/older environments.
- Add an append-transaction plugin modeled on Tiptap `UniqueID`, but governed by `IdentityPolicy` from the engine manifest.
- Make `pmNodeToV2` reject missing IDs on required nodes. Remove its `?? id(...)` fallback.
- Centralize subtree identity handling in `web/src/identity.ts`: strip/regenerate copy identities, preserve move identities, and remap future internal references.

Implementation:

1. On initial document creation, scan required identity-bearing nodes and fail mount if imported v2 contains duplicates. Assign only genuinely missing IDs in a non-history maintenance transaction if the import policy allows normalization.
2. In `appendTransaction`, inspect changed ranges, assign IDs to new nodes, and regenerate duplicate occurrences introduced by split/copy behavior. Do not rescan the entire document for every keystroke.
3. Mark maintenance transactions `addToHistory: false`, suppress them as separate public changes, and preserve the user's selection/stored marks.
4. Strip identity attributes from copied internal slices before insertion so the plugin assigns fresh IDs. For a native move in the same document, preserve the original slice IDs. Explicit duplicate and copy-drag use the copy policy.
5. Give nested table nodes and custom atoms the same policy. Text and marks remain identity-free according to the catalog.
6. Separate `document_catalog_fingerprint` from `pm_clipboard_schema_fingerprint`; the latter includes ordered PM node/mark specs and clipboard codec version.

Backward compatibility:

- Existing valid IDs are preserved.
- Invalid imported typed documents with duplicate IDs are rejected rather than silently rewritten, unless an explicit migration/normalization entry point is invoked outside the editor.
- Internal clipboard version increments when identity stripping/remapping semantics change. Older envelopes safely fall back to HTML/plain text.

Tests and acceptance:

- Enter at beginning/middle/end, block split/merge, list split/lift/sink, table row/column creation, TSV growth, duplicate, internal paste, HTML paste, and copy-drag always leave all required IDs present and unique.
- Two snapshots without an intervening document change are byte-for-byte ID stable.
- One undo reverses the user command, not an identity-maintenance transaction.
- Property tests run arbitrary supported command sequences and assert identity invariants after every transaction.
- Copy creates new nested IDs; move preserves them.

### P0.4 Validate every snapshot before state mutation or encoding

Evidence addressed: [correctness defect 12](./review-correctness.md#12-medium--browser-events-are-trusted-as-v2-documents-before-rust-validation), plus duplicate IDs and permissive clipboard attributes.

Targets and interfaces:

- `component.rs` document/blur event handling.
- `protocol.rs::EngineEvent`: replace unstructured errors with a serializable structured engine error carrying code, message, optional path, local revision, and recoverability.
- `component_spec.rs::validate_component_document` and `ValidationLimits`.
- `web/src/index.ts::decodeInternalClipboard` and per-node clipboard attribute validators.

Implementation:

1. Add one Rust helper that validates an incoming v2 snapshot with the active catalog, limits, and unknown-component policy before updating counters, `current_document`, dirty state, or calling `encode_editor_payload`.
2. Keep `last_valid_document`, `last_encoded_payload`, and the rejected revision separately. On failure, leave the session dirty and show a blocking recoverable error with copy/export of the typed snapshot where safe.
3. Never let output format determine whether a browser snapshot is considered valid.
4. In TypeScript, validate required/unique IDs and schema shape before emit. Emit a structured engine error on failure and retain the editor DOM for recovery.
5. Tighten internal clipboard validation to node/mark-specific attributes. Apply byte, depth, node-count, text-length, and array-size budgets before `Slice.fromJSON`, then apply the copy identity policy.

Tests and acceptance:

- Inject missing/duplicate IDs, bad child shapes, invalid attributes, unsafe URLs, invalid tables, excessive nesting, oversized arrays/text, and unknown kinds for Markdown and typed outputs.
- Invalid events do not change `current_document` or call `on_change`; the prior valid payload is retained.
- Clipboard fuzzing has bounded runtime/memory and never panics.
- Rust and JS fixtures agree on valid/invalid standard documents.

## Phase 3 — Revision ownership, exact flushing, and history isolation

Priority: P0

Release status: blocker

Dependencies: Phase 0; integrate after snapshot validation

### P0.5 Remove debounce from the correctness boundary

Evidence addressed: [correctness defects 2 and 3](./review-correctness.md#2-critical--a-local-edit-can-be-silently-overwritten-during-the-180-ms-browser-debounce), [upstream P0-3](./review-upstream-comparison.md#p0-3--destroy-drops-pending-input-blur-does-not-actually-flush-the-debounce).

Targets and interfaces:

- `web/src/index.ts` update scheduling, `EditorSession`, `replaceDocument`, and `destroy`.
- `protocol.rs::{EngineCommand, EngineEvent, SessionRevisionGuard}`.
- `bridge/mod.rs::EditorBridge` teardown and command transport.
- `component.rs::Editor` controlled-value effect and event handler.

Preferred protocol:

- Emit one `documentChange` synchronously for every document-changing user transaction after identity maintenance has settled. Increment the local revision once and attach the validated v2 snapshot.
- Coalesce non-document transactions and identity-only append transactions; they do not advance a public revision.
- Debounce storage/network work in the consumer of `on_change`, never between ProseMirror and Rust.

Implementation:

1. Replace `updateTimer`, `suppressUpdate`, loose `revision`, and blur emission with a `SnapshotScheduler` owning `revision`, `dirtySinceEmit`, `lastEmittedDocHash`, and `flush(reason)`.
2. Call the scheduler from the final transaction event after append transactions. `flush` emits only if the document differs from the last emitted state.
3. Add explicit `Flush { reason }` and a teardown path that synchronously returns or emits the final snapshot before registry/session cleanup. `destroy()` must not cancel unowned work.
4. Blur calls `flush("blur")` and emits a focus event/state transition, not a duplicate document payload. If retaining a `Blur` protocol event, make its document optional and revision equal to the last emitted revision.
5. Before external replacement, flush and compare the expected local revision. If local work is unacknowledged, return a conflict event instead of replacing.
6. Consolidate Rust's `dirty`, `last_emitted`, `last_local_revision`, and external revision logic behind a small session state machine. Use `SessionRevisionGuard` rather than duplicating partial checks in closures.
7. Acknowledgement must name the exact local revision; only that revision or earlier becomes clean. A newer local revision remains dirty.
8. Encoding failure keeps the validated v2 revision dirty and recoverable; it cannot be acknowledged as saved.

Teardown limitation:

- Component unmount inside the running app must flush exactly once.
- `pagehide`/`visibilitychange` may request a best-effort flush, but the plan must not promise completion of arbitrary async network saves during browser termination. Durable drafts or `sendBeacon` are a separate application-level feature.

Tests and acceptance:

- Type, then external replace at 0/50/179/180 ms: local text is neither lost nor mislabeled.
- Type and unmount at 0/1/179/181 ms; the host receives the final state once.
- Type then blur before/after former debounce boundaries; one monotonic revision and no duplicate callback.
- Controlled prop echo is acknowledged; genuinely newer remote content conflicts while dirty and replaces when clean.
- Random state-machine sequences of edit, acknowledge, replace, blur, flush, focus, and destroy preserve revision monotonicity and last-local-content durability.

### P0.6 Implement both history policies end to end

Evidence addressed: [correctness defect 5](./review-correctness.md#5-high--historypolicyreset-is-discarded-so-undo-can-cross-an-external-replacement) and [upstream P0-4](./review-upstream-comparison.md#p0-4--historypolicy-is-ignored-during-external-replacement).

Targets and interfaces:

- Preserve `history_policy` in `bridge/mod.rs::run_engine_command`.
- Change `web/src/index.ts::EditorSession.replaceDocument(document, historyPolicy, expectedLocalRevision)`.
- Add tested replacement helpers in `web/src/history.ts` or the editor session module.
- Return command-state updates for optional undo/redo buttons.

Behavior:

- `Reset`: after validating/flushing ownership, install the replacement into a fresh `EditorState` or remount the editor with new history plugin state. Preserve only documented view state: editable flag, focus when requested, and a valid mapped/default selection. No pre-replacement step is undoable.
- `Preserve`: replace through an explicitly non-history transaction, close the previous history group, and map selection through the replacement. Define this as preserving earlier local undo items only when the caller intentionally requests it and the external revision is compatible. If a safe mapping cannot be proven, return a structured rejection rather than silently falling back.
- Both policies cancel/dismiss stale menus, async suggestions, and node targets and recompute command availability.

Backward compatibility:

- `HistoryPolicy::Reset` remains the default and is used by controlled external updates.
- Do not remove `Preserve`; it is public protocol surface. Keep it narrowly specified rather than inferring semantics from Tiptap `setContent`.

Tests and acceptance:

- Local edit -> acknowledge -> reset replacement -> undo/redo never reveals old content.
- Preserve replacement has explicit assertions for mapped selection and retained history.
- Replacement during open history groups, focused popups, pending suggestions, and stale command requests is deterministic.
- Optional history buttons receive correct enabled/disabled state after replacement.

## Phase 4 — Safe native v2 read rendering and shared validators

Priority: P1

Release status: required before broad release; readonly mode must be disabled until complete

Dependencies: Phases 1 and 2

### P1.1 Render v2 directly and degrade per node

Evidence addressed: [correctness defect 7](./review-correctness.md#7-medium--read-only-rendering-downgrades-valid-v2-documents-through-the-incomplete-v1-model).

Targets:

- Replace `component.rs::{ReadOnlyDocument, ReadOnlyBlock, ReadOnlyContent, ReadOnlyInline}` v1 signatures with v2 render functions, ideally in a focused `src/render_v2.rs`.
- Evolve `render.rs::EditorRenderRegistry` to accept v2 node/mark context rather than only v1 `BlockNode`/`InlineNode`.
- Use `ComponentSpec::DomDescriptor` only for safe allowlisted structure; custom behavior remains an explicit renderer registration.

Behavior:

- Render every standard v2 kind with semantic HTML: headings, lists/tasks, `pre/code`, `table` with `thead`/`tbody` and `th`, images with alt text, hard breaks, links, mentions, and inert opaque nodes.
- Unknown/custom nodes degrade individually to their safe fallback. One unsupported node must not collapse the entire document.
- No raw HTML source reaches `dangerous_inner_html`.

Migration:

- Keep `DocumentView(EditorDocument)` for public v1 callers, but do not use it for v2 `Editor` readonly mode.
- Add a v2 view entry point if needed; avoid changing existing public type behavior without a compatibility shim.

Acceptance:

- SSR/DOM tests cover every standard v2 node/mark and mixed known/unknown documents.
- Accessible table headers, task state, image alt, and safe link attributes are asserted.
- Malicious opaque source renders as text.

### P1.2 Unify URL policies by semantic role

Evidence addressed: [correctness defect 9](./review-correctness.md#9-medium--url-policy-differs-across-rust-validation-browser-editing-and-readonly-rendering).

Targets:

- Replace `component_spec.rs::is_safe_url`, `component.rs::safe_link_url`, and TypeScript `safeUrl`/`safeImageUrl` with one documented policy and a shared conformance fixture.
- Introduce distinct Rust validators/policy IDs for hyperlink, media source, and internal entity; expose the policy ID in attribute/manifest metadata.

Policy:

- Hyperlinks: relative, fragment, `http`, `https`, `mailto`, and `tel`; internal navigation only through a separately defined internal policy.
- Media: first release allows only absolute `http`/`https`. `data`, `blob`, filesystem, `mailto`, `tel`, and `semantic` are rejected unless a later upload/blob lifecycle explicitly supports them.
- Mentions remain typed entities; `semantic:` is not a generally navigable hyperlink unless the application registers and renders it deliberately.
- Reject control characters, ambiguous whitespace, scheme obfuscation, and dangerous schemes. Persistence permission and click/navigation permission are distinct decisions.

Acceptance:

- The same JSON corpus runs in Rust and TypeScript and produces identical decisions for absolute/relative/fragment/protocol-relative/mixed-case/control-character and all named schemes.
- Paste, link editing, image insertion, v2 validation, Markdown encoding, and readonly rendering use the appropriate role policy.

### P1.3 Validate table geometry with a TableMap-equivalent model

Evidence addressed: [correctness defect 8](./review-correctness.md#8-medium--the-table-validator-does-not-validate-a-prosemirror-compatible-table-map).

Targets:

- Replace `component_spec.rs::Validator::validate_table_shapes` with occupancy-grid validation in a focused helper.
- Add maximum effective rows, columns, cells, and span work to `ValidationLimits`.

Invariants:

- positive `rowspan`/`colspan`, no overlaps or holes, consistent effective width, spans within table bounds;
- `colwidth` is absent or has exactly `colspan` positive bounded widths;
- header placement is validated separately from geometry;
- Markdown capability validation rejects spans/widths before encoding, while typed formats may preserve them.

Acceptance:

- Valid/invalid span, overlap, hole, width, mixed-header, and maximum-boundary fixtures.
- Every Rust-accepted table can be constructed by the actual ProseMirror schema without repair/error.

## Phase 5 — Contextual UI state, suggestions, and accessibility

Priority: P1

Release status: product/accessibility gate for broad release

Dependencies: Phases 1–3; identity and revision behavior must be stable first

### P1.4 Replace transaction polling with one contextual UI plugin

Evidence addressed: [correctness defect 11](./review-correctness.md#11-medium--overlay-positioning-and-transaction-handling-force-synchronous-layout-work-and-go-stale-on-viewport-changes) and [upstream P1-2](./review-upstream-comparison.md#p1-2--contextual-surfaces-use-the-wrong-selectionlifecycle-abstraction).

Targets:

- Extract UI code from `web/src/index.ts` into `web/src/ui/contextual-state.ts`, `surface-controller.ts`, and component-specific surface modules.
- Add one keyed ProseMirror plugin whose state is a compact `UiState`.
- Use Tiptap Bubble/Floating Menu extensions plus Floating UI, or equivalent directly reviewed primitives, for anchor lifecycle and positioning.

`UiState` minimum shape:

- selection kind: text, cursor, node, cell, all, none;
- active semantic target and mapped range/position;
- winning surface and subordinate popdown;
- command enabled/active/mixed states;
- composition/focus state and active format capabilities;
- passive hovered semantic block target distinct from focused caret target.

Implementation:

1. Derive state once per relevant document/selection/plugin-state change. Remove duplicate `onTransaction` plus `onSelectionUpdate` layout work.
2. Arbitration order is modal/popdown -> node/table -> link -> selected text -> suggestion -> passive gutter.
3. Skip surface changes while `view.composing`; coalesce geometry writes in `requestAnimationFrame`.
4. Subscribe only while a surface is open to clipping-ancestor scroll, window/visual viewport resize, and relevant `ResizeObserver` changes. Tear down every listener/observer on close/destroy.
5. Resolve a semantic block explicitly. A caret inside a cell/list item may target that nested block; it must not implicitly duplicate/delete the depth-1 table/list.
6. Preserve editor selection while pointer-interacting with a popup, restore focus predictably, and dismiss on Escape/outside click without reopening from the same state.

Acceptance:

- `CellSelection` never opens text formatting; node, link, text, table, and gutter states are mutually deterministic.
- Nested lists/tables target the displayed block.
- Scroll, transformed/nested containers, resize, 200% zoom, visual keyboard viewport, RTL, and edge flipping remain anchored.
- No surface updates during composition; no listener/observer leaks over 100 mount/open/close/destroy cycles.
- Performance trace shows no duplicate layout pass per selection transaction.

### P1.5 Move slash and mentions to mapped suggestion plugins

Evidence addressed: [correctness defect 10](./review-correctness.md#10-medium--contextual-menus-are-not-keyboard-complete-and-the-public-aria-label-is-ignored) and [upstream P1-3](./review-upstream-comparison.md#p1-3--slash-and-mention-menus-are-mouse-only-regex-polling-not-mapped-editor-state).

Targets:

- Add separate keyed Tiptap Suggestion/ProseMirror plugins for `/` and `@`.
- Extract an accessible listbox controller shared by slash and mention UI.
- Pass `aria_label` from `component.rs` through `MountOptions` to the actual contenteditable.

Behavior:

- Plugin state owns active range, query, request ID, selected option, dismissed range, and composition state; ranges map through transactions.
- Eligibility excludes code blocks, opaque nodes, invalid table contexts, readonly mode, and any other format/component-declared exclusion.
- Arrow Up/Down, Home/End, Page Up/Down where useful, Enter, Tab policy, and Escape work without moving DOM focus from the editor under the `aria-activedescendant` model.
- Escape records dismissal for the mapped trigger so the menu stays closed until the query/range changes.
- Mention requests retain current debounce, abort, and stale-response protection.
- Insertion atomically deletes the plugin-owned trigger range and inserts the selected node/block.

Acceptance:

- Full keyboard matrix, pointer parity, persistent Escape dismissal, cursor relocation, mutation while results are pending, zero/error/many results, and stale suppression.
- Enter during IME composition never accepts a suggestion.
- Two editors receive distinct public accessible names.
- Contenteditable exposes expanded/controls/active-descendant state while a suggestion is open.

### P1.6 Complete toolbar/menu semantics

Evidence addressed: [upstream P2-2](./review-upstream-comparison.md#p2-2--toolbarmenu-semantics-and-command-state-are-incomplete).

Targets:

- Shared surface/button helpers in the TypeScript UI modules.
- Optional Rust history actions in `component.rs` consume browser command-state updates.

Behavior:

- Formatting controls expose `aria-pressed` including mixed state where applicable.
- Toolbars use roving tabindex plus Arrow/Home/End navigation and provide a documented shortcut to move from editor to the active toolbar.
- Menus use `menuitem`; listboxes use option/selected/active-descendant semantics; unavailable actions are actually disabled with an accessible explanation.
- Focus returns to the editor selection after command activation unless a dialog/popdown intentionally retains focus.

Acceptance:

- Keyboard-only Playwright scenarios for every open surface and table action.
- Axe scans with each surface open, plus accessibility-tree snapshots.
- Manual NVDA, JAWS, VoiceOver, forced-colors, reduced-motion, and 200% zoom checklist is documented and executed before broad release.

## Phase 6 — Table correctness and contextual table UX

Priority: P1 correctness, then P2 interaction enhancement

Release status: correct basic tables are a broad-release gate; advanced handles may follow

Dependencies: Phases 1, 2, 4, and contextual state from Phase 5

### P1.7 Replace whole-table TSV rebuilding

Evidence addressed: [upstream P1-1](./review-upstream-comparison.md#p1-1--table-tsv-paste-bypasses-table-maps-cell-selection-and-span-invariants).

Targets:

- Replace `web/src/index.ts::pasteTableGrid` with a module using `prosemirror-tables` `TableMap`, `CellSelection`, and compatible cell-slice insertion primitives.
- Keep `parseTsvGrid` as a bounded parser with its existing quoted-field support, moved to a focused module if helpful.

Implementation:

1. Resolve the target rectangle from a cell selection or current cell.
2. Convert TSV to a rectangular cell slice, respecting configured limits before allocation/growth.
3. Apply upstream-compatible clip/repeat/grow behavior, preserve header type and allowed cell attributes, and let `fixTables` normalize.
4. Set an explicit post-paste `CellSelection` and make the operation one undo event.
5. Let the identity extension assign IDs to newly created cells/paragraphs; do not hand-roll a competing ID allocator.
6. In Markdown mode, normalize/reject spans rather than pretending to preserve them.

Acceptance:

- Single-cell and rectangular selections; source smaller/larger than selection; right/bottom growth; headers; typed spans; HTML versus TSV; quoted tabs/newlines; 100x100 and total-cell limits.
- One undo reverses the paste, selection remains valid, and all IDs are unique.
- Large paste changes only the necessary table range rather than replacing the whole table node.

### P2.1 Add Notion-like table handles after correctness

Evidence addressed: [upstream P2-3](./review-upstream-comparison.md#p2-3--table-ui-is-a-generic-toolbar-rather-than-contextual-rowcolumncell-handles).

Targets and behavior:

- Build row, column, cell, and table handles from table plugin state with explicit target indices.
- Offer select row/column/table; add before/after; delete; header toggle; alignment; and mobile overflow.
- Keep merge/split and persistent widths hidden in Markdown. Enable only when a typed format declares them lossless.
- Retain keyboard commands and discoverable menu alternatives for every pointer handle.

Acceptance:

- Precise targets after row/column changes, scroll, and selection mapping.
- Touch targets and narrow-screen bottom/overflow surface meet the interaction and accessibility gates.

## Phase 7 — Bridge loading, performance, and extensibility hardening

Priority: P1 for repeated bundle eval/CSP if multiple editors ship; otherwise P2

Release status: measurement-driven hardening

Dependencies: stable protocol from Phase 3

### P1.8 Load the engine once and narrow the bridge

Evidence addressed: [correctness defect 13](./review-correctness.md#13-lowmedium--the-engine-bundle-and-full-document-pipeline-are-unnecessarily-expensive-per-editor) and [upstream P1-5](./review-upstream-comparison.md#p1-5--the-468-kb-engine-bundle-is-parsedevaluated-for-every-mounted-editor).

Targets:

- `bridge/mod.rs`: separate one-time engine installation from per-editor mount/command transport.
- `web/dist/editor.iife.js` build and application asset loading.
- Install `DXEDITOR_STYLE` once rather than emitting one style node per editor.

Implementation:

1. Add an application/page-level once-only loader promise or a Rust-side shared loader state that evaluates/loads the hashed engine exactly once and lets concurrent mounts await readiness.
2. Prefer a static hashed module/script asset compatible with strict CSP. If Dioxus currently requires `document::eval` for message transport, confine eval to small, fixed bridge calls with all data passed as serialized structured values; do not interpolate the 468 KB bundle per mount.
3. Preserve `serde_json` serialization for all data crossing any source-string boundary.
4. Add performance marks for engine load, editor construction, first editable paint, transaction-to-Rust event, Rust validation/encode, and second-editor warm mount.
5. Measure full-document serialization before introducing deltas. Do not add incremental persistence complexity without evidence.

Acceptance:

- N editors initialize the bundle once and styles once.
- Strict CSP test documents the remaining requirements and, at target completion, works without `unsafe-eval`.
- Mount/destroy loop has no registry, listener, observer, or mention-request leaks.
- Benchmarks cover 1/10/50 editors and 1k/10k/50k-word documents with agreed p95 input and mount budgets.

### P2.2 Finish catalog-driven typed component extensibility

Evidence addressed: [correctness defect 6](./review-correctness.md#6-high--the-runtime-catalog-does-not-actually-configure-the-browser-schema-commands-or-menus) and [upstream P2-1](./review-upstream-comparison.md#p2-1--rust-catalog-wire-model-and-prosemirror-schema-are-manually-duplicated).

Targets:

- Make `engine_manifest.rs` the generated/versioned structural contract.
- Generate TypeScript wire types, standard descriptors, and golden fixtures from the Rust-owned schema/manifest or a single neutral schema source.
- Add an explicit TypeScript behavior-adapter registry for extensions, NodeViews, commands, input rules, clipboard behavior, read rendering, and format capabilities.

Implementation:

1. Structural declarations are generated; executable behavior remains explicitly registered and reviewed.
2. Catalog build/mount validates that every editable component has its required adapter and every output format declares fidelity.
3. Separate document catalog and clipboard schema fingerprints.
4. Add one real custom typed atomic component fixture proving typed load, edit, command insertion, copy/paste identity, snapshot, read rendering, and typed re-encode.
5. Unknown components remain inert opaque fallbacks only when the configured unknown policy permits them; they are not treated as proof of editable extensibility.

Acceptance:

- Changing a Rust attribute/default changes generated fixtures/fingerprint and fails stale TypeScript checks.
- The custom component round trip passes end to end without hard-coding its document shape in the core conversion switch.
- Missing adapters and unsupported formats fail early with exact component/adapter/format names.

## Phase 8 — Notion-like block interaction and release hardening

Priority: P2

Release status: interaction completion and final confidence

Dependencies: identity and contextual UI plugins

### P2.3 Add hovered-block drag and reorder

Evidence addressed: [upstream P2-4](./review-upstream-comparison.md#p2-4--notion-like-block-drag-interaction-is-not-implemented).

Targets and behavior:

- Extend the contextual UI plugin with a hovered semantic-block target, frozen-menu state during interaction, mapped drop target, and cleanup.
- Use ProseMirror native drag slices/transactions. Move preserves IDs; copy-drag strips/remaps IDs.
- Add drag image and clear insertion indicator. Preserve move-up/down keyboard commands as accessible parity.
- Do not allow an inner-cell hover to move the entire table unless the table itself is explicitly selected.

Acceptance:

- Same-editor move, modifier copy, cross-editor copy, cancelled drag, scroll during drag, nested lists/tables, and teardown all preserve document and identity invariants.
- Complete keyboard alternative and screen-reader instructions exist.

### P2.4 Cross-browser, IME, mobile, fuzz, and performance gate

Evidence addressed: all coverage gaps in the correctness report and [upstream P2-5](./review-upstream-comparison.md#p2-5--test-breadth-does-not-match-the-implementations-claims).

Automated matrix:

- Chromium, Firefox, and WebKit in CI;
- Markdown corpus and codec fuzzing with size/depth/time limits;
- clipboard/event JSON fuzzing;
- revision state-machine and command-sequence identity property tests;
- RTL, cross-block deletion, browser HTML paste corpus, cell selection, popup scrolling/resize, strict CSP, and multi-editor leak/performance tests.

Manual matrix:

- macOS Safari and iOS Safari; Chrome Android;
- Japanese, Chinese, and Korean IMEs; dead keys; dictation; emoji; bidi/RTL;
- VoiceOver, NVDA, and JAWS;
- touch selection, visual keyboard viewport, 200% zoom, forced colors, and reduced motion.

Acceptance:

- No open P0/P1 correctness, security, data-loss, or keyboard-accessibility defect.
- All enabled Markdown commands pass the exact public component-boundary round trip.
- Performance budgets and supported browser/device matrix are documented from measurements, not inferred.

## 5. Work-item dependency graph and merge order

1. Phase 0 harness and temporary command gates.
2. Native v2 Markdown and minimum capability manifest.
3. Identity extension and Rust snapshot validation. This may be developed alongside step 2 but merges after shared fixtures agree.
4. Synchronous revision ownership, exact teardown flush, and history policies.
5. Direct v2 readonly rendering, URL roles, and table geometry validation.
6. Contextual UI plugin, suggestion plugins, and accessibility semantics.
7. Correct table paste, followed by contextual table handles.
8. Once-only engine loading/performance instrumentation and full typed-component adapter architecture.
9. Block drag and the final browser/IME/mobile/AT/performance gate.

Do not begin table-handle or drag polish before identity and revision gates pass. Their correctness depends on stable IDs, mapped selections, and reliable snapshots.

## 6. Definition of done by release tier

### Bounded internal Markdown canary

- All P0 phases pass.
- Only lossless Markdown commands are visible.
- Browser events are validated and recoverable.
- Reset replacement and teardown durability pass in the real Dioxus/browser harness.
- Readonly rendering is either direct v2 and safe or explicitly unavailable in the canary.

### Broad Markdown release

- All P0 and P1 work passes.
- Native v2 readonly rendering, URL policy, table validation/paste, contextual-state, keyboard suggestions, and toolbar semantics are complete.
- Chromium/Firefox/WebKit automation and the defined manual IME/mobile/AT smoke matrix pass.
- No “saved” state can contain silently discarded active-format data.

### Typed-component-ready architecture

- P2.2 custom-component fixture passes end to end.
- Generated manifest/types remove structural drift.
- Behavior adapter and capability failures are detected at catalog build/mount.
- Clipboard and catalog fingerprints have distinct meanings.

### Notion-like interaction completion

- Contextual table handles, hovered block gutter, drag/reorder, touch adaptations, and keyboard parity pass the accepted interaction plan.

## 7. Recommendations intentionally not adopted

### Do not make v1 migrations complete as the primary fix

Both reports mention completing migrations as a possible interim measure. That would preserve the duplicated, lossy architecture and still block future typed components. Keep v1 compatibility stable, but implement Markdown directly on v2. Temporary command gating is safer than deepening the v1 dependency.

### Do not wholesale copy Tiptap or BlockNote internals

Use public Tiptap/ProseMirror primitives where available and adapt the cited algorithms narrowly. BlockNote is an interaction reference, not a source dependency or code donor. Wholesale copies add license/update burden and couple this editor to private upstream behavior.

### Do not retain browser debounce with only a `localDirty` event

An immediate dirty event could plug the overwrite race, but it leaves teardown, duplicate flush, full-snapshot ownership, and two-state-machine complexity. Emit each editor revision synchronously and debounce only application persistence.

### Do not repair IDs during serialization or normalize every imported duplicate silently

Serialization-time IDs are inherently unstable, and silent repair changes external identity. IDs belong in ProseMirror transactions. Reject invalid imported typed documents; provide any deliberate repair as a separate migration tool.

### Do not expose Markdown-only illusions

Do not persist `colwidth` in v2 during a Markdown session, expose merge/split spans, or report a save after discarding them. Disable these controls or keep clearly ephemeral view state until a typed format owns the data.

### Do not build a generic runtime schema from untrusted manifest data

The Rust manifest can describe structure and capabilities, but executable NodeViews, DOM parsing/rendering, commands, and upload behavior require allowlisted JS adapters. Treating arbitrary manifest data as executable schema/DOM behavior would expand the security boundary.

### Do not optimize to transaction deltas before measuring

Once repeated bundle evaluation and duplicate layout work are removed, measure full-snapshot costs. Delta protocols add recovery, ordering, and compatibility complexity and are not justified by the current evidence.

### Do not promise synchronous network durability on browser termination

Guarantee component-unmount flush into the Rust/application layer. Browser/tab termination requires a separate durable-draft or beacon design and cannot be made reliable by editor blur handlers.

## 8. Validation commands for each merge

Run through the Nix devshell when available, in this order:

```sh
nix develop --command bash -lc 'cargo check --quiet --message-format=short -p dxeditor'
nix develop --command bash -lc 'cargo test --quiet --message-format=short -p dxeditor'
nix develop --command bash -lc 'cargo fmt --all -- --check'
cd crates/dxeditor/web
npm run check
npm test
npm run test:e2e:chromium
```

For release candidates, also run all configured Playwright projects in the CI image, the bundle-diff target, fuzz/property suites, strict-CSP fixture, and performance/leak benchmarks. Finish with `git diff --check`.

## 9. Evidence preservation

The detailed reproductions, exact baseline line references, and pinned upstream source links remain in the two input reports:

- [Editor revamp correctness review](./review-correctness.md)
- [Editor revamp upstream implementation comparison](./review-upstream-comparison.md)

If source line numbers move during remediation, update this plan only when scope or behavior changes; keep the review reports immutable as baseline evidence.
