# `@semantic/sdk`

TypeScript types, builders, code generation, value codecs, and HTTP, WebSocket,
and file clients for [Semantic](https://github.com/theduke/semantic).

The SDK has two layers:

- An ergonomic client layer for application code: `SemanticClient`, transports,
  file operations, package builders, and JavaScript-friendly values.
- Schema tooling for package authors: complete package/query/type definitions,
  generated definitions for Semantic's built-in packages, and a package-to-TypeScript
  generator.

The main package and the programmatic generator work in modern browsers and in
Node.js 20 or newer. The command-line generator and repository regeneration script
are Node-only.

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Runtime support](#runtime-support)
- [Transports](#transports)
- [Queries and mutations](#queries-and-mutations)
- [Commands and scopes](#commands-and-scopes)
- [Files](#files)
- [Values and serialization](#values-and-serialization)
- [Defining packages](#defining-packages)
- [Generating TypeScript](#generating-typescript)
- [Built-in generated types](#built-in-generated-types)
- [Errors](#errors)
- [Current limitations](#current-limitations)
- [Package exports](#package-exports)
- [Development](#development)

## Install

```sh
npm install @semantic/sdk
```

The package is ESM-only and includes its TypeScript declarations. There is no
default export.

## Quick start

```ts
import { HttpTransport, SemanticClient } from "@semantic/sdk";

interface Post {
  title: string;
  published: boolean;
  views: number | bigint;
}

const client = new SemanticClient(
  new HttpTransport("http://localhost:8888/api/v1/rpc"),
);

await client.insert<Post>(
  "post-1",
  { title: "Hello, Semantic", published: true, views: 0 },
  { collection: "posts" },
);

const result = await client.sql<Post>(
  "SELECT title, published, views FROM posts",
);

if (result.kind === "select") {
  for (const post of result.rows) console.log(post.title);
}
```

Generic result types describe the object stored inside an entity or returned as a
query row. They provide static checking only; the SDK does not validate server data
against them at runtime.

## Runtime support

The package targets ES2022 and requires:

- Node.js 20 or newer, or a modern browser.
- The Fetch API (`fetch`, `Headers`, `Request`, and `Response`) for HTTP and file
  operations. Node.js 20 provides it globally.
- `structuredClone` when using builders or package serialization.
- A global `WebSocket`, or an injected compatible implementation, for WebSocket
  RPC.

The main `@semantic/sdk` entry point and `@semantic/sdk/generator` contain no
Node-only imports. Only the `semantic-ts-generate` CLI and the repository's
regeneration script use Node filesystem and process APIs.

Browser applications must be able to reach the Semantic server; normal browser
origin, CORS, mixed-content, and credential rules still apply.

## Transports

`SemanticClient` operates on the small `RpcTransport` interface. Use a built-in
HTTP or WebSocket transport, or provide your own implementation.

### HTTP

HTTP is the simplest choice for ordinary request/response use. Headers and an
abort signal can be configured on the transport and apply to every invocation.
Each operation also accepts `signal`; either signal can cancel that operation.
The SDK imposes no timeout policy.

```ts
import { HttpTransport, SemanticClient } from "@semantic/sdk";

const controller = new AbortController();
const transport = new HttpTransport("https://semantic.example/api/v1/rpc", {
  headers: {
    authorization: `Bearer ${accessToken}`,
    "x-client-name": "example-web-app",
  },
  signal: controller.signal,
});
const client = new SemanticClient(transport);

const pending = client.sql("SELECT * FROM notes");

// Cancels this and any other in-flight request using this transport. Once the
// signal is aborted, construct a new transport before making more requests.
controller.abort();
await pending.catch(() => undefined);
```

You can inject a Fetch-compatible function with the `fetch` option, which is useful
for testing or centrally adding credentials and observability.

### WebSocket

WebSocket RPC keeps one connection open and correlates concurrent responses by
request ID. `invoke()` waits for the socket to open, so awaiting `ready` is optional
but useful when connection failure should be detected before other work starts.

```ts
import { SemanticClient, WebSocketTransport } from "@semantic/sdk";

const transport = new WebSocketTransport("wss://semantic.example/api/v1/rpc");
await transport.ready;

const client = new SemanticClient(transport);
const [notes, people] = await Promise.all([
  client.sql("SELECT * FROM notes"),
  client.sql("SELECT * FROM people"),
]);

client.close();
```

Browsers provide `WebSocket` globally. Runtimes without one must inject a factory:

```ts
import {
  WebSocketTransport,
  type WebSocketFactory,
  type WebSocketLike,
} from "@semantic/sdk";

const createSocket: WebSocketFactory = (url) =>
  new MyWebSocketImplementation(url) as WebSocketLike;

const transport = new WebSocketTransport(
  "ws://localhost:8888/api/v1/rpc",
  createSocket,
);
```

The injected object must expose `readyState`, `send`, `close`, and
`addEventListener`. This also lets Node applications configure connection details
supported by their chosen WebSocket implementation. The SDK does not bundle one.

WebSocket transport does not currently expose per-call cancellation or a built-in
headers option.

### Custom transports

```ts
import {
  SemanticClient,
  type RpcTransport,
  type SemanticValue,
} from "@semantic/sdk";

const transport: RpcTransport = {
  async invoke(
    command: string,
    payload: SemanticValue,
  ): Promise<SemanticValue | undefined> {
    // Send command and payload through an application-specific channel.
    return undefined;
  },
};

const client = new SemanticClient(transport);
```

Custom transports receive decoded `SemanticValue`s. The built-in transports handle
RPC envelopes and tagged-value encoding themselves.

## Queries and mutations

SQL accepts named expression parameters. Names are case-sensitive ASCII identifiers;
map keys omit the colon. Repeated names reuse the same typed value.

```ts
await client.sql("SELECT id FROM default WHERE name = :name", {
  params: { name: userInput },
});
```

Parameters use the tagged value codec, including UUIDs, bytes and wide integers.
Bindings cannot replace identifiers. Missing, unused, invalid and non-SQL bindings
return `RpcError` with code `query_parameter_error` and `data.reason` (plus `data.name`
when applicable). Quoted SQL text and comments do not introduce parameters.

### SQL and PRQL

`sql()` and `prql()` are conveniences around `query()`. All accept an optional
`scopeId`.

```ts
const sqlResult = await client.sql<{ id: string; title: string }>(
  "SELECT id, title FROM notes",
  { scopeId: "workspace-1" },
);

const prqlResult = await client.prql<{ title: string }>(
  "from notes | select {title}",
);

const sameAsSql = await client.query("SELECT * FROM notes", {
  format: "sql",
});
```

`QueryResult<T>` is a discriminated union. Check `kind` before accessing
operation-specific fields such as `rows`, `returning`, `inserted`, or `deleted`.
Counts may be `bigint` when they exceed JavaScript's safe integer range.

### Get, insert, and delete

```ts
interface Note {
  title: string;
  note_content: string;
  note_format: "text" | "markdown";
}

await client.insert<Note>(
  "note-1",
  {
    title: "SDK notes",
    note_content: "Typed from end to end",
    note_format: "markdown",
  },
  { collection: "notes", scopeId: "workspace-1" },
);

const entity = await client.get<Note>("note-1", {
  collection: "notes",
  scopeId: "workspace-1",
});

if (entity) console.log(entity.id, entity.collection, entity.object.title);

await client.delete("note-1", {
  collection: "notes",
  scopeId: "workspace-1",
});
```

The collection is optional for these methods and is omitted from the RPC payload
when not supplied.

### Batch operations

`BatchBuilder` takes snapshots of supplied objects and of the completed operation
list, so later mutation of inputs does not modify an already built batch.

```ts
import { BatchBuilder } from "@semantic/sdk";

const operations = new BatchBuilder()
  .upsert("note-1", { title: "First" }, "notes")
  .upsert("note-2", { title: "Second" }, "notes")
  .delete("obsolete-note", "notes")
  .deleteMany(["draft-1", "draft-2"], "notes")
  .build();

const outcome = await client.batch(operations, { scopeId: "workspace-1" });
console.log(outcome.stats.upserted, outcome.stats.deleted);
```

By default the result contains `{ dataset, stats }`. Select a compact response
with `returning`; its literal value determines the TypeScript result type:

```ts
const { stats } = await client.batch(operations, { returning: "stats" });
const { changes } = await client.batch(operations, { returning: "changes" });
const { rows } = await client.batch(operations, {
  returning: { projection: { fields: ["title"] } },
});
```

`changes` contains `{ collection, id, kind: "upsert" | "delete" }`, sorted by
collection and ID. It describes net committed changes, so unchanged rows and
create-then-delete sequences are absent. Stats retain operation counts. Projection
also includes changes and returns `{ collection, id, object }` for surviving changed
rows, with only selected canonical attribute keys; absent values are omitted.
An empty field list returns identities with empty objects. Unknown/ambiguous fields,
duplicate fields (including aliases), and invalid modes fail before commit with
`batch_return_error` and `data.reason` (`unknown_field`, `duplicate_field`, or
`unknown_mode`). Explicit `"dataset"` retains the default shape.
`commands.batch` keeps its dataset-only contract; `commands.batchReturning` accepts
all modes and returns the reply union.

The remotely callable `BatchOperation` deliberately includes only the operations
accepted by the current application RPC command: create, upsert, delete by ID, and delete
by multiple IDs. The reflected core schema also describes query-based update and
delete variants, but `SemanticClient.batch()` does not expose them as supported
remote operations.

## Commands and scopes

`command<P, O>()` creates a typed command descriptor. It has no runtime schema or
validation; it associates a command name with compile-time payload and output types.

```ts
import { command } from "@semantic/sdk";

const greet = command<{ name: string }, { message: string }>("example.greet");
const response = await client.invoke(greet, { name: "Ada" });
console.log(response.message);
```

Generated package commands use the same descriptors. The main entry point also
exports descriptors for the built-in server commands as `commands`.

```ts
import { commands } from "@semantic/sdk";

const scope = await client.invoke(commands.scopeOpen, {
  uri: scopeUri,
  mode: "auto_create",
  set_current: false,
});

const notes = await client.sql("SELECT * FROM notes", {
  scopeId: scope.scope_id,
});
```

Explicit `scopeId` options are available on all high-level database methods,
package upserts, and file operations. The `commands` object additionally exposes
`scopeOpen`, `scopeUse`, `scopeCurrent`, and `scopeList` for lifecycle management.
Which scope URI schemes and visibility modes are usable depends on the server.

## Files

Uploads are create-only: an existing explicit or automatic hash identity returns
`RpcError` with code `file_already_exists`. Native bytes use immutable generated
hash locators; custom upload locators are rejected, while existing locators remain
readable. The separate importer publication operation retains upsert behavior.

Use `await files.delete(id, { scopeId, signal })` for explicit native deletion.
It succeeds if already absent and returns `file_referenced` when an enforced
reference survives. Removing a domain file link does not delete the native file.
Deletion atomically records cleanup intent with metadata removal. Bytes are
currently retained: destructive cleanup is disabled until physical store
ownership across scopes and a retention policy can be guaranteed.

File transfer uses a separate HTTP endpoint and `FileClient`. Use
`createHttpClient` to share Fetch, base headers, credentials, and a connection
abort signal across RPC, upload, and download. Protocol headers for each operation
override base headers. Set `fileEndpoint` if the file endpoint cannot be derived.

```ts
import { createHttpClient } from "@semantic/sdk";

const rpcEndpoint = "https://semantic.example/api/v1/rpc";
const { client, files } = createHttpClient(rpcEndpoint, {
  headers: { authorization: `Bearer ${accessToken}` },
  credentials: "include",
});

const uploaded = await files.upload({
  content: new Blob(["Hello from Semantic\n"], { type: "text/plain" }),
  filename: "hello.txt",
  mimeType: "text/plain",
  scopeId: "workspace-1",
  entity: { title: "Greeting", description: "Created by the SDK" },
  onProgress(uploadedBytes, totalBytes) {
    console.log(uploadedBytes, totalBytes);
  },
});

const complete = await files.read(uploaded.id, { scopeId: "workspace-1" });
const firstKilobyte = await files.read(uploaded.id, {
  scopeId: "workspace-1",
  offset: 0,
  size: 1024,
});
const downloadUrl = files.url(uploaded.id, "workspace-1");

// Bounded-memory consumption; cancellation also releases the response body.
const controller = new AbortController();
const { stream, contentLength, totalSize } = await files.readStream(
  uploaded.id,
  {
    scopeId: "workspace-1",
    signal: controller.signal,
  },
);
await stream.pipeTo(destination);
```

`content` accepts any Fetch `BodyInit`, including `Blob`, `Uint8Array`, and
`ReadableStream`. For a stream, the SDK sets `duplex: "half"`, as required by
Node's native Fetch implementation and accepted by supporting browsers.

`onProgress` currently reports upload lifecycle boundaries (start and completion),
not incremental network byte progress. `read` buffers the entire response;
`readStream` returns a `ReadableStream<Uint8Array>` with status and optional
content type, content length, content range, and total size. A zero-sized read
returns empty content without a request (synthetic stream status `200`); `416`
also returns empty content. Ranged reads require a valid `206` response and
`Content-Range`. Headers are validated before exposing bytes and declared byte
counts are checked at EOF; malformed or truncated responses raise `TransportError`.
Cancel the stream, its reader, or an associated signal when abandoning a download.
Signal cancellation rejects with `AbortError`.

Existing standalone constructors and Fetch wrappers remain supported:

```ts
const authenticatedFetch: typeof fetch = (input, init) => {
  const headers = new Headers(init?.headers);
  headers.set("authorization", `Bearer ${accessToken}`);
  return fetch(input, { ...init, headers });
};

const files = new FileClient(
  "https://semantic.example/api/v1/file",
  authenticatedFetch,
);
```

## Values and serialization

### `SemanticValue`

`SemanticValue` is the JavaScript-facing value model used by entities and custom
commands. It includes ordinary primitives, `bigint`, `Uint8Array`, `Date`, arrays,
objects, `Map`, Semantic variants, and explicit tagged values.

The automatic RPC mapping includes:

| JavaScript value                         | RPC value                        |
| ---------------------------------------- | -------------------------------- |
| `undefined`, `null`, `boolean`, `string` | `void`, `null`, `bool`, `string` |
| Integer `number` or `bigint`             | `i64` by default                 |
| Non-integer `number`                     | `f64`                            |
| `Uint8Array`                             | `bytes`                          |
| `Date`                                   | nanosecond `date_time`           |
| Array, `Map`, plain object               | `list`, `map`, `object`          |
| `{ $variant, value, type? }`             | `variant`                        |

Decoded integers are `number` when safely representable and `bigint` otherwise.
A decoded `date_time` is its numeric nanosecond representation; decoding does not
automatically construct a `Date`.

### Exact scalar tags

JavaScript cannot distinguish a UUID from a string or `u8` from `i64`. Use the
`value` helpers when the exact Semantic scalar kind matters:

```ts
import { decodeTaggedExact, encodeTagged, value } from "@semantic/sdk";

const payload = {
  id: value.uuid("0f8fad5b-d9cb-469f-a165-70867728950e"),
  priority: value.int("u8", 7),
  elapsed: value.durationMs(250),
  state: value.variant("published", null, "PostState"),
};

const tagged = encodeTagged(payload);
const exact = decodeTaggedExact(tagged);
```

`encodeTagged()` and the built-in transports recognize values produced by these
helpers. Integer helpers validate both the declared width and JavaScript safe-number
rules. `decodeTaggedExact()` recursively preserves scalar tags as explicit wrappers;
ordinary `decodeTagged()` returns convenient JavaScript scalars.

### Three JSON domains

Similar-looking JSON appears in three different wire contexts. Use the matching
codec rather than applying one representation everywhere.

| Codec                                     | Use                                                                                                                                                  |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rpcJson` / `stringifyJson` / `parseJson` | Lossless JSON tokens in RPC envelopes. The transport separately applies tagged-value encoding.                                                       |
| `facetValueJson`                          | A standalone externally tagged Semantic value.                                                                                                       |
| `packageJson`                             | Facet package documents; it tags only schema locations that contain Semantic values, such as defaults, examples, expressions, and migration objects. |
| `flatJson`                                | Ordinary untagged JSON used for file entity metadata.                                                                                                |

```ts
import {
  facetValueJson,
  flatJson,
  packageJson,
  parseJson,
  stringifyJson,
} from "@semantic/sdk";

const exactInteger = stringifyJson({ sequence: 9_007_199_254_740_993n });
const parsed = parseJson(exactInteger); // unsafe integer tokens become bigint

const valueDocument = facetValueJson.stringify(value.uuid(uuid));
const packageDocument = packageJson.stringify(pkg);
const metadataDocument = flatJson.stringify({
  title: "Photo",
  taken: new Date(),
});
```

`stringifyJson()` emits `bigint` as an exact JSON number token rather than a quoted
string. `flatJson` turns `Date` into an ISO string and bytes into number arrays; it
rejects explicit tagged wrappers because they have no flat representation.

## Defining packages

The exported interfaces let applications construct a package directly. Builders
fill repetitive container/default fields, reject duplicate definitions, clone
inputs, and return detached snapshots. They do not infer schema IDs or validate a
package against a server catalog.

This example defines a type, attribute, class, contract command, secondary module,
DDL migration, and seed entity:

```ts
import {
  PackageBuilder,
  emptyMeta,
  type Package,
  type TypeNode,
} from "@semantic/sdk";

const stringType: TypeNode = {
  kind: { string: { format: null, normalization: null } },
  constraints: [],
  annotations: [],
};

const pkg: Package = new PackageBuilder("acme.blog", "blog")
  .version({ major: 1, minor: 0, patch: 0, pre: null, build: null })
  .meta({ title: "Acme Blog", description: "Blog schema and commands" })
  .root((module) => {
    module.type({
      name: "Slug",
      module: null,
      params: [],
      ty: stringType,
      visibility: "public",
      meta: emptyMeta(),
    });

    module.attribute("title", {
      id: "acme.blog/title",
      name: "title",
      ty: stringType,
      constraints: [],
      meta: emptyMeta(),
    });

    module.class("Article", {
      id: "acme.blog/Article",
      name: "Article",
      inherits: null,
      extends: [],
      "semantic:class:strict_schema": true,
      attributes: {
        title: {
          attribute: { id: "acme.blog/title" },
          required: true,
          ui_order: 0,
          computed: null,
          constraints: [],
          meta: emptyMeta(),
        },
      },
      constraints: [],
      meta: emptyMeta(),
    });

    module.contract("api", (contract) => {
      contract.command("acme.blog.article.get", {
        params: [{ name: "slug", ty: stringType }],
        results: [stringType],
        throws: null,
        async_fn: true,
      });
    });
  })
  .module("admin", (module) => {
    module.meta({ description: "Administrative definitions" });
  })
  .migration("blog", "0001-create-articles", (migration) => {
    migration
      .description("Create and seed the article collection")
      .operation({
        ddl: {
          upsert_collection: {
            name: "articles",
            kind: "schema",
            integrity_mode: "strict_registered_schema",
          },
        },
      })
      .insert("articles", "welcome", { title: "Welcome" });
  })
  .build();

await client.upsertPackage(pkg, { scopeId: "workspace-1" });
```

`root()` and `module()` configure a `ModuleBuilder`; `contract()` configures a
`ContractBuilder`; and `migration()` configures a `MigrationBuilder`. Lower-level
types such as `TypeNode`, `TypeDef`, `AttributeType`, `ClassType`, `FunctionType`,
`MigrationOperation`, and query AST types are all exported for package authors who
need more control.

## Generating TypeScript

The generator converts a `Package` into declarations for its modules, contracts,
types, attributes, classes, interfaces, and commands. It resolves lexical and
qualified references, supports generics, avoids TypeScript identifier collisions,
and emits class references as entity IDs (`string`) rather than nested objects.

### CLI

```sh
semantic-ts-generate package.json src/generated/acme-blog.ts
```

When working in this repository:

```sh
npm run generate:package -- package.json src/generated/acme-blog.ts
```

The input must be a JSON representation of `Package`; `packageJson.stringify(pkg)`
produces the appropriate Facet package representation. The CLI parses integers
losslessly and formats the generated TypeScript with Prettier.

A generated module exports:

- `packageName`, a string literal containing the package name.
- Type aliases for package declarations and classes.
- Typed command descriptors ready for `client.invoke()`.

Names that are not valid TypeScript identifiers are sanitized. If two declarations
would share a name, the generator qualifies or suffixes them; use the symbols in
the generated output rather than predicting their spelling.

### Programmatic API

The browser-safe generator accepts an in-memory package:

```ts
import { packageModel, renderPackage } from "@semantic/sdk/generator";

const model = packageModel(pkg);
const typescriptSource = renderPackage(model);
```

`packageModel()` produces the normalized `PackageModel` intermediate form;
`renderPackage()` renders that model. The programmatic renderer does not write a
file or run Prettier.

## Built-in generated types

Canonical definitions generated from the Rust source of truth are checked in with
the SDK.

```ts
import type { Person, Note } from "@semantic/sdk/generated/base";
import type { File as SemanticFile } from "@semantic/sdk/generated/filestore";

const person: Person = { display_name: "Ada Lovelace" };
const note: Note = {
  note_content: "Generated types are ordinary TypeScript types.",
  note_format: "text",
};
const file: SemanticFile = { filename: "notes.txt", mime_type: "text/plain" };
```

The main entry point also exposes the exact Rust-reflected core wire schema under a
namespace, while the `core` subpath provides direct imports:

```ts
import type { CoreSchema } from "@semantic/sdk";
import type {
  Package as ExactPackage,
  expression_Expr,
} from "@semantic/sdk/core";

declare const reflected: CoreSchema.Package;
const exact: ExactPackage = reflected;
declare const expression: expression_Expr;
```

Use the main entry point's ergonomic types for normal client/package authoring and
the reflected core types when exact correspondence with Rust's full schema is
important. Some reflected types include server-internal variants that the ergonomic
client intentionally does not accept.

## Errors

```ts
import { ProtocolError, RpcError, TransportError } from "@semantic/sdk";

try {
  await client.sql("SELECT * FROM missing_collection");
} catch (error) {
  if (error instanceof RpcError) {
    console.error(error.code, error.message, error.data);
  } else if (error instanceof TransportError) {
    console.error(error.status, error.message, error.cause);
  } else if (error instanceof ProtocolError) {
    console.error("Malformed RPC response", error.cause);
  } else {
    throw error;
  }
}
```

- `RpcError` represents a structured error returned by the Semantic command and
  exposes `code` and optional decoded `data`.
- `TransportError` represents Fetch failures, non-success HTTP statuses, WebSocket
  failures, and file transfer errors. HTTP status is available when applicable.
- `ProtocolError` represents malformed or inconsistent RPC responses.
- All three extend `SemanticError`, which extends `Error`.

## Current limitations

- The high-level query client sends SQL or PRQL text. Query AST types are available
  for package/schema authors, but there is no high-level AST query method yet.
- Remote batches support create, upsert, and ID-based deletion only.
- Types and generated command descriptors are compile-time contracts; the SDK does
  not perform runtime schema validation.
- The SDK is ESM-only and does not bundle a WebSocket implementation for Node.

## Package exports

| Import                              | Contents                                                                                                                        |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `@semantic/sdk`                     | Ergonomic types, codecs, errors, commands, transports, client, builders, files, built-in command descriptors, and `CoreSchema`. |
| `@semantic/sdk/core`                | Direct exports of the complete Rust-reflected core schema.                                                                      |
| `@semantic/sdk/generator`           | Browser-safe package model and rendering API.                                                                                   |
| `@semantic/sdk/generated/base`      | Generated types for `semantic.base`.                                                                                            |
| `@semantic/sdk/generated/filestore` | Generated types for `semantic.filestore`.                                                                                       |
| `@semantic/sdk/generated/commands`  | Built-in typed command descriptors; these are also exported as `commands` from the main entry point.                            |

The wildcard `@semantic/sdk/generated/*` export also makes additional checked-in
generated modules importable by subpath.

## Development

From `lib/js/sdk`:

```sh
npm ci
npm run build
npm test
npx prettier --check .
```

The checked-in `core.ts`, `base.ts`, and `filestore.ts` files are generated from
Rust reflection and canonical package values through the `semantic_sdk_export`
crate:

```sh
npm run generate        # rewrite generated files
npm run generate:check  # fail if checked-in files are stale
```

Do not edit those generated files directly. `generate` requires a working Rust
workspace in addition to Node dependencies.
