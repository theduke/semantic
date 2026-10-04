# semantic_vdb

Read-only plugin collections exposed through `semantic.vdb/v1/VirtualDatabase`.
Implement `VirtualDatabase`, return canonical entities, and wrap an async factory
with `VirtualDatabasePlugin::new`. The host negotiates scan pushdown and executes
filters, joins, aggregates and pagination that the plugin does not implement.

The descriptor declares only the types, attributes, classes and relationships
visible through the collection. These definitions stay in memory and do not
install a package or create a migration. Increment `schema_revision` whenever the
exposed schema changes.

- [Plugin-author guide](../../docs/plugin-virtual-databases.md): contract,
  minimal Rust implementation, negotiation and lifecycle rules.
- [JSON directory example](examples/json_dir_vdb.rs): one entity per file,
  runnable in process or through the stdio provider.
- The `testing` feature exposes `FixtureVdb`, configurable negotiation modes,
  schema mutation handles and cancellation counters for integration tests.

Build and run the example from the workspace root:

```sh
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
nix develop -c cargo build -p semantic_vdb --example json_dir_vdb --quiet --message-format=short
"$CARGO_TARGET_DIR/debug/examples/json_dir_vdb" ./documents
"$CARGO_TARGET_DIR/debug/examples/json_dir_vdb" ./documents --activation fx > /tmp/fx.json
```

When developing in a worktree, set `CARGO_TARGET_DIR` to the main clone's target
directory before running these commands.
Configure the generated activation against a running Semantic server with
`semantic api plugin configure /tmp/fx.json`, then query it with
`semantic api query 'SELECT * FROM fx ORDER BY id'`.
