# Step 07: Orchestrator core (`semantic_agent_orchestrator`)

Wave W3. Depends on: 01, 02 (fake driver, testkit), 06 (domain), 08 (VCS crate
interface; only the trait is used here, the integration is step 10). Agent: opus.
Runs in parallel with steps 03, 04, 05, 09 and 12.

Read: plan.md (all K1-K12; K4, K6, K7 and K12 are central), steps/01, steps/06,
steps/02 (fake driver incl. history),
research/t3code-orchestration.md §1, §5, §8, §9, §21 (lessons and suggested Rust shape),
research/t3code-providers.md §7 (state machines), research/semantic-data-layer.md §7,
research/semantic-app-ui-layers.md §1.5-1.6 (jobs runtime as the structural template),
`crates/jobs/src/{runtime,store}.rs`.

## Goal

The runtime that turns domain commands into agent sessions. It:

* persists **orchestration metadata** per K4;
* keeps the live transcript in memory and serves it per K6;
* composes snapshots from provider history plus the live overlay per K12;
* recovers per K7.

This step builds the **core**: actors, the state machine, metadata persistence, the
live log and bus, snapshot composition with the history cache, recovery, send modes,
interrupt and requests. Provider config and probing, VCS integration, MCP, delegation
and handoff are added in step 10 on top of the extension points defined here.

## Structure

```
crates/agent_orchestrator/src/
  lib.rs
  runtime.rs        AgentRuntime (app-wide): DriverRegistry, InstanceRegistry (Arc<dyn AgentInstance> by InstanceId),
                    Clock, IdSource, RuntimeConfig, hooks (SessionDecorator chain, see below), shutdown()
  scope.rs          ScopeOrchestrator (per scope): store, thread registry (ThreadHandle by ThreadId), list bus, history cache,
                    recovery on open, shutdown(); `ScopeOrchestrator::open(runtime, store) -> Arc<Self>`
  db.rs             AgentDb seam trait (query/get/commit/upsert_package)
  store.rs          AgentStore trait (metadata persistence port) + DbAgentStore<D: AgentDb> + MemoryAgentStore (tests)
  thread/
    mod.rs          ThreadHandle (mpsc sender + watch of ThreadSummary), spawn/evict logic
    actor.rs        ThreadActor task loop (shell)
    state.rs        ThreadState: persisted metadata (thread row, active/queued runs) + live state (Transcript of the current
                    session, open requests, plan, usage, history boundary)
    decide.rs       pure decide(&ThreadState, ThreadCommand, &DecideCtx) -> Result<Decision, Rejection>
    ingest.rs       pure SessionEvent (+ attempt fence) -> Decision (stream events + metadata changes + effects)
    evolve.rs       pure ThreadState::evolve(&Decision)
    effects.rs      Effect enum + executor (talks to AgentSession / runtime)
    live_log.rs     LiveLog: epoch + seq allocation, bounded ring of Arc<ThreadEvent> (20k events / 32 MB), replay from cursor
    batching.rs     DeltaBatcher: merges consecutive ItemTextAppended for the same (item, channel) within 50 ms before publishing
                    (UI message rate only; nothing is persisted)
  snapshot.rs       compose_snapshot(metadata, history page, live state) per K12 (pure) + run/turn alignment (pure)
  history.rs        HistoryCache: LRU (default 64 threads) of HistoryPage per (thread, native ref); single-flight loads;
                    invalidated on run end; respects Capabilities.history (Cheap | Process | None)
  bus.rs            list broadcast<ThreadListStreamItem>; watch_thread / watch_list streams
  recovery.rs       recovery pass (K7) executed on ScopeOrchestrator::open
  instructions.rs   InstructionComposer: builds Vec<InstructionBlock> from registered InstructionSources
  policy.rs         pure helpers: send mode resolution (wraps semantic_agent::resolve_send_mode), access clamping per instance
  error.rs          OrchestratorError (+ to RpcError mapping helper living in domain error codes)
  testing.rs        (cfg test / feature "testing") harness: memory DB + fake driver runtime builder
```

## Key interfaces

```rust
/// Minimal DB seam (precedent: semantic_base LabelStore). Implemented by semantic_app for Arc<dyn SemanticDb>,
/// and in tests directly for semantic_db_core::Db over semantic_db_kv memory.
pub trait AgentDb: Send + Sync + 'static {
    fn query(&self, query: QueryInput /* AST with params preferred */) -> BoxFuture<'_, Result<Dataset, AgentDbError>>;
    fn get(&self, collection: &str, id: &str) -> BoxFuture<'_, Result<Option<Object>, AgentDbError>>;
    fn commit(&self, batch: Batch) -> BoxFuture<'_, Result<(), AgentDbError>>;     // execute_batch_returning(Stats)
    fn upsert_package(&self, package: Package) -> BoxFuture<'_, Result<(), AgentDbError>>;
}

/// Metadata persistence port; everything the orchestrator needs, nothing more. No transcript methods (K4).
pub trait AgentStore: Send + Sync + 'static {
    fn ensure_schema(&self) -> BoxFuture<'_, Result<(), StoreError>>;
    fn load_thread(&self, id: &ThreadId) -> BoxFuture<'_, Result<Option<ThreadRecordSet>, StoreError>>; // thread + non-terminal runs + recent runs
    fn commit(&self, commit: MetadataCommit) -> BoxFuture<'_, Result<(), StoreError>>;                 // one Batch: thread/run/checkpoint/link upserts+deletes
    fn list_threads(&self, q: ThreadQuery) -> BoxFuture<'_, Result<ThreadPage, StoreError>>;
    fn runs(&self, id: &ThreadId, before_ordinal: Option<u64>, limit: u32) -> BoxFuture<'_, Result<Vec<Run>, StoreError>>;
    fn recovery_candidates(&self) -> BoxFuture<'_, Result<Vec<ThreadId>, StoreError>>;               // threads with non-terminal runs/status
    fn delete_thread(&self, id: &ThreadId) -> BoxFuture<'_, Result<(), StoreError>>;                 // thread + runs + checkpoints + links (chunked internally)
    // workspaces + provider instance configs CRUD (used by step 10 and the app)
    ...
}
```

`DbAgentStore` builds queries with the query AST and bound `Value::U64` parameters
(no string interpolation of user data). It encodes and decodes via the domain class
codecs. On `ensure_schema` it upserts `semantic_agent_domain::package()`, and it
fails closed if the stored package differs incompatibly (copy `crates/app/src/jobs/store.rs`).

## Thread actor (normative)

* Exactly one actor per active thread per scope. It is spawned lazily by
  `ScopeOrchestrator::thread(id)` and is the only writer for that thread's rows (K4).
* Each actor incarnation has a random `epoch`. Its `LiveLog` assigns
  `StreamCursor{epoch, seq}` to every `ThreadEvent`.
* Mailbox messages:
  * `Command(ThreadCommand, oneshot reply)`;
  * `Session { attempt: AttemptId, generation: u64, event: SessionEvent }`;
  * `SessionEnded { attempt, generation }`;
  * `EffectResult(..)`;
  * `BatchTick`;
  * `Shutdown`.
* Loop step:

  ```
  1. receive msg
  2. pure: decide (commands) or ingest (session events) -> Decision { stream: Vec<ThreadEventBody>, metadata: Option<MetadataCommit>, effects: Vec<Effect> }
  3. if metadata changed: store.commit(metadata)   (state transitions only: run queued/started/ended, status/attention, config, native ref, checkpoint)
  4. evolve in-memory state
  5. append stream events to the LiveLog (cursor assigned) and publish to watchers; publish ThreadSummary on the list bus if it changed
  6. execute effects (async; results come back as EffectResult; never block the loop on provider I/O except short acks with timeouts)
  ```

  If a metadata commit fails, the in-memory state is not evolved and the command
  gets an error. For session-originated transitions, the commit is retried once.
  After that, the actor ends the attempt as failed (failure class `Unknown`, "could
  not persist thread state"), closes the session and emits a Notice. No silent
  divergence between DB and memory.
* Attempt fencing: session events carry `(attempt, generation)`. Events from a
  superseded or ended attempt are dropped. Exception: item updates for background
  subagent items of the *latest ended attempt* (Claude background tasks) are accepted
  and update existing live items only.
* Idle: after `idle_session_timeout` (default 30 min) with no active run, the actor
  closes the session (`SessionCloseReason::IdleRelease`). After `idle_actor_timeout`
  (default 10 min) with no session and no watchers, the actor exits and its live
  log is dropped. Watchers reconnecting later get `Reset` and reload a snapshot
  composed from provider history.
* Ordering: commands and session events for one thread are processed strictly in
  mailbox order. Different threads run fully in parallel.

## Commands (decide)

| ThreadCommand | Behaviour |
|---|---|
| `Create(config, initial_message?)` | Validate the workspace (store) and instance (runtime). Persist the thread row. If there is an initial message, continue as `Send(Auto)`. |
| `Import{native_session_id, ...}` | Create a thread with `native = Strong(id)`, validated by a `read_history` call (must succeed or return `Unsupported`, which is accepted with a warning). No session is opened until the first send. |
| `Send{client_request_id, message, mode}` | Idempotent: if a run with the derived id exists, return its outcome. If no active run: persist the run (`starting`, `input_summary`), emit the `UserMessage` live item (intent turn_start) and the effect `StartAttempt`. If active, resolve the mode with capabilities (`resolve_send_mode`): steer → effect `Steer` + live UserMessage(intent steer); interrupt_restart → new attempt on the same run (reason steer_restart) after the interrupt ack; queue → persist the run as `queued` with `queued_input` and `queue_position`. Reject when archived or `invalid_state`. |
| `Interrupt{run?, hold_queue}` | No active attempt: mark the run interrupted directly. Otherwise effect `Interrupt` (ack only). The run becomes interrupted when `TurnEnded(Interrupted)` arrives. After `interrupt_timeout` (default 20 s) escalate: close the session, mark the attempt interrupted (failure `Transport`, "agent did not stop; session closed"). |
| `Respond{request, response}` | Validate with `RequestBody::validate_response`; the request must be open in the live state. Mark it resolved in memory (`RequestResolved{by: user}` on the stream) **before** the side effect, so a double click cannot answer twice. Effect `Respond` for `Live` mode; `Message` mode becomes a `Send` with the rendered answer. If the session is gone, return `invalid_state` ("agent process ended; send a new message"). Thread status and attention leave `awaiting_user` when no request remains (metadata commit). |
| `UpdateConfig(patch)` | Title, pin, settle, archive and visited are pure metadata. Model, access and interaction: use `AgentInstance::classify_change` (ApplyLive → effect `UpdateConfig`; NextTurn → store for the next attempt's overrides; Restart → close the session after the active run; Reject → error). Access escalation while a run is active is allowed but emits a Notice. |
| `Queue(op)` | Reorder, edit (`queued_input`), cancel (deletes the run row), promote_to_steer and resume (clear `queue_held`, start the next run if idle). |
| `Revert{to_run, restore_files}` | Defined here (state transitions, `RunsReverted`, later runs → `reverted`). The VCS work is a pluggable effect implemented in step 10; until then `restore_files: true` returns `unsupported_capability`. Conversation rollback requires `Capabilities::rollback`; otherwise `native` is cleared so the next attempt starts a fresh native session with a handoff (step 10). |
| `Delete` | Close the session, delete the thread metadata via `store.delete_thread`, end watchers with `WatchEnd{deleted}`, emit `Removed` on the list bus. Provider session files are left untouched (document this). |

Effects (executed by the actor shell): `OpenSession`, `StartAttempt(run)`, `Steer`,
`Interrupt`, `Respond`, `UpdateConfig`, `CloseSession`, plus the `Hook(HookEffect)`
extension point used by step 10 (checkpoint capture, worktree prep, handoff,
delegation notifications).

## Session lifecycle in the actor

* `OpenSession` builds a `SessionSpec`:
  * cwd from the thread;
  * model, access and interaction;
  * instructions from the `InstructionComposer`;
  * MCP servers from session decorators (step 10 registers the MCP decorator);
  * resume from `thread.native` when the instance supports resume.

  On open, the actor records the **history boundary** (K12): either the history
  turn count at open time, from the cached `HistoryPage.total_turns` or the last
  history turn's native ref, or "unknown" if history is unsupported.

  On a resume failure (`DriverError::NotFound` or a protocol error during open),
  retry once without resume and emit a Notice "previous agent session could not be
  resumed; continuing with a fresh session". Step 10 adds the handoff context.
  Persist `native` from `SessionReady` (metadata commit).
* `StartAttempt` persists the attempt record (in the run's `attempts`, status
  `running`) **before** calling `start_turn(TurnInput{content, overrides, context})`.
  A crash then leaves a recoverable record. When starting a queued run, clear its
  `queued_input` in the same commit.
* Event ingestion (`ingest.rs`, pure):
  * items are folded into the session's live `Transcript` (`semantic_agent`) and
    emitted as `Item{ItemView}` with keys per K9;
  * deltas become `ItemTextAppended{offset}` (batched by `DeltaBatcher`);
  * requests become `RequestOpened`/`RequestResolved`. Thread status changes to
    `awaiting_user` when an approval or question opens, and back to `running` when
    none remain (metadata commit, because the inbox depends on it);
  * `UsageUpdated` updates the live state; usage is persisted on the run at attempt
    end;
  * `TurnEnded` gives `AttemptEnded`, then:
    * persist the run as `finalizing` if finalize hooks exist (step 10), else as
      completed/interrupted/failed, with usage, failure and the native turn ref;
    * persist the thread status, attention, `last_completed_at` and `preview` (first
      280 chars of the final agent message);
    * invalidate the history cache entry;
    * `ThreadDisposition::Broken` closes the session; clear `native` only if the
      provider said so;
  * `SessionClosed` while an attempt runs fails the attempt (`Transport`, stderr tail
    in the failure message, bounded) and the run. The session state becomes
    `closed`/`failed`.
* After a run reaches a terminal state, start the next queued run unless
  `queue_held`. If the run failed with a non-validation failure, set
  `queue_held = true` (t3code lesson: do not burn the queue on a broken provider).

## Snapshot composition (K12)

`ScopeOrchestrator::snapshot(thread_id, max_turns) -> ThreadSnapshot`:

1. Load the metadata (thread, recent runs, checkpoints, links) from the store.
2. If an actor with a live session exists:
   * take history up to the recorded boundary (cached `HistoryPage`, loaded if
     missing);
   * append the live transcript (turns since session open), the open requests, plan,
     usage and session state;
   * set `cursor = live_log.head()`.

   The live overlay must not duplicate history turns. The boundary guarantees this;
   assert it in tests with a provider whose history already contains the live turns
   once they are flushed.
3. Otherwise:
   * history only (`read_history` via the cache), `cursor = None`;
   * `HistoryAvailability::Unavailable{reason}` when the capability is `None`, the
     native ref is missing, or the read fails. The snapshot still returns the
     metadata and runs, so the UI can show the run list with summaries.
4. Align runs to turns (pure `align_runs(runs, turns)`):
   * by `attempts[].native_turn` where strong;
   * otherwise by order, from the most recent backwards, matching user-message turns
     to runs by ordinal;
   * turns without a run (for example continued in the CLI) stay unaligned and render
     normally.
5. `threads.history(before)` pages older history through the cache and the driver
   cursor.

## Live log and watch streams (K6)

```rust
impl ScopeOrchestrator {
    pub fn watch_thread(&self, id: ThreadId, cursor: Option<StreamCursor>) -> BoxStream<'static, Result<ThreadStreamItem, OrchestratorError>>;
    pub fn watch_list(&self, filter: ThreadListFilter) -> BoxStream<'static, Result<ThreadListStreamItem, OrchestratorError>>;
}
```

`watch_thread`:

1. Attach to the thread actor's live log. This spawns the actor if needed (cheap; no
   session is opened).
2. If `cursor` is in the same epoch and still in the ring, replay `seq > cursor.seq`.
   If the cursor is `None` and the client just loaded a snapshot without a cursor,
   start at the head. Otherwise emit `Reset{reason}` and continue from the head.
3. Tail new events. A slow watcher that lags beyond its bounded buffer gets `Reset`
   (it reloads the snapshot) instead of blocking the actor.
4. End with `WatchEnd{deleted}` on thread deletion, or `shutdown` on orchestrator
   shutdown. An actor eviction while watchers exist is prevented: watchers keep the
   actor alive, the session may still idle-close.

## Recovery (K7)

On `ScopeOrchestrator::open`, for each thread in `recovery_candidates()`, one metadata
commit per thread:

| State found | Action |
|---|---|
| run `starting`/`running`/`awaiting_user`/`finalizing` | open attempt → ended (Interrupted); run → interrupted with failure `{class: Transport, message: "Interrupted because the app restarted"}` |
| queued runs | kept, with `queue_held = true` |
| thread status / attention | recomputed (`interrupted` and `failed` attention as appropriate) |

Requests and live items were never persisted, so there is nothing to clean. Auto-continue
is not done in v1. The config flag `auto_continue_after_restart` exists, defaults to
false, and is documented as later.

## Instruction composer

`InstructionSource` trait: `fn blocks(&self, ctx: &InstructionCtx) -> Vec<InstructionBlock>`,
where ctx holds the thread, workspace, capabilities and MCP capabilities. The core
registers `runtime_info`: host app name, model and access mode, and "You can embed
images using Markdown with absolute file paths" only if the UI supports it (true).
Step 10 registers the MCP instructions. Blocks are ordered by key.

## Configuration (`RuntimeConfig`)

`idle_session_timeout`, `idle_actor_timeout`, `interrupt_timeout`, `delta_batch_window`
(50 ms), `live_log_max_events` / `live_log_max_bytes`, `history_cache_threads`,
`max_concurrent_sessions` (default 8 per runtime; extra `StartAttempt`s wait in FIFO
with a Notice "waiting for a free agent slot"), `workspace_roots` allow-list,
`auto_continue_after_restart` (false), `include_fake_driver`.

## Tests (fake driver incl. its history files + memory DB, all deterministic with test clock and ids)

1. Create a thread, send, receive a streamed answer:
   * the live events in cursor order;
   * the run persisted through its states;
   * the thread status cycles idle → starting → running → idle;
   * attention becomes unread_done, then none after visit;
   * `preview` set;
   * **no** transcript content in the DB, apart from `input_summary` (assert by
     scanning the collection).
2. Snapshot composition:
   * with a live session (history boundary plus live overlay, no duplicates);
   * after actor eviction (history only, same items and keys);
   * after an orchestrator restart (history only, runs aligned to turns);
   * history unavailable (fake `history: false`): metadata and runs are returned with
     `Unavailable`;
   * paging with `threads.history`.
3. Idempotent send with the same `client_request_id`.
4. Steer native (fake supports it) vs `InterruptRestart` (fake configured without
   native steering): attempts recorded, the superseded attempt's late events ignored.
5. Queue: two sends while running; the second starts after the first completes and
   its `queued_input` is cleared; reorder, edit and cancel; `queue_held` after a
   failure; resume; queued runs survive an orchestrator restart (held).
6. Interrupt during `/slow`; interrupt timeout escalation (fake configured to ignore
   interrupt).
7. Approval round trip (`/approve`): the thread is awaiting_user (persisted), then
   respond, then running. Respond with an invalid option is rejected. A double
   respond is rejected. Respond after the session closed returns `invalid_state`.
8. Questions and plan approval round trips.
9. Session crash (`/crash`): attempt failed, stderr tail in failure, session closed;
   the next send opens a new session with resume, and history continues.
10. Recovery: a mid-run state, drop the orchestrator, reopen: the run is interrupted
    with a reason, the queue is held.
11. `watch_thread`:
    * replay from cursor;
    * `Reset` on an unknown epoch and after eviction;
    * `Reset` for a lagging watcher;
    * delta offsets reconstruct identical text in a `ThreadView`, including
      re-delivery after reconnect.
12. `watch_list` receives summary upserts and attention changes.
13. Concurrency: 5 threads streaming simultaneously; per-thread ordering holds; the
    max concurrent sessions cap is honoured.
14. Delete removes the thread, runs, checkpoints and links rows; the fake history
    file still exists.
15. Persistence failure injection (MemoryAgentStore failing commit): command error,
    no state divergence.
16. Import: adopting an existing fake native session shows its history immediately.

## Acceptance

* All tests green.
* No provider-name matching anywhere in the orchestrator (grep for "claude", "codex",
  "acp" must only find docs and tests).
* One commit: "Add agent orchestrator core: thread actors, metadata store, live log, history composition, recovery".
