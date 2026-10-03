# Graph views review 5 disposition

The review confirms that the implementation has converged and proposes optional
cleanup. Its uncommitted-work premise is stale: review 4 was committed as
`b427f589`. Original review and plan inputs remain unchanged.

| Item | Disposition |
| --- | --- |
| L1: remaining explorer targets | Applied the optional cleanup. Explorer identity, object updates, ancestor requests, root storage, and expansion requests use entity IDs directly. `ExpansionRequest::entity_id` replaces `target`. Internal node IDs use the `entity:{id}` namespace; default `EntityTarget` metadata remains in node payloads for navigation and EntityCard. These are transient graph IDs, with no database ID or schema changes. |
| L2: redundant picker prop | Already resolved in review 4. The picker does not specify `collection`; the remaining `collection: None` initializes a separate BrowsePage navigation route and is retained. The shared autocomplete search-field override remains its documented follow-up. |
| L3: commit working tree | Already resolved before this review by `b427f589`, which committed the coherent review 4 implementation, documentation and fixtures. This optional review 5 cleanup was subsequently committed after the user's separate explicit request. No push was performed. |

## Verification

All Rust checks and tests used the Nix devshell with
`--quiet --message-format=short`; final `cargo fmt` and `git diff --check` passed.

- Full `semantic_ui_core --no-default-features`: 132 unit and 22 integration
  tests passed. The 27 focused graph tests overlap that coverage and are not
  counted twice. Regressions cover punctuation in identities, ancestor
  deduplication/collapse, and missing objects receiving data without changing
  node/edge identity, layout, or loading state.
- Native UI graph route suite: 3 tests passed. Together with the core suite,
  this accounts for 157 Rust tests.
- Supported UI wasm check and final default-feature workspace check passed.
  Both workers cross-reviewed explorer/state and navigation/Card consumers.
- Fresh browser verification passed all 39 recorded checks at 1440×1000 and
  390×844 with zero page/console errors. Relations and mobile screenshots were
  visually inspected; simplified node IDs were exercised through expansion,
  ancestors, EntityCard, focus/navigation, drag/reset and existing mobile flows.

T3 preview status/open reported no connected automation host, so verification
used its prescribed headless Playwright fallback and the retained
[browser script](browser.cjs). The isolated database was
`/tmp/semantic-graph-browser-review5-20261003-476035`; artifacts are retained at
`target/graph-views-browser/review5/`. Both owned server/preview processes were
stopped and ports 8080/8888 released. Native input behavior was unchanged, so
its earlier native smoke was not repeated.

## Limits

Internal node IDs are transient; existing persisted entity IDs are unchanged.
Native input and position ownership are unchanged, including the accepted FIFO
stall and session-remount limits. No core schema, historical migration, or
database behavior changes are made.

The missing browser endpoint remains an RPC response fixture because live
database integrity rejects dangling references; incoming/outgoing entity
objects and source loads use live RPC. The shared autocomplete alias follow-up
is unchanged and outside this optional helper cleanup.
