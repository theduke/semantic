# Current package and interface system research

Date: 2026-09-09

## Executive summary

The repository already has two adjacent pieces needed by plugins, but they are not yet joined:

- `semantic_data` has a serializable package schema with modules, contracts, interfaces, function signatures, metadata, semantic versions, and migrations.
- `semantic_rpc_core`/`semantic_rpc` has type-erased asynchronous command dispatch over the same `semantic_data::value::Value` model, plus HTTP and WebSocket transports.

The recommended plugin seam is therefore **an interface implementation bound to a package-defined interface**, with transport/runtime adapters behind it. Importing should be one standard interface (or a small family of versioned interfaces), not a special capability exposed differently by Rust, subprocess, WebSocket, and Wasmer plugins.

Important limitations in the current implementation:

1. Interfaces are schema only. There is no implementation descriptor, interface registry, conformance check, or generic invocation by `(interface, method)`.
2. Runtime packages expose flat RPC commands independently of their schema. Nothing checks that commands implement a declared interface.
3. Package normalization currently normalizes types, attributes, and classes, but not module interfaces/contracts. `normalize_function_type` exists but is not called for them.
4. Migration validation compares only types, attributes, and classes. Interfaces/contracts/functions have no migration operations and are not checked against a reconstructed catalog.
5. Catalog persistence retains the complete `Package`, but there are no indexed/resolved runtime interface entries in `Catalog`.
6. Runtime package registration is builder-time and application-wide; schema is copied into each lazily initialized scope. There is no dynamic unload/reload or scope-specific implementation lifecycle.

These are design constraints and implementation work, not reasons to introduce a parallel plugin IDL.

## Schema model

### Package and module

`Package` is the deployment/schema boundary (`crates/data/src/schema/package/package.rs:3`). It contains:

- globally meaningful `name`;
- a distinguished root `Module`;
- additional modules keyed by name;
- ordered package-scoped migrations;
- optional `SchemaVersion`;
- `Meta`.

`Module` (`crates/data/src/schema/module/module.rs:3`) contains constants, type definitions, attributes, classes, interfaces, contracts, and metadata. Collections are deterministic `BTreeMap`s throughout the schema model. A plugin API should preserve this determinism for manifests, fingerprints, cache keys, diagnostics, and generated bindings.

There is no dependency/version-requirement field on `Package`. `SchemaImport` exists (`crates/data/src/schema/core/schema_import.rs`), but it is not present on `Package` or `Module`, so dependency resolution must either extend the package model or live in a plugin manifest initially.

`SchemaVersion` is semver-shaped (`major`, `minor`, `patch`, optional `pre` and `build`) at `crates/data/src/schema/core/schema_version.rs:2`. It has ordering but no compatibility/range logic. Do not confuse package schema version, plugin artifact version, interface ABI version, and host protocol version; the design should name and negotiate these separately.

`Meta` (`crates/data/src/schema/core/meta.rs:3`) provides title, description, stable optional ID, deprecation, aliases, examples, tags, docs URL, and string annotations. Annotations are a useful non-breaking escape hatch for experimental plugin metadata, but durable execution/security fields deserve typed manifest fields rather than opaque annotations.

### Interfaces and contracts

The core callable shapes are deliberately small:

- `InterfaceType { methods: Vec<InterfaceMethod> }` (`crates/data/src/schema/behavior/interface_type.rs:2`).
- `InterfaceMethod { name, signature }` (`crates/data/src/schema/behavior/interface_method.rs:2`).
- `FunctionType { params, results, throws, async_fn }` (`crates/data/src/schema/behavior/function_type.rs:2`).
- `FunctionParam { name: Option<String>, ty }` (`crates/data/src/schema/behavior/function_param.rs:2`).

`TypeKind::Interface` allows an interface to participate in the general type graph (`crates/data/src/schema/core/type_kind.rs:41`). Module interfaces are keyed names pointing directly to `InterfaceType`; unlike `ContractInterface`, they do not carry their own `Meta` or explicit name field.

Contracts add named, documented declarations for constants, types, functions, attributes, classes, and interfaces (`crates/data/src/schema/contract/contract.rs:3`). `ContractFunction` and `ContractInterface` carry names and metadata (`crates/data/src/schema/contract/contract_function.rs:1`, `contract_interface.rs:1`). Nothing in runtime code consumes contracts today.

The type system is richer than JSON and RPC uses it only as description today. Function parameters/results can express the complete schema `Type`, while wire values are `semantic_data::value::Value`. This is a strong base for generated bindings and validation, provided a canonical calling convention is specified for multiple/named parameters and multiple results.

### Package registration and persistence

`normalize_package_definition` clones a package and normalizes each module (`crates/db_core/src/managed_schema.rs:38`). Module normalization currently visits type definitions, attributes, and classes only (`managed_schema.rs:230`). Although `normalize_function_type` can qualify parameter/result/error type references (`managed_schema.rs:450`), it is currently unused. Any interface-based plugin implementation must first extend normalization to interfaces, contract functions, and contract interfaces, with regression tests for nested/local type references.

`validate_package_migrations` replays DDL migrations into a fresh catalog and compares resulting types, attributes, and classes with module declarations (`managed_schema.rs:47`). There are no interface/contract migration operations. A near-term plugin design can treat callable declarations as package metadata updated atomically with package registration; if historical compatibility/auditing is required, explicit callable DDL operations will be needed later.

`StoredPackage` stores the full package (`crates/db_core/src/catalog/snapshot.rs:85`), and `Catalog` indexes packages by name (`crates/db_core/src/catalog/catalog.rs:39`, lookup at line 176). Thus interface declarations survive database snapshots, but callers currently need to walk package/module maps and no runtime implementation binding is persisted.

`Backend::upsert_package` is async and implemented by database adapters (`crates/db_core/src/backend.rs:127`). Embedded registration validates migrations and applies them transactionally (`crates/db_core/src/embedded/db.rs:354`). `PackageRegistrationOutcome` reports applied migrations (`crates/db_core/src/managed_schema.rs:29`). Plugin installation should distinguish artifact persistence/verification from schema activation, and preserve this database transaction boundary.

Relevant tests:

- package normalization and migration replay: `crates/db_core/src/managed_schema.rs:529`;
- package registration and mismatch policy: `crates/db_core/src/embedded/db.rs:4128`;
- multi-version package fixtures: `crates/db_test/src/suite/mod.rs:3082`;
- built-in file package structure/migrations: `crates/data/src/filestore.rs:871`.

## Runtime execution model

`RuntimePackage<Ctx>` (`crates/rpc_core/src/package.rs:4`) returns a schema `Package` and owned `DynCommand` handlers. `SemanticAppBuilder::register_package` registers every command in the global RPC registry, then retains the schema for database initialization (`crates/app/src/command.rs:170`). The schema and commands are related only by the Rust implementation; there is no conformance enforcement.

Commands use these contracts (`crates/rpc_core/src/command.rs`):

- `RpcCommandSpec` declares associated payload/output/error types, a static flat `NAME`, and a `FunctionType` signature.
- `RpcCommand<Ctx>` is `Send + Sync + 'static` and returns a boxed `Send` future borrowing command and context.
- `DynCommand<Ctx>` erases typed codecs to `Value` and retains a `FunctionType`.
- `CommandAdapter` performs `RpcDecode`/`RpcEncode`, returning structured `RpcResult` errors.

`RpcRegistry` stores commands in a `BTreeMap<String, Box<dyn DynCommand<Ctx>>>`, rejects duplicate names, and awaits invocation (`crates/rpc/src/registry.rs:6`). It has no enumeration API, replacement, deregistration, ownership attribution, cancellation, timeout, concurrency limit, or streaming method result. Those lifecycle facilities should be added above or alongside it rather than encoded independently in every plugin backend.

`RpcRequest` is `{ id: u64, command: String, payload: Value }`; `RpcResponse` echoes the ID and contains `Ok(Value)` or `Err(RpcError)` (`crates/rpc_core/src/protocol.rs:7`). `RpcError` has stable-looking string `code`, message, and optional typed `Value` data (`rpc_core/src/error.rs:3`). This protocol is a natural envelope for stdio and WebSocket host plugins, but needs an explicit protocol version/hello handshake, capability negotiation, notifications/events, cancellation, and lifecycle messages before being treated as a plugin protocol.

`RpcClient` already erases transport behind `RpcClientDyn`, with native `Send + Sync` bounds and a monotonically increasing relaxed-atomic request ID (`crates/rpc/src/client.rs:26`, `client.rs:194`). Existing transports include native/web WebSocket and native/web HTTP under feature flags (`crates/rpc/src/transport`). The server exposes Axum HTTP/WS adapters (`crates/rpc/src/server`). Reuse the request/result encoding and client abstraction, but do not assume network WebSocket behavior maps perfectly to stdio framing or Wasm guest calls.

`SemanticApp` is `Arc`-backed and request context resolves a scope/database and object store asynchronously (`crates/app/src/context.rs:8`). A plugin-facing host context should be a least-authority facade over these capabilities, not unrestricted access to `AppRequestContext` internals.

### Scope and lifecycle constraints

Commands are application-wide, while registered package schemas are applied lazily to scopes on first use (`crates/app/src/command.rs:167`, `crates/app/src/scope.rs:81`). Explicit scope behavior differs from default database initialization, as the builder documentation notes. Plugin design must answer independently:

- Is an installation global, principal-owned, or scope-owned?
- Is an implementation process/instance global, per plugin, per principal, per scope, or per invocation?
- Which package schemas are activated in newly opened versus existing scopes?
- How are install/enable/disable/uninstall and rollback made race-safe?

Current managers use `RwLock`-protected maps and cloneable `Arc` handles. Object-store resolution deliberately opens once under concurrent access and has a concurrency regression test (`crates/app/src/object_store.rs:184`). Plugin instance startup should offer the same single-flight guarantee without holding a blocking lock across process/network startup.

## File and blob abstractions

The schema-level `File` is the class `semantic:filestore:file` in built-in package `semantic.filestore` (`crates/data/src/filestore.rs:9`, `filestore.rs:18`). Its persisted entity contains a blob locator, filename, byte size, MIME type, file kind, SHA-256 content hash, upload timestamp, and optional media metadata. The package is always added by `SemanticAppBuilder::build` (`crates/app/src/command.rs:197`).

`FileService` (`crates/app/src/file.rs:27`) stores content in the default scope object store and the corresponding typed entity in `DEFAULT_COLLECTION`:

- accepts bytes or a `Send + 'static` sized/unsized byte stream;
- hashes while streaming;
- uses content-derived IDs by default;
- stages streamed content to a temporary locator then copies to its final locator;
- supports ranged reads via `FileReader`/`FileByteRange`;
- verifies the entity has exactly `FILE_CLASS_ID` before reading.

The blob layer is `objstore`. `ObjectStoreManager` attaches stores per `DbScopeId`, supports a per-scope default store, and resolves lazy provider-backed stores (`crates/app/src/object_store.rs:56`). `AppRequestContext::default_file_store` and `resolve_object_store` are currently crate-private (`crates/app/src/context.rs:57`), so plugins should normally receive controlled file APIs or opaque file handles, not raw store access.

For `WebcPackageFile`, prefer a subclass/new class in a dedicated plugin package that inherits or structurally includes the File class attributes and adds typed WebC metadata (manifest/package digest, registry reference, format version, signatures/provenance). Reuse `FileService` storage and streaming rather than creating a second blob pipeline. However, `FileService::open` currently requires the exact base File class ID (`crates/app/src/file.rs:476`), so subclass support requires an intentional compatibility change (for example catalog-aware `is-a File` validation) or a composition model. Do not weaken validation to accept arbitrary entities.

Also note that blob write and database entity insert are not one transaction: content can be persisted before `db.insert` fails. Plugin artifact installation needs explicit compensation/garbage collection and idempotency rules.

Relevant tests:

- File schema and migration fidelity: `crates/data/src/filestore.rs:871`;
- MIME/file-kind unit coverage: `crates/app/src/file.rs:547`;
- upload/download and transport behavior: `crates/rpc/src/file.rs:176` and server/client transport test modules;
- object store concurrent open: `crates/app/src/object_store.rs:183`.

## Recommended integration seams

1. **Define canonical interfaces in a built-in package.** Put versioned plugin lifecycle/introspection and importer interfaces in `semantic_data` package declarations. Importer parameters/results/errors must use normal schema types. Give contract/interface declarations stable IDs in `Meta.id` and semver them deliberately.

2. **Add an implementation registry above transport adapters.** Key bindings by a canonical interface identity plus implementation/plugin identity, not only a flat command string. The registry should expose discovery, conformance status, health, lifecycle state, and invocation. A Rust implementation, stdio RPC child, remote WS endpoint, and Wasmer component then implement the same erased invocation contract.

3. **Bridge existing commands during migration.** An interface method adapter can map a canonical `(package, module, interface, method)` to a namespaced RPC command and reuse `Value`, `RpcError`, codecs, and `FunctionType`. Avoid making the legacy flat command name the durable identity.

4. **Validate schema/implementation conformance at activation.** Compare method presence, parameter/result/error types, async/streaming mode, and interface version before publishing a binding. Fix interface/contract normalization first. Decide whether method order is semantically meaningful; currently it is a `Vec`.

5. **Separate durable descriptors from live instances.** Persist plugin artifact/source, digest, declared implementations, configuration schema, requested permissions, activation state, and version. Keep child processes, sockets, Wasmer stores/instances, connection pools, health/backoff, and cancellation tokens in an in-memory runtime manager.

6. **Make capabilities explicit.** Provide an invocation context containing scoped, authorized host interfaces (database mutation/query, files, logging, secrets if ever supported), and propagate principal/scope/correlation/deadline/cancellation. Remote plugins must not implicitly inherit local host authority.

7. **Keep Wasmer optional at crate and runtime levels.** Put Wasmer/WebC adapter and dependencies in a separate crate or feature-gated crate. Core manifests and interface registries must compile and operate without Wasmer. Persisted Wasmer descriptors should remain inspectable while their runtime reports a clear unavailable capability.

8. **Treat importing as a streaming job protocol.** Current RPC methods return one `Value`; large imports need bounded batches or streams, progress/checkpoints, cancellation, diagnostics, and backpressure. Initially, an importer can return/push bounded `Batch` values through a host-owned job runner. Do not materialize an entire import in one `Value` or give a guest direct unbounded database access.

9. **Use host-owned writes and file reads.** The host should validate and commit importer output through `SemanticDb` batches and expose ranged/streamed File access. This centralizes schema integrity, authorization, transaction retry behavior, quotas, and observability across every runtime.

10. **Design lifecycle for concurrency.** Use single-flight initialization, bounded per-plugin/global concurrency, request deadlines, cooperative cancellation, process kill fallback, WS reconnect/backoff, circuit breaking, and deterministic shutdown. Never hold registry locks across awaited plugin I/O.

## Concrete preliminary work before runtime adapters

- Add canonical identity helpers for packages/modules/interfaces/methods and document name/version compatibility.
- Normalize all callable declaration type references in `managed_schema.rs`; add tests covering interfaces and nested contract functions/interfaces.
- Add read-only catalog/package interface lookup APIs and ambiguity diagnostics.
- Specify a canonical `FunctionType` wire calling convention and structural equality/conformance rules.
- Add runtime implementation registry traits independent of process/WebSocket/Wasmer details.
- Add ownership-aware registration and safe deregistration/draining; retain duplicate rejection.
- Specify plugin manifest, protocol hello/negotiation, errors, cancellation, deadlines, and capability grants.
- Define File subclass handling before introducing `WebcPackageFile`; test range reads and content integrity for derived file entities.
- Add feature/build matrix tests proving core + Rust/host plugins work with Wasmer disabled.

## Architectural risks and open decisions

- `InterfaceType` lacks metadata, inheritance, explicit version, method IDs, and streaming semantics. Decide which belong in the general interface system rather than plugin-specific extensions.
- A `FunctionType` supports multiple params/results, while command RPC has one payload/output value. Canonical object/tuple encoding must be fixed before cross-language plugins.
- Package semver is optional and has no dependency ranges. Installation/update resolution needs a policy.
- Package replacement semantics may allow callable schema drift independently of live implementations. Activation and schema update need a coordinated state machine.
- Dynamic native Rust shared libraries introduce ABI and memory-safety constraints. A “Rust plugin” can instead mean statically linked `RuntimePackage`/implementation factories unless stable dynamic loading is explicitly required.
- Remote WS endpoints have different trust, availability, and secret handling from host-spawned subprocesses. Sharing the method interface should not erase operational policy differences.
- Exactly-once import is unrealistic across plugin and database boundaries without idempotency keys/checkpoints. Specify at-least-once delivery and host-side deduplication/transaction rules.
- Artifact authenticity, registry trust, WebC signature verification, permission review, and supply-chain policy must precede auto-activation.

## Bottom line

The current package interface model should be the source of truth for plugin and importer APIs. The missing layer is a versioned, capability-aware implementation registry and job/lifecycle runtime that binds those declarations to interchangeable adapters. Build that core first, bridge Rust packages and existing RPC commands, then add stdio and WebSocket adapters; keep Wasmer/WebC as an optional adapter using the existing File/object-store pipeline.
