# `@semantic/embedded`

Run Semantic inside Node.js through the Rust native addon, without an HTTP server.

```ts
import { openEmbedded } from "@semantic/embedded";
const semantic = await openEmbedded({ dataDir: "./semantic-data" });
try {
  await semantic.client.insert("example", {
    id: "example",
    "semantic:title": "Embedded Semantic",
  });
  console.log(await semantic.client.get("example"));
} finally {
  await semantic.close();
}
```

`dataDir` is required and relative paths are resolved once. `dbUri` and `blobUri`
support the CLI storage targets (`redb:`, `logfs:`, `log:`, `fs://`, and
`logfs://`). An explicit database URI wins. `blobPassword` is accepted only for
a `logfs://` blob URI; the binding never prompts or reads environment fallback.

Always await the owner's `close()` before process exit. It is the durability and
resource-release boundary. The file API buffers complete values (64 MiB by
default), accepts `Uint8Array`, `ArrayBuffer`, `Blob`, or strings, and returns
`Uint8Array`. It intentionally has no `url()` because no HTTP listener exists.

This package is Node-only and platform-specific. Keep it external in bundlers.
It never downloads binaries or silently falls back to a remote server.
