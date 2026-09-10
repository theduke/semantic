# Jobs implementation handoff

The initial implementation covers the generic native runner, portable metadata/package, application DB adapter and scope integration, CLI controls, SDK type roots, and polling Jobs view. Plugin/importer adapters (J5) and their end-to-end release qualification remain in the subsequent plugin/import implementation.

## Public entry points

- `semantic_jobs::{JobsBuilder, JobHandler, RegisteredJob, JobsRegistry}` register native typed handlers. Inputs/outputs require only `Send + 'static`.
- `SemanticAppBuilder::with_jobs(registry, config)` configures the registry and limits; `AppRequestContext::jobs` / `SemanticApp::jobs` resolve the scope coordinator.
- `ScopeJobs` provides submit, list/get, cancel, groups, clear-completed, health, and shutdown. Tickets contain ephemeral typed results; no serialized generic submission command exists.
- `SemanticApp::shutdown` and `ScopeManager::close_scope_with_jobs` await cleanup. Attached jobs scopes pin their DB until explicit async close, including while idle. This conservative initial policy avoids overlapping owners; idle coordinator eviction is a future optimization.
- The server has `serve_with_shutdown`, used by the server/CLI binaries for Ctrl-C. Cooperatively cancelling handlers can delay shutdown indefinitely by design.
- CLI: `semantic api jobs list|get|cancel|clear-completed|kinds`. Jobs UI: `/jobs`.
- Settings: `AppConfig.jobs`, `SEMANTIC_JOBS_CONCURRENCY` (default 4), `SEMANTIC_JOBS_HISTORY_THRESHOLD` (default 1000). Values are positive nonzero integers. Invalid environment values fall back to defaults, following existing app configuration conventions.

## Integration findings

The app's original `SemanticDb::query` rejects AST queries, and its SQL parser cannot represent a native datetime cursor literal. The additive `query_data(QueryInput)` adapter method executes the existing public AST without converting typed values to SQL text. Its default reports unsupported; the standard `Db` adapter implements it. Custom app adapters enabling jobs must implement this method.

Existing catalog code normalizes `MigrationCollectionKind::Polymorphic` to `CollectionKind::Schema`. The jobs migration remains polymorphic, and the store accepts the current normalized representation with strict registered schema integrity. No core collection semantics were changed. A preexisting collection without the jobs package is rejected.

Progress snapshots coalesce once per second; status transitions flush immediately. Clear-completed captures terminal IDs at the command's coordinator ordering point and deletes chunks between control-loop turns. Periodic/terminal-triggered retention removes oldest terminal rows by creation time and ID.

## Operational runbook

- **Interrupted on reopen:** runtime inputs are gone. Supply fresh domain inputs to start new work. Existing domain writes may already exist; jobs never replays or rolls them back.
- **Storage unavailable:** health reports degradation, dispatch/admission stops, active cancellation is signalled, and metadata/results stay in memory. Maintenance reconciles exact snapshots and retries only settled metadata operations. Never automatically resubmit a lost submission response.
- **Unknown unsettled write:** an exact matching read can confirm it. Otherwise the coordinator remains degraded; absence alone cannot justify racing another write.
- **Cancelling indefinitely:** the trusted handler must observe cancellation and finish its own child tasks/resources. There is no forced cancellation timeout.
- **Unknown historical kind:** list/view uses the stored kind key when no registered title exists.
- **History missing:** retention or clear-completed removed metadata; domain entities/files/blobs are unaffected.
- **Collection/package incompatible:** initialization fails. Do not replace or delete existing history to bypass the error.
- **Ownership:** designate one application process per writable scope. Aliases for the same DB must not be configured as separate scopes/coordinators. No distributed fencing/lease is claimed.

## Review corrections

Delayed provider opens now recheck the captured scope-entry identity before publishing and preserve a database already installed by another resolver, including a coordinator's pinned database. Deterministic gated-provider tests cover explicit and lazy opens racing coordinator initialization. Jobs-store initialization compares the normalized stored package with the supported V1 definition and rejects unknown or changed applied migrations before any upsert. Redb regressions assert catalog snapshots and job history remain unchanged when newer versions, incompatible definitions, or extra applied migrations are encountered. Future jobs schema upgrades must explicitly extend this compatibility policy.

Scope coordinator initialization now uses one captured owner/scope key throughout database resolution. Application shutdown publishes a persistent admission gate before awaiting any initialization; an already initializing coordinator is included in the shutdown snapshot and is never returned as a new usable admission point. Scope close uses an entry-specific lock and identity check, allowing unrelated scopes to initialize while a handler drains and preventing concurrent close calls from removing a replacement scope. Initial coordinator creation remains serialized by the existing application lifecycle mutex.

Any metadata degradation now completes pending shutdown requests with a storage error, including terminal-write failures occurring after shutdown started. The coordinator retains its live state and recovery ownership; callers can retry shutdown after recovery.

Successful group invalidation always returns its drain handle once in-memory invalidation occurs. `GroupCancellation::persistence_error()` exposes an initial persistence failure without losing the handle; `wait()` continues to track durable terminal snapshots through recovery. Invalid/foreign group handles still return an ordinary error, and invalidated groups leave no permanent coordinator tombstones.

Regression coverage includes captured-key shadowing, waiting and in-flight initialization versus shutdown, unrelated-scope progress during close, replacement protection, late shutdown storage failure, recoverable group drain, unsettled writes with absent rows/failed reads, failed Running snapshot recovery without duplicate execution, deletion failure, and oldest-first retention ties. A conditional PostgreSQL jobs adapter test runs when `POSTGRES_URI` is set, in its own generated schema; otherwise that test explicitly skips database execution.

## Validation

Focused deterministic runtime tests cover FIFO/concurrency, native channel inputs, coalesced progress, queued and noncooperative cancellation, group invalidation and stale/foreign handles, panic isolation, dropped tickets, lost acknowledgements, outage recovery without domain replay, restart interruption, and active-safe clearing.

Real Redb integration tests exercise migration replay, collection separation, wide counters, stable creation/ID cursors, reopen reconciliation, coordinator reuse, scope close, and RPC list. `crates/app/examples/jobs.rs` demonstrates native channel inputs and typed output against Redb.

PostgreSQL execution requires `POSTGRES_URI`; it was unset during implementation. Browser interaction and representative production performance measurements are not claimed by compile/unit-test validation. Independent review should verify the remaining acceptance matrix and deployment lifecycle before importer integration.

Commands executed successfully through the Nix devshell:

- `cargo check --quiet --message-format=short` (workspace).
- `cargo test --quiet --message-format=short -p semantic_jobs -p semantic_data -p semantic_app -p semantic_cli -p semantic_server -p semantic_sdk_export -p semantic_ui --lib --tests` (233 tests).
- `cargo run --quiet -p semantic_app --example jobs` (result 42, cancelled second job, two history rows cleared).
- `cargo fmt` and `git diff --check`.
