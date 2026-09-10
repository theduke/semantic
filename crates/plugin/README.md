# semantic_plugin

A Rust `Plugin` exposes a manifest and asynchronous per-scope instance factory.
Instances implement the same `semantic_rpc::interface::InterfaceImplementation`
contract used by stdio and WebSocket peers. Immutable bindings retain their
activation generation, cancellation and generic jobs group.

Register Rust implementations with `PluginRegistry::register`, or use
`SemanticAppBuilder::register_plugin` for application embedding. Implementations
declare canonical package/module/interface references, exact package versions,
and fingerprints. The application resolves these against the scope catalog
before publishing a conforming instance. See the executable custom factory
example in `../import/examples/rust_url_plugin.rs`.

Provider features are optional: `stdio` enables a Tokio child process and
`websocket` enables plain remote `ws://`. Rust-only callers need neither.
Unavailable compiled providers remain inspectable but cannot activate. The app
crate enables its corresponding provider features by default.

An activation descriptor contains:

```json
{
  "id": "example.document-importer",
  "revision": "1",
  "provider": {
    "kind": "stdio",
    "program": "/absolute/path/to/importer",
    "args": [],
    "cwd": null,
    "env": {}
  },
  "enabled": true,
  "generation": 1,
  "configuration": null,
  "priority": 10,
  "exports": [
    {
      "export": "source",
      "package": "semantic.import",
      "module": "v1",
      "contract": null,
      "interface": "Source",
      "package_version": "1.0.0",
      "fingerprint": "REPLACE_WITH_CANONICAL_FINGERPRINT"
    }
  ]
}
```

This is a descriptor template, not a runnable importer: include every actual
export and generate its fingerprint from the matching package declaration.
Use `{"kind":"rust","key":"registered.factory.id"}` for a registered Rust
factory, or `{"kind":"websocket","url":"ws://host:port/path"}` for a remote
provider. Configure with `semantic api plugin configure descriptor.json` and
inspect desired settings/live readiness with `semantic api plugin list`.

Stdio reserves stdout for Content-Length-framed protocol traffic; diagnostic
stderr is drained separately. WebSocket peers negotiate `semantic.plugin.v1`
and exchange one JSON envelope per text message. Handshake checks revision,
exports, profile and fingerprints before readiness. The SDK dispatch helper in
`semantic_rpc::plugin` uses the same invocation/session protocol.

Configuration changes and disable close old admissions, cancel its jobs/direct
streams, and wait for handlers/provider cleanup before replacement. Cooperative
Rust cleanup or a child that does not exit can keep a generation stopping;
there is no forced teardown deadline. Stdio reaps the direct child, without an
OS-specific descendant guarantee. A closed remote stream does not prove the
remote service stopped.

There is no plugin authentication, WSS/TLS, sandbox, secret provider,
auto-reconnect/replay, executable installation, Wasmer dependency or registry.
Installed plugins are trusted code. Provider failures require a fresh user
operation/activation; they never silently replay imports.
