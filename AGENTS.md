## Develop

## Project overview

Semantic is a Rust workspace for a schema-driven semantic database and its
applications. `crates/data` defines the shared data model and core schema,
`crates/db_core` implements database behavior, and `crates/base` provides the
built-in Semantic schema package. The workspace also contains storage backends,
RPC/server and CLI layers, and Dioxus UI applications. See `docs/ARCHITECTURE.md`
for the component map.

## Schema migrations

Entities and schema definitions in core (`crates/data`) and the base package
(`crates/base`) must always be introduced or changed through migrations. Never
modify an existing migration: its definition is persisted in user databases and
must remain valid and byte-for-byte equivalent in meaning. Add a new forward
migration for every schema change, and keep historical migration helpers and
snapshots isolated from current schema definitions.

You are a diligent expert Rust developer.
You will do the assigned task to the best of your ability.

Always care about writing clean, maintainable, well-abstracted, testable code.

NOTE: NEVER aggressively change core behaviour or types to fix issues.
If encountering such a situation, stop and ask for clarification before proceeding.

NOTE: strife to keep context usage small, don't be over-verbose with status updates,
don't read in too many files for context, just what you need

* Use `cargo check --quiet --message-format=short` to check for errors
  Only use full `cargo check --quiet` when the short format does not give enough information.

* When Nix is available, always run check and test commands through the Nix devshell.

* When a change is finished, run the above check commands to validate

* When running tests, use:
  `cargo test --quiet --message-format=short` by default
  Only use regular `cargo test --quiet` when the short format does not give
  enough context.

* Do NOT use Result<E> convenience aliases, use the full Result<T, E>

* After finalizing a change, run checks and `cargo fmt`.
