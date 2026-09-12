# Semantic storage and SDK implementation plan

Baseline: `fdde51a991463650de49f22489a0eba200c6e96e`. Proposed APIs below are additions; existing APIs retain defaults unless specified.

## Shared contracts and ownership

RPC errors retain `{code,message,data}`; add stable codes listed below as structured `DbError`/`AppError` variants. HTTP file errors use the same object; SDK `RpcError.data` retains its fields. Regenerate descriptors/JS bindings whenever public types change.

| Owner | Modules |
| --- | --- |
| Query/API | `crates/data/src/query/mod.rs`, `crates/db_core/src/query{.rs,/sql/enabled.rs}`, `crates/app/src/{command,db}.rs` |
| SDK | `lib/js/sdk/src/{client,types,builder,transport,files,index}.ts` |
| Storage | `crates/db_core/src/embedded/{db,storage}.rs`, `crates/db_kv/src/storage.rs`, `crates/db_postgres/src/managed.rs` |
| Validation | `crates/db_core/src/{validation.rs,catalog/catalog.rs}`, package normalization/migrations |
| Files | `crates/app/src/{file,db,error}.rs`, `crates/server/src/{file,router}.rs` |

## 1. Named SQL parameters — Query/API, SDK

Add `params: BTreeMap<String, Value>` to Rust text-query input, default empty. RPC `semantic.db.query` accepts `{query,format,scope_id?,params?}`; JS `sql(query, {params?: Record<string,SemanticValue>, scopeId?, signal?})`.

```ts
client.sql("SELECT id FROM default WHERE type = :kind AND name = :name", {
  params: { kind: "example:person", name: input }
});
```

Names are case-sensitive `[A-Za-z_][A-Za-z0-9_]*`; map keys omit `:`. Repeated names share one value. Tagged encoding preserves value types. Reject missing, unused, invalid names and non-SQL bindings with `query_parameter_error {reason,name?}`. Bind expression values only; never identifiers.

sqlparser 0.61 already parses `:name` as `Value::Placeholder`; Semantic `parse_literal` currently rejects it (`enabled.rs:2384`). Thread `Bindings {values, used_names}` through lowering, replacing placeholders with typed literals. Validate original token spans to reject quoted/numeric names. No textual substitution.

Errors enumerate `reason: missing|unused|invalid_name|unsupported_format|invalid_position`. Gate: HTTP tests cover missing/extra/repeated names, own-property lookup, quotes/comments, `::` casts, invalid syntax, identifier rejection, typed values and unchanged no-parameter queries.

## 2. Batch returning — Query/API, SDK

Add Rust `execute_batch_returning(Batch, BatchReturn) -> Result<BatchReply, DbError>`; existing `execute_batch` remains dataset-returning. `BatchReply` has matching Rust enum variants; the RPC serializer emits the unwrapped shapes below. Define:

```text
BatchReturn = Dataset | Stats | Changes | Projection { fields: Vec<String> }
RPC returning = "dataset" | "stats" | "changes" | {"projection":{"fields":["name"]}}
JS batch(operations, {returning?, scopeId?, signal?})

Dataset reply = existing {dataset,stats}
Stats reply   = {stats}
Changes reply = {stats,changes:[{collection,id,kind:"upsert"|"delete"}]}
Projection   = Changes reply + {rows:[{collection,id,object}]}
```

Omitted mode means dataset. JS overloads require mode-specific fields. Changes are net before/after differences, sorted by collection/ID; absent→absent and equal→equal disappear. Statistics retain operation-count semantics. Project only final surviving changed rows; identities remain outside projected objects. Resolve field names canonically; missing fields are omitted, duplicate fields rejected. Errors: `batch_return_error {reason: unknown_mode|unknown_field|duplicate_field,field?}`; an empty projection is valid and returns identity-only records.

Capture output from the committing change set. Gate: repeated-ID operations, no-ops, deletion and projection tests; compact response size stays constant as unrelated data grows.

## 3. Shared HTTP and streaming — SDK

```ts
interface HttpOptions {
  fetch?: typeof fetch; headers?: HeadersInit;
  credentials?: RequestCredentials; signal?: AbortSignal; fileEndpoint?: string;
}
declare function createHttpClient(rpcEndpoint: string, options?: HttpOptions): {
  client: SemanticClient; files: FileClient;
};
interface RequestOptions { scopeId?: string; signal?: AbortSignal }
interface FileReadOptions extends RequestOptions { offset?: number; size?: number }
interface FileStreamResult {
  stream: ReadableStream<Uint8Array>; status: number;
  contentType?: string; contentLength?: number;
  contentRange?: string; totalSize?: number;
}
interface FileClient {
  readStream(id: string, options?: FileReadOptions): Promise<FileStreamResult>;
}
```

Add optional `{signal}` argument to `RpcTransport.invoke`. Share fetch/base headers/credentials; operation headers override protocol-specific values. Combine connection/request signals. Preserve old constructors and buffered `read`, implemented by consuming `readStream`.

Server already streams (`server/src/file.rs:158`); no Rust read API change. Preserve existing range validation: zero-size avoids fetch; 416 returns empty stream/status. Validate headers before exposing body and byte count at EOF. Cancellation cancels/releases the body; WebSocket cancellation removes only the local pending request. Invalid ranges use `RangeError`; malformed/truncated responses use `TransportError`; cancellation exposes `AbortError`.

Gate: authenticated RPC/upload/download parity, first-chunk consumption before EOF, cancellation and malformed-range tests. A 1-GiB streaming fixture using 64-KiB chunks permits at most two prefetched chunks; buffered `read` remains explicitly whole-content.

## 4. Incremental writes — Storage

Today `transact_with_options` → `persist_dataset_delta` clears changed collections and resets indexes; `lower_write_ops` expands clears into per-key deletes. All rows are reinserted, and `rebuild_relationship_edges` rebuilds every relation (`embedded/db.rs:1031,1565,1809`; `db_kv/storage.rs:444`). One-row edits therefore write collection-sized data.

No public API changes. Define `ChangeSet = BTreeMap<EntityKey, RowChange>`, `EntityKey=(collection,id)`, `RowChange={before:Option<Object>,after:Option<Object>}`. Normalize first; remove equal pairs. Add `DeleteEntity` and `UnindexEntity(index,id,old_object)` storage operations. One shared `index_keys(index,id,object)` computes old/new key-set differences, preserving index markers.

Maintain contributors keyed `(relation,source_record,source,target)` and counts keyed `(relation,source,target)`. Apply old/new contribution differences; create/delete direct edges only on count transitions 0↔positive. Scalar edits produce no edge changes. Recompute only affected transitive relations and diff results. Commit objects, indexes, contributors and edges together; retain clear/reset for DDL rebuilds.

Gate: one-row edits issue no collection clear/index reset; duplicate contributors, unique-key swaps, reopen and failed-commit tests pass on memory/redb/PostgreSQL.

## 5. Compact ID execution — Storage

Phase 4 above removes excess writes but still scans collections, clones datasets and validates every row (`load_dataset_for_batch:1531`, `execute_batch_with_prepare` in `query.rs:1257`). Growing append workloads remain linear in unrelated data.

No public API changes. Implement snapshot-bound `TxView`; lookup methods return `Result<...,DbError>`:

```text
get(EntityKey) -> Option<Object>                 // overlay before snapshot
unique(index,key) -> Vec<EntityKey>             // overlay-adjusted
incoming(target) -> Vec<(owner,field_path)>      // overlay-adjusted
put(EntityKey,Object); delete(EntityKey)
changes() -> ChangeSet
```

Execute ID operations sequentially into the overlay; validate final changed rows, outgoing targets, unique keys and surviving incoming dependents of deleted/retyped targets. Maintain reverse-reference entries from the validator's resolved references in the same commit. Generate persistence/returning directly from `changes()`. Preserve snapshot/catalog retry behavior.

Use this path for compact ID batches; full-dataset output and predicate mutations retain the existing executor. Unsupported dependency shapes use an explicit counted scan fallback. Gate: ordinary append reads only addressed rows, referenced targets and relevant indexes; compare results/errors with the dataset executor, including dangling-reference rejection.

## 6. Validation and endpoints — Validation

Add `validate(value,type,path,TxView) -> Result<Vec<ResolvedReference>,ValidationError>`, where `ResolvedReference={owner:EntityKey,path:FieldPath,target:EntityKey}`; error `validation_failed {class,attribute,path,rule,expected,actual}`. Apply defaults/optional-null pruning first. Enforce required stored attributes, excluding computed fields; recursively validate enum/union, records/classes, lists/arrays/tuples/maps and references. Try all union alternatives.

Initial constraints: Min/Max, Length, Pattern, Prefix/Suffix, Min/MaxItems, Min/MaxProperties, RequiredFields and scalar ForeignKey. Explicitly reject unsupported enforcement with `unsupported_constraint {kind}`. Collect global, class-attribute and inherited `ClassConstraint::Field` constraints.

Use existing endpoint constraints:

```json
{"foreign_key":{"to":{"name":"example:Person","args":[]},"fields":["id"]}}
```

On a scalar attribute this means target primary-ID reference, accepting subclasses. Resolve target classes/aliases during registration; reject unknown targets/non-ID tuples. Enforce on builtin relation endpoints despite their current ref-loop exemption. Existing typed refs and unconstrained endpoints retain behavior. Existing class builders/`upsert_class` migrations carry these declarations.

Gate: nested/union refs, aliases/inheritance, builtin endpoints, forward targets, deletion/retyping and atomic-rejection tests. Provide validation preflight; activate stricter enforcement only after reported legacy violations are repaired.

## 7. Create-only files and deletion — Files, Storage

Ordinary upload must never replace existing identity or metadata. Existing `FileService::create` calls upserting `db.insert`; replace publication with `BatchOperation::Create {collection,id,object}` in core/public batch types (`kind:"create"` on RPC). Check absence inside transaction/overlay; duplicate returns `entity_exists {collection,id}`, mapped to file `file_already_exists`/409. Create participates in compact execution and increments existing `stats.upserted`; ordinary insert/upsert remain unchanged. Automatic hash-ID collisions also return 409.

Persist new content under immutable hash locators, never caller-ID locators; reject overwriting a live custom locator. Failed publication leaves reclaimable orphan bytes. Preserve existing locator reads and the separate importer publication contract.

Add Rust `FileService::delete(&self, ctx: &AppRequestContext, scope: Option<DbScopeId>, id: String) -> Result<(),AppError>`, HTTP `DELETE {file_api_prefix}/{id}?scope=...`, JS `files.delete(id,options):Promise<void>`. Return 204 if absent. Reject surviving enforced references with `file_referenced {id}`/409. Delete metadata and create a native cleanup record `Cleanup {store:String,locator:String,not_before:DateTime,attempts:u32,last_error:Option<String>}` atomically. Changing/deleting domain `file_id` links never triggers native deletion.

Worker rechecks live records across every scope sharing the store under an App-owned keyed async mutex `(store,locator)`, also acquired for publication. Enable destructive GC only for stores exclusively managed by that App; otherwise retain bytes. After retention, delete unreferenced bytes and cleanup entry; missing bytes count as success. Persist failures; retry after `min(3600,2^min(attempts,12))` seconds. Configure `retention_seconds` before enabling GC. Sweep orphans only during upload-quiescent maintenance after that retention, with dry-run mode. Gate: duplicate uploads leave original bytes/metadata untouched; shared/unlinked files survive; crashes/retries and reference-only deletion pass.

## Delivery gates

Order: fixtures → 1/2/3 → 4 → 5+6 → 7 cleanup; create-only upload can land after fixtures. Old requests keep dataset returning, empty SQL bindings and existing constructors/read behavior. Duplicate upload rejection is a documented behavior change. Backfill contributor/reverse-reference indexes with a persisted completion marker before enabling fast execution. Run validation preflight before enforcement; enable GC after configured retention and shared-store tests.

Benchmark 1k/10k/100k default records: append, edit, membership removal, dependent delete. Record visited rows, writes, response bytes, CPU, memory and latency. Require constant unrelated-data operation counts for compact append. Run backend parity, SDK/generated-output checks, and repository-required Rust checks/tests/formatting through Nix when available.
