# Query workbench — `/query`

## Current implementation

`QueryPage` owns a SQL text signal and an optional result signal. Run starts an untracked task; results render object rows as comma-joined strings in one-cell rows, while other values use Rust debug output (`crates/ui/src/views/query.rs:9-111`).

## Findings

| Priority | Finding | Evidence / impact |
|---|---|---|
| P0 | Request race | Repeated Run starts concurrent tasks; an older response can overwrite a newer query (`query.rs:23-30`). |
| P0 | No safety contract | No read-only default, destructive-query detection/confirmation, or result limit policy. |
| P1 | Missing async states | No running, cancel, empty, retry, duration, or stale-result indicator. Run never disables/changes. |
| P1 | Results are not a data table | Rows are flattened into one string cell with no schema/headers (`:63-81`). Empty rows create a blank table. |
| P1 | Prototype styling/accessibility | `.semantic-query` has no CSS; textarea lacks a label/editor semantics; table lacks caption/headers. |
| P1 | Missing workbench tools | No shortcut, history, saved queries, format/copy/export, explain, pagination, schema reference, or result metadata. |
| P1 | Large-result cost | Full values are cloned/summarized into large strings/DOM (`:54-79`). |

## Target experience

- `PageHeader`: Query workbench, active scope, History/Saved, and safety mode.
- `QueryEditor`: labelled monospace editor, `Ctrl/Cmd+Enter` Run, Escape/Cancel, format when parser support exists, and inline parse/safety feedback.
- Default read-only SELECT mode with a safe result limit. Mutation mode is an explicit opt-in with statement preview and target-naming confirmation; do not infer authorization for backend operations.
- Result summary: status, duration, row count/affected count, truncation, format, Copy/Download CSV/JSON.
- Schema-aware `DataTable`: union/stable columns, typed cell rendering, horizontal/responsive behavior, row-to-entity actions when ID/collection are known.
- Bounded local history first; saved/shared queries require a portable storage decision.

States: idle guidance, parsing, running, cancelling, success rows, success empty, non-row success, truncated, error with retained query/retry, connection loss.

## Component boundaries and reuse

Consume `PageHeader`, `AsyncState`, `InlineNotice/Toast`, `ResponsiveDataTable`, `CopyableCode`, and `ExportMenu`. Evolve `QueryEditor { value, language, read_only_policy, running, diagnostics, on_change, on_run, on_cancel }`; Browse advanced mode and Player filters reuse it. Evolve `QueryResultView { result: Rc<QueryResult>, presentation, on_entity_open }`; Browse may reuse typed cell/table primitives, not necessarily the whole workbench.

## Dioxus state architecture

- Local `Signal<String>` owns editor draft; a separate submitted immutable `QueryRequest` identifies the run.
- Use a typed async state signal/resource: `Idle | Pending { generation } | Ready(Rc<QueryResult>) | Error`. A cancellable spawned task or generation token guarantees latest submission wins.
- Use `Memo` for expensive column derivation and display-row projections from `Rc<QueryResult>`; cheap labels stay inline.
- An `Effect` is appropriate for editor focus, document title, and bounded history persistence after successful submission. It must not copy result into another signal.
- Avoid a coroutine unless it is a non-blocking reducer; direct tasks with cancellation are simpler. Do not serialize UI input behind a network request.
- Paginate/stream backend results where supported; cap DOM and export from data, not rendered cells.

## Visual direction

Present a purposeful split workbench: editor surface with subtle mono treatment above/beside a dense result surface. Status/duration/row count form a quiet metadata bar. Syntax/error emphasis must not depend on saturated backgrounds; destructive mode gets an unmistakable but restrained warning banner.

## Accessibility/responsive/performance

- Editor has a real label/help/shortcut; running state uses `aria-busy`; errors are announced once and associated with relevant query range where possible.
- Result table has caption, scoped headers, keyboard-accessible scroll, non-color type indicators, and readable empty response.
- On narrow screens editor/result stack; result actions stay visible; no full-page horizontal overflow.

## Missing functionality

Cancel, history/saved queries, parser diagnostics/formatting, safe mutation mode, typed results, explain, export, pagination/streaming, schema sidebar, and shareable query IDs.

## Acceptance criteria

- Older requests never overwrite newer results; Cancel leaves a clear state and preserves query text.
- Read-only/mutation policy is explicit and tested.
- Empty, error, non-row, large/truncated, and normal results are meaningfully rendered.
- Keyboard execution and full workbench operation pass at narrow width and with screen reader.
- The same QueryEditor contract is used in Query, Browse advanced mode, and Player advanced SQL.
