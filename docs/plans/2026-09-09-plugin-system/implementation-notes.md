# Implementation notes and operational handoff

These notes describe the implementation added on 2026-09-10. The [design](design.md)
and [phase plan](implementation-plan.md) remain the acceptance checklist; the
existence of an implementation is not evidence that every qualification gate
has passed.

## User flows

The app registers the generic URL Rust plugin and canonical `semantic.import`
package. Open **Import URL**, enter a URL, and find importers. Supported
candidates are ordered by configured/default priority, then stable plugin and
export identity. The chooser selects the highest-priority supported candidate;
users can choose another explicitly. A selected activation generation is checked
before submission. Failure never silently falls back to another importer.

**Fetch preview** opens the real interface stream, reads the first item's
metadata, and drops it. It saves no domain entities, files, blobs or job results.
**Start import** performs a fresh source operation and returns a generic job ID.
The Jobs view handles progress, cancel, interrupted work and history clearing.
A preview is not a saved source snapshot.

The corresponding CLI flow is:

```sh
semantic api import candidates https://example.com/document.pdf
semantic api import fetch https://example.com/document.pdf
semantic api import start https://example.com/document.pdf
semantic api jobs get JOB_ID
semantic api jobs cancel JOB_ID
semantic api plugin list
semantic api plugin configure activation.json
semantic api plugin uninstall PLUGIN_ID
```

`fetch` prints each content event followed by a terminal `end` summary. It does
not buffer a whole file. `start --plugin ID --export EXPORT --generation N`
selects a particular candidate generation. Place shared `--scope`/`--rpc-url`
options before the action subcommand. See the [CLI guide](../../../crates/cli/README.md)
and [provider descriptor guide](../../../crates/plugin/README.md).

The canonical `Application` interface exposes `list_candidates`, `fetch_source`,
`start_import_source`, and `start_import_fetched`. Content is always an owned
stream, including fetched-input transfers; it is never a unary byte aggregate.
Native callers use scope-bound app APIs. Client/server adapters use the shared
interface session. Callers transferring a stream must keep its producer/session
alive until terminal/cancellation, including while an import is queued.

## Storage and failure behavior

Entity identity derives from source namespace, canonical requested source URL
and source-local item key. Requested query order/values and fragments remain
significant; redirects do not change the source identity. Jobs, configuration,
implementation revision and content hash do not enter the ID.

The app writer validates and replaces ordinary entity/File records. Blob
locators are content-addressed independently of stable entity IDs. Complete
items publish individually; a later failure or cancellation leaves prior
successful items visible. An incomplete current file is not published.
Cancellation checks admission of each publication and awaits writes already
admitted. Job metadata and domain writes do not form one transaction.

Failed publication can leave finalized orphan content. Do not delete shared
content when clearing job history or handling a failed job. There is no staging
ledger, receipt, import revision, checkpoint, resume or automatic retry. Restart
interrupts unfinished jobs and requires new input for another operation.

## Runbook

| Symptom | Meaning and action |
| --- | --- |
| No supported candidate | Inspect candidate reasons and installed plugin readiness; generic URL rejects HTML and other non-file representations. |
| Incompatible activation | Regenerate exact exports/fingerprints from the scope's canonical packages and configure a compatible revision. |
| Provider unavailable | Check compiled provider feature, executable/endpoint and protocol negotiation; descriptors remain inspectable. |
| Plugin changed | Refresh candidates and explicitly start a new operation against the new generation. |
| Stopping/cancelling persists | A cooperative handler, store write or provider cleanup is still pending; no forced timeout is claimed. |
| Fetch/input disconnected | The owned stream failed; start a new operation with new input. No replay cache exists. |
| Import failed after progress | Previously published items may remain. Inspect ordinary entity/File data before starting another operation. |
| Jobs store unavailable | Use the generic jobs health/status semantics; do not assume a failed metadata write rolled back domain changes. |
| Interrupted after restart | Working requests/streams were memory-only and cannot resume. |
| History cleared | Only completed operational history is removed; imported domain data remains. |

Stdio drains stderr, reserves stdout for frames and waits for the direct child
to exit. WebSocket plugins use plain unauthenticated `ws://`; source fetching
still supports HTTP/HTTPS. No WSS, secret framework, authentication, process-tree
containment, reconnect/backoff, download quotas or force-stop policy is added.

## Verification boundaries

The import crate's focused tests cover stable identity component separation,
sequential framing, missing terminals, incorrect lengths/summaries, MIME policy,
empty/text HTTP streams, HTTP rejection/truncation, and fetched-input reuse
without another network request. App integration and interface/provider tests
are maintained with their owning crates. Run their current suites rather than
treating this document as a release certificate.

The first review correction pass adds executable and WebSocket provider
acceptance fixtures, all-provider ordinary DB/blob publication/replacement
parity, lifecycle cleanup barriers, pending-input cancellation, and upstream
generation invalidation through real application sessions. Reproducible local
transport measurements are recorded below. Browser/desktop interaction,
production-scale memory/throughput and wide-area simulated-latency qualification
remain environment-dependent follow-up measurements. Extended
Content-Disposition filename encodings are not yet decoded. No measured global
memory bound is claimed.

The app integration suite additionally verifies fetch without domain writes,
URL File replacement, failed replacement preserving old bytes, history clearing
preserving files, fetched-input reuse, cancellation during download, stale
bindings, idle fetch cancellation and durable uninstall of registered defaults.
The correction-pass aggregate app/data/import/jobs/plugin/RPC test run passed
161 tests. Three fixture-prerequisite tests were ignored by that ordinary run
and separately passed after explicitly building the real stdio executable.
Workspace all-target and native-only app checks passed. The initial pass also
checked the UI both natively and for wasm.

Trusted upstream job-group metadata is retained for native fetched content and
output transferred back over the same RPC session, including after completed
preview reads. Transfers occur between demands; an outstanding read cannot be
transferred. Producer-side metadata is recovered locally, never accepted from
peer-provided values. Cross-session relays remain independent caller inputs:
their originating runtime groups are not authenticated by another session.

## First review corrections

Native invocation tasks remain owned by the runtime after caller cancellation.
Generation replacement waits for cooperative invocation and stream cleanup;
dropping a stream consumer signals its producer even during a pending read.
Host cleanup is retained across abandoned lifecycle waits and caches actual
cleanup failures. A disconnected session can be explicitly restarted in one
operation after its transport/process cleanup succeeds.

Configuration schemas use ordinary package `Type` nodes and a reachable
definition table. Installations persist the declared schema; Rust registrations
must match it. The shared interface validator checks configuration before
desired state is persisted or the old generation is disturbed, for both native
and host providers. The generic URL plugin declares null configuration.
Constraints at every reference/alias layer are enforced for configuration,
arguments, results, stream items and terminal data.

Package changes compare resolved export bindings against the candidate catalog,
including transitive references. Update ownership survives request cancellation;
every affected activation is reconciled against the observed catalog following
registration success or failure. If persistence/catalog access itself fails,
the error is reported and unsafe stale generations remain stopped.

The acceptance fixture is deliberately built explicitly. Its prerequisite tests
are marked ignored with a build instruction, never silently skipped:

```sh
nix develop --command cargo build --quiet --message-format=short -p semantic_import --example url_provider_fixture
nix develop --command cargo test --quiet --message-format=short -p semantic_import --test provider_acceptance -- --include-ignored --nocapture
nix develop --command cargo test --quiet --message-format=short -p semantic_app stdio -- --include-ignored
```

For a 256 KiB loopback HTTP file, initial debug-build fetch/forward/import
measurements were approximately 78–79 ms through stdio and 116–118 ms through
WebSocket. These include JSON transport, local HTTP fetching and frame
validation, rather than isolating network RTT. A paused WebSocket reader
received a 4 KiB first body chunk after two demand envelopes and issued zero
additional demands during a 20 ms pause. The executable test asserts this
demand behavior; it is evidence of incremental frame delivery, not a process
RSS or total HTTP-stack memory limit. Each fetched-input reuse path asserts
exactly one HTTP request, and source-import paths verify identical bytes.

Use the repository devshell for verification:

Second review corrections: initial scope plugin opening now runs in an owned
task through lifecycle locking and runtime publication. A deterministic test
aborts the initiating caller with one plugin live and the next startup gated,
then verifies a single shared runtime and cancellation of both on shutdown.
Missing Rust registrations and changed configuration schemas are reported as
unavailable on reopen; disabling and uninstalling remain possible. Invalid
enabled configuration is rejected before persistence or live generation
replacement. Startup failures also publish health errors and cancel their
instance token, including job group allocation failures.

A nonzero stdio child exit reports provider failure through the session, while
successful child reaping counts as successful cleanup. The crash fixture exits
with status 17 during invocation; its regression verifies unavailable health,
explicit restart, another crash, and successful disable and shutdown. Actual
wait or cleanup failures remain errors. Server and native/browser clients share
interface endpoint derivation from the configured RPC path: `/rpc` is replaced
by `/interface/ws`, while other paths append `/interface/ws`. This preserves
the default endpoint and supports custom paths and trailing slashes.

Third-review lifecycle corrections serialize package upserts with initial scope
plugin publication, so an update always reconciles the published runtime. Scope
close and application shutdown now drive all generation stops and the generic
jobs coordinator concurrently; cooperative cleanup cannot prevent cancellation
from reaching other plugins or unrelated jobs. Generation cleanup also accepts
the coordinator-wide shutdown barrier when scope cancellation wins the race
with per-generation invalidation.

Deterministic regressions cover an update during gated initial plugin creation,
and both scope-close and application-shutdown cancellation of two plugins plus
an unrelated job while all job cleanup is gated. The focused app/plugin library
suite passed (45 tests, one existing ignored fixture test), along with app/plugin
checks and workspace formatting through the Nix base shell.
The broader app/jobs test suites also passed (63 tests, one ignored fixture;
PostgreSQL coverage remains conditional on `POSTGRES_URI`).

Focused second-review transport verification passed (2 server tests, 1 crash
regression, and 4 RPC transport tests):

```sh
nix develop .#base -c cargo test --quiet --message-format=short -p semantic_server interface_client
nix develop .#base -c cargo build --quiet --message-format=short -p semantic_rpc --example interface_fixture --features plugin-stdio
nix develop .#base -c cargo test --quiet --message-format=short -p semantic_plugin --features stdio stdio_crash_allows_explicit_restart_and_disable -- --ignored
nix develop .#base -c cargo test --quiet --message-format=short -p semantic_rpc --features plugin-stdio,client-http-native plugin::
nix develop .#base -c cargo check --quiet --message-format=short -p semantic_rpc -p semantic_rpc_core -p semantic_server -p semantic_plugin --features semantic_rpc/plugin-stdio,semantic_rpc/client-http-native,semantic_plugin/stdio
nix develop .#base -c cargo fmt --all
```

Source capability association is installation metadata: manifest and persisted
activation `source_bindings` map each Fetcher/Importer export name to its Source
export name. Native activations must preserve the registered manifest mapping.
For legacy plugins with exactly one Source, omitted bindings resolve to that
Source; plugins with multiple Sources must explicitly map every operation export.
Invalid references and wrong interface kinds are rejected before activation.
Discovery enumerates every mapped operation and retains the Source and operation
from the same immutable generation, including explicit selection of any importer.

Fourth-review lifecycle correction: each scope publishes a cancellation token
before plugin initialization. Closing a scope or the app first closes admission
and cancels this token, then drains startup and cleanup locks. Job shutdown runs
concurrently with that drain, so unrelated jobs receive cancellation even while
plugin creation is pending. Plugin generation tokens descend from the published
token; cooperative Rust `create` implementations must observe it. Host stdio
negotiation and WebSocket connection/negotiation also observe it and settle owned
transport cleanup before returning. Scope removal retains its identity/close
guard until both plugin and jobs cleanup finish.

Fifth-review corrections: Application interface invocation cancellation now
interrupts discovery and pre-return fetch work. Dropped plugin binding callers
signal cooperative cancellation while runtime-owned invocations retain cleanup
ownership; admitted jobs remain owned by their coordinator. Regressions exercise
describe, probe, and pending fetch under both dropped RPC calls and disconnects.
Candidate API records and their schema now expose `source_export`, allowing UI
preview selection to match the selected importer's mapped Source, including
crossed multi-Source mappings and generation validation.

Host providers expose session termination independently of invocation results.
A generation observer serializes against replacement, marks an idle disconnected
provider unavailable, closes admission, and drains its job group. Observers use
generation identity checks so obsolete termination cannot invalidate replacements.
Coverage includes idle WebSocket disconnect cancelling a grouped job, idle stdio
child exit followed by restart, and delayed obsolete-provider termination.

Sixth-review lifecycle correction: invocation/session failures, provider
termination, and explicit stop now join one generation-owned cleanup task.
Generation cancellation always starts cleanup, including invalidation/draining of
the jobs group, and provider shutdown runs exactly once even with concurrent or
abandoned waiters. Health updates retain generation identity checks and the first
invocation failure. A deterministic regression covers an active failed call,
running and queued grouped jobs, a blocked provider cleanup, replacement waiting
for cleanup, and stale termination leaving the replacement healthy. Nix plugin
checks, all nine plugin tests (including stdio fixtures), and all 41 app library
tests passed; formatting and whitespace validation passed.

Fifth-review validation through Nix: 41 app library tests, 11 import integration
tests (one separate fixture test ignored), 8 plugin tests including both stdio
fixtures, 5 RPC transport tests, and 2 UI mapping tests passed. Checks passed for
app, plugin, RPC, UI, server, and CLI crates; workspace formatting and
`git diff --check` passed. RPC tests now declare their Tokio time feature directly
so transport tests also compile independently of workspace feature unification.

Regression coverage includes startup gated solely on cancellation for both scope
close and app shutdown, unrelated-job cancellation, WebSocket Hello negotiation
cancellation, and crossed mappings for two Sources, Fetchers, and Importers.
Validation through `nix develop .#base`: app library tests (41 passed), plugin
tests with stdio/WebSocket (5 passed, 1 ignored fixture), RPC plugin transport
tests (5 passed), import integration suite (9 passed, 1 ignored fixture), affected
crate checks, and workspace formatting. Explicit fixture-dependent tests retain
their documented build prerequisites.

Seventh-review fixes: the server test database now retains normalized canonical
packages in a catalog, allowing real plugin/import scope initialization in server
tests. Job registration handles now have an individual identity validated against
the coordinator's registry snapshot. Existing handles survive registry extension;
later registrations and registrations from divergent registry clones are rejected
by coordinators that do not contain them. Regression tests cover both cases.
Validation through `nix develop .#base`: all 12 server library tests and all 13 jobs
runtime tests passed; server/jobs checks, workspace formatting, and
`git diff --check` passed.

```sh
nix develop --command cargo check --quiet --message-format=short
nix develop --command cargo test --quiet --message-format=short
nix develop --command cargo fmt
```
