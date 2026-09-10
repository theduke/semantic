# semantic_import

Streaming source discovery, fetch, and ordinary entity/File import. The canonical
declarations live in `semantic_data::import::package()` (`semantic.import/v1`).
Plugins implement `Source`, `Fetcher`, and/or `Importer` through the shared
`InterfaceImplementation`; there is no parallel importer trait hierarchy.

`fetch` returns an owned content stream and never uses an `ImportWriter`.
`ImportJobHandler` runs on the ordinary `semantic_jobs` coordinator. Its input
owns the selected generation, source request or supplied stream, and scope-bound
writer. None of these working values belong in persistent job records.

Content is serial: entities or `FileStart`, zero or more `FileBytes`, and
`FileEnd`, followed by one summary. The validator rejects interleaved or
incomplete files, incorrect declared lengths, missing terminals, and inaccurate
summaries. File bytes are forwarded incrementally. `ImportWriter` validates and
publishes domain entities; its `FileWrite` prepares bytes and publishes only on
`finish`. Incomplete files call `abort`. Cancellation prevents admission of the
next write, while admitted writes finish and prior successful items remain.

`stable_entity_id` hashes length-delimited namespace, source identity and item
key with a versioned domain separator. Content hashes, jobs, and plugin revisions
do not affect entity identity. The app writer uses content-addressed blob
locators independently of these stable entity IDs.

The built-in `GenericUrlPlugin` is an ordinary Rust plugin at priority -100. It
uses one streaming HTTP GET and the HTTP client's normal redirects and
decompression. HTML/XHTML, event streams and multipart responses are rejected.
Unknown or octet-stream MIME requires a recognized extension. Original requested
URLs supply identity; redirects do not change that identity. Fetching accepts
HTTP and HTTPS independently of the plugin provider's `ws://` policy.

Run the custom Rust factory example without database/blob writes:

```sh
nix develop --command cargo run -p semantic_import --example rust_url_plugin -- \
  https://example.com/document.pdf
```

The example constructs descriptors from the canonical declaration, wraps the
generic reader in a custom `Plugin` factory, registers it, creates a scoped
instance and consumes the fetch stream. Application embedding normally calls
`SemanticApp::builder().register_plugin(plugin)` and lets scope activation own
creation, conformance and cancellation.

Reqwest's inherited redirect policy and response buffers still apply. This crate
adds no timeout, size/bandwidth limit, durable content cache, retry, checkpoint,
version history, or whole-import transaction. Extended Content-Disposition
filename encodings are not currently decoded.
