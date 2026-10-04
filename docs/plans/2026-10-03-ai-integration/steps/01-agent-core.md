# Step 01: Agent core model and traits (`semantic_agent`)

Wave W1. Depends on: 00. Agent: opus. This is the most important step. Every
other crate builds on these types, so take care over naming, docs and tests.

Read: plan.md (K1-K3, K5, K9-K11), research/t3code-providers.md §2-4, §11-13,
research/agent-protocols.md §7.2 (capability and event model, mapping cheat sheet),
research/semantic-data-layer.md §1.3, §2 (derives and their limits),
`crates/data/src/jobs.rs` and `crates/data/tests/derive.rs` (derive usage).

## Goal

A protocol-neutral, runtime-agnostic, wasm-safe crate that defines:

1. the **agent model**: content, items, requests, plans, usage, failures, policies,
   model catalog and capabilities;
2. the **`SessionEvent` vocabulary** drivers emit, and the **history types** drivers
   return when reading a native session's transcript from the agent's own storage
   (transcripts are not stored in the Semantic DB in v1; see plan K4, K12);
3. the **driver traits** (`AgentDriver`, `AgentInstance`, `AgentSession`);
4. the pure **`Transcript` reducer** that folds session events into items and requests;
5. a **unified diff model and parser**;
6. small shared utilities: id derivation, clock and id source traits, bounded text.

All types derive `SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq` (plus
`Eq`/`Hash` where meaningful). No serde. No tokio. Strings carrying ids are newtypes.

## Module layout

```
crates/agent/src/
  lib.rs          re-exports, crate docs (layering rules)
  ids.rs          DriverKind, InstanceId, ModelId, TurnId, ItemId, RequestId, NativeRef, RefStrength,
                  derive_id(prefix, parts) (sha256), IdSource trait, Clock trait, SystemClock/RandomIds (cfg not wasm or using getrandom-free fallback)
  content.rs      ContentPart, AttachmentRef, EntityMention
  policy.rs       AccessMode, InteractionMode, EffectivePolicy, Enforcement
  model.rs        ModelSelection, OptionValue, OptionScalar, ModelInfo, OptionDescriptor, OptionChoice
  item.rs         Item, ItemStatus, ItemBody + body structs, ToolRef, ToolCategory, FileChangeEntry, OutputText
  request.rs      Request, RequestBody, ApprovalSubject, ApprovalOption, ApprovalDecision, Question, QuestionOption,
                  RequestResponse, ResponseMode, RequestResolution
  plan.rs         TodoPlan, TodoEntry, TodoStatus
  usage.rs        TokenUsage, Cost, CostBasis, ContextWindow, UsageReport, UsageScope, RateLimitWindow
  failure.rs      Failure, FailureClass, bounded/redacted constructors
  capabilities.rs Capabilities and sub-enums, degradation helpers
  session.rs      SessionSpec, McpServerSpec, InstructionBlock, EnvVar, TurnInput, TurnOverrides, SessionConfigPatch,
                  SessionInfo, TurnOutcome, ThreadDisposition, SessionCloseReason
  event.rs        SessionEvent (+ payload structs), DeltaChannel
  status.rs       ProviderStatus, InstallState, AuthState, AuthMethod, AccountInfo, Compatibility
  driver.rs       AgentDriver, AgentInstance, AgentSession traits; DriverDescriptor; InstanceConfig; errors
  transcript.rs   Transcript reducer
  diff.rs         unified diff model + parser + stats
  text.rs         BoundedText helpers (truncate on char boundary, redaction helper for secrets)
```

## Types (normative sketches; adjust names only with good reason, then document)

### Ids (`ids.rs`)

```rust
/// Open slug: "claude_code", "codex", "acp", "fake", or unknown future kinds (must round-trip).
pub struct DriverKind(pub String);
pub struct InstanceId(pub String);          // user slug, default instance id == driver kind
pub struct ModelId(pub String);
/// Session-local ids assigned by drivers. Unique within one AgentSession.
pub struct TurnId(pub String);
pub struct ItemId(pub String);
pub struct RequestId(pub String);

#[semantic(rename_all = "snake_case")]
pub enum RefStrength { Strong, Weak, None }
/// Provider-native identifier: evidence, never identity.
pub struct NativeRef { pub id: String, pub strength: RefStrength }

pub fn derive_id(prefix: &str, parts: &[&str]) -> String; // "{prefix}-{hex(sha256(len-prefixed parts))[..32]}"
pub trait Clock: Send + Sync { fn now(&self) -> DateTime; }
pub trait IdSource: Send + Sync { fn new_id(&self, prefix: &str) -> String; } // "{prefix}-{uuid v4}"
```

The `derive_id` encoding must be unambiguous: length-prefix each part. It must be
stable forever because it is persisted. Write a test with fixed vectors.
`SystemClock`/`UuidIds` implementations live behind `cfg(not(target_arch = "wasm32"))`
if they need `uuid`'s rng or the time crate's `now_utc`. Otherwise make them available
everywhere.

### Policy (`policy.rs`)

```rust
/// User-facing permission level. Drivers compile it to native settings.
pub enum AccessMode { Supervised, AcceptEdits, Auto, FullAccess }   // ranked, impl Ord by rank
pub enum InteractionMode { Default, Plan }
pub enum Enforcement { Native, ClientBoundary }
/// What the driver actually applied (reported in SessionReady / ConfigChanged).
pub struct EffectivePolicy { pub access: AccessMode, pub interaction: InteractionMode,
    pub enforcement: Enforcement, pub native_mode: Option<String> /* e.g. "acceptEdits", "workspace-write" */ }
```

Docs must state the mapping contract. An unsupported mode degrades to the next
**safer** supported mode, never to a looser one. The degradation function lives here:
`fn clamp_access(requested, supported: &[AccessMode]) -> AccessMode`.

### Content (`content.rs`)

```rust
#[semantic(tag = "kind", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    Image { attachment: AttachmentRef },
    File { attachment: AttachmentRef },
    /// A path inside the workspace the agent should look at (rendered as @path / resource_link).
    WorkspacePath { path: String },
    /// A Semantic entity reference; drivers render it as text plus optional MCP hint.
    Entity { mention: EntityMention },
}
pub struct AttachmentRef { pub file_id: Option<String>, pub local_path: Option<String>,
    pub name: String, pub mime_type: String, pub byte_size: u64 }
pub struct EntityMention { pub collection: Option<String>, pub id: String, pub label: String, pub class_id: Option<String> }
```

### Model selection (`model.rs`)

```rust
pub struct ModelSelection { pub model: ModelId, pub options: Vec<OptionValue> }
pub struct OptionValue { pub id: String, pub value: OptionScalar }
#[semantic(tag = "kind")] pub enum OptionScalar { Text { value: String }, Flag { value: bool } }
pub struct ModelInfo { pub id: ModelId, pub name: String, pub description: Option<String>,
    pub is_default: bool, pub hidden: bool, pub context_window: Option<u64>,
    pub supports_images: bool, pub options: Vec<OptionDescriptor> }
#[semantic(tag = "kind", rename_all = "snake_case")]
pub enum OptionDescriptor {
    Select { id: String, label: String, description: Option<String>, choices: Vec<OptionChoice>, default: Option<String> },
    Toggle { id: String, label: String, description: Option<String>, default: bool },
}
pub struct OptionChoice { pub id: String, pub label: String, pub description: Option<String> }
```

Reasoning effort, fast mode and thinking are **option descriptors**, not typed fields
(t3code lesson). Drivers declare them per model.

### Items (`item.rs`)

```rust
pub enum ItemStatus { Pending, Running, Completed, Failed, Declined, Cancelled, Interrupted }
pub struct Item {
    pub id: ItemId, pub turn: TurnId, pub parent: Option<ItemId>,   // subagent nesting
    pub status: ItemStatus, pub native: Option<NativeRef>, pub body: ItemBody,
}
#[semantic(tag = "kind", rename_all = "snake_case")]
pub enum ItemBody {
    UserMessage { content: Vec<ContentPart>, intent: UserMessageIntent },     // turn_start | steer | queued
    AgentMessage { text: String },
    Reasoning { text: String, summary: Option<String> },
    CommandExecution { command: String, cwd: Option<String>, output: OutputText, exit_code: Option<i64>,
                       duration_ms: Option<u64> },
    FileChange { changes: Vec<FileChangeEntry> },
    ToolCall { tool: ToolRef, input: Value, output: Option<Value>, error: Option<String> },
    WebSearch { query: String, results: Vec<WebSearchResult> },
    Subagent { agent_type: Option<String>, description: String, prompt: Option<String>,
               native_thread: Option<NativeRef>, result: Option<String> },
    PlanProposal { markdown: String },
    TodoList { plan: TodoPlan },
    Compaction { tokens_before: Option<u64>, tokens_after: Option<u64>, summary: Option<String> },
    Error { failure: Failure },
    Notice { level: NoticeLevel, message: String },
}
pub struct OutputText { pub text: String, pub truncated: bool, pub total_bytes: u64 }   // drivers cap live output at 256 KiB (head 32 KiB + tail)
pub struct FileChangeEntry { pub path: String, pub kind: FileChangeKind /* add|delete|update|move */,
    pub move_from: Option<String>, pub diff: Option<String> /* unified */, pub additions: Option<u32>, pub deletions: Option<u32> }
pub struct ToolRef { pub name: String, pub server: Option<String> /* MCP server */, pub category: ToolCategory, pub title: Option<String> }
pub enum ToolCategory { Read, Search, Edit, Execute, Fetch, Mcp, Think, Other }
```

`Value` is `semantic_data::Value` (free-form tool input and output). Provide
`ItemBody::kind_str()` and `ItemBody::text_mut(channel) -> Option<&mut String>` for
delta application. Provide `Item::is_terminal()`.

### Requests (`request.rs`)

```rust
pub enum ResponseMode { Live, Message }
pub struct Request { pub id: RequestId, pub turn: Option<TurnId>, pub item: Option<ItemId>,
    pub response_mode: ResponseMode, pub native: Option<NativeRef>, pub body: RequestBody }
#[semantic(tag = "kind", rename_all = "snake_case")]
pub enum RequestBody {
    Approval { subject: ApprovalSubject, reason: Option<String>, options: Vec<ApprovalOption> },
    Questions { questions: Vec<Question> },
    PlanApproval { markdown: String },
    Elicitation { message: String, schema: Option<Value>, url: Option<String> },
}
#[semantic(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalSubject {
    Command { command: String, cwd: Option<String> },
    FileChange { paths: Vec<String> },
    FileRead { paths: Vec<String> },
    Tool { tool: ToolRef, input: Value },
    Permission { description: String },
}
pub enum ApprovalDecision { AllowOnce, AllowForSession, AllowAlways, Deny, DenyAndInterrupt }
/// Provider-advertised choice; `id` is the native option id and must survive normalization.
pub struct ApprovalOption { pub id: String, pub label: String, pub decision: ApprovalDecision, pub warning: Option<String> }
pub struct Question { pub id: String, pub header: Option<String>, pub prompt: String,
    pub options: Vec<QuestionOption>, pub multi_select: bool, pub allow_custom: bool, pub required: bool }
pub struct QuestionOption { pub label: String, pub description: Option<String>, pub value: Option<String> }
#[semantic(tag = "kind", rename_all = "snake_case")]
pub enum RequestResponse {
    Approval { option_id: String },
    Answers { answers: Vec<QuestionAnswer> },
    PlanDecision { approve: bool, feedback: Option<String> },
    Elicitation { action: ElicitationAction /* accept|decline|cancel */, content: Option<Value> },
}
pub struct QuestionAnswer { pub question_id: String, pub selected: Vec<String>, pub custom: Option<String> }
pub enum RequestResolution { Answered, Cancelled, Expired, AnsweredElsewhere }
```

Add `RequestBody::validate_response(&RequestResponse) -> Result<(), ResponseError>`
(kind match, option id exists, required answers present). The orchestrator relies on it.

### Usage and failure (`usage.rs`, `failure.rs`)

```rust
pub struct TokenUsage { pub input: u64, pub output: u64, pub cached_input: u64, pub cache_write: u64, pub reasoning_output: u64 }
pub enum UsageScope { Turn, SessionCumulative }
pub struct Cost { pub amount: f64, pub currency: String, pub basis: CostBasis /* estimated|billed */ }
pub struct ContextWindow { pub used_tokens: u64, pub max_tokens: Option<u64> }
pub struct UsageReport { pub scope: UsageScope, pub tokens: Option<TokenUsage>, pub cost: Option<Cost>, pub context: Option<ContextWindow> }
pub struct RateLimitWindow { pub id: String, pub label: String, pub used_percent: Option<f64>, pub resets_at: Option<DateTime> }

pub enum FailureClass { Auth, UsageLimit, RateLimit, Overloaded, ContextOverflow, Refusal, Sandbox,
                        Transport, Protocol, Provider, Cancelled, Unknown }
pub struct Failure { pub class: FailureClass, pub message: String /* <=4096, redacted */, pub code: Option<String>,
                     pub retryable: Option<bool>, pub reset_at: Option<DateTime> }
```

`Failure::new` bounds and redacts the message. Redaction strips bearer tokens,
`sk-…`/`ghp_…`-like keys and URL query strings, using `text.rs`.

### Capabilities (`capabilities.rs`)

Use a small, enum-heavy struct. The research (t3code-providers §12.4) warns against
60 booleans.

```rust
pub struct Capabilities {
    pub steering: SteeringSupport,          // Native | InterruptRestart | None
    pub interrupt: bool,
    pub queued_input: bool,                 // provider queues messages sent mid-turn natively
    pub resume: bool,
    pub fork: ForkSupport,                  // FromTurn | Latest | None
    pub rollback: bool,                     // provider can drop the last N turns
    pub model_switch: ConfigSwitch,         // Live | NextTurn | Restart
    pub access_switch: ConfigSwitch,
    pub access_modes: Vec<AccessMode>,
    pub plan_mode: PlanModeSupport,         // Native | Emulated | None
    pub approvals: Vec<ApprovalSubjectKind>,
    pub approval_scopes: Vec<ApprovalDecision>,
    pub questions: bool, pub plan_proposals: bool, pub todo_lists: bool, pub subagents: bool,
    pub streaming: StreamingSupport,        // { text, reasoning, tool_output: bool }
    pub mcp: McpSupport,                    // { http, stdio: bool }
    pub input: InputSupport,                // { images, files, workspace_paths: bool }
    pub usage: UsageSupport,                // { tokens, cost, context_window, rate_limits: bool }
    pub instructions: InstructionChannel,   // SystemPrompt | DeveloperContext | FirstMessage
    pub history: HistorySupport,            // Cheap (local file read) | Process (needs agent process) | None   (plan K12)
    pub enforcement: Enforcement,
    pub identity: IdentitySupport,          // { turn: RefStrength, item: RefStrength }
}
```

Provide `Capabilities::none()` (most conservative) and pure degradation helpers:
`resolve_send_mode(requested: SendIntent, caps) -> EffectiveSend`, where
`SendIntent` is `Steer`/`Queue`/`Restart` and `EffectiveSend` is
`SteerNative | InterruptRestart | Queue | Reject(reason)`. The orchestrator calls these
helpers; it never matches on driver names.

### Session (`session.rs`)

```rust
pub struct SessionSpec {
    pub cwd: String, pub extra_dirs: Vec<String>, pub model: ModelSelection,
    pub access: AccessMode, pub interaction: InteractionMode,
    pub instructions: Vec<InstructionBlock>, pub mcp_servers: Vec<McpServerSpec>,
    pub resume: Option<NativeRef>, pub env: Vec<EnvVar>, pub hermetic: bool,
}
pub struct InstructionBlock { pub key: String, pub text: String }   // keyed so drivers re-send only on change
pub struct McpServerSpec { pub name: String, pub url: String, pub bearer_token: Secret,
    pub stdio_bridge: Option<CommandSpec>, pub tool_timeout_ms: u64 }
pub struct CommandSpec { pub program: String, pub args: Vec<String>, pub env: Vec<EnvVar> }
pub struct EnvVar { pub name: String, pub value: Secret }
/// String wrapper whose Debug prints "<redacted>". IntoValue writes the plain value: it is never persisted
/// by the domain layer (enforced in step 06 by not using it in classes).
pub struct Secret(pub String);
pub struct TurnInput { pub content: Vec<ContentPart>, pub overrides: TurnOverrides, pub context: Vec<InstructionBlock> }
pub struct TurnOverrides { pub model: Option<ModelSelection>, pub access: Option<AccessMode>, pub interaction: Option<InteractionMode> }
pub struct SessionConfigPatch { pub model: Option<ModelSelection>, pub access: Option<AccessMode>, pub interaction: Option<InteractionMode> }
pub struct SessionInfo { pub native_session: Option<NativeRef>, pub model: Option<ModelId>, pub policy: EffectivePolicy,
    pub agent_version: Option<String>, pub tools: Vec<String>, pub slash_commands: Vec<SlashCommand>, pub mcp_servers: Vec<McpServerStatus> }
#[semantic(tag = "kind")] pub enum TurnOutcome { Completed, Interrupted, Failed { failure: Failure } }
pub enum ThreadDisposition { Reusable, Broken }
pub enum SessionCloseReason { Requested, ProcessExited, ProtocolError, IdleRelease, HostShutdown }
```

### Events (`event.rs`)

```rust
pub enum DeltaChannel { Text, Reasoning, Output, PlanText, ToolInput }
#[semantic(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    SessionReady { info: SessionInfo },
    ConfigChanged { info: SessionInfo },
    TurnStarted { turn: TurnId, native: Option<NativeRef> },
    ItemStarted { item: Item },
    /// Appends `text` to the channel of `item`. Drivers never resend already-sent text.
    ItemDelta { item: ItemId, channel: DeltaChannel, text: String },
    ItemUpdated { item: Item },        // non-text changes, full snapshot
    ItemCompleted { item: Item },      // authoritative final state (replaces accumulated deltas)
    RequestOpened { request: Request },
    RequestClosed { request: RequestId, resolution: RequestResolution },
    PlanUpdated { turn: TurnId, plan: TodoPlan },
    UsageUpdated { turn: Option<TurnId>, usage: UsageReport },
    RateLimitsUpdated { windows: Vec<RateLimitWindow> },
    TurnEnded { turn: TurnId, outcome: TurnOutcome, disposition: ThreadDisposition, usage: Option<UsageReport> },
    Notice { level: NoticeLevel, message: String },
    SessionClosed { reason: SessionCloseReason, failure: Option<Failure>, stderr_tail: Option<String> },
}
```

Event ordering contract, documented on the enum and enforced by the transcript
reducer's debug assertions and by driver tests:

* `TurnStarted` precedes items of that turn.
* `ItemStarted` precedes `ItemDelta`/`ItemUpdated`/`ItemCompleted` of that item. A
  driver that only sees completions emits `ItemStarted` and `ItemCompleted`
  back-to-back.
* Exactly one `TurnEnded` per started turn.
* Requests opened during a turn are closed (by the driver or by `TurnEnded`
  implicitly cancelling them) before or with `TurnEnded`.

### Driver traits (`driver.rs`)

```rust
pub struct DriverDescriptor { pub kind: DriverKind, pub display_name: String, pub supports_multiple_instances: bool,
    pub config_schema: semantic_data::schema::Type /* derived from the instance config record */ }
pub struct InstanceConfig { pub id: InstanceId, pub driver: DriverKind, pub display_name: String,
    pub binary_path: Option<String>, pub home_dir: Option<String>, pub env: Vec<EnvVar>,
    pub launch_args: Vec<String>, pub driver_config: Value /* driver-specific record, decoded by the driver */ }

pub trait AgentDriver: Send + Sync + 'static {
    fn descriptor(&self) -> DriverDescriptor;
    fn create_instance(&self, config: InstanceConfig) -> BoxFuture<'static, Result<Arc<dyn AgentInstance>, DriverError>>;
}
pub trait AgentInstance: Send + Sync + 'static {
    fn id(&self) -> &InstanceId;
    fn driver(&self) -> &DriverKind;
    fn capabilities(&self) -> Capabilities;
    /// Side-effect free: version, auth, models. Must never start a billable session or a login flow.
    fn probe(&self) -> BoxFuture<'_, Result<ProviderStatus, DriverError>>;
    fn classify_change(&self, current: &SessionSpec, next: &SessionConfigPatch) -> ChangePlan; // ApplyLive | NextTurn | Restart | Reject{reason}
    fn open_session(&self, spec: SessionSpec) -> BoxFuture<'_, Result<SessionHandle, DriverError>>;
    /// Read the transcript of a native session from the agent's own storage (plan K12), newest page first.
    /// Must not mutate the native session (no new turns, no compaction). `Unsupported` when capabilities.history == None.
    fn read_history(&self, native: &NativeRef, cwd: &str, query: HistoryQuery) -> BoxFuture<'_, Result<HistoryPage, DriverError>>;
}
/// Returned by open_session: the control half and the single event stream.
pub struct SessionHandle { pub control: Box<dyn AgentSession>, pub events: BoxStream<'static, SessionEvent> }
pub trait AgentSession: Send + Sync + 'static {
    fn capabilities(&self) -> Capabilities;   // may be narrower than the instance after negotiation
    /// Ack only. Progress and the terminal state arrive as events.
    fn start_turn(&self, input: TurnInput) -> BoxFuture<'_, Result<TurnId, DriverError>>;
    fn steer(&self, turn: &TurnId, input: TurnInput) -> BoxFuture<'_, Result<(), DriverError>>;   // DriverError::Unsupported distinct
    fn interrupt(&self, turn: &TurnId) -> BoxFuture<'_, Result<(), DriverError>>;                 // ack only
    fn respond(&self, request: &RequestId, response: RequestResponse) -> BoxFuture<'_, Result<(), DriverError>>;
    fn update_config(&self, patch: SessionConfigPatch) -> BoxFuture<'_, Result<(), DriverError>>;
    /// Graceful close with escalation; idempotent. Must end the event stream with SessionClosed.
    fn close(&self, reason: SessionCloseReason) -> BoxFuture<'_, Result<(), DriverError>>;
}
pub enum DriverError { Unsupported { operation: String }, InvalidRequest { message: String },
    NotFound { what: String }, Auth { failure: Failure }, Spawn { message: String },
    Protocol { message: String }, Closed, Timeout { operation: String }, Io { message: String } }
```

`DriverError` is a plain enum with `thiserror` (no semantic derives needed) plus
`fn to_failure(&self) -> Failure`. Use `futures::future::BoxFuture` and
`futures::stream::BoxStream`.

### Status (`status.rs`)

```rust
pub struct ProviderStatus { pub install: InstallState, pub auth: AuthState, pub models: Vec<ModelInfo>,
    pub compatibility: Compatibility, pub rate_limits: Vec<RateLimitWindow> }
#[semantic(tag = "kind")] pub enum InstallState { Installed { version: Option<String>, path: String }, NotFound, Error { message: String } }
#[semantic(tag = "kind")] pub enum AuthState { Authenticated { account: Option<AccountInfo> }, Unauthenticated { methods: Vec<AuthMethod> }, Unknown { reason: Option<String> } }
pub struct AccountInfo { pub label: Option<String> /* email or org */, pub plan: Option<String>, pub method: Option<String> }
pub struct AuthMethod { pub id: String, pub label: String, pub kind: AuthMethodKind /* browser|terminal|api_key_env|device_code */, pub command: Option<CommandSpec> }
#[semantic(tag = "kind")] pub enum Compatibility { Supported, Untested { version: String }, Unsupported { reason: String } }
```

### History (`history.rs`)

Transcripts are provider-owned in v1 (plan K4, K12). Drivers normalize the agent's
stored session into the same `Item` model the live stream uses:

```rust
pub struct HistoryQuery { pub before: Option<HistoryCursor>, pub max_turns: u32 /* default 20 */ }
pub struct HistoryCursor(pub String);                 // opaque, driver-defined
pub struct HistoryTurn { pub turn: TurnId, pub native: Option<NativeRef>, pub ordinal: u64,
    pub outcome: Option<TurnOutcome>, pub usage: Option<UsageReport>, pub started_at: Option<DateTime>, pub ended_at: Option<DateTime> }
pub struct HistoryPage { pub turns: Vec<HistoryTurn>, pub items: Vec<Item> /* chronological, item.turn refers to turns */,
    pub before: Option<HistoryCursor> /* None = start reached */, pub total_turns: Option<u64>, pub warnings: Vec<String> }
```

* Item ids in history must follow the same derivation as the live mapper of that
  driver. Use the native item id when strong. Otherwise use
  `(turn native ref or ordinal, item ordinal)`. A live item and the same item read
  back from history then share the key whenever the protocol allows it (K9).
* History pages contain only completed turns. A turn still running in another process
  appears without an outcome.
* `Transcript::from_history(&HistoryPage)` builds a transcript from a page.
  `Transcript::prepend_history(&HistoryPage)` merges an older page in front,
  idempotent by item id. Both are used by the orchestrator snapshot and by the UI
  "load earlier" paging.

Add `history.rs` to the module layout.

### Transcript reducer (`transcript.rs`)

```rust
pub struct Transcript { /* turns in order, items by id with insertion order (Vec + index map), open requests, plan per turn, last usage */ }
impl Transcript {
    pub fn apply(&mut self, event: &SessionEvent) -> Applied;   // Applied describes changed item/request ids (for UI fine-grained updates)
    pub fn items(&self) -> impl Iterator<Item = &Item>;
    pub fn item(&self, id: &ItemId) -> Option<&Item>;
    pub fn open_requests(&self) -> impl Iterator<Item = &Request>;
    pub fn turn_state(&self, id: &TurnId) -> Option<&TurnState>;
}
```

Rules:

* `ItemDelta` appends to the channel's text. A delta for an unknown item, or for a
  channel the item body does not have, increments an `anomalies` counter (exposed for
  tests and diagnostics) and is dropped. Never panic in release.
* `ItemCompleted` replaces the body and sets a terminal status.
* `TurnEnded` marks all non-terminal items of that turn: `Interrupted` if the outcome
  was interrupted, `Cancelled` otherwise. It also closes the turn's open requests as
  `Cancelled`.
* `SessionClosed` terminates everything that is still open.

The domain `ThreadView` (step 06) embeds this logic per attempt. Keep the reducer
generic enough for that: expose `apply` plus `Applied`.

### Diff (`diff.rs`)

Parse git-style unified diffs (`diff --git`, `---`/`+++`, `@@ -a,b +c,d @@`, rename and
copy headers, `new file mode`/`deleted file mode`, `Binary files … differ`,
`\ No newline at end of file`) into:

```rust
pub struct DiffFile { pub old_path: Option<String>, pub new_path: Option<String>, pub status: DiffFileStatus,
    pub binary: bool, pub hunks: Vec<DiffHunk>, pub additions: u32, pub deletions: u32 }
pub struct DiffHunk { pub header: String, pub old_start: u32, pub old_lines: u32, pub new_start: u32, pub new_lines: u32, pub lines: Vec<DiffLine> }
pub struct DiffLine { pub kind: DiffLineKind /* context|added|removed|no_newline */, pub text: String, pub old_line: Option<u32>, pub new_line: Option<u32> }
pub fn parse_unified_diff(patch: &str) -> Result<Vec<DiffFile>, DiffParseError>;
```

It must be lenient: unknown header lines are skipped, and it never panics on malformed
input. Fuzz-ish property tests use generated inputs (no new deps; a simple
seeded generator is fine).

## Tests

* Round trip `T::from_value(t.clone().into_value()) == t` for every public model type.
  Use a `fixtures.rs` test module with one representative value per variant.
* Snapshot of the derived `SemanticType` for `ItemBody`, `SessionEvent` and
  `RequestBody`: check the tag field name and variant names (guards accidental renames).
* `derive_id` fixed vectors.
* `clamp_access` and `resolve_send_mode` tables.
* Transcript:
  * an event sequence per scenario: simple message, streaming deltas, tool call with
    output, approval opened/closed, interrupted turn, session crash;
  * the anomaly path;
  * delta-after-complete ignored.
* `RequestBody::validate_response` table.
* `Transcript::from_history` and `prepend_history`: ordering, idempotent merge, and a
  live transcript plus history page with overlapping ids (no duplicates).
* Diff parser against real `git diff` samples (add, delete, rename, binary, no-newline,
  multiple hunks), plus the malformed-input property test.
* wasm32 check of the crate.

## Acceptance

* Public API documented: each type and variant has a doc comment explaining its
  semantics and which protocols produce it.
* No serde, no tokio, compiles for wasm32.
* All tests green; `cargo fmt`; one commit: "Add semantic_agent core model, events and driver traits".
