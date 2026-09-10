# TODO

## Permissions

- Integrate plugin installation, enablement, configuration, and invocation with the future permission system. Until that system exists, these operations are available to all users; per-scope runtime isolation still applies. Define separate permissions for installing artifacts/schemas, activating plugins in a scope, changing configuration or secrets, and invoking exported interfaces.
- Add WSS/TLS, authentication, and transport identity for remote WebSocket plugins, including a decision among service credentials, mTLS, and delegated user identity. Initial remote transport is unauthenticated `ws://` over HTTP upgrade.
- Add an application-level secret provider so plugin configuration can persist references rather than raw secrets.
- Define and implement sandboxing for untrusted plugins, including filesystem, network, environment, resource, and subprocess restrictions. Process separation alone is not a security boundary.

## Jobs

- Qualify the generic jobs system with the plugin/importer adapters in phases J5/J6 of [the phased plan](docs/plans/2026-09-10-jobs-system/implementation-plan.md). The native runner, scope store, controls, and Jobs view are implemented; plugin/import integration follows separately.
- Include job cancellation, history clearing, and direct jobs-collection editing in the future permission system.
- Beyond the initial configurable concurrent-jobs limit, add resource controls as justified by use: stream item/byte buffering, per-plugin invocation admission, file-size limits, timeouts, cancellation grace periods, bandwidth/duration limits, and measured defaults.

## Plugin/import follow-ups

- Implement the plugin/import system according to its revised [design](docs/plans/2026-09-09-plugin-system/design.md) and [phased plan](docs/plans/2026-09-09-plugin-system/implementation-plan.md).
- When generic entity versioning exists, allow importer configuration and individual imports to request it by delegating to that system. Until then, imports use normal upsert/replace for matching source identities. Never implement importer-specific revision entities or manual version chains.
