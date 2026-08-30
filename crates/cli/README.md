# Semantic CLI

The `semantic_cli` crate provides the `semantic` command-line application.

## Command architecture

The top-level parser is `cmd::Args`, and its subcommand enum is
`cmd::SubCmd`. Keep both definitions in `src/cmd/mod.rs`.

Each top-level command belongs in its own module under `src/cmd/`. The module
should normally define its command arguments as a namespaced `Args` type and
provide the function that runs the command. For example, the `semantic fuse`
implementation lives in `src/cmd/fuse.rs` as `fuse::Args` and `fuse::run`.

Nested command groups follow the same structure. Put a group in
`src/cmd/<group>/mod.rs`, define its subcommand enum with a descriptive name
such as `SomeSubCmd`, and place every leaf command in a further module below
that directory.
