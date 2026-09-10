# Plugin/importer design questions

Date: 2026-09-09. These are product/architecture choices not settled by repository evidence. The [design](design.md) uses the recommended defaults below and the [implementation plan](implementation-plan.md) identifies the dependency gates. Answers can be recorded here with a date and decision owner. Do not block independent phases on lower-priority questions.

The user's Wasmer correction is already resolved: generic external CLI launch only; no runtime/WEBC/registry/package-file integration. It is not a question to reopen during these phases.

## Priority 0 — Resolve or explicitly adopt defaults before the dependent phase

### Q1. Does “Rust plugin” mean programmatic trait registration, or independently loadable native libraries?

Recommended default: ordinary Rust plugin trait implementations registered programmatically with the Semantic runtime by embedding/custom code. Registration may use a value or factory as required by lifecycle and scope policy; Cargo features are merely an application packaging choice. External deployment uses stdio/WS. This preserves normal Rust type/allocator safety and a straightforward crate API.

Consequence of requiring native dynamic loading: define a stable ABI boundary, memory ownership, compiler/version policy, panic handling and unload restrictions in a separate ADR; Rust trait objects cannot simply cross that boundary. It adds an execution provider and delays P1 if mandatory.

Blocks: P0/P1 API contract. Recorded answer (2026-09-09, project owner): Rust plugins are implementations of a Rust plugin trait and are registered with the Semantic runtime in custom code. Native dynamic-library loading is out of scope.

### Q2. Who installs/enables plugins, and what is the isolation/tenancy unit?

Original recommended default: operator-approved host installations; independent scope activation/config/grants; one runtime instance per scope/activation/config generation. Ordinary users invoke authorized importers rather than installing plugins or schemas. The project owner selected a temporary open-access policy because no permission system exists yet.

Consequence: clear scope separation, at the cost of more processes when many scopes activate the same plugin. Per-user installations, user-authenticated remote connections, or cross-scope shared pools require explicit ownership/credential/resource isolation and UI/API policy. Do not infer these from the current app-wide command registry.

Blocks: P0/P1 activation/persistence model. Recorded answer (2026-09-09, project owner): activation and runtime isolation are per scope, with a separate runtime instance for each scope/configuration generation. Until a permission system exists, every user may install, enable, configure, and invoke plugins. `TODO.md` records that these operations must later receive explicit permissions.

### Q3. Must the first release run jobs safely from several host processes sharing one scope/database?

Recommended default: one configured coordinator process per scope; concurrent in-process jobs are allowed. The initial job commit/start/cancel state is serialized by that coordinator.

Consequence: suitable for an embedded/single-host deployment but not safe active-active scheduling. If multiple hosts are required, add atomic claim/renew/fence semantics and enforce fences in the commit transaction before P2. Current unconditional batch upserts do not provide this, and an expiring row or process-local mutex does not solve it.

Blocks: P2 durability contract; P1/P3 can proceed independently. Recorded answer (2026-09-09, project owner): exactly one job coordinator owns each scope; concurrent jobs within that coordinator are allowed. Job execution must use the generic `semantic_jobs` crate rather than importer-specific infrastructure; its design and implementation plan are available under `docs/plans/2026-09-10-jobs-system/`.

### Q4. Should imports create immutable revisions, or update/merge existing user-visible entities immediately?

Original recommended default: append-only imported revisions. The project owner requires importers to use normal entity upsert/replace semantics until a generic entity-versioning system exists. Importers must never implement custom revision entities or manual version chains.

Consequence: repeated imports with the same source identity replace the existing entity through normal database upsert behavior. A future importer-level default and per-import override may request versioning only by delegating to a future generic entity-versioning capability; until then, versioned mode is unavailable. Conflict behavior remains a general database/entity concern rather than import-specific version logic.

Blocks: P2 output mapping/storage and sample importer UX; not P1/P3. Recorded answer (2026-09-10, project owner): use normal upsert/replace for an existing source identity. Do not implement import-specific versioning. Later expose importer and per-import versioning settings only as delegates to generic entity versioning.

### Q5. How should large import output be transferred and committed?

Original recommended default: one validated page plus its receipt/checkpoint commits atomically. The project owner rejected pagination as the import-processing abstraction: large imports must use async streams supported by the interface system. Pagination remains relevant to user browsing only.

Consequence: the revised design requires stream-aware native bindings and shared RPC transport because the schema already models streams while generic RPC is currently unary. Fetch streams do not persist data. Imports publish each complete entity or File atomically, may leave visible partial results, and persist no payload receipts or checkpoints in job state. Pagination is not exposed as an importer-processing abstraction.

Blocks: P2 interface, transport, job state, and commit semantics. Recorded answer (2026-09-10, project owner): use interface-system async streams for large imports; do not model importer execution as pagination. Have the planner resolve RPC support and detailed stream/commit semantics.

## Priority 1 — Resolve before the relevant provider/data milestone

### Q6. Which OS platforms and database/object-store backends are first-release requirements?

Original recommended default: qualify specific native platform/backend combinations and implement platform-specific process-tree containment. The project owner instead requires portable host execution through `tokio::process` with asynchronous piped stdin/stdout, without OS-specific lifecycle logic in this design.

Consequence: stdio host plugins are supported wherever the project and dependencies compile and the platform can spawn child processes. Stronger OS-specific sandboxing, Job Objects, process groups, or descendant-process containment are not first-release guarantees. Imported data remains backend-agnostic ordinary entities and files/blobs.

Blocks: P0 release matrix, P4 portable process implementation, P6 blob-store support, P7 qualification. Recorded answer (2026-09-10, project owner): imported items are always ordinary entities in the existing database and ordinary files/blobs in the existing blob store; no importer-specific or new storage kind is allowed. The generic `semantic_jobs` crate persists job state in a dedicated jobs collection. Host plugins use `tokio::process` and asynchronous piped stdin/stdout, with no OS-specific logic required by this design.

### Q7. What trust policy and secret provider are available for executable/remote plugins?

Original recommended default: trusted executables with authenticated WSS endpoints and secret references resolved through an app-level provider. The project owner selected a simpler initial trust model with no authentication or sandboxing.

Consequence: all configured local and remote plugins are trusted deployment components. Remote plugins initially use unauthenticated, unencrypted WebSockets (`ws://` via HTTP upgrade); WSS/TLS, authentication, transport identity, filesystem/network restrictions, secret-provider integration, and execution sandboxing are explicitly deferred future work. Documentation must not present process separation as a security boundary.

Blocks: P4/P5 production activation. Recorded answer (2026-09-10, project owner): no plugin authentication, WSS/TLS, or sandboxing initially; remote plugins use `ws://` over HTTP upgrade. Track the deferred security work in `TODO.md`.

### Q8. Which first real importer and input/output types should validate the design?

Original recommended default: a deterministic URL metadata importer first, with file/blob output later. The project owner requires a built-in generic URL importer implemented as a Rust plugin trait implementation and registered programmatically with the Semantic runtime. It fetches URL content, checks that it has a reasonable regular-file content type, stores the bytes in the existing blob store, and creates the corresponding ordinary File entity. Its importer capability is exposed through the same package interfaces as other providers.

Consequence: feeds, browser-dependent scraping, authenticated SaaS, directory imports, media pipelines and queue consumers stress different requirements. A source with external mutations or interactive authentication is materially beyond the initial read-only source contract. Required first-release formats may move file input or a narrow host capability earlier, but should use the same interfaces.

Blocks: P2 sample/product acceptance and file/blob support. Recorded answer (2026-09-10, project owner): first implement a built-in generic URL-to-File importer as a registered Rust plugin trait implementation. The revised plan defines content-type validation, redirects, streamed transfer, error behavior, and deterministic fixtures; additional resource limits remain deferred under Q11.

### Q9. Are large file inputs/attachments required for the first usable importer milestone, and who may fetch remote bytes?

Original recommended default: semantic records first and large files later. The project owner requires file/blob output in the first usable milestone. The built-in generic URL importer performs the remote fetch and imports accepted regular-file content as an ordinary File entity backed by the existing blob store.

Consequence: the initial milestone includes streamed File/blob preparation and publication. The importer performs the fetch; there is no separate host HTTP-fetch callback. Complete File publication uses existing File/blob storage without embedding whole blobs in entity payloads.

Blocks: first usable importer milestone. Recorded answer (2026-09-10, project owner): large/streamed file handling is required for the initial generic URL importer; the importer fetches remote bytes. The planner must integrate this with interface async streams and existing File/blob APIs without embedding complete blobs in record payloads.

### Q10. How long must old plugin/config revisions and source checkpoints remain resumable?

Original recommended default: jobs pin implementation/config/interface/checkpoint versions so old jobs can resume with retained revisions. The project owner instead requires plugin implementation or configuration changes to cancel all affected jobs.

Consequence: the runtime does not retain old plugin/configuration revisions solely for job resumption and does not migrate checkpoints across revisions. Affected active, queued, or otherwise resumable jobs transition to cancelled with a clear reason; users start a new job after the change.

Blocks: P2 recovery policy, P4/P5 change behavior, and P7 runbook. Recorded answer (2026-09-10, project owner): changing a plugin implementation or configuration cancels affected jobs. No cross-change resume; start a new job.

## Priority 2 — Tune after the first working milestones

### Q11. What expected concurrency, data volume and latency targets should drive budgets?

Original recommended default: several invocation, page, byte, and asset limits with measured tuning. The project owner requires only a simple configurable concurrent-jobs limit initially.

Consequence: the first release does not add stream item/byte limits, file-size limits, bandwidth controls, general invocation limits, deadlines, or cancellation grace-period policy. These remain explicit follow-up work; the generic jobs system enforces the concurrent-jobs limit.

Blocks: generic jobs crate and P7 basic concurrency tests. Recorded answer (2026-09-10, project owner): initially implement only a configurable concurrent-jobs limit; defer all other resource controls.

### Q12. What retention and privacy policy applies to inputs, previews, prepared pages, receipts and diagnostics?

Original recommended default: persist richer receipts/provenance and manage several retention classes. The project owner requires minimal persistent job records containing only status, progress, and simple errors. Job input, output, and other working data remain runtime-only and are not persisted as job state.

Consequence: the jobs collection is operational history rather than a durable import-data ledger. When its configurable job-count threshold is exceeded, cleanup deletes the oldest completed jobs first and never active jobs. A command clears completed jobs explicitly and is exposed in a jobs UI view. Imported entities and files/blobs remain normal durable domain data and are not owned by job cleanup.

Blocks: generic jobs crate persistence/API and jobs UI. Recorded answer (2026-09-10, project owner): persist only job status, progress, and errors; keep working data in memory. Prune oldest completed jobs above a configurable count threshold and provide a clear-completed command surfaced in a jobs view. Simple stored errors are sufficient.

### Q13. Is an automatic “best importer” choice desired when several plugins match?

Recommended default: explicit user selection wins; otherwise deterministic configured priority, bounded probe match strength, then stable plugin/export ID. The project owner adopted priority ordering and specifically requires the generic URL importer to have low priority so more specialized importers win automatically.

Consequence: priority gives predictable automatic selection and lets specialized URL handlers supersede the generic fallback. The API/UI must also list every available importer for a URL and let the user explicitly override automatic selection. Selection for a running job remains fixed.

Blocks: P2 discovery/start API and UI acceptance. Recorded answer (2026-09-10, project owner): use priority ordering; give the generic URL importer low priority and specialized importers higher priority. Also expose available importers for a URL and let the user choose explicitly.

## Decision recording

When answering a question, update its recorded answer and the affected sections of design/plan together. For decisions that add core transaction semantics, a dynamic native ABI, broader authority, or another runtime/package ecosystem, attach a focused ADR with alternatives and tests before expanding implementation tasks. Lower-priority unanswered questions should not stop an independently useful earlier phase.
