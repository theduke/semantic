# Editor design and architecture

## Purpose

`dxeditor` is the structured rich-text editor used by the Semantic UI. It presents a quiet,
Notion-like editing canvas while accepting and emitting application formats such as Markdown.

The editor is not a Markdown text area with formatting layered on top. Its live state is a typed
component tree. Markdown is one import/export format for that tree. This distinction lets the
editor support richer, typed components in the future without replacing its editing engine or
public integration model.

The design has four primary goals:

1. Reliable browser editing, including selection, history, composition/IME, clipboard, tables,
   and cross-block operations.
2. No silent content loss when converting between external formats and the component document.
3. A project-owned document model and component catalog that do not make Tiptap JSON a storage
   contract.
4. A contextual, low-chrome interface built from project-owned UI rather than a conventional
   persistent formatting toolbar.

## Architectural overview

The editor is split across two runtimes with one owner for each concern:

```text
External value
Markdown | plain text | legacy v1 | typed v2
                    |
                    v
       Rust DocumentFormat registry
         decode / validate / encode
                    |
                    v
          ComponentDocumentV2
                    |
          versioned bridge protocol
                    |
                    v
       Tiptap / ProseMirror session
   document, selection, history, IME, DOM
                    |
          synchronous snapshots
                    |
                    v
       Rust validation and encoding
                    |
                    v
              on_change output
```

Rust and Dioxus own:

- the public component API;
- external-format decoding and encoding;
- the canonical component schema and catalog;
- validation and security policy;
- session identity and revision checks;
- controlled-value conflict handling;
- persistence-facing change events;
- the optional editor-wide action strip and status presentation; and
- semantic read-only rendering.

Tiptap and ProseMirror own, for the lifetime of a mounted editing session:

- the live document and editable DOM;
- selections and stored marks;
- browser input reconciliation and IME composition;
- history and transaction mapping;
- clipboard operations;
- table editing; and
- contextual surface state and positioning.

The ownership rule is strict: Dioxus mounts an empty editor host and does not reconcile its
descendants while ProseMirror is active. Rust must not rebuild the document on every keystroke,
and browser code must not become the authority for persistence formats or validation.

## Repository layout

The principal modules are:

```text
crates/dxeditor/
  src/
    component.rs          Dioxus public components and session orchestration
    bridge/mod.rs         Browser-engine installation and typed command/event transport
    protocol.rs           Versioned sessions, revisions, commands, events, and guards
    document_v2.rs        Framework-neutral component document model
    component_spec.rs     Component schema, capabilities, identity, and validation
    engine_manifest.rs    Rust-generated browser capabilities and schema fingerprints
    format.rs             External-format abstraction and registry
    markdown_v2.rs        Native Markdown <-> ComponentDocumentV2 codec
    migrate.rs            Legacy v1 compatibility migrations
    render_v2.rs          Safe semantic read-only rendering
    catalog.rs            Application editor catalog
  assets/
    component_document_v2.schema.json
    url_policy_cases.json
  web/
    src/index.ts          Tiptap session, adapters, commands, and contextual surfaces
    src/identity.ts       Semantic-ID validation and clipboard remapping
    src/extensions/       Editor-specific ProseMirror/Tiptap extensions
    dist/editor.iife.js   Deterministic checked-in browser bundle
    e2e/                  Real-browser tests
```

The older `document.rs`, `markdown.rs`, and related v1 APIs remain compatibility boundaries. New
editing behavior and new formats should target `ComponentDocumentV2` directly.

## Canonical document model

`ComponentDocumentV2` is the product-owned interchange model:

```rust
ComponentDocumentV2 {
    schema,
    version,
    root,
    metadata,
}

ComponentNode {
    kind,
    id,
    attrs,
    content,
    text,
    marks,
}
```

Its tree shape intentionally maps efficiently to ProseMirror, but its identifiers and semantics
belong to this project. Storage, UI integrations, and format codecs must not depend directly on
Tiptap extension names or Tiptap JSON.

The standard catalog includes document, paragraph, heading, blockquote, list and task nodes, code
blocks, thematic breaks, tables, images, mentions, opaque Markdown nodes, text, hard breaks, and
the supported text marks.

### Component specifications

Every component is described by a `ComponentSpec`. A specification declares:

- whether the component is a block, inline, atom, or mark;
- its allowed content shape;
- typed attributes, defaults, bounds, and URL roles;
- identity requirements;
- editing behavior such as selectable, draggable, defining, or isolating;
- safe semantic DOM information;
- clipboard and plain-text fallback behavior; and
- capabilities for Markdown, plain text, and the typed format.

`ComponentCatalog` validates registrations and produces a deterministic schema fingerprint. The
fingerprint is part of every live editor session. A browser session must not accept commands,
documents, or clipboard payloads for a different schema.

### Identity

Semantic block and atom IDs are stable within a document and unique wherever their component
specification requires identity.

IDs are assigned and repaired by a ProseMirror transaction extension. Serialization does not
invent new IDs. This ensures repeated snapshots of unchanged state are stable.

Identity behavior differs by operation:

- Moving a node preserves its ID.
- Splitting or creating a node assigns a fresh ID to the new semantic node.
- Copying and pasting remaps copied IDs.
- Internal clipboard payloads strip or remap identities, including IDs nested in opaque embedded
  documents.
- Rust validates required IDs and uniqueness before accepting a browser snapshot.

Markdown cannot persist semantic IDs. IDs created while editing Markdown are therefore session
identities; the future typed format can persist them.

## Formats and codecs

`DocumentFormat` is the boundary between external values and the canonical document. Each codec
publishes a `FormatDescriptor` and implements:

```text
decode(EditorPayload) -> DecodedDocument<ComponentDocumentV2>
encode(ComponentDocumentV2) -> EncodedPayload
```

The registry currently supports:

- native Markdown;
- plain text;
- the versioned typed component-document JSON format; and
- legacy v1 documents through migration adapters.

The interactive Markdown editor uses `MarkdownDocumentFormat` directly. It must not route through
the v1 model, because the v1 shape cannot faithfully express the complete supported Markdown
profile.

### Markdown contract

Markdown is the initial source and output format. The native v2 codec supports the selected
CommonMark/GFM profile, including nested lists, task lists, strike, images, hard breaks, tables,
alignment, header cells, nested quotes, links, and complete fenced-code info strings.

The codec follows these rules:

1. Supported constructs round-trip semantically.
2. Serialization is canonical rather than byte-identical after semantic edits.
3. Unsupported source is preserved as an inert opaque node where possible.
4. Raw HTML is never executed by the editor or read-only renderer.
5. A component that Markdown cannot represent is rejected or preserved according to its declared
   format capability; it is never silently converted to an empty paragraph.
6. Table features that Markdown cannot encode, such as spans and persisted widths, are disabled
   for Markdown sessions.

Codec failures and fidelity reductions are represented as structured diagnostics. Callers must
not replace invalid content with an empty document.

### Future typed format

The typed format serializes `ComponentDocumentV2` in a versioned envelope. It can preserve stable
node IDs, typed attributes, custom components, and metadata that Markdown cannot represent.

Adding a new external format requires a `DocumentFormat` implementation and capability entries;
it does not require changing the browser editing engine. Adding a new component requires both a
Rust `ComponentSpec` and a browser adapter declared in the engine manifest.

## Engine manifest

Rust produces an `EditorEngineManifest` for each session. It is the contract that configures the
browser engine and prevents UI capabilities from drifting away from the Rust schema.

The manifest includes:

- protocol and schema fingerprints;
- active external format;
- accessible editor label;
- registered component adapters and their format capabilities;
- command availability and disabled reasons; and
- feature flags such as media, task, table-header, alignment, width, and span support.

Browser controls must be gated by the manifest. For example, Markdown table sessions do not offer
merged cells or persistent widths. Hard-coded browser commands must not advertise capabilities the
active catalog or output format cannot preserve.

## Browser engine

The browser engine is built from pinned Tiptap 3 and ProseMirror packages. Tiptap supplies the
extension and command layer; ProseMirror supplies immutable editor state, mapped transactions,
selection, history, browser reconciliation, tables, and plugin infrastructure.

The engine bundle is generated from `web/src`, checked into `web/dist`, and verified
deterministically. The bridge installs it once per page. Each editor mount creates an independent
session through the installed global engine API; it does not evaluate the entire bundle for every
editor instance.

`v2ToPm` and `pmToV2` are wire adapters, not persistence codecs. They map the project document to
the active ProseMirror schema and back. Snapshot conversion validates semantic IDs before emitting
the v2 document.

## Protocol and lifecycle

The Rust/browser boundary uses versioned serialized commands and events from `protocol.rs`.
`ProtocolSession` carries:

- a unique session ID;
- the schema fingerprint;
- the current external revision; and
- the protocol version.

Local editor changes have monotonically increasing local revisions. `SessionRevisionGuard`
rejects stale events, mismatched sessions, schema changes, and regressing external revisions.

### Mount

1. Rust decodes the input payload through the selected format.
2. Rust validates and normalizes the v2 document against the active catalog and limits.
3. Rust creates a protocol session and engine manifest.
4. The bridge ensures the browser bundle is installed.
5. The browser mounts Tiptap into the provided empty host.

### Local change

1. A ProseMirror transaction updates the live document synchronously.
2. Identity repair runs in the transaction pipeline.
3. The engine immediately establishes local ownership and increments its revision.
4. The engine emits a v2 document snapshot.
5. Rust verifies the session and revision, validates the snapshot, encodes it to the requested
   output format, and invokes `on_change`.
6. Application persistence may debounce the resulting value above the editor boundary.

Correctness must never depend on a delayed browser snapshot. In particular, an external property
update must not overwrite a keystroke that is waiting behind a timer.

### External replacement

Controlled values include an optional external revision. An echoed value acknowledges the latest
local output. A genuinely newer external value is applied only through the revision guard.

Replacement carries a `HistoryPolicy`:

- `Reset` creates fresh editor/history state so undo cannot cross the external replacement.
- `Preserve` maps a non-history replacement when the caller deliberately wants to retain the
  current session history.

If external content arrives while local state is dirty, the editor enters a conflict state instead
of silently overwriting local work.

### Blur and destroy

Blur and destroy flush the latest document synchronously when necessary. Flushes are deduplicated
by revision so an unchanged document is not emitted repeatedly. Destroy removes editor instances,
listeners, observers, pending suggestion work, and contextual surfaces.

## Editing interface

The default interface is a quiet document canvas. There is no persistent formatting or
content-specific toolbar.

An optional `EditorActionsMode` may expose only editor-wide actions such as undo, redo, and status.
Formatting, insertion, links, media, and table commands must remain contextual.

The contextual surfaces are:

- a floating mark toolbar for non-collapsed text selections;
- an add affordance for an empty or focused block;
- a block gutter and popdown menu for duplicate, move, convert, and delete operations;
- a searchable slash menu for inserting supported components;
- a mapped link popover for URL/title editing and removal;
- contextual image controls for source, alternative text, and title;
- an asynchronous, cancellable mention listbox; and
- table-local controls for rows, columns, headers, alignment, deletion, and supported resizing.

Commands are capability checked. Selection is retained while a contextual control has focus, and
surfaces are repositioned on transactions, scroll, resize, and viewport changes. Composition
suppresses disruptive contextual UI updates.

Only the relevant surface should be active. Node/table selection takes priority over link and text
selection; modal or open-menu state takes priority over passive gutter affordances.

## Tables

Tables are structured nodes, not rendered Markdown strings. ProseMirror supplies cell selection,
normalization, navigation, row/column operations, and resizing.

Rust performs bounded, span-aware table geometry validation using the same essential invariants as
a table map:

- rows form a valid rectangular geometry after accounting for spans;
- cells do not overlap;
- row and column spans remain within bounds;
- node, depth, and table-size limits are enforced; and
- format capabilities prohibit unrepresentable geometry.

Spreadsheet-style TSV paste can grow a table to accommodate a rectangular input grid. Internal
clipboard payloads and semantic HTML are preferred when richer table structure is available.

For Markdown, alignment and header cells persist, while spans and persistent column widths are not
offered. Wide tables use contained horizontal scrolling rather than shrinking cells below a useful
size.

## Clipboard

Copy writes formats in descending richness:

1. a versioned internal component slice;
2. sanitized semantic HTML; and
3. useful plain text.

The internal envelope includes a version and clipboard schema fingerprint. Paste treats it as
untrusted input: size, depth, shape, node kinds, attributes, semantic IDs, and URLs are validated
before constructing a ProseMirror slice. A mismatched or invalid internal payload falls back to
HTML or plain text.

Copying remaps identities rather than cloning persistent semantic IDs. Cut uses the same validated
serialization and deletes content only after the clipboard path succeeds.

## Security

The editor never injects Markdown-generated HTML into `dangerous_inner_html`. Both editing and
read-only display render validated structural nodes. Raw Markdown HTML is displayed as inert
opaque source.

Security is enforced at multiple boundaries:

- external payloads are decoded and validated in Rust;
- browser snapshots are validated again before mutating Rust state or encoding output;
- internal clipboard data is bounded and schema checked;
- component DOM descriptors are restricted to safe semantic tags;
- hyperlinks and media use separate URL policies;
- unsafe protocols are rejected in Rust and TypeScript using a shared fixture corpus; and
- telemetry and structured errors must not include document or clipboard contents.

Client-side validation is not a server trust boundary. Persisted typed documents must also be
validated when received or loaded by backend code.

## Read-only rendering

`ReadOnlyDocumentV2` renders the canonical tree directly into semantic Dioxus elements. It covers
headings, lists, tasks, tables, blockquotes, code, links, marks, media, mentions, breaks, and opaque
content.

Unknown or invalid nodes use a safe per-node fallback rather than downgrading the entire document
through the legacy v1 format. Links and images pass the same role-specific URL policy used by the
editor.

## Accessibility and input

The editor surface has a caller-configurable accessible label. Contextual controls use toolbar,
menu, dialog, listbox, and active-descendant semantics appropriate to their behavior.

Key requirements are:

- formatting buttons expose pressed state;
- slash and mention results are keyboard navigable without moving focus out of the editor;
- every pointer operation has a keyboard path;
- table navigation provides a way to leave the table and editor;
- contextual UI does not interrupt IME composition;
- focus and selection are restored after dismissing a surface;
- controls remain visible under scroll, zoom, viewport, and software-keyboard changes; and
- the editor does not use `role="application"` or trap browser/assistive-technology shortcuts.

Automated axe coverage is a baseline, not proof of accessibility. Release validation also requires
real browser, screen-reader, mobile keyboard, and IME testing.

## Performance model

The hot typing path is contained in ProseMirror. It does not round-trip through Rust before the DOM
updates, rerender the Dioxus document tree, or encode Markdown on every browser input event.

The engine nevertheless publishes ownership and revisions synchronously so correctness does not
depend on debounce timing. Persistence debounce belongs in the application integration above
`dxeditor`.

Other performance rules are:

- install the browser bundle once per page;
- emit UI command state only when it changes;
- avoid full-tree normalization except at defined boundaries;
- clean up every listener and observer on destroy;
- bound document, clipboard, depth, and table sizes; and
- measure transaction, DOM, layout, snapshot, encoding, and save time separately.

Hard virtualization is not part of the current design. It should be introduced only after profiling
shows that representative large documents require it.

## Public integration

The primary Dioxus entry points are:

- `Editor` for arbitrary registered input and output formats;
- `MarkdownEditor` for string-based Markdown compatibility;
- `PlainTextEditor` for plain text;
- `DocumentEditor` for legacy v1 callers; and
- `DocumentView` and the internal v2 renderer for read-only display.

`Editor` accepts an `EditorPayload`, output format, catalog, read-only flag, optional editor actions,
external revision, accessible label, and change/focus/blur callbacks.

Applications should:

- treat `on_change` as the authoritative validated output;
- debounce persistence outside the editor if needed;
- flush application persistence on blur, submit, navigation, and unmount;
- pass monotonic external revisions when integrating remote state; and
- surface conflict and encoding errors instead of substituting empty content.

## Extension workflow

To add a component:

1. Define and register its Rust `ComponentSpec`, typed attributes, identity policy, format
   capabilities, and validation rules.
2. Add its browser adapter and, if necessary, a Tiptap extension or node view.
3. Expose adapter and command capabilities through `EditorEngineManifest`.
4. Implement safe semantic read-only rendering and a plain-text fallback.
5. Implement or explicitly reject/preserve it in every enabled external format.
6. Define clipboard behavior and identity remapping.
7. Add Rust codec/validation tests, TypeScript adapter tests, and real-browser editing tests.

Simple semantic nodes should use normal ProseMirror DOM serialization. Custom node views are for
atomic or genuinely interactive components; they must dispatch transactions rather than keeping
private document state.

## Testing strategy

The test stack is intentionally layered:

- Rust unit tests cover document normalization, schemas, validation, formats, migrations, URL
  policy, protocol guards, and read-only rendering.
- Rust seam tests cover native v2 Markdown behavior and validation before state acceptance.
- Vitest covers browser adapters, identity, clipboard validation, lifecycle behavior, command
  state, and conversion seams.
- Playwright exercises a real ProseMirror DOM, contextual controls, history, immediate ownership,
  stable identities, table paste, and automated accessibility checks.
- Deterministic bundle verification ensures `dist/editor.iife.js` matches the checked-in source and
  lockfile.

Repository validation uses the Nix development shell and the commands prescribed by `AGENTS.md`.

## Known limitations and planned hardening

The current architecture is in place, but these release-hardening items remain:

- a mounted Dioxus/WASM controlled-parent integration harness;
- broader property and fuzz testing for revision sequences, arbitrary documents, clipboard JSON,
  and Markdown;
- moving the remaining slash, mention, and contextual-controller state into mapped ProseMirror
  plugin state;
- replacing table-rebuild TSV insertion with a complete `TableMap`/cell-slice algorithm;
- contextual table edge handles and pointer block drag/reorder;
- generated TypeScript structural contracts for custom component adapters;
- a representative custom typed-component adapter fixture;
- a strict-CSP bridge that does not require evaluation during bundle installation;
- durable local drafts and offline retry at the application persistence layer;
- performance and leak benchmarks; and
- Firefox/WebKit CI plus manual mobile, IME, and assistive-technology matrices.

These are not reasons to weaken the ownership or format boundaries described above. They should be
implemented within the existing document, manifest, protocol, and engine abstractions.

