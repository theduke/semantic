# Research: host-plugin transports and process supervision

Status: focused research input for the plugin/importer design. This document intentionally does **not** design an embedded Wasmer runtime, WEBC persistence, registry integration, or a `WebcPackageFile` type. A `wasmer` command is only one possible externally supervised executable.

## Executive recommendation

Define one transport-neutral, interface-driven plugin session protocol and implement it behind two adapters:

1. `StdioHost`: spawn a configured executable, use framed messages on stdin/stdout, reserve stderr for logs, and supervise the process.
2. `WebSocketHost`: connect to an authenticated `wss://` endpoint and put exactly one protocol envelope in each WebSocket message.

Keep request semantics above transport. Do not emulate WebSocket frames over stdio. Reuse the project's RPC value/request concepts where useful, but introduce a versioned session envelope because the current `RpcRequest { id, command, payload }` / `RpcResponse { id, result }` cannot express handshake, notifications, cancellation, streaming, or flow control. Both transports should feed the same bounded session actor and interface dispatcher.

The first production phase should support handshake, concurrent unary calls, cancellation, bounded queues, deadlines, graceful shutdown, crash reporting, and restart policy. Streaming/progress can be negotiated but implemented in the next phase. Bidirectional host callbacks should remain a separately negotiated capability because they significantly complicate authorization and deadlock avoidance.

## Current and historical evidence

The current project and `../semantic-original` have essentially the same WebSocket RPC shape:

- WebSocket text and binary messages deserialize directly as one `RpcRequest`/`RpcResponse`.
- responses are correlated through numeric IDs;
- each server request is spawned concurrently;
- the server response queue is unbounded;
- malformed messages use response ID `0`;
- client pending calls all fail when the reader exits;
- ping/pong are transport-only and ignored by the RPC layer;
- there is no handshake, protocol negotiation, cancellation, per-call deadline, stream, notification, or admission limit.

These are useful minimal semantics, but the unbounded response queue and unbounded request spawning must not be copied into a plugin runner. The existing RPC implementation should be adapted or extended, not made transport-dependent. `../semantic-new` contains ordinary child-process stdin/stdout use (for example media/CLI integrations), but no reusable long-lived plugin session or supervision protocol was found.

JSON-RPC is a useful design reference because its IDs correlate out-of-order responses and its notifications intentionally receive no reply, but Semantic need not claim JSON-RPC 2.0 conformance unless it adopts the exact envelope and rules ([JSON-RPC 2.0 specification](https://www.jsonrpc.org/specification)). LSP is a practical precedent for an initialize phase, capability negotiation, cancellation notifications, and token-correlated progress; notably, LSP still requires a terminal response after cancellation ([LSP 3.18 specification](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.18/specification.md)).

## Common session protocol

### Envelope

Use a tagged envelope rather than overloading `command` names:

```text
Envelope {
  protocol: "semantic.plugin",
  version: ProtocolVersion,
  session_id: opaque string,
  message: Message
}

Message =
  Hello { supported_versions, plugin_identity, implementation, interfaces, capabilities, limits }
  Welcome { selected_version, session_id, accepted_interfaces, capabilities, limits, heartbeat }
  Reject { code, message }
  Request { id, interface, method, payload, deadline_ms?, trace? }
  Response { id, outcome }
  Cancel { id, reason? }
  Progress { id, sequence, payload }
  Credit { channel, amount }
  Ping { nonce } | Pong { nonce }
  Shutdown { reason?, grace_ms? } | ShutdownAck
```

Use `u64` IDs scoped to the connection/session, never `0` as a magic parse-error ID. Protocol/framing errors that cannot recover an ID are connection-level `Reject`/close events. Serialize payloads initially with the project's typed `semantic_data::Value` JSON encoding. Put a maximum encoded size on every message before allocation/decoding. A future codec (for example CBOR) is a handshake option, not a silent format switch.

### Handshake and versioning

- No functional request is legal before `Hello`/`Welcome` completes.
- Negotiate a protocol **major/minor**, not just the plugin package version. Major means incompatible wire semantics; minor only adds optional fields/messages.
- Exchange plugin identity/version, build/runtime information, declared interface IDs and versions, optional features, and desired/hard limits.
- The host resolves declarations against the package system's interface definitions. Negotiate exact compatible interface versions and retain the resolved mapping in the session; do not trust self-reported method schemas as authority.
- Unknown optional fields are ignored within a compatible major version. Unknown message tags, impossible state transitions, duplicate in-flight IDs, or responses for unknown IDs are protocol violations.
- Put an explicit handshake deadline on both transports. Record a structured incompatibility reason rather than entering a restart loop.
- Capabilities should be granular: `cancel`, `progress`, `stream_v1`, `host_callbacks_v1`, `heartbeat`, codec, max concurrency, and maximum frame/message size. Avoid a single “supports v2” feature bucket.

Plugin package compatibility, Semantic interface compatibility, and wire-protocol compatibility are three independent checks. Persist them separately for diagnostics and safe upgrades.

### Calls, deadlines, and cancellation

- Calls may finish out of order; IDs provide correlation.
- The host applies an admission deadline and execution deadline even if the caller supplied none. Timeouts initiate `Cancel`, then discard/record late responses.
- Cancellation is cooperative: send `Cancel { id }`, keep the ID reserved, and require one terminal `Response` (`cancelled`, success if it raced, or another error). This follows the robust LSP pattern and prevents leaked pending state.
- Disconnect cancels all in-flight work locally. Whether work continues remotely is transport/provider policy and must be surfaced; it is not guaranteed cancellation.
- For a local process that ignores cancellation, optionally escalate per request only if the plugin is single-tenant/request-isolated. Otherwise killing it also fails unrelated calls.
- Use stable error classes: protocol, incompatible, unavailable, timeout, cancelled, permission denied, invalid input, resource exhausted, plugin failure, and internal. Preserve a bounded diagnostic payload, never raw secrets/environment.

### Backpressure and streams

Bound every layer:

- outbound command queue;
- inbound decoded-message queue;
- concurrent requests per session and per interface;
- encoded message size and decompressed size;
- buffered progress/stream bytes;
- stderr capture/log rate.

Use a semaphore before enqueueing a request, and reject/timeout admission rather than accumulating unlimited futures. A single writer task owns the transport and drains a bounded priority queue: shutdown/cancel/control before ordinary requests, with fair scheduling so streams cannot starve calls.

For phase-one unary calls, transport write completion is sufficient flow control. For streaming, introduce explicit per-stream credit/window messages and monotonic sequence numbers; never infer application consumption from TCP/WebSocket buffering. Progress should be lossy/coalescible and separately bounded, while result chunks are reliable and credit-controlled. Avoid JSON-RPC batch arrays: they delay independently completed responses and do not solve backpressure.

## Stdio transport

### Framing

Use an LSP-style header frame, which is debuggable and handles embedded newlines:

```text
Content-Length: <decimal bytes>\r\n
Content-Type: application/vnd.semantic.plugin+json\r\n
\r\n
<exactly Content-Length bytes>
```

Parsing rules must cap header bytes/count and body length, reject duplicate/invalid `Content-Length`, read the body with `read_exact`, and treat EOF mid-frame as a crash/protocol failure. JSON Lines is inadequate because a stray stdout log corrupts framing and large records encourage unbounded line buffers. A raw fixed-width length prefix is more compact but much harder to diagnose manually; it can be a later negotiated codec.

Stdout is protocol-only. Stderr is logs only, always drained concurrently into a bounded line/byte processor. The host should prefix/structure logs with plugin/session identity and truncate oversized lines. Never wait for process exit before draining both pipes: full OS pipes can deadlock the child.

### Launch configuration

```text
ExecutableLaunch {
  program, args, cwd?, environment_allowlist,
  startup_timeout, shutdown_grace, kill_grace,
  restart_policy, resource_policy, sandbox_profile?
}
```

Do not run through a shell. `program` and each argument are distinct configured values. Start from an empty or allowlisted environment; inject only session/config values meant for the plugin. Resolve executable identity at installation/activation time (trusted path plus digest/signature policy), not repeatedly via mutable `PATH` if avoidable.

`wasmer run <package-or-file> ...` is simply an `ExecutableLaunch`; the supervisor has no Wasmer-specific wire behavior. The executable inside that environment must speak the same stdio session protocol. Availability, version pinning, sandbox flags, and filesystem/network grants belong to deployment/launch policy, not the plugin interface or transport abstraction.

### Process supervision

Model explicit states: `Disabled -> Starting -> Handshaking -> Ready -> Draining -> Stopping -> Stopped`, with terminal/side states `Incompatible`, `Crashed`, and `Quarantined`.

- One actor/task owns the child handle, stdin writer, stdout decoder, stderr drain, timers, and restart decisions.
- Configure `kill_on_drop(true)` as a last-resort guard, but still explicitly shut down and wait. Tokio documents that dropping a `Child` does **not** cancel it by default ([Tokio process documentation](https://docs.rs/tokio/latest/tokio/process/)).
- Graceful stop: stop admissions, optionally drain bounded in-flight calls, send `Shutdown`, wait for acknowledgement/exit, close stdin, then terminate, wait a short kill grace, force-kill, and reap.
- Treat EOF, exit status, signal, handshake timeout, malformed output, and heartbeat expiry as distinct events.
- Kill the whole process tree/job, not only the immediate child. Implement platform-specific containment (Unix process group/session; Windows Job Object) behind a supervisor abstraction and test grandchildren. `kill_on_drop` alone is insufficient for descendants.
- Restart only for configured crash/unavailability classes, with capped exponential backoff plus jitter, a rolling restart budget, and a stable-period reset. Never auto-restart incompatibility/authentication/configuration failures. Quarantine crash loops and require operator action or a configuration/package change.
- Decide singleton vs pool in plugin configuration. Default to one long-lived process; allow a bounded pool for explicitly stateless/thread-safe importers. Never silently replay a mutating request after a crash. Retry only methods declared idempotent, with a host-generated idempotency key and caller policy.

## WebSocket transport

Use one complete protocol envelope per WebSocket text message initially. RFC 6455 fragmentation is transport detail; the library reassembles a message before the protocol decoder. Advertise a subprotocol such as `semantic.plugin.v1` using `Sec-WebSocket-Protocol`; RFC 6455 defines this application-subprotocol negotiation as part of the handshake ([RFC 6455](https://www.rfc-editor.org/rfc/rfc6455)). Keep application `Ping/Pong` for end-to-end session health/timing if intermediaries may satisfy WebSocket control ping/pong.

- Require `wss://` except explicitly configured loopback/development endpoints.
- Authenticate during HTTP upgrade (short-lived bearer/service credential or mutual TLS); do not place secrets in URLs. Authorize the plugin identity and allowed interfaces after authentication.
- Validate the server certificate and expected host; support deliberate CA/pin configuration. Rotate credentials without protocol changes.
- For browser-accessible endpoints, validate `Origin`; RFC 6455 makes Origin part of the browser security model. Do not mistake Origin for authentication.
- Set handshake, idle, heartbeat, write, and request deadlines; limit message and decompressed payload sizes. Either disable compression initially or bound post-decompression size to prevent compression bombs.
- Use meaningful close codes/reasons but truncate and sanitize them. A transport reconnect creates a new session ID, invalidates all pending IDs, and renegotiates capabilities.
- Remote WebSocket plugins are independently deployed services. Do not apply local-process restart semantics; use reconnect backoff/circuit breaking and health state. Never assume reconnect permits request replay.

## Security model

Transport security does not authorize interface use. At activation, calculate an effective grant from administrator/user policy, package declaration, plugin identity, requested capabilities, and scope. Enforce it in the host-side interface dispatcher for every call and host callback.

For local processes:

- least-privilege OS identity where practical;
- explicit cwd, environment, filesystem, network, CPU, memory, process-count, and open-file limits;
- no inherited stdin/stdout/stderr beyond the protocol pipes, and no inherited descriptors/handles;
- sandboxing is platform/deployment policy and should be pluggable;
- redact secrets and sensitive payload fields from logs/traces;
- distinguish trusted in-process Rust plugins from external processes: process separation is not automatically a sandbox.

Protocol inputs are untrusted even from a local child: bounded decoding, recursion/depth limits, schema validation against the negotiated interface, state-machine validation, rate limits, and no panics on unknown data. Host callbacks create an ambient-authority risk and should be deny-by-default with explicit per-interface grants.

## Observability and operations

Emit structured events/metrics keyed by plugin ID, configured instance ID, transport, session ID, interface/method, and outcome (avoid raw payloads):

- starts, successful handshakes, negotiated versions/capabilities, readiness duration;
- requests admitted/rejected, queue time, execution latency, timeouts/cancellations, in-flight and queue gauges;
- bytes/messages, decode failures, unknown/duplicate IDs, late responses, backpressure stalls;
- heartbeat latency/loss, disconnect/close reason;
- process exit status/signal, stderr truncation/drop count, restarts/backoff/quarantine;
- WebSocket connect/TLS/auth failures and reconnect/circuit state.

Propagate a trace context in the request envelope, but treat incoming trace identifiers as untrusted. Generate host spans around queue, transport, and plugin execution. Provide a bounded ring buffer of recent lifecycle/protocol metadata for diagnostics, without payloads or credentials. Expose readiness separately from liveness: a running process/socket can still be incompatible or saturated.

## Suggested abstractions

```text
PluginSession          # common state machine, pending calls, limits, cancellation
  -> PluginTransport   # send/receive envelope, close; no interface knowledge
       StdioTransport
       WebSocketTransport
  -> InterfaceBinding  # resolved package-system interface + method schemas
  -> PluginDispatcher # authorization, validation, call routing

ProcessSupervisor
  -> ExecutableLaunch
  -> ProcessContainment (Unix/Windows implementations)
  -> RestartPolicy
```

Keep `PluginTransport` small and test it with an in-memory duplex transport. Keep launch/supervision out of `StdioTransport` so an already-open stdio pair and test harness can use it. A `HostPluginProvider` composes supervisor plus session. The same interface invocation API should be implemented by Rust, stdio-host, and WebSocket-host providers; transport-specific types must not leak into importer interfaces.

## Implementation phases and acceptance tests

### Phase 1: protocol kernel

Define envelopes, handshake/state machine, compatibility algorithm, limits, error taxonomy, and in-memory transport. Add golden encoding, unknown-field, incompatible-version, invalid-transition, duplicate/unknown-ID, deadline, late-response, and queue-saturation tests.

### Phase 2: supervised stdio

Implement strict framing, bounded reader/writer queues, stderr draining, executable launch, shutdown escalation, containment, restart/quarantine, and a tiny adversarial fixture plugin. Test partial reads/writes, embedded newlines, oversized headers/body, stdout contamination, stderr flood, stdin backpressure, crash mid-request, ignored shutdown, grandchildren, crash loop, and no replay of mutating calls.

### Phase 3: WebSocket

Implement subprotocol/auth configuration, TLS policy, bounded messages, reconnect/circuit behavior, close mapping, and heartbeat. Run the same conformance suite over loopback WebSocket; add origin/auth/TLS rejection, fragmentation, compression limit, disconnect, slow-reader, and reconnect-with-new-session tests.

### Phase 4: importer integration and streaming

Bind negotiated package-system interfaces to importer calls. Add credit-controlled result streams/progress, cancellation races, per-interface quotas, and optional pools. Test parity across Rust/stdio/WebSocket providers using the same importer conformance suite.

## Decisions the planner should make explicitly

1. Is the new envelope an evolution of `semantic_rpc_core` or a separate `semantic_plugin_protocol` crate sharing only `Value`, errors, and interface identifiers? Separation is cleaner if public RPC compatibility must remain stable.
2. Which interface-version compatibility rule does the package system guarantee, and how are interface IDs canonicalized?
3. Are plugins allowed to call host interfaces in phase one? Recommendation: no; reserve and negotiate it later.
4. Is streaming required for the first importer milestone? If large imports are expected, at least a bounded pull/page API is required even if general streams wait.
5. What is the default trust/sandbox profile for local executables, and which platforms must enforce process-tree containment initially?
6. Which importer operations are declared idempotent/retriable, and where are idempotency keys persisted?
7. Does a configured plugin represent one process, a pool, or a remotely scaled service, and what readiness semantics does routing use?
8. What authentication/credential provider supplies WebSocket credentials and handles rotation?

## Bottom line

The important unification boundary is the negotiated package-system interface, not the transport. Stdio and WebSocket should differ only in framing, connection establishment, and lifecycle ownership. Bounded concurrency and queues, explicit cancellation semantics, deterministic session versioning, and observable supervision are baseline requirements—not later hardening—because importer workloads are naturally long-running and data-heavy.
