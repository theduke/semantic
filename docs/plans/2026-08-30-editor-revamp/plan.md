# Editor Revamp Implementation Plan

Date: 2026-08-30
Status: proposed
Scope: `crates/dxeditor` and its note-editor integration in `crates/ui_core`

## 1. Executive decision

Replace the handcrafted browser-editing core with a **Tiptap-on-ProseMirror editing island**, while keeping:

- the public editor API and persistence integration in Rust/Dioxus;
- the visual chrome and product UI in the project's own Dioxus/`dxcomp` components;
- a project-owned, versioned component document as the interchange model;
- Markdown as the initial input and output format through Rust-owned codecs; and
- a second, lossless typed-component codec as an explicit future format.

Do not continue expanding the current custom `contenteditable` event/selection engine. The expensive part of a rich editor is not drawing paragraphs: it is making selection, composition/IME, autocorrect, history, cross-block edits, clipboard, drag/drop, bidirectional text, mobile keyboards, and browser DOM recovery agree. MDN explicitly notes that `beforeinput` can be absent or non-cancelable for IME, spellcheck, autocorrect, autofill, and other browser/OS paths ([MDN: `beforeinput`](https://developer.mozilla.org/en-US/docs/Web/API/Element/beforeinput_event)). ProseMirror already separates schema, immutable document state, mapped selections, transactions, browser view, history, commands, and collaboration primitives ([ProseMirror guide](https://prosemirror.net/docs/guide/)). Its 2026 changelog still contains fixes for composition, selection, drag, browser-inserted DOM, and surrogate-pair boundaries, which illustrates both active maintenance and the depth of this problem ([ProseMirror changelog](https://prosemirror.net/docs/changelog/)).

Use Tiptap only as a framework-agnostic extension/configuration layer over ProseMirror. Do **not** adopt Tiptap UI, React bindings, cloud services, its JSON as the permanent product format, or `@tiptap/markdown` as the canonical Markdown implementation. Tiptap's extension-composed schema, commands, paste hooks, node views, content validation, and vanilla-JavaScript integration reduce plumbing while preserving direct ProseMirror escape hatches ([Tiptap schema](https://tiptap.dev/docs/editor/core-concepts/schema), [extensions](https://tiptap.dev/docs/editor/core-concepts/extensions), [node views](https://tiptap.dev/docs/editor/extensions/custom-extensions/node-views)). Its Markdown feature is currently marked beta, so the existing Rust format boundary should remain authoritative ([Tiptap Markdown integration](https://tiptap.dev/docs/editor/markdown/guides/integrate-markdown-in-your-extension)).

The target data flow is:

```text
stored Markdown (now)                         typed component payload (later)
          |                                              |
          v                                              v
  MarkdownFormatCodec <----> ComponentDocumentV2 <----> TypedDocumentCodec
                                   |
                          validated wire document
                                   |
                                   v
 Dioxus chrome <---- typed bridge ----> Tiptap/ProseMirror session
 (contextual surfaces,                    (DOM, selection, history,
  dialogs, save state)                     IME, transactions, clipboard)
```

During an editing session, ProseMirror owns its state and editable DOM. Rust must not process or rerender the document on every keystroke. Rust owns load validation, snapshots, Markdown encoding, persistence, and domain-level component definitions. This is a deliberate two-runtime boundary, not two competing sources of truth.

## 2. Goals, non-goals, and quality bar

### 2.1 Goals

1. Deliver a reliable Notion-like block editor for Markdown-backed notes: a quiet canvas by default, contextual controls near the current selection or block, and block-first insertion/reordering interactions.
2. Support normal rich-editing expectations: native typing, selection across blocks, undo/redo, common marks and blocks, rich paste, Markdown shortcuts, slash commands, keyboard navigation, IME, mobile input, and safe links.
3. Make conversion behavior explicit. No supported or unsupported Markdown construct may disappear silently.
4. Make components first-class in the architecture rather than hard-coded render branches.
5. Keep current callers working through compatibility façades while a v2 implementation is rolled out.
6. Make a later typed-component input/output format additive, not a rewrite.
7. Establish security, accessibility, browser, durability, and performance gates before broad rollout.

### 2.2 Initial Markdown profile

Adopt **CommonMark 0.31.2 plus an explicitly selected GFM subset**. CommonMark defines the core block and inline constructs; GFM additions must be listed rather than enabled accidentally ([CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/)). The initial editable profile should cover:

- paragraphs and soft/hard breaks;
- ATX and Setext headings, levels 1-6;
- blockquotes, including nested blocks;
- ordered and unordered nested lists;
- task-list items;
- fenced and indented code blocks, retaining the complete fence info string;
- thematic breaks;
- emphasis, strong emphasis, strikethrough, inline code, and links;
- images with URL, alt text, and optional title;
- autolinks;
- GFM tables; and
- the existing semantic entity mention convention, documented as a project extension.

Footnotes, raw HTML, comments, link-reference-definition placement, and any unknown extension must initially be **preserved as explicit opaque nodes** if they cannot be edited without loss. They may become fully editable in later increments. Raw HTML is never executed in the editor.

### 2.3 Non-goals for the first release

- Real-time multi-user collaboration.
- Byte-for-byte retention of every Markdown spelling after an edited block is serialized.
- Arbitrary HTML authoring.
- A general page-layout or database-block product.
- Hard virtualization of the editing surface before measurement proves it necessary.
- Replacing all existing editor public types in one breaking change.

### 2.4 Definition of quality

The release is not complete merely when bold and headings work. It is complete only when the acceptance matrix in section 13 passes across Chromium, Firefox, WebKit, desktop keyboard use, representative real mobile devices, and representative IMEs; conversion and sanitization properties pass; the contextual, Notion-like interaction contract in section 9 passes; and rollback is demonstrated.

## 3. Repository audit and problems to solve

The existing implementation has useful boundaries worth retaining: `EditorPayload` and `EditorCodec`, `EditorCatalog`, action/command registries, a serializable document, a transaction concept, and a debounced note integration. The issue is that these abstractions are not carried through the browser editing path.

| Area | Current implementation | Consequence | Plan response |
|---|---|---|---|
| DOM ownership | Dioxus renders the root and every block, while an inline JS bridge also mutates/reconciles the same `contenteditable` subtree (`component.rs:210-315`, `bridge/editor_bridge.js`) | Browser composition and Dioxus rerenders can race; selection is restored manually after model changes | ProseMirror exclusively owns descendants of one mounted host; Dioxus owns only the host and surrounding chrome |
| Editable blocks | The root is editable and each rendered block is also individually editable; editable rendering is a hard-coded `match` (`component.rs:533-673`) | Nested editing hosts, browser-dependent behavior, and registered custom renderers are bypassed | One editing host; schema-driven nodes/node views |
| Input handling | The bridge calls `preventDefault()` for every non-composition `beforeinput`, but Rust handles only insert text, paragraph, and single-character delete (`input.rs:32-59`) | Unsupported operations such as word deletion, line breaks, history, format commands, drop, autocorrect, and some replacement paths can become no-ops/data loss | Let ProseMirror own browser input and reconciliation; regression-test all supported input categories |
| Selections | Positions are block ID plus a character offset and multi-block replacement returns an empty transaction (`input.rs:68-93`) | Cross-block cut/delete/paste/format cannot work; node selections and direction/affinity are missing | Use ProseMirror selections in-session; expose a logical selection summary over the bridge |
| Unicode | Deletion advances by Rust `char` | A visible grapheme can contain multiple code points and be split incorrectly | Use the engine/browser editing behavior and add emoji/combining-mark regression tests; Unicode recommends grapheme clusters for cursor movement and deletion ([UAX #29](https://unicode.org/reports/tr29/)) |
| IME | Browser mutates a block during composition, then the whole block is minimally diffed and the Dioxus projection rerenders | Composition can be disrupted; only one flat block is reconciled | Delegate composition to ProseMirror and prohibit host remount/controlled updates during active composition |
| Paste | Only `text/plain` is read and it is inserted into one block | Rich structure is discarded and multi-block paste is not modeled | Layered custom/HTML/plain clipboard pipeline with schema parsing and sanitization |
| Markdown parsing | `pulldown-cmark` uses `Options::empty()` and the builder ignores lists, images, tables, task lists, strikethrough, footnotes, and other events (`markdown.rs:150-205`) | Existing Markdown can be silently deleted on first edit/save | Full profile translator, opaque nodes, diagnostics, and fixture/property gates |
| Markdown serialization | Unknown blocks fall back to inline text and custom content can serialize to empty (`markdown.rs:225-345`) | Typed/unknown components are lossy | Every component declares a format capability; encoding fails or preserves an opaque representation instead of silently degrading |
| Component extensibility | The catalog registers lists/tables and renderers, but editable rendering supports only headings, quote, code, divider, and paragraph fallback | Registry presence does not imply editability or serialization | One `ComponentSpec` drives schema, validation, editing behavior, rendering, commands, and format adapters |
| Controlled value | `Editor` consumes the input only at initialization; note integration increments a key to remount on external changes (`notes.rs:140-181`) | External changes reset history/focus/selection and can race with pending edits | Revisioned `load`, `acknowledge`, and `conflict` protocol; no remount for normal prop echoes |
| Errors | Decode errors become an empty paragraph (`component.rs:219-222`); several call sites ignore errors | Corrupt or unsupported content can be replaced without notice | Typed diagnostics and visible recovery/raw-source mode; never replace invalid input silently |
| Rendering security | The formatted note view renders `pulldown-cmark` HTML through `dangerous_inner_html` without sanitization (`notes.rs:301-306`) | Raw Markdown HTML can reach an executable DOM sink | Immediate P0 remediation; structural rendering or reviewed sanitizer, URL allowlists, CSP/Trusted Types defense-in-depth |
| Tests | Rust tests cover model operations, codecs, and SSR smoke output, but not a live browser | The hardest browser behavior is untested | Add TypeScript unit tests, Rust contract/property tests, and Playwright browser projects |

The security item is independent of the editor rewrite and should be fixed first. OWASP calls direct HTML sinks a framework escape hatch and recommends structural safe sinks or maintained HTML sanitization when authored HTML is unavoidable ([OWASP XSS Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html)).

## 4. Architecture decision record

### 4.1 Options considered

| Option | Browser correctness | Typed schema/components | Markdown path | Dioxus fit | Decision |
|---|---:|---:|---:|---:|---|
| Extend current Rust/custom `contenteditable` engine | Low without a long browser-engine effort | High in theory | Existing but lossy | Native | Reject |
| Direct ProseMirror | Excellent | Excellent strict schema/node views | Official semantic parser/serializer exists | Vanilla JS island | Viable fallback; more integration plumbing |
| Tiptap on ProseMirror | Excellent | Excellent extension-composed strict schema/node views | Beta Tiptap layer is replaceable | Vanilla JS; no React required | **Choose** |
| Lexical | Excellent | Strong custom/Decorator nodes and JSON state | Transformer-oriented; no full fidelity promise | Vanilla JS capable | Strong second choice |
| Slate | Good but application-heavy | Flexible, schema-light | Mostly application work | Official view layer is React | Reject for this repo |
| Milkdown | Strong Markdown-first ProseMirror architecture | Extensible | Best Markdown-first alternative | Another JS abstraction | Spike only if Markdown minimal-diff behavior dominates |

Lexical is credible: it keeps the DOM as a projection of immutable, JSON-serializable editor state and supports custom nodes ([Lexical editor state](https://lexical.dev/docs/concepts/editor-state), [Lexical nodes](https://lexical.dev/docs/concepts/nodes)). ProseMirror/Tiptap is a better fit here because strict content expressions, mapped transactions, clipboard hooks, node views, and Markdown-shaped schemas align more directly with typed component validation and this repository's current catalog concepts.

### 4.2 Dependency policy

- Use `@tiptap/core` and a curated list of project-reviewed extensions, not a broad starter kit whose behavior changes implicitly.
- Pin exact versions in a lockfile and use an automated dependency-update policy.
- Keep every editor-framework value behind `EditorEngine`/bridge interfaces. No UI or persistence crate may depend on Tiptap JSON names directly.
- Treat Tiptap/ProseMirror packages as browser implementation dependencies, not the product schema.
- Confirm licenses and record them in the repository's dependency notices before merge.
- Maintain a small direct-ProseMirror escape hatch through `addProseMirrorPlugins` and direct command APIs.

## 5. Target architecture

### 5.1 Ownership boundaries

**Rust/Dioxus owns:**

- public component properties and callbacks;
- `EditorPayload` compatibility and v2 payload envelopes;
- `ComponentDocumentV2`, schema versions, validation, and migrations;
- Markdown/typed/plain-text codecs and conversion diagnostics;
- the application component catalog;
- contextual interaction surfaces, optional editor-wide actions, dialogs, save/conflict UI, and design-system styling;
- external revisions, debounce/flush, persistence, and telemetry policy.

**Tiptap/ProseMirror owns during a mounted editing session:**

- the live document projection and editable DOM;
- selection, stored marks, composition state, history, transaction mapping, and native input reconciliation;
- core keymaps, input rules, paste/drop insertion, and node-view lifecycle;
- lightweight derived UI state such as active marks and available commands.

**Invariant:** Dioxus renders an empty host element and never reconciles descendants after the engine mounts. Dioxus officially provides the required escape hatches—Web Components, `eval`, `web-sys`, mounted-element access, and Rust/JS messaging ([Dioxus Web Components and direct DOM access](https://dioxuslabs.com/learn/0.7/essentials/ui/escape/), [Dioxus web platform](https://dioxuslabs.com/learn/0.7/guides/platforms/web/)). Prefer a typed `wasm-bindgen`/custom-event module over interpolated JavaScript strings.

### 5.2 Session flow

1. Caller passes `{format, value, external_revision}`.
2. Rust codec decodes to `ComponentDocumentV2`, producing diagnostics and a fidelity classification.
3. Rust validates the document against the active `ComponentCatalog`.
4. The bridge mounts a session with `{session_id, schema_fingerprint, revision, document, readonly}`.
5. Tiptap creates one ProseMirror schema and state and exclusively owns the editing host.
6. User input stays inside JS. Each transaction updates ProseMirror state synchronously.
7. JS emits small `UiStateChanged` events as necessary and coalesces `DocumentChanged` notifications.
8. Rust requests/receives a full component-document snapshot after an idle debounce, on blur, before submit/navigation, and on explicit save.
9. Rust validates and encodes the snapshot to the configured output format.
10. Parent receives `EditorChange {revision, origin, value, diagnostics}` and persistence acknowledges that exact revision.

No document snapshot or Markdown serialization occurs in the hot typing path.

### 5.3 External updates

Replace keyed remounts with an explicit policy:

- An echoed value carrying the latest emitted revision is an acknowledgement; do not reload.
- A newer external revision while local state is clean calls `replaceDocument` while preserving focus when possible and resets history deliberately.
- A newer external revision while local state is dirty enters `conflict`; do not overwrite. The caller chooses reload, keep local, or future merge.
- An external update during composition is queued until composition ends.
- Every async response carries `session_id` and revision; stale responses are ignored.

## 6. Component document v2 and schema

### 6.1 Wire shape

Introduce a v2 model parallel to the legacy structs. Its serialized shape should intentionally align with ProseMirror's node/mark tree to avoid lossy per-snapshot mapping, while remaining Rust-owned and framework-neutral:

```rust
pub struct ComponentDocumentV2 {
    pub schema: DocumentSchemaId,       // e.g. "semantic.component-document"
    pub version: u32,
    pub root: ComponentNode,
    pub metadata: DocumentMetadata,
}

pub struct ComponentNode {
    pub kind: ComponentId,
    pub id: Option<NodeId>,             // required by spec for semantic block/atom nodes
    pub attrs: AttributeMap,
    pub content: Vec<ComponentNode>,
    pub text: Option<String>,            // valid only for text nodes
    pub marks: Vec<ComponentMark>,       // valid only for inline/text nodes
}

pub struct ComponentMark {
    pub kind: ComponentId,
    pub attrs: AttributeMap,
}
```

This is conceptual, not a mandate to use unvalidated `serde_json::Value` everywhere. Standard components should expose typed Rust attribute structs/accessors; the wire boundary can use JSON-safe maps after schema validation.

### 6.2 IDs and positions

- Use globally unique, opaque IDs for semantic blocks, atoms, and objects referenced outside their tree position. Do not expose `block-1` counters.
- Text leaf IDs are unnecessary unless a future feature has an external reference to them; ProseMirror positions handle in-session text.
- Markdown-derived IDs are session-local because Markdown cannot persist them. The typed format persists them.
- Copy creates new IDs unless the operation is a move.
- Normalization must enforce uniqueness and deterministically repair only explicitly repairable imported collisions; otherwise return an error.
- Persistent comments/presence are out of scope, but future anchors must not be raw DOM paths or bare integer offsets. Yjs documents why relative positions survive concurrent edits while integer positions do not ([Yjs relative positions](https://docs.yjs.dev/api/relative-positions)).

### 6.3 `ComponentSpec`

Replace the metadata-only registration with a schema-bearing spec:

```text
ComponentSpec
  id + component_version
  kind: block | inline | atom | mark
  group/content expression
  attribute definitions, defaults, validators, migrations
  identity policy
  editable/selectable/draggable/isolating/defining behavior
  DOM semantic/render descriptor
  command and input-rule contributions
  clipboard policy
  plain-text fallback
  per-format capability and adapter
  optional custom node-view module key
```

The catalog build must fail on duplicate IDs, incompatible content expressions, missing required format handlers, unsafe DOM descriptors, or conflicting command/shortcut IDs. Produce a deterministic `schema_fingerprint` from ordered specs and versions. Rust and TypeScript schema builders must be checked against shared golden fixtures in CI.

Initial component pack:

- root/document;
- paragraph, heading, blockquote;
- bullet list, ordered list, task list, list item, task item;
- code block, thematic break;
- table, table row, header cell, cell;
- text, hard break, image, mention, opaque Markdown block/inline;
- bold, italic, strike, code, link marks.

ProseMirror schemas explicitly constrain allowed node nesting and marks, while node views separate complex editing UI from serialized output ([ProseMirror schema and node views](https://prosemirror.net/docs/guide/)). Tiptap exposes useful behavior flags such as atom, defining, isolating, groups, content expressions, and table roles ([Tiptap schema](https://tiptap.dev/docs/editor/core-concepts/schema)).

### 6.4 Validation and unknown components

Validation occurs after every external decode, typed clipboard read, migration, and engine snapshot:

- schema/version recognized;
- node/mark kind registered;
- tree satisfies content expressions and depth/count/size limits;
- required/allowed attributes and types;
- stable ID policy and uniqueness;
- URL/media policies;
- component-specific invariants;
- no executable or untrusted DOM payload in attributes.

Typed v2 input must preserve unknown component payloads as `unknown_component` atoms only when the caller explicitly enables forward-compatible preservation and a safe text fallback exists. It must never silently coerce them to paragraphs.

## 7. Format architecture and Markdown fidelity

### 7.1 Codec API

Evolve, rather than abruptly replace, `EditorCodec`:

```rust
pub trait DocumentFormat: Send + Sync {
    fn descriptor(&self) -> &FormatDescriptor;
    fn decode(
        &self,
        input: &EditorPayload,
        catalog: &ComponentCatalog,
        options: DecodeOptions,
    ) -> Result<DecodedDocument, FormatError>;
    fn encode(
        &self,
        document: &ComponentDocumentV2,
        catalog: &ComponentCatalog,
        options: EncodeOptions,
    ) -> Result<EncodedPayload, FormatError>;
}

pub struct DecodedDocument {
    pub document: ComponentDocumentV2,
    pub diagnostics: Vec<FormatDiagnostic>,
    pub fidelity: Fidelity,
    pub source_state: Option<FormatSourceState>,
}
```

`FormatDescriptor` declares media type/version and supported component/mark capabilities. Diagnostics include stable code, severity, source range/node ID, and recovery action. Replace the ambiguous `fallback_unknown_components: bool` with explicit policies: `Reject`, `PreserveOpaque`, or `ConvertWithWarning`.

Keep adapters for the existing formats:

- `markdown` -> v2 Markdown codec;
- `plain_text` -> v2 plain-text codec;
- `dxeditor.document.v1` <-> v2 migration adapter;
- future `semantic.component-document` versioned typed codec.

### 7.2 Markdown parser

Keep `pulldown-cmark` for the first implementation to minimize dependencies, but replace the flat ad-hoc builder with a complete stack-based translator. The crate supports optional footnotes, GFM tables, task lists, and strikethrough and exposes source ranges through `into_offset_iter()` ([pulldown-cmark 0.13 documentation](https://docs.rs/crate/pulldown-cmark/0.13.4)). Use source ranges to preserve unsupported constructs as raw slices and to produce diagnostics.

Before committing to the translator, run the codec corpus against `markdown-rs` as a bounded alternative spike. `markdown-rs` provides a CommonMark/GFM mdast with positional information and reports CommonMark, GFM, coverage, and fuzz testing ([markdown-rs](https://github.com/wooorm/markdown-rs)). Switch parser internals only if it materially reduces custom tree-building or opaque-source handling. The `DocumentFormat` boundary makes that choice non-architectural.

Do not use raw HTML rendering as the parser's output. Decode to component nodes, validate, then render structurally.

### 7.3 Fidelity contract

Publish these guarantees in crate docs and tests:

1. **No silent loss.** Every source construct is editable, preserved opaquely, or rejected with a diagnostic.
2. **Semantic round trip for supported constructs:**
   `decode(encode(normalize(document))) == normalize(document)`.
3. **Canonical stability:**
   `encode(decode(encode(document))) == encode(document)`.
4. **Unchanged-input optimization:** if a loaded document has no semantic change, emit the original Markdown bytes rather than canonicalizing it.
5. **Opaque preservation:** unchanged opaque source nodes emit their exact raw source slice.
6. **Edited supported blocks:** emit documented canonical Markdown. Equivalent spellings such as `_x_`/`*x*`, bullet characters, indentation, reference links, or heading forms may normalize.
7. **Edited opaque blocks:** require a raw-source editor or explicit conversion; never discard them.
8. **Line endings/final newline:** retain original line-ending style for an unchanged document; canonical edited output uses LF and one configured final-newline policy.

Byte-exact Markdown round trip is not the general promise. Parsing reduces many spellings to one semantic tree; even ProseMirror's official Markdown example says its schema expresses Markdown constructs and converts between Markdown and a document, not that source trivia is retained ([ProseMirror Markdown example](https://prosemirror.net/examples/markdown/)). If minimal diffs later become a product requirement, add block-level source-span patching behind the same codec interface rather than contaminating the editor model with Markdown trivia.

### 7.4 Construct policy table

Maintain a machine-readable version of this table and generate documentation/tests from it:

| Construct | Decode | Edit | Encode | Initial policy |
|---|---|---|---|---|
| Paragraph/headings/marks/links | semantic | full | canonical | required |
| Nested blockquotes/lists | semantic tree | full | canonical | required |
| Task lists | semantic tree + checked attr | full | GFM | required |
| Code blocks/info strings | semantic | full | safe dynamic fence | required |
| Tables | semantic | cell editing | GFM table | required before default-on |
| Image | semantic atom | attributes/dialog | Markdown image | required; URL validation |
| Autolink/reference link | semantic link | full | canonical link form | reference-definition placement may normalize |
| Footnote | opaque initially | move/delete/raw edit | exact raw | no silent loss |
| Raw HTML/comment | opaque text atom | raw edit only | exact raw | never execute |
| Semantic mention | typed inline atom | picker/edit/remove | documented `semantic:` extension | validate entity ID and safe rendering |
| Unknown extension | opaque or decode error | raw edit/delete | exact raw | controlled by decode policy |

### 7.5 Future typed format

Add `application/vnd.semantic.component-document+json` (final name to be confirmed) with:

- envelope schema ID and version;
- component IDs and component versions;
- persisted stable node IDs;
- lossless typed attributes and document metadata;
- pure sequential migrations;
- unknown-component preservation rules;
- JSON Schema or equivalent generated validation artifact;
- size/depth/node-count limits.

Markdown remains an import/export and current storage adapter. The typed format can represent components that Markdown cannot without inventing hidden syntax. If a typed-only component must be saved as Markdown before the typed format ships, it needs an explicitly specified fenced-directive escape syntax and a tested parser/serializer; otherwise insertion is disabled in Markdown mode with a clear explanation.

## 8. Browser engine, bridge, and command design

### 8.1 Web package and build

Add a small browser package under `crates/dxeditor/web/`:

```text
web/
  package.json
  package-lock.json
  tsconfig.json
  src/
    index.ts
    session.ts
    bridge.ts
    schema.ts
    extensions/
    node_views/
  tests/
```

Use TypeScript, an exact lockfile, and a small deterministic bundler such as esbuild. Node 22 is already present in the Nix UI shell. Add explicit `editor-web-build`, `editor-web-check`, and `editor-e2e` Make targets and CI steps. Decide in the Phase 0 spike whether the compiled asset is checked in or always built; whichever is chosen, CI must verify that the served asset matches the locked sources. Cargo-only native builds must not unexpectedly run a networked npm install.

### 8.2 Typed bridge protocol

Define Rust `serde` enums and matching TypeScript discriminated unions. Generate types or verify shared JSON fixtures so the protocol cannot drift.

Rust -> engine commands:

- `Mount {session, schema, document, config}`;
- `SetReadonly`;
- `RunCommand {request_id, command, args}`;
- `RequestSnapshot {request_id, revision}`;
- `ReplaceDocument {external_revision, document, history_policy}`;
- `Focus {position?}`;
- `Destroy`.

Engine -> Rust events:

- `Ready {schema_fingerprint}`;
- `TransactionApplied {revision, origin, doc_changed, history_group}`;
- `UiStateChanged {selection_summary, active_marks, block_kind, can_undo, can_redo, active_surfaces, anchors, command_availability}`;
- `Snapshot {request_id, revision, document}`;
- `FocusChanged`;
- `CompositionChanged`;
- `Diagnostic`;
- `FatalError {recoverable, last_good_revision}`.

Validate message size, session, version, request ID, and schema fingerprint at the boundary. Do not send arbitrary JS snippets, DOM selectors, or user content interpolated into executable strings.

### 8.3 Commands and actions

Retain the useful action concept, but make it a UI-facing typed command catalog rather than a parallel transaction engine:

- namespaced IDs (`history.undo`, `mark.toggle.bold`, `block.set.heading`, `link.open`, `component.insert`);
- typed argument schema;
- label/icon/group/shortcut metadata;
- visibility/enabled/active state derived by the engine;
- explicit eligible surfaces: `editor_actions`, `text_selection`, `empty_block_add`, `slash_insert`, `block_menu`, `link_popover`, `media_controls`, `table_controls`, and `keyboard`;
- placement/order, surface-specific presentation hints, and a reason when a visible command is disabled;
- predicates over selection kind, active node/mark, editability, and schema/format capabilities; UI code must not duplicate command eligibility rules;
- execution delegated to the engine command;
- stable telemetry code with no content.

`editor_actions` is intentionally narrow: history undo/redo, optional editor-level status, and an optional overflow for document-wide operations such as raw-source/recovery mode. It must never contain marks, block conversion, insertion, links, media, or table commands. The editor API should allow this surface to be omitted entirely. Content-specific commands belong to the contextual surface anchored to their content.

The bridge emits one compact `UiStateChanged` projection containing the active contextual surface(s), logical anchor, command states, and selection summary. The Dioxus layer renders product-owned UI from that projection and sends typed commands back. The engine retains the live selection while a contextual control has focus, maps anchors through transactions, and dismisses stale surfaces on selection/document changes. Define deterministic arbitration so overlapping states do not stack incompatible menus: an open modal/menu wins; then node/table selection; then link; then non-collapsed text selection; then slash/mention; then the passive block gutter/add affordance.

Use ProseMirror's mapped transactions and history as the session mechanism. Do not attempt to keep the old Rust `Transaction` and ProseMirror transactions in lockstep. Preserve the old model/command types under a legacy module until v1 callers are migrated, then deprecate them.

### 8.4 Input rules and Notion-like behavior

Implement in focused layers:

- standard keymap and history first;
- Markdown-at-start rules (`# `, `> `, `- `, `1. `, task item, fences) as engine transactions;
- `/` slash menu only when the selection is collapsed in an eligible text block;
- `@` mention provider through an async request with cancellation, stale-request guards, and no selection loss;
- `Enter`: context-aware split/create/exit list or code block;
- `Shift+Enter`: hard break where allowed;
- `Backspace`: context-aware lift/join/convert-empty-block, never a hard-coded previous-block string merge;
- `Tab`/`Shift+Tab`: indent/outdent or navigate table cells only in contexts where that is expected; always provide a documented way to leave the editor or active contextual surface;
- platform-correct shortcuts using `Mod`, without intercepting browser/OS/assistive-technology shortcuts.

ProseMirror's base command chains deliberately combine selection deletion, backward joins, node selection, and native browser behavior rather than mapping Backspace to one operation ([ProseMirror guide, commands](https://prosemirror.net/docs/guide/)).

### 8.5 Clipboard, paste, cut, and drop

Copy emits, in order of richness:

1. versioned internal MIME, for example `application/x-semantic-component-document+json`;
2. sanitized `text/html` with semantic tags;
3. `text/plain`, using Markdown for structured selections only if product testing shows that is less surprising than visible text.

Paste preference:

1. internal typed MIME, validated as untrusted input against schema/version/limits;
2. sanitized HTML parsed through the schema;
3. plain text, with an explicit "paste as Markdown" command and conservative auto-detection only for unmistakably multi-line Markdown.

Preserve multi-block selections and marks. Normalize office-suite/Google Docs HTML through an allowlist; discard styles and elements not represented by the schema. Paste/drop never injects HTML directly into the live document. ProseMirror exposes clipboard parsers/serializers and transforms for slices, HTML, and text ([ProseMirror reference](https://prosemirror.net/docs/ref/)).

Apply payload byte, node-count, depth, table-size, and URL limits. Add paste fixtures for browsers, Office, Google Docs, GitHub, Notion, plain Markdown, malformed HTML, and the security corpus.

## 9. Internal components and editor UX

### 9.1 UI composition

Use existing `dxcomp` primitives for buttons, popovers, menus, comboboxes, dialogs, tooltips, and toasts. Move the large inline CSS constant out of `component.rs` into the crate asset pipeline and design tokens.

The interaction target is deliberately close to Notion: the normal typing state is a quiet page, not a conventional word processor. There is **no persistent formatting or content toolbar**. Formatting, conversion, insertion, link, media, and table actions appear only in a floating or popdown surface relevant to the current selection or element. Animations, density, spacing, corner radii, shadows, iconography, and hover/focus transitions should use project design tokens while preserving this interaction model; do not copy proprietary assets.

An optional minimal editor-actions strip may be configured above or beside the canvas. It is hidden by default when the host already exposes equivalent actions. Its allowed contents are undo, redo, save/sync status, and an editor-level overflow for recovery or document-wide actions. It must remain visually subordinate and must not acquire bold/italic, headings, block types, insertion, links, media, table, or other content-specific actions.

Contextual surfaces and ownership:

| Context | Surface and anchor | Required behavior |
|---|---|---|
| Non-collapsed text selection | Floating text-selection bubble anchored to the mapped selection range | Bold, italic, strikethrough, inline code, link create/edit, and only other marks valid across the whole selection. Show active/mixed/disabled state; preserve the selection while controls are used; dismiss on collapse, Escape, or invalidating edit. Block-type changes belong in the block menu, not this bubble. |
| Empty or focused text block | Low-chrome add affordance in the block gutter, aligned to the block | Opens the insertion picker without inserting placeholder data. Keyboard users can invoke the same picker with a documented shortcut; the affordance remains discoverable without hover. |
| Focused/hovered block | Block gutter/drag handle plus popdown block menu anchored to the handle | Drag reorder, turn into, color/style only if supported by the schema/format, duplicate, copy link when stable identity supports it, move up/down, and delete. Multi-block selection applies compatible commands to the selection. Drag is additive, never the only reorder path. |
| Collapsed caret after `/` in an eligible text block | Searchable slash insertion menu anchored to the caret | Filter by label/aliases, group standard/internal components, show recent items and shortcuts, explain disabled results, support arrows/Home/End/Enter/Escape, and remove the trigger query atomically on insertion. Async results carry session/revision guards. |
| Caret in or selection over a link | Link popover anchored to the link range | Validated URL and label editing, open, copy, and remove. Opening/focusing the popover must retain a mapped link range; unsafe protocols never become commands or previews. |
| Selected image/media/internal visual node | Floating media controls anchored to the node, with a popdown/dialog for detailed attributes | Replace/upload or URL action as product capabilities allow, alt text, caption, alignment/size choices representable by the active format, open, and delete. Selection handles and resizing must not mutate persisted attributes outside an engine transaction. Unsupported Markdown presentation attributes are hidden or produce an explicit conversion policy, never silent loss. |
| Caret/selection in a table | Table-local floating controls, row/column edge handles, and focused popdown menus anchored to the table/cell/handle | Insert/delete row or column, toggle header row where representable, alignment, select row/column/table, and delete table. Support rectangular cell selection, keyboard navigation, `Tab`/`Shift+Tab`, adding a row at the final cell, column resizing when representable as editor-only state, and structured paste from spreadsheets/HTML/TSV. Do not offer merged cells or other operations that GFM Markdown cannot encode unless the active typed format declares them lossless. |
| Editor/document state | Status outside the editable tree; optional minimal editor-actions strip | Save/offline/conflict/error state is always perceivable without occupying a formatting toolbar. Raw-source/recovery mode is available from error UI or editor-level overflow. |

Only one primary popdown is open at a time. A non-interactive gutter and a text bubble may coexist only when they do not obscure the selection; opening any menu dismisses competing popdowns. Positioning must collision-flip within the editor viewport, remain attached through scroll/resize/transactions, and avoid covering the caret or selected content when a viable alternative placement exists. Escape closes the topmost surface and restores the mapped editor selection; outside pointerdown closes it without swallowing the intended editor action.

### 9.2 Table editing quality bar

Tables are first-class blocks, not merely a serializer feature. Use ProseMirror table primitives or an equivalently mature implementation for table maps, cell selections, row/column operations, and normalization. Maintain a rectangular, structurally valid table after every transaction; commands must preserve headers and alignment metadata or report why they cannot.

Required table behavior includes:

- create a table from the slash menu with sensible defaults and immediate first-cell focus;
- keyboard and pointer cell/row/column/table selection with visible selected states that survive contextual-control focus;
- add/remove/reorder rows and columns where supported, with confirmation only for unusually destructive multi-cell loss;
- `Tab`/`Shift+Tab`, arrow-key, Home/End, Enter, and Escape behavior documented and tested without trapping keyboard users;
- paste a rectangular spreadsheet/HTML/TSV selection into cells, growing the table within configured limits; copy emits internal MIME, semantic HTML, and useful plain text;
- column width as non-persisted editor presentation for Markdown unless the future typed codec explicitly supports width; and
- narrow viewport behavior using contained horizontal scrolling rather than compressing cells below usability.

### 9.3 Node views

- Simple text blocks use ProseMirror DOM serialization, not bespoke node views.
- Atomic internal components use vanilla node views or framework-neutral custom elements.
- Complex Dioxus UI inside a node view requires a lifecycle spike. Prefer a custom element that receives validated properties and emits typed custom events over mounting a new independent Dioxus tree per node.
- Separate editor-only chrome from serialized content. Tiptap explicitly treats node-view UI and output rendering as different concerns ([Tiptap node views](https://tiptap.dev/docs/editor/extensions/custom-extensions/node-views)).
- Node-view mutation must dispatch an engine transaction; it may not mutate document data privately.
- Provide safe unknown/read-only fallbacks with label, version, and raw-source access where permitted.

### 9.4 Accessibility

Target WCAG 2.2 AA. Specific requirements:

- preserve semantic headings, lists, blockquotes, tables, links, and code in both edit and read projections;
- give the editing surface a visible label and accessible name;
- do not use `role="application"`;
- prototype the accessibility tree with and without a single `role="textbox"` before freezing semantics. ARIA 1.2 supports `role="textbox"` plus `aria-multiline`, but the current ARIA 1.3 draft restricts textbox children in ways that conflict with a rich structured document; choose from real assistive-technology results, not assumption ([ARIA textbox guidance](https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Reference/Roles/textbox_role), [WAI-ARIA 1.3 draft](https://www.w3.org/TR/wai-aria-1.3/));
- each actual toolbar-like surface (text-selection bubble, media/table action row, and optional editor-actions strip) follows the APG toolbar pattern: one tab stop, roving focus, arrows, Home/End, labels, pressed/mixed state where applicable, and a documented editor-to-surface shortcut; do not expose a nonexistent general toolbar ([WAI-ARIA APG toolbar](https://www.w3.org/WAI/ARIA/apg/patterns/toolbar/));
- slash and mention popups follow combobox/listbox focus and announcement patterns; block, link, media, and table popdowns use the appropriate menu/dialog semantics rather than treating every popup as a toolbar;
- visible `:focus-visible` indicators, focus not obscured by sticky UI, and minimum 24 by 24 CSS-pixel targets or compliant spacing ([WCAG focus visible](https://www.w3.org/WAI/WCAG22/Understanding/focus-visible), [WCAG target size](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum));
- no keyboard trap. If Tab is used for indentation or table navigation, provide and document an escape route and contextual-surface shortcut; W3C uses a WYSIWYG editor as its explicit example ([WCAG no keyboard trap](https://www.w3.org/WAI/WCAG22/Understanding/no-keyboard-trap));
- save/error messages use a non-chatty status/live region;
- every pointer-only interaction, including the add affordance, block drag handle, and table edge controls, has keyboard/button parity;
- opening, executing, dismissing, or losing an anchor for a contextual surface has deterministic focus restoration; announcements identify the surface and the block/cell it affects without repeatedly reading document content;
- support zoom, forced colors, reduced motion, text spacing, and RTL content.

Automated checks are necessary but insufficient. Axe itself reports that it finds only a subset of WCAG issues and flags others for manual review ([axe-core](https://github.com/dequelabs/axe-core)).

### 9.5 Mobile and touch

- Do not introduce a sticky formatting toolbar on small screens. Keep the optional editor-actions strip limited to undo/redo/status, or omit it when space is constrained.
- Adapt contextual controls to touch: text-selection actions may use a compact floating row above the software keyboard or a bottom sheet triggered from the native selection flow; block, media, and table actions use tap targets and popdown/bottom-sheet menus rather than hover UI.
- Observe `VisualViewport` so the software keyboard does not cover the caret, popup, or focused block.
- Preserve native long-press selection, panning, and pinch zoom.
- Use Pointer Events for handles and pointer capture only on the handle; never set `touch-action: none` on the editor. Long-press or an explicit block-handle tap may start reordering, but must not steal native text selection.
- Avoid hover-only affordances: expose add/block actions on focus, tap, or a stable mobile entry point, and make table row/column handles operable by touch.
- Use contained horizontal scrolling for wide tables and reposition table controls against the visible table viewport.
- Test real iOS Safari and Android Chrome. Playwright mobile projects emulate browser/device parameters but cannot validate real IME, OS clipboard, autocorrect, virtual keyboard, or selection handles ([Playwright browsers and devices](https://playwright.dev/docs/browsers)).

## 10. Security plan

### 10.1 Immediate P0

Before the new editor ships, remove the current unsanitized Markdown-to-`dangerous_inner_html` path or put it behind a maintained, allowlist-based sanitizer. Prefer rendering `ComponentDocumentV2` through Dioxus text/element APIs. If HTML rendering remains temporarily:

- disable raw HTML parsing by default;
- sanitize after Markdown-to-HTML conversion and immediately before the sink;
- do not mutate sanitized HTML afterward;
- keep the sanitizer patched;
- add stored-XSS tests around note view and editor preview.

OWASP specifically recommends safe text/DOM sinks and DOMPurify when rich HTML sanitization is required; it also warns that post-sanitization mutation can undo safety ([OWASP XSS Prevention](https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html)).

### 10.2 Structural defenses

- Treat Markdown, typed payloads, clipboard formats, node-view messages, and server-loaded documents as untrusted.
- Allowlist node types, attributes, DOM tags, and CSS classes from the component schema.
- Never accept event-handler attributes, arbitrary style, `srcdoc`, forms, scripts, active embeds, clobbering `id`/`name`, or unreviewed `data-*` attributes.
- Canonicalize URLs with the platform URL parser and allow schemes per attribute. Default links: `https`, `http`; deliberately add `mailto`/`tel` only where desired. Permit the existing `semantic:` URI only in the mention codec, not as a general clickable URL. Reject `javascript`, `vbscript`, `file`, and unexpected `data`/`blob` schemes.
- Render text via text nodes. Node views receive validated data, never HTML strings.
- Enforce equivalent validation server-side/on load; client sanitation is not a trust boundary.
- Add Content Security Policy and Trusted Types in report-only mode, then enforce after violation count is zero. OWASP recommends Trusted Types as defense-in-depth for DOM sinks, not as a replacement for sanitization.
- Limit document/paste sizes and recursion to prevent memory/CPU denial of service.
- Telemetry never contains document text, pasted markup, URLs, mention queries, selection contents, or IME content.

Security corpus: scripts/event handlers, SVG/MathML namespaces, encoded/mixed-case protocols, CSS URLs, DOM clobbering, comments, malformed nesting, sanitizer mutation, deeply nested lists/tables, huge attributes, and internal-MIME schema spoofing.

## 11. Performance, save behavior, and collaboration readiness

### 11.1 Performance

Hot-path rules:

- no Rust bridge round trip for each keystroke;
- no full-document normalization, serialization, Dioxus rerender, or Markdown encode per transaction;
- lightweight UI-state events only when derived state changes;
- debounce snapshots/encoding, with immediate flush for explicit save/blur/submit;
- keep contextual-surface/editor-action signals separate from the document payload;
- instrument input dispatch, transaction apply, DOM update, layout/paint, snapshot, encode, and parent save separately.

Initial measured budgets, subject to calibration on representative hardware:

- no editor-created main-thread task of 50 ms or more during ordinary typing;
- no visible caret jump or scroll jump;
- page p75 INP at or below 200 ms, segmented by mobile/desktop, matching the web responsiveness threshold ([web.dev INP](https://web.dev/articles/inp));
- responsive editing at the agreed supported-document fixture (start with 100 KiB Markdown / 2,000 ordinary blocks and record results rather than claiming an unmeasured maximum);
- schema/build asset sizes tracked in CI with an explicit regression threshold after the spike establishes baseline.

Do not hard-virtualize initially. First use CSS containment and measure `content-visibility: auto`. Hard unmounting is particularly risky for native selection, find-in-page, cross-block copy, composition, scroll anchoring, and accessibility. If later required, never unmount composition/focus/selection endpoints and add a measurement cache plus dedicated browser/AT gates.

### 11.2 Dirty/save state

The editor reports state; the containing form remains responsible for persistence. Replace the implicit debounce with an explicit monotonic state model:

```text
clean -> dirty -> encoding -> ready_to_save -> saving -> saved
                  \-> encode_error          \-> save_error/conflict/offline
```

- Every local document revision is monotonic.
- A save acknowledgement applies only to the exact revision or an older one; it cannot mark newer edits saved.
- Blur and component drop flush pending snapshots synchronously as far as the platform permits.
- Navigation/unload is not the correctness mechanism. If durable drafts are a product requirement, enqueue snapshots/operations to IndexedDB before network save and test kill/reload recovery.
- When backend revision support is added, use optimistic preconditions such as `ETag`/`If-Match`; HTTP defines `If-Match` specifically for preventing lost updates ([RFC 9110, `If-Match`](https://httpwg.org/specs/rfc9110.html#field.if-match)).

### 11.3 Collaboration readiness, not implementation

- Preserve transaction origin, transaction ID, local revision, user-event/history group, and schema version in engine events.
- Stable semantic component IDs and deterministic normalization.
- Separate ephemeral selection/presence from persisted content.
- Do not invent a Rust OT layer now.
- If collaboration ships, let Yjs/ProseMirror or ProseMirror's central-authority step protocol own the live collaborative state; Markdown/typed JSON become import/export, index, backup, and snapshot formats. ProseMirror documents step-based rebasing and a collaboration plugin ([ProseMirror collaborative editing](https://prosemirror.net/docs/guide/#collab)); Yjs provides a ProseMirror binding ([Yjs ProseMirror binding](https://docs.yjs.dev/ecosystem/editor-bindings/prosemirror)).
- A plain JSON or Markdown snapshot is not a merge algorithm.

## 12. Proposed repository layout and APIs

The exact split may adjust during implementation, but responsibilities should land approximately here:

```text
crates/dxeditor/
  src/
    lib.rs                    public v1 facade + v2 exports
    api.rs                    Editor/MarkdownEditor/DocumentEditor props and events
    document/
      mod.rs
      v2.rs                   ComponentDocumentV2 and typed standard attrs
      validate.rs
      migrate.rs              v1 <-> v2 and sequential v2 migrations
      normalize.rs
    catalog/
      mod.rs
      spec.rs                 ComponentSpec and schema fingerprint
      standard.rs
    format/
      mod.rs                  DocumentFormat, options, diagnostics, capabilities
      markdown/
        mod.rs
        decode.rs
        encode.rs
        profile.rs
        opaque.rs
      plain_text.rs
      typed.rs
      legacy_v1.rs
    bridge/
      mod.rs                  lifecycle only
      protocol.rs             serde messages and revisions
    component/
      editor.rs               Dioxus host and session orchestration
      editor_actions.rs       optional undo/redo/status-only surface
      text_selection.rs
      block_controls.rs
      slash_menu.rs
      link_popover.rs
      media_controls.rs
      table_controls.rs
      status.rs
    legacy/                   old state/transaction/input path during rollout
  assets/
    dxeditor.css
  web/                        TypeScript/Tiptap package
  tests/
    fixtures/
      markdown/
      protocol/
      security/
      migration/
    codec.rs
    property.rs
    migration.rs
  e2e/                        Playwright config/specs

crates/ui_core/src/ui_catalog/notes.rs
  note form integration, revision/save state, v2 feature flag
```

Suggested public v2 shape:

```rust
pub struct EditorProps {
    pub value: EditorPayload,
    pub output_format: FormatId,
    pub external_revision: Revision,
    pub catalog: EditorCatalog,
    pub readonly: bool,
    pub editor_actions: EditorActionsMode,
    pub aria_label: String,
    pub on_change: EventHandler<EditorChange>,
    pub on_state_change: Option<EventHandler<EditorStatus>>,
}

pub enum EditorActionsMode {
    Hidden,
    History,
    HistoryAndStatus,
}

pub struct EditorChange {
    pub local_revision: Revision,
    pub base_external_revision: Revision,
    pub origin: ChangeOrigin,
    pub value: EditorPayload,
    pub diagnostics: Vec<EditorDiagnostic>,
}
```

Keep `MarkdownEditor(value: String, on_change: String)` as a compatibility wrapper over v2 until callers migrate.

## 13. Test strategy and release acceptance

### 13.1 Test layers

**Rust unit/fixture tests**

- document validation and normalization;
- every migration, forward and rollback-reader compatibility;
- Markdown construct fixtures and canonical expected output;
- CommonMark official examples plus focused GFM cases;
- opaque-source preservation and diagnostic ranges;
- URL and attribute policies;
- legacy v1 compatibility.

**Property/fuzz tests**

- `normalize(normalize(doc)) == normalize(doc)`;
- encode/decode structural round trip for all representable generated documents;
- canonical serialization idempotence;
- unique IDs and valid tree after arbitrary supported operations/snapshots;
- decoder and sanitizer are crash-free under arbitrary bytes and bounded malicious trees;
- no forbidden node/attribute/protocol survives validation.

Proptest automatically generates values and shrinks failures to minimal counterexamples, making it appropriate for document/codec invariants ([Proptest](https://proptest-rs.github.io/proptest/)). Persist every found seed/minimal fixture.

**TypeScript engine tests**

- schema and component-extension registration;
- commands, input rules, history grouping, selection mapping;
- command-surface eligibility/arbitration and anchor mapping for every selection/node state;
- table maps, rectangular selections, structural normalization, row/column commands, keyboard navigation, and rectangular paste;
- internal/HTML/plain clipboard transforms;
- bridge revisions, stale response rejection, lifecycle/destroy;
- node-view update/event behavior.

**Contract tests**

- the same JSON fixtures decode in Rust and TypeScript;
- schema fingerprints agree;
- every bridge message round-trips and unknown versions fail safely;
- every registered action maps to an engine command and capability;
- no formatting/content command is eligible for `editor_actions`, and each content command has at least one contextual or keyboard surface.

**Browser E2E**

Run Playwright projects for Chromium, Firefox, and WebKit; it officially supports all three and mobile device profiles ([Playwright browsers](https://playwright.dev/docs/browsers)). Cover real `contenteditable`, not jsdom:

- insert/delete/word delete/line break/paragraph;
- cross-block selection, replace, cut, copy, paste;
- marks across mixed nodes and collapsed stored marks;
- split/join/lift/list/task behavior;
- table creation, cell/row/column/table selection, navigation, row/column operations, contextual controls, resizing, structured paste, overflow scrolling, and Markdown round trip;
- undo/redo grouping and redo invalidation;
- mouse drag selection, keyboard selection, RTL;
- composition event fixtures where automation can generate them;
- controlled external updates, focus preservation, conflict, and stale snapshots;
- paste/drop fixtures and XSS cases;
- read-only mode and print/copy;
- quiet/default canvas, optional undo/redo/status editor actions, text-selection bubble, empty-block add affordance, slash/mention menus, block gutter/menu, link popover, media controls, and table-local controls;
- contextual-surface positioning, collision handling, exclusivity, dismissal, mapped selection preservation, and focus restoration while scrolling/editing;
- axe checks in each surfaced state.

**Manual release matrix**

- macOS: Safari + VoiceOver, Chrome, Firefox;
- Windows: Chrome/Edge + NVDA, Firefox + NVDA; JAWS where available;
- iOS: Safari + software keyboard, dictation, autocorrect, VoiceOver;
- Android: Chrome + Gboard and TalkBack;
- IMEs: Japanese, Korean, Simplified and Traditional Chinese;
- dead keys/diacritics, emoji ZWJ sequences, combining marks, RTL;
- OS clipboard from Office/Google Docs/Notion/GitHub;
- touch invocation and dismissal of every contextual surface, block reorder, and wide-table editing;
- zoom 200%/400%, forced colors, reduced motion.

Automation cannot establish accessibility or faithfully reproduce real mobile IME/clipboard/selection-handle behavior. Record the manual matrix and defects as a release artifact, not tribal knowledge.

**Performance and durability harnesses**

- scripted typing and selection in small/medium/large fixture documents;
- measure transaction, DOM, layout/paint, snapshot, encode, and save separately;
- paste large but allowed documents;
- load/replace/destroy repeated sessions and check leaks/listener cleanup;
- kill/reload or component unmount at each dirty/save state;
- delayed, duplicate, reordered, failed, and conflicting save acknowledgements.

### 13.2 Required acceptance criteria

1. Existing Markdown fixtures load with no silent content loss. Every unsupported construct is visible/preserved or load is blocked with a diagnostic.
2. The supported profile passes structural round-trip and canonical-idempotence properties.
3. Typing, replacement, selection, cut/copy/paste, marks, blocks, lists, tables, links, media, mentions, undo/redo, drag/drop, and shortcuts pass in Chromium, Firefox, and WebKit.
4. Cross-block operations and forward/backward selections preserve expected content and caret.
5. The real-device IME/mobile matrix has no composition loss, duplicated text, caret jumps, broken undo, or hidden focused content.
6. No Dioxus rerender/remount occurs during normal input or active composition.
7. Every custom component has schema, validation, editor rendering, read rendering, plain-text fallback, clipboard behavior, and a declared format policy.
8. Invalid external/component/clipboard content cannot enter the model silently.
9. Security corpus produces no executable DOM, unsafe URL, sanitizer bypass, or Trusted Types/CSP violation in supported flows.
10. Editor and every contextual surface, optional editor-actions strip, menu, dialog, table control, and reorder path are fully keyboard operable, labelled, focus-visible, and axe-clean; manual AT blockers are zero.
11. Measured performance meets the section 11 budgets on the agreed baseline devices/documents.
12. Local revisions/save acknowledgements are monotonic; blur/unmount flushes; external dirty updates never overwrite silently.
13. v1 Markdown editor callers continue to work through the compatibility wrapper.
14. Feature flag and kill switch can return a user to the legacy/raw editor without data migration or content loss.
15. The default desktop and mobile typing state has no persistent formatting/content toolbar. If configured, the minimal editor-actions strip contains only undo/redo/status/document-wide overflow; formatting and content actions appear only in the contextually correct surface listed in section 9.
16. Add-block, block menu/reorder, slash insertion, text marks, links, media, and tables are discoverable and complete by pointer, touch, and keyboard; contextual surfaces retain mapped selection, do not obscure the active target when avoidable, and restore focus predictably.
17. Tables meet the section 9.2 quality bar, including rectangular selection, row/column operations, keyboard navigation, structured paste, narrow-screen overflow, and lossless GFM behavior for the supported feature set.

## 14. Phased implementation

Each phase ends with a mergeable, testable gate. Do not start broad UX work before the engine/codec vertical slice passes real-browser and round-trip tests.

### Phase 0 - Safety fix, decision spike, and baselines

Deliverables:

1. Fix or disable the unsanitized `dangerous_inner_html` note rendering path and add stored-XSS regressions.
2. Write a short ADR recording Tiptap/ProseMirror selection and ownership boundaries.
3. Build a disposable vertical spike: Dioxus host -> vanilla Tiptap paragraph/heading/bold -> typed bridge -> Rust snapshot -> Markdown.
4. Verify Web, desktop webview, lifecycle cleanup, CSP compatibility, local asset loading, and bundle/build strategy.
5. Prototype one internal atomic component node view, one async slash-menu command, and the selection/anchor bridge needed for a floating contextual surface.
6. Prototype accessibility trees with/without textbox role in Chrome/Firefox/Safari plus one screen reader.
7. Record current editor bundle/load/typing behavior and a Markdown-loss fixture as baselines.

Exit gate: the spike proves single DOM ownership, stable IME in a real browser smoke test, bridge lifecycle, and internal component feasibility. If Tiptap adds material friction or size with no value, switch the island to direct ProseMirror; do not fall back to custom `contenteditable`.

### Phase 1 - Document v2, catalog, codecs, and migration

Deliverables:

1. `ComponentDocumentV2`, typed standard attributes, validation, normalization, IDs, and schema fingerprint.
2. Schema-bearing `ComponentSpec` and standard component pack.
3. `DocumentFormat`, diagnostics, capabilities, and explicit unknown policies.
4. Full Markdown profile decoder/encoder, source ranges, opaque nodes, and fidelity contract.
5. v1 document/plain-text/Markdown adapters and pure migration tests.
6. Typed component codec skeleton and JSON validation artifact, even if no production caller uses it yet.
7. CommonMark/GFM/security/property fixture suites.

Exit gate: all current notes decode without silent loss; `decode -> encode -> decode` and canonical idempotence pass; every standard component has a declared Markdown capability.

### Phase 2 - Production engine island and bridge

Deliverables:

1. Locked TypeScript package, reproducible bundle, CI checks.
2. Tiptap schema generated/verified from component specs.
3. Typed session bridge with mount/destroy, commands, snapshots, revisions, diagnostics, and stale-response guards.
4. ProseMirror history, standard keymap, core input rules, and selection-derived UI state.
5. Typed contextual-surface state, anchor mapping, eligibility/arbitration, dismissal, and focus-restoration protocol.
6. Dioxus `Editor` v2 host and compatibility `MarkdownEditor` wrapper.
7. Controlled external-update/conflict semantics without keyed remount.
8. Chromium/Firefox/WebKit vertical E2E and leak/lifecycle tests.

Exit gate: paragraph/heading/lists/code/marks/links work end-to-end; IME smoke tests do not rerender the editor; undo/redo and cross-block replacement are correct; snapshots persist Markdown.

### Phase 3 - Editing completeness and clipboard

Deliverables:

1. Full initial Markdown node/mark pack, including nested lists, tasks, tables, images, opaque nodes, and mentions.
2. Context-sensitive Enter/Backspace/Tab behavior.
3. Layered copy/cut/paste/drop and sanitization.
4. Slash/mention async request cancellation and stale guards.
5. Link and image attribute dialogs with URL policy.
6. Production table engine behavior: table maps, rectangular selection, row/column operations, navigation, normalization, spreadsheet/HTML/TSV paste, and GFM-safe capabilities.
7. Unicode, RTL, autocorrect/spellcheck, cross-block selection, and history grouping coverage.

Exit gate: all automated editing and clipboard acceptance cases pass across browser projects; real desktop IME matrix passes.

### Phase 4 - Product-quality internal UI and accessibility

Deliverables:

1. Quiet Notion-like canvas plus the complete `dxcomp` contextual surface set: text-selection bubble, empty-block add, block gutter/menu, slash menu, link popover, media controls, and table-local controls.
2. Optional configurable editor-actions strip restricted to undo/redo/status/document-wide overflow; verify that formatting and content commands cannot register there.
3. Keyboard-accessible block movement and insertion; drag is additive.
4. Contextual positioning, collision handling, surface arbitration, mapped selection retention, dismissal, and focus restoration.
5. Mobile/touch adaptations using floating action rows or bottom sheets as appropriate, VisualViewport handling, touch-safe handles, and wide-table overflow.
6. Semantic read-only component renderer shared with note display; eliminate unsafe HTML sink.
7. Accessibility semantics decision from AT results; labels, roving focus for toolbar-like contextual surfaces, combobox/listbox/menu patterns, and live status.
8. Axe CI plus documented manual AT/mobile matrix.

Exit gate: WCAG 2.2 AA automated gates and manual blocker criteria pass; the default canvas has no static formatting/content toolbar; every contextual surface passes pointer, keyboard, and touch scenarios; and the mobile editor remains usable with real keyboards/selection handles.

### Phase 5 - Integration, durability, performance, and canary

Deliverables:

1. Migrate `render_markdown_note_content_form` to revisioned v2 events and explicit save state.
2. Flush/error/conflict/retry behavior and, if required, durable local draft queue.
3. Performance instrumentation, fixtures, budgets, leak tests, and bundle threshold.
4. Content-free telemetry and error taxonomy.
5. Feature flag, staff cohort, deterministic canary cohorts, kill switch, and raw/legacy fallback.
6. Shadow conversion comparing canonical documents/Markdown without saving.

Exit gate: canary metrics meet thresholds, no lossy conversions, security/a11y blockers are zero, and rollback is rehearsed.

### Phase 6 - Default-on and legacy retirement

1. Ramp cohorts gradually by browser/device/document-size segments.
2. Preserve old reader/editor fallback until at least one stable release after default-on.
3. Stop writing `dxeditor.document.v1`, but keep its decoder/migration for the declared compatibility window.
4. Remove the custom DOM input/selection bridge and hard-coded editable renderer only after rollback no longer depends on them.
5. Retain corpus, browser, migration, and security tests permanently.

## 15. Rollout and observability

Feature-flag stages:

```text
local/developer -> staff -> 1% canary -> 10% -> 50% -> default-on -> legacy removal
```

Promotion gates should be numeric and segmented. Record only:

- editor/schema/format versions;
- browser engine, OS, device class, document-size bucket;
- load/decode/encode/snapshot/save durations;
- transaction/input categories, not inserted data;
- recoverable/fatal diagnostic codes;
- DOM/model divergence recovery count;
- composition abort/recovery count;
- paste accepted/rejected/reduced counts;
- undo failure, selection recovery, external conflict, save retry/error;
- INP/long-task/bundle metrics;
- CSP/Trusted Types violations;
- feature-flag cohort and fallback activation.

Never record content, raw Markdown, HTML, URLs, mention queries, clipboard data, selections, or composition text. Preserve the original stored Markdown during canary and do not perform destructive bulk rewrites. Shadow conversion compares hashes and structural diagnostics only.

Rollback requirements:

- server data remains Markdown during initial rollout;
- old/raw editor can open every saved value;
- new editor is disabled by flag without a deployment;
- any future typed-format writer is introduced only after the previous release can read or safely present it;
- migration backups and recovery are verified before irreversible format rollout.

## 16. Principal risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Rust/JS dual-runtime complexity | Lifecycle/protocol bugs | Single DOM owner, typed messages, schema fingerprints, session/revision checks, contract fixtures, explicit destroy |
| Framework schema leaks into storage | Future lock-in | Rust-owned v2 document and codecs; engine JSON is only a wire projection |
| Markdown normalization surprises | Noisy diffs/user mistrust | Unchanged-source fast path, documented canonical style, opaque raw nodes, preview/diff diagnostics |
| Unsupported Markdown loss | Data loss | Construct capability matrix, source spans, reject/preserve policy, no empty fallback, corpus/property gates |
| Tiptap Markdown beta | Conversion bugs | Do not use as authority; Rust codec only |
| Custom node-view/Dioxus lifecycle | Leaks/focus bugs | Prefer vanilla/custom-element node views; one lifecycle spike; never mount Dioxus per simple node |
| Accessibility ambiguity for rich `contenteditable` | Screen-reader regression | Preserve semantics, no application role, test textbox-role alternatives with real AT, manual release gate |
| Unsafe HTML/URLs | Stored XSS | P0 sink fix, structural rendering, sanitization, URL allowlist, CSP/Trusted Types, security corpus |
| Large-document lag | Poor input quality | No per-key bridge/serialization, measure first, containment, explicit budgets, virtualization only behind separate gate |
| External update overwrites local edits | Data loss | Monotonic revisions, clean/dirty/conflict state, no keyed remount, stale-response guards |
| Collaboration retrofit | Expensive later rewrite | Stable semantic IDs, deterministic schema, ProseMirror transactions, separated presence; avoid premature custom OT |
| Dependency/build-chain expansion | CI/supply-chain cost | Exact lockfile, Nix Node already available, reproducible local bundle, dependency scanning, small curated extension set |

## 17. Decisions to confirm during Phase 0

These do not block the plan, but the spike must resolve them before production implementation:

1. Exact GFM subset and whether footnotes are editable in the first default-on release or preserved opaquely.
2. Whether canonical Markdown ends with one newline.
3. Whether structured plain-text copy uses visible text or Markdown by default.
4. Tiptap versus direct ProseMirror after measuring the thin extension set's bundle and integration cost.
5. Checked-in generated JS bundle versus mandatory deterministic pre-build in all UI builds.
6. Custom-element versus vanilla node-view implementation for the first complex internal component.
7. Accessibility semantics selected from the Phase 0 AT prototype.
8. Whether durable offline drafts are part of this editor project or a later form-persistence project.
9. Supported maximum document size and baseline devices, set from actual product data if available.

## 18. Research basis and stopping point

Research prioritized official specifications, primary project documentation, and current first-party sources:

- editor architecture and browser behavior: [ProseMirror guide](https://prosemirror.net/docs/guide/), [ProseMirror reference](https://prosemirror.net/docs/ref/), [Tiptap core concepts](https://tiptap.dev/docs/editor/core-concepts/introduction), [Lexical concepts](https://lexical.dev/docs/concepts/editor-state), [MDN `beforeinput`](https://developer.mozilla.org/en-US/docs/Web/API/Element/beforeinput_event), and [Input Events Level 2](https://www.w3.org/TR/input-events-2/);
- Markdown: [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/), [pulldown-cmark 0.13](https://docs.rs/crate/pulldown-cmark/0.13.4), [markdown-rs](https://github.com/wooorm/markdown-rs), and the [ProseMirror Markdown example](https://prosemirror.net/examples/markdown/);
- Dioxus integration: [Dioxus escape hatches](https://dioxuslabs.com/learn/0.7/essentials/ui/escape/) and [web platform guide](https://dioxuslabs.com/learn/0.7/guides/platforms/web/);
- accessibility/mobile: [WCAG 2.2](https://www.w3.org/TR/WCAG22/), [WAI-ARIA APG](https://www.w3.org/WAI/ARIA/apg/), [Pointer Events Level 3](https://www.w3.org/TR/pointerevents3/), [Unicode text segmentation](https://unicode.org/reports/tr29/), and [Playwright browser support](https://playwright.dev/docs/browsers);
- security: [OWASP XSS Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html) and [Trusted Types](https://www.w3.org/TR/trusted-types/);
- testing/performance/collaboration: [Proptest](https://proptest-rs.github.io/proptest/), [axe-core](https://github.com/dequelabs/axe-core), [web.dev INP](https://web.dev/articles/inp), [ProseMirror collaboration](https://prosemirror.net/docs/guide/#collab), and [Yjs relative positions](https://docs.yjs.dev/api/relative-positions).

The evidence converged on the core decision: use a mature browser editing engine, keep a project-owned typed schema/codec boundary, make Markdown fidelity explicit, and validate in real browsers and assistive technologies. Further broad framework research is unlikely to change that architecture. The Phase 0 spike is the appropriate place to resolve repository-specific bundle, lifecycle, node-view, and accessibility details.
