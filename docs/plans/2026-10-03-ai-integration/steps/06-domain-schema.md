# Step 06: Domain schema and API contract (`semantic_agent_domain`)

Wave W2. Depends on: 01. Agent: opus. Runs in parallel with steps 02 and 08.

Read: plan.md (K1, K4-K7, K9, K11, K12, data model overview), steps/01-agent-core.md,
research/semantic-data-layer.md §1-3, §7, §9 (the end-to-end recipe is the template),
research/semantic-app-ui-layers.md §1.4, §2.3, §7 Recipe B,
`crates/data/src/jobs.rs` (package and class codec pattern), `crates/base/src/tasks/`
(`migration_v1.rs`, `schema.rs`, `commands.rs`), `crates/base/src/migration_support_v1.rs`,
`crates/base/tests/listing_migrations.rs`, and the memory rule on qualified attribute keys.

## Goal

The wasm-safe contract between orchestrator, app and UI:

1. the persisted package `semantic.agents`, containing **orchestration metadata only**
   (plan K4): current schema module, frozen migration `001_init`, collection and
   indexes;
2. class structs with codecs (qualified keys) and consistency tests;
3. the `ThreadEvent` live-stream vocabulary (a wire type, **not persisted**), the
   snapshot DTO and the pure `ThreadView` reducer;
4. status and attention derivation for threads (pure);
5. RPC DTOs (records, plain keys) and command specs (unary plus stream).

Transcripts (items, requests, events) are **not** stored in the DB. They come from
the live session or from provider history (plan K12).

## Module layout

```
crates/agent_domain/src/
  lib.rs
  ids.rs          WorkspaceId, ThreadId, RunId, AttemptId, ItemKey, RequestKey, CheckpointId, HostId("local"), StreamCursor{epoch, seq}
  schema/
    mod.rs        constants (PACKAGE_NAME "semantic.agents", MODULE_NAME "v1", COLLECTION "semantic_agents", class ids, attribute ids)
    current.rs    attributes() / classes() / indexes() for the CURRENT schema (root module)
    migration_v1.rs  frozen copy (no calls into current.rs or into Rust-derived types; literal enum variant lists)
    package.rs    package(): shared module + v1 module + migrations [shared::migration_v1(), v1::001_init]
  entity/         class structs (#[derive(Class)] with #[semantic(id = ...)]): Workspace, ProviderInstanceRecord, Thread,
                  Run, CheckpointRecord, ThreadLink
  event.rs        ThreadEvent { cursor, at, run, attempt, body: ThreadEventBody } (wire only)
  snapshot.rs     ThreadSnapshot DTO (metadata + composed transcript page + live state + cursor)
  view.rs         ThreadView reducer (pure)
  status.rs       ThreadStatus / Attention derivation, RunStatus transitions
  api/
    mod.rs        command specs (RpcCommandSpec / RpcStreamCommandSpec impls), names
    dto.rs        payloads and outputs
  error.rs        AgentApiErrorCode constants (stable string codes used in RpcError)
```

## Schema (current = v1)

One collection `semantic_agents` (Polymorphic, StrictRegisteredSchema).

All classes set `creatable_in_ui: Some(false)` and `include_in_ui_listings: Some(false)`;
they get custom UI. Reuse the shared attributes `semantic:title`, `semantic:parent`,
`semantic:created_at` and `semantic:updated_at`, declaring the `shared` module and
migration as `crates/data/src/filestore.rs` does.

Classes and attributes (prefix `semantic:agents:<class>:`). R = required.

| Class | Attributes |
|---|---|
| `workspace` | title(shared, R), root_path(String, R), host(String, R, default "local"), vcs(enum none/git, R), default_instance(String?), default_model(record ModelSelection?), default_access(enum?), archived(bool, R), created_at, updated_at |
| `provider_instance` | title(shared, R), instance_id(String, R, unique), driver(String, R), host(R), enabled(bool, R), binary_path(String?), home_dir(String?), launch_args(List<String>), env(List<record {name, value, secret: bool}>; secret values are **not stored**; see below), driver_config(Any), created_at, updated_at |
| `thread` | title(shared, R), workspace(Ref → workspace, Restrict), host(R), cwd(String, R), worktree(record {path, branch, base_ref}?), instance(String, R), model(record ModelSelection, R), access(enum, R), interaction(enum, R), status(enum ThreadStatus, R), attention(enum Attention, R), parent(shared semantic:parent, optional), relationship(enum fork/delegated/subagent?), subject(Ref any?), native(record {driver, id, strength}?), next_run_ordinal(u64, R), active_run(String?), queue_held(bool, R), handoff_turns(u64?; number of old-session turns already delivered via context handoff, step 10), delegation(record DelegationRecord {parent_thread, parent_run, task_id, mode: async|wait, delivery: pending|delivered|acknowledged|disposed, generation: u64}?; only on delegated child threads, step 10), pinned_at(DateTime?), settled_at(DateTime?), archived(bool, R), last_visited_at(DateTime?), last_activity_at(DateTime, R), last_completed_at(DateTime?), preview(String?; ≤ 280 chars of the last agent answer, for the inbox), mcp_capabilities(List<String>), created_at, updated_at |
| `run` | thread(String, R), ordinal(u64, R), status(enum RunStatus, R), origin(enum user/agent/delegation/system/schedule, R), queued_input(record QueuedInput {content: List<ContentPart>, overrides: TurnOverrides}?; present **only while queued**, cleared on dispatch), input_summary(String?; first 280 chars of the user text, kept for the inbox and the run list), client_request_id(String?), queue_position(u64?), attempts(List<record AttemptRecord {id, ordinal, reason, native_turn: NativeRef?, started_at, ended_at?, outcome?}>), usage(record UsageReport?), failure(record Failure?), checkpoint_before(String?), checkpoint_after(String?), queued_at(R), started_at?, completed_at? |
| `checkpoint` | thread(R), run?, ordinal(u64, R), ref_name(String, R), commit(String, R), files(List<record {path, status, additions, deletions}>), status(enum ready/missing/stale, R), created_at |
| `thread_link` | thread(R), kind(enum entity/url/pull_request/task, R), target(String, R), label(String?), created_at |

Notes:

* `run`, `checkpoint` and `thread_link` store `thread` as a **plain indexed string**,
  not a Ref, keeping writes cheap and deletes chunkable. `thread.workspace` is a Ref
  (low volume, integrity matters).
* `queued_input` and `input_summary` are the only user content stored. The queued
  input has not reached the agent yet, so the DB is its only home until dispatch. The
  280-char summary is the minimum the inbox needs. Document this in `schema/mod.rs`
  as the deliberate exception to "no transcripts in the DB".
* Secrets: `env` entries with `secret: true` store an empty value. In v1, secret env
  values are supplied by the process environment (`SEMANTIC_AGENTS_ENV_<NAME>`). Never
  persist tokens.
* `Any` (K5) is used only for `driver_config`. Verify that `SemanticType for Value`
  (`Any`) is a storable attribute type (`Catalog::apply_batch` must not reject it). If
  it is rejected, use `TypeKind::Json`/the closest storable open type and document
  the choice.
* Records embedded in classes (`ModelSelection`, `UsageReport`, `Failure`,
  `ContentPart`, `TurnOverrides`) are `semantic_agent` types. In the frozen migration
  their record types are **written out literally** (frozen copies), never derived from
  the Rust types. A later change to these Rust types therefore needs a new migration
  if it is persisted; document this in `semantic_agent`'s crate docs, listing which
  types are persisted.
* Integer width: all counters are `u64` (`Value::U64`) in data, indexes and query
  parameters (see the mixed-width compare caveat).

Indexes (field = qualified attribute id):

| Name | Field (+extra) | Kind | Unique |
|---|---|---|---|
| `agents_thread_activity` | thread.last_activity_at | Range | no |
| `agents_thread_workspace` | thread.workspace (+ last_activity_at) | Range | no |
| `agents_thread_title` | thread title (`semantic:title`, full-text) | FullText | no |
| `agents_run_thread_ordinal` | run.thread (+ ordinal) | Range | yes |
| `agents_checkpoint_thread` | checkpoint.thread (+ ordinal) | Range | yes |
| `agents_link_thread` | thread_link.thread | Equality | no |
| `agents_instance_id` | provider_instance.instance_id | Equality | yes |

Unique indexes on the polymorphic collection apply only to rows that have the field.
Verify this with a test. If unique indexes are not class-scoped, add a partial
predicate on `type`. If a full-text index on the shared title attribute is already
provided by core migrations, reuse it instead of adding one.

## Entity ids (K9)

| Entity | Id |
|---|---|
| workspace | `workspace-<uuid>` |
| thread | `thread-<uuid>` |
| run | `derive_id("run", [thread, client_request_id])` when a client id is given, else `run-<uuid>` |
| checkpoint | `"{thread}:cp:{ordinal:08}"` |
| thread_link | `derive_id("link", [thread, kind, target])` |

Item and request keys are UI and stream identities only (not entity ids):
`ItemKey = "{run or turn key}:{driver item id}"`, derived per plan K9 so live and
history renderings match.

## ThreadEvent (`event.rs`, wire only)

```rust
pub struct StreamCursor { pub epoch: String /* random per actor incarnation */, pub seq: u64 }
pub struct ThreadEvent { pub cursor: StreamCursor, pub at: DateTime, pub run: Option<RunId>, pub attempt: Option<AttemptId>, pub body: ThreadEventBody }
#[semantic(tag = "kind", rename_all = "snake_case")]
pub enum ThreadEventBody {
    ThreadConfigChanged { thread: ThreadSummary },           // title, model, access, interaction, cwd/worktree, flags, status, attention
    RunUpdated { run: RunSummary },                           // queued/started/status/usage/failure/checkpoints (full small snapshot)
    RunRemoved { run: RunId },                                // cancelled queued run
    AttemptStarted { attempt: AttemptRecord },
    AttemptEnded { attempt: AttemptId, outcome: TurnOutcome, disposition: ThreadDisposition },
    Item { item: ItemView },                                  // started/updated/completed: full item snapshot
    ItemTextAppended { key: ItemKey, channel: DeltaChannel, offset: u64, text: String },  // idempotent by offset
    RequestOpened { request: RequestView },
    RequestResolved { key: RequestKey, resolution: RequestResolution, response: Option<RequestResponse>, by: ResolvedBy },
    PlanUpdated { run: Option<RunId>, plan: TodoPlan },
    UsageUpdated { run: Option<RunId>, usage: UsageReport },
    SessionStateChanged { state: SessionState /* none|starting|ready|busy|closed|failed */, info: Option<SessionInfo>, failure: Option<Failure> },
    CheckpointCaptured { checkpoint: CheckpointSummary },
    RunsReverted { to_run_ordinal: u64, restored_files: bool },
    LinkAdded { link: ThreadLinkSummary }, LinkRemoved { target: String },
    Notice { level: NoticeLevel, message: String },
}
```

* `ItemView = { key, run: Option<RunId>, item: semantic_agent::Item, source: Live|History }`.
* Offsets in `ItemTextAppended` count **Unicode scalar values** (chars) in the channel
  text; this is documented and tested. A delta applies only if
  `current_len == offset`. When `offset < current_len` and the text matches, it is
  skipped as a duplicate. A gap (`offset > current_len`) is an anomaly: reload the
  snapshot.
* Notices are stream-only. "Interrupted because the app restarted" is not a
  notice; it is derived from the run's failure/outcome record, which **is** persisted.

## Snapshot (`snapshot.rs`)

```rust
pub struct ThreadSnapshot {
    pub thread: ThreadSummary, pub runs: Vec<RunSummary> /* last N, ordinal desc paging */,
    pub transcript: TranscriptPage,          // composed per K12: history up to boundary + live overlay
    pub open_requests: Vec<RequestView>, pub plan: Option<TodoPlan>, pub usage: Option<UsageReport>,
    pub session: SessionState, pub cursor: Option<StreamCursor> /* None when no live actor */,
    pub links: Vec<ThreadLinkSummary>, pub checkpoints: Vec<CheckpointSummary>,
}
pub struct TranscriptPage { pub items: Vec<ItemView>, pub turns: Vec<TurnView /* turn + aligned run id */>,
    pub before: Option<HistoryCursorDto> /* load earlier */, pub history: HistoryAvailability }
pub enum HistoryAvailability { Available, Partial { reason: String }, Unavailable { reason: String } /* provider has no history or parse failed */ }
```

## ThreadView reducer (`view.rs`)

```rust
pub struct ThreadView { pub thread: ThreadSummary, pub runs: OrderedMap<RunId, RunSummary>, pub items: OrderedMap<ItemKey, ItemView>,
    pub turns: Vec<TurnView>, pub requests: OrderedMap<RequestKey, RequestView>, pub plan: Option<TodoPlan>,
    pub usage: Option<UsageReport>, pub session: SessionState, pub cursor: Option<StreamCursor>, pub history: HistoryAvailability,
    pub before: Option<HistoryCursorDto> }
impl ThreadView {
    pub fn from_snapshot(snapshot: ThreadSnapshot) -> Self;
    pub fn apply(&mut self, event: &ThreadEvent) -> Result<ViewChange, ViewError>;  // Err(EpochMismatch | Gap) => client reloads snapshot
    pub fn prepend_history(&mut self, page: TranscriptPage) -> ViewChange;          // "load earlier", idempotent by item key
}
```

* Events with `cursor.seq <= self.cursor.seq` in the same epoch are ignored.
* A different epoch or a seq gap returns an error, and the client reloads the
  snapshot.
* `ViewChange` lists the changed item, request and run keys plus a `thread_changed`
  flag, so the UI can update fine-grained signals.

## Status and attention (`status.rs`)

* `ThreadStatus` (persisted): `idle | queued | starting | running | awaiting_user | finalizing | failed | interrupted`.
  The current run state is projected onto the thread. `awaiting_user` is persisted
  while a request is open, so the inbox pill is right. Recovery resets it (K7).
* `Attention` (persisted, drives inbox pills): ordered by priority
  `approval > question > plan_ready > failed > limited > working > unread_done > none`.
* `fn derive_attention(thread, active_run, open_request_kinds, last_visited_at, last_completed_at) -> Attention`.
  A pure, table-tested function, also used by the UI for optimistic display.
* `RunStatus` enum with a transition table, `fn can_transition(from, to) -> bool`:
  `queued → starting → running ⇄ awaiting_user → finalizing → completed`. Also
  `interrupted`, `failed` and `cancelled` from any active state, `queued → cancelled`,
  and `completed|interrupted|failed → reverted`.

## API (`api/`)

All payloads carry `scope_id: Option<String>` (convention). Mutations carry
`client_request_id: Option<String>`. Names follow `semantic.agents.<area>.<verb>`.

Unary commands:

| Command | Payload → Output |
|---|---|
| `semantic.agents.drivers.list` | `()` → `Vec<DriverDescriptorDto>` |
| `semantic.agents.providers.list` | `{scope_id}` → `Vec<ProviderInstanceDto {config, status: Option<ProviderStatus>, capabilities}>` |
| `semantic.agents.providers.refresh` | `{instance_id}` → `ProviderInstanceDto` |
| `semantic.agents.providers.upsert` / `.delete` | config → dto / `{instance_id}` → `()` |
| `semantic.agents.workspaces.list/create/update/delete` | standard CRUD; create validates path (exists, inside allow-list, detects git) |
| `semantic.agents.workspaces.branches` | `{workspace_id, query?, limit}` → `Vec<BranchInfoDto {name, is_remote, is_current}>` (for the new-thread worktree picker) |
| `semantic.agents.threads.list` | `{workspace?, status?, attention?, include_archived, search? (title full-text), cursor?, limit}` → `ThreadPage {threads: Vec<ThreadSummary>, next_cursor}` |
| `semantic.agents.threads.get` | `{thread_id, max_turns (default 20)}` → `ThreadSnapshot` |
| `semantic.agents.threads.history` | `{thread_id, before: HistoryCursorDto, max_turns}` → `TranscriptPage` (older provider history) |
| `semantic.agents.threads.create` | `{workspace_id, title?, instance_id, model, access, interaction, worktree?: WorktreeRequest, subject?, initial_message?: MessageInput}` → `ThreadSummary` |
| `semantic.agents.threads.import` | `{workspace_id, instance_id, native_session_id, title?}` → `ThreadSummary` (adopt an existing agent session as a thread; possible because history is provider-owned) |
| `semantic.agents.threads.update` | `{thread_id, patch: ThreadPatch}` → `ThreadSummary` |
| `semantic.agents.threads.delete` | `{thread_id, remove_worktree: bool (default false)}` → `()` (does not delete provider session files) |
| `semantic.agents.threads.visit` | `{thread_id}` → `()` (last_visited_at, attention recompute) |
| `semantic.agents.threads.send` | `{thread_id, client_request_id, message: MessageInput {content: Vec<ContentPart>, overrides}, mode: SendMode(auto|queue|steer|restart)}` → `SendOutcome {run_id, delivery: started|queued|steered|restarted}` |
| `semantic.agents.runs.list` | `{thread_id, before_ordinal?, limit}` → `Vec<RunSummary>` |
| `semantic.agents.runs.interrupt` | `{thread_id, run_id?, hold_queue}` → `InterruptOutcome {status: requested|no_active_run}` |
| `semantic.agents.queue.update` | `{thread_id, op: QueueOp(reorder|edit|cancel|promote_to_steer|resume)}` → `()` |
| `semantic.agents.requests.respond` | `{thread_id, request_key, response: RequestResponse, client_request_id}` → `()` |
| `semantic.agents.threads.diff` | `{thread_id, from: DiffPoint(run_before(run)|run_after(run)|thread_start|working_tree), to: DiffPoint}` → `DiffDto {files: Vec<DiffFileSummary>, patch: String, truncated: bool}` |
| `semantic.agents.threads.revert` | `{thread_id, to_run: RunId, restore_files: bool}` → `()` |
| `semantic.agents.threads.links` (list/add/remove) | link management |

Stream commands (`semantic_rpc::stream_command::RpcStreamCommandSpec`, `Input = ()`):

| Command | Payload | Output stream item / End |
|---|---|---|
| `semantic.agents.threads.watch` | `{scope_id, thread_id, cursor: Option<StreamCursor>}` | `ThreadStreamItem = Event(ThreadEvent) \| Reset{reason}` / `WatchEnd{reason: deleted|shutdown}` |
| `semantic.agents.threads.watch_list` | `{scope_id, workspace?}` | `ThreadListStreamItem = Upsert(ThreadSummary) \| Removed{thread_id} \| Resync` / `WatchEnd` |
| `semantic.agents.providers.watch` | `{scope_id}` | `ProviderInstanceDto` upserts / `WatchEnd` |

`Reset` tells the client to reload `threads.get`. It is sent when the cursor epoch is
unknown (actor restarted or evicted) or the seq fell out of the live log.

Errors use `RpcError` with stable codes from `error.rs`:
`agents_disabled`, `not_found`, `invalid_state`, `unsupported_capability`,
`validation`, `conflict`, `workspace_forbidden`, `provider_unavailable`,
`history_unavailable`.

## Tests

* `validate_package_migrations(&package())` passes.
* Package installs into an in-memory DB (`semantic_db_kv`) twice; the second run
  executes no migrations.
* Codec consistency per class (the jobs pattern): the record type's fields equal the
  class attribute ids and required flags, encoded keys are declared attributes, and
  round trips work.
* Insert and query test per index. Duplicate `(thread, ordinal)` run fails. Unique
  index scoping on the polymorphic collection. Title full-text search finds a thread.
* Persisted embedded records (`ModelSelection`, `UsageReport`, `Failure`,
  `ContentPart` in `queued_input`) round trip through the DB, and the frozen record
  types in the migration match the current Rust derive output (a test compares the two
  type definitions, so drift is detected when a Rust type changes).
* `ThreadView` reducer scenarios: snapshot plus events, duplicates ignored, epoch
  mismatch and gap errors, text offset idempotency (the same delta twice gives no
  double text), request lifecycle, run status transitions, revert, `prepend_history`
  idempotency.
* `derive_attention` table; `RunStatus::can_transition` table.
* Add the package's migrations to `crates/base/tests/listing_migrations.rs` and its
  hash file, following that file's procedure for new migrations. If the test lives in
  `semantic_base` and cannot depend on this crate without a cycle, add an equivalent
  hash-pin test in this crate instead.
* wasm32 check.

## Acceptance

* All of the above green. Every DTO and command spec is documented.
* One commit: "Add semantic.agents package and agent API contract".
