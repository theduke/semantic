# @semantic/sdk

TypeScript definitions, package/mutation builders, code generation, exact RPC value codecs, and HTTP/WebSocket/file clients for Semantic.

The SDK supports modern browsers and Node.js 20 or newer. Its main entry point is
browser-safe and has no Node-only imports. Both environments must provide the
standard Fetch API; Node 20 includes it. Browsers provide `WebSocket` globally.
Node versions without a global WebSocket can inject any compatible implementation:

```ts
import { WebSocketTransport, type WebSocketFactory } from "@semantic/sdk";

const createSocket: WebSocketFactory = (url) => new MyWebSocket(url);
const transport = new WebSocketTransport(
  "ws://localhost:8888/api/v1/rpc",
  createSocket,
);
```

Streaming `ReadableStream` uploads work with both browser fetch and Node's native
fetch; the SDK sets the request's `duplex: "half"` option in both environments.

```ts
import { HttpTransport, SemanticClient, BatchBuilder } from "@semantic/sdk";

const client = new SemanticClient(
  new HttpTransport("http://localhost:8888/api/v1/rpc"),
);
await client.batch(
  new BatchBuilder().upsert("post-1", { title: "Hello" }, "posts").build(),
);
const result = await client.sql("select * from posts");
```

RPC uses Semantic's externally-tagged value encoding. Integers outside JavaScript's safe range decode as `bigint`; `stringifyJson` emits bigint as an exact JSON number. Use the `value` helpers when an exact integer width or scalar tag matters, and `decodeTaggedExact` when a decoded value must preserve those tags.

The three JSON domains are explicit: `rpcJson` handles tagged RPC envelopes, `packageJson` handles package documents with embedded Facet `Value` fields, and `flatJson` handles untagged file metadata.

Regenerate the checked-in core, base, and filestore declarations from Rust reflection and canonical package values with:

```sh
npm run generate
npm run generate:check # CI/staleness check
```

Generate declarations from any exported package document with:

```sh
npm run generate:package -- package.json generated.ts
# or, after installation:
semantic-ts-generate package.json generated.ts
```

The package generator traverses root and nested modules, type definitions, attributes, classes, generics, references, and contract functions. Generated command descriptors can be passed to `client.invoke()`.

The complete Rust-reflected core wire schema is exposed without colliding with
the ergonomic SDK types:

```ts
import type { CoreSchema } from "@semantic/sdk";
import type { Package, expression_Expr } from "@semantic/sdk/core";

const pkg: CoreSchema.Package = /* ... */;
```
