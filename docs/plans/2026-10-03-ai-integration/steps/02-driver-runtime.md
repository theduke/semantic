# Step 02: Driver runtime, process supervisor, fake driver, test kit (`semantic_agent_drivers`)

Wave W2. Depends on: 01. Agent: opus.

Read: plan.md (K2, K8, K10), steps/01-agent-core.md (traits and events),
research/agent-protocols.md §7.4 (pitfalls 2-6, 8, 10, 13), research/t3code-providers.md
§12.11 (process lifetime), §5.1 (spawn matrix).

## Goal

Shared infrastructure for all process-based drivers, plus the fake driver used
by orchestrator tests and e2e. Later driver steps (03-05) add one module each and must
not need to change this foundation beyond small additions.

## Module layout

```
crates/agent_drivers/src/
  lib.rs              crate docs; `pub fn builtin_drivers(opts) -> Vec<Arc<dyn AgentDriver>>` (feature-gated list)
  process/
    mod.rs            ManagedProcess, ProcessSpec, ExitInfo
    group.rs          unix process-group spawn + kill-tree (setsid / process_group(0)), windows fallback (job-less best effort)
    stderr.rs         StderrRing: bounded tail (default 16 KiB) + redaction on read
    env.rs            ChildEnv builder: allow-list of inherited vars + instance env + spec env; never logs values
  framing/
    ndjson.rs         NdjsonReader (no fixed line limit; configurable max e.g. 64 MiB; oversize -> skip+anomaly; strips \r; non-JSON lines -> anomaly not error),
                      NdjsonWriter (serialized writes via a single writer task)
    jsonrpc.rs        JsonRpcPeer: lenient JSON-RPC 2.0 (optional "jsonrpc" member for Codex), bidirectional:
                      outgoing requests with id allocation + oneshot correlation + per-call timeout,
                      incoming requests dispatched to a handler channel (never blocks the reader),
                      notifications channel, exact echo of server ids (string or number), cancellation hook
  discovery.rs        locate binaries (instance binary_path > PATH via `which` > known install dirs), run `--version` with timeout
  shell.rs            DriverShell helper: spawn reader/mapper task, event channel (bounded), control channel; the common
                      skeleton for "codec + pure mapper + shell" (K2)
  raw_log.rs          optional bounded rotating NDJSON raw-frame log per session (off by default; 64 KiB per payload cap; text deltas dropped)
  fake/
    mod.rs            FakeDriver ("fake"), FakeInstance, FakeSession
    script.rs         Script DSL: a sequence of steps per turn (emit item, stream text in chunks with delays, open request and wait
                      for response, emit usage, end turn with outcome, crash); deterministic; configurable via InstanceConfig.driver_config
  testkit/ (feature "testkit")
    transport.rs      in-memory duplex transport (pair of byte streams) to drive codecs/shells without processes
    replay.rs         fixture replay harness: load NDJSON fixture {dir: in|out, json} lines, feed `out` lines (agent->host) into a driver
                      shell over the in-memory transport, assert host->agent lines match `in` (normalized), collect SessionEvents
    golden.rs         compare collected SessionEvents with a golden file (plain-JSON rendering of IntoValue), with an env var to bless
```

## ManagedProcess (normative behaviour)

```rust
pub struct ProcessSpec { pub program: PathBuf, pub args: Vec<String>, pub cwd: PathBuf, pub env: ChildEnv,
    pub stdin: bool, pub kill_grace: Duration /* default 3s */, pub name: String /* for logs */ }
pub struct ManagedProcess { /* child, pgid, stdin, stdout, stderr ring, exit watch */ }
impl ManagedProcess {
    pub async fn spawn(spec: ProcessSpec) -> Result<Self, ProcessError>;
    pub fn take_stdin(&mut self) -> Option<ChildStdin>; pub fn take_stdout(&mut self) -> Option<ChildStdout>;
    pub fn stderr_tail(&self) -> String;                 // redacted
    pub fn exited(&self) -> watch::Receiver<Option<ExitInfo>>;
    /// Escalation: close stdin -> wait grace -> SIGTERM group -> wait grace -> SIGKILL group. Idempotent.
    pub async fn shutdown(&self) -> ExitInfo;
}
impl Drop for ManagedProcess { /* best-effort synchronous SIGKILL of the process group if still running */ }
```

* Each child gets its own process group (unix), so `npx` wrappers, shells and MCP
  servers die with it. On Linux, also set `PR_SET_PDEATHSIG` via `pre_exec` (behind
  `cfg(target_os = "linux")`) so children die if the host crashes.
* stderr is drained continuously into the ring. It is never parsed for control flow.
* Redaction: replace the home directory with `~`, and replace bearer tokens,
  `sk-[A-Za-z0-9_-]{8,}`, `ghp_…`, `xox…` and `Authorization:` header values with
  `<redacted>`.
* `ChildEnv` inherits only an allow-list: `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`,
  `LANG`, `LC_*`, `TERM`, `TMPDIR`, `XDG_*`, `SSH_AUTH_SOCK`, plus proxy vars
  (`HTTP(S)_PROXY`, `NO_PROXY`) and provider auth vars declared by the driver
  (e.g. `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, `CURSOR_API_KEY`).
  Instance env and spec env override inherited values.

## JsonRpcPeer

* It is generic over the transport (`AsyncRead`/`AsyncWrite`), so it is testable with
  the in-memory transport.
* `request<P, R>(method, params) -> Result<R, RpcCallError>`. Params and results
  are `serde_json::Value`, with typed helpers in drivers.
* Incoming server requests are delivered as `IncomingRequest { id: RawId, method, params, responder }`
  on a bounded channel. `responder.respond(result)` and `responder.error(code, msg)`
  must be called exactly once; the drop guard sends an error `-32603` "request dropped"
  so the agent never hangs. Approvals fail closed.
* On EOF or a decode failure of the stream, all pending calls fail with
  `RpcCallError::Closed`, and the notifications channel ends.
* Options: `jsonrpc_field: Required | Optional | Omit` (Codex sends and accepts it
  missing), plus a default request timeout.

## Driver shell pattern (K2)

`shell.rs` provides the common skeleton every protocol driver uses:

```
open_session:
  spawn ManagedProcess -> codec reader task -> mapper (pure, owned by the task) -> mpsc<SessionEvent> (bounded, e.g. 1024)
  control calls (start_turn, respond, ...) -> encode -> writer task
  on process exit: mapper.finish(exit) -> TurnEnded(failed, Broken) for open turns -> SessionClosed{stderr_tail}
```

The events channel uses backpressure toward the reader, never drops events. The
orchestrator consumes promptly and coalesces downstream.

## Fake driver

* Kind `fake`. It is always compiled and registered only when the runtime opts in
  (`builtin_drivers(opts)` with `opts.include_fake`). This allows e2e and dev
  servers to use it via config (`SEMANTIC_AGENTS_FAKE=1`, wired in step 11).
* Capabilities are configurable via `driver_config`. Defaults to "everything native".
* Default behaviour without a script is an echo agent:
  * replies "You said: <text>" streamed in 3 chunks;
  * the prompt keyword `/approve` triggers a command approval request;
  * `/ask` triggers a two-question request;
  * `/plan` emits a plan proposal plus todo list;
  * `/fail` ends the turn failed (class `provider`);
  * `/slow` streams over about 5 s so interrupt and steer can be tested;
  * `/write <relative path> <content>` writes a file inside the session cwd (path
    containment enforced) and emits a `FileChange` item, so checkpoints and diffs
    can be tested end to end;
  * `/crash` simulates process exit.

  The script DSL offers the same steps (`write_file`) for programmatic tests.
  This makes Playwright tests and manual demos possible without real CLIs.
* `probe()` returns `Installed`, `Authenticated` and two models with an effort select
  option.
* **History** (plan K12): the fake driver mimics agents that persist their own
  sessions.
  * Every completed item and turn is appended as NDJSON to
    `<history_dir>/<native session id>.jsonl`. `history_dir` comes from
    `driver_config` and defaults to a per-instance directory under the OS temp dir.
  * `read_history` pages it back with `HistorySupport::Cheap`, so restart and
    reload scenarios work in orchestrator tests and e2e.
  * `resume` reopens the same file.
  * Config flags `history: false` and `reject_resume: true` simulate providers
    without history and failed resumes.
* It honours `interrupt`, `steer` (native) and `respond`, and emits proper
  `RequestClosed`.

## Tests

* `ManagedProcess` (unix):
  * spawning `sh -c` that starts a grandchild and sleeps; `shutdown()` kills the whole
    group (assert the grandchild pid is gone);
  * stderr tail bounded and redacted;
  * exit info for normal exit and a signal.
* `NdjsonReader`:
  * a 10 MB line, CRLF, a non-JSON banner line (anomaly), an oversize line skip;
  * chunk boundaries split at every byte position (property test over a sample).
* `JsonRpcPeer`:
  * request/response correlation with interleaved server requests and notifications;
  * id echo for string and number ids;
  * a missing `jsonrpc` field accepted;
  * timeout;
  * a dropped responder sends an error;
  * EOF fails pending calls.
* Fake driver:
  * scenario tests through `Transcript`: echo, approval round trip, questions,
    interrupt during `/slow`, crash produces `TurnEnded(Failed, Broken)` and
    `SessionClosed`;
  * steer appends a `UserMessage{intent: steer}` item;
  * history round trip: the live transcript after two turns equals
    `Transcript::from_history(read_history(..))` (same item ids); paging with
    `max_turns = 1`; resume continues the same history file.
* Testkit replay harness self-test with a tiny synthetic protocol fixture.

## Acceptance

* `cargo test -p semantic_agent_drivers` green (unix-only tests gated with `cfg(unix)`).
* No dependency on `semantic_agent_domain`, `semantic_app` or any DB crate.
* One commit: "Add agent driver runtime, process supervisor and fake driver".
