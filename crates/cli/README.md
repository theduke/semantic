# Semantic CLI

The `semantic_cli` crate provides the `semantic` command-line application.

## Server storage

With no storage flags, `semantic server` uses redb at `<data-dir>/db/default`
and filesystem blobs at `<data-dir>/blob/default`. `--data-dir` overrides
`SEMANTIC_DATA_DIR`; the default is the platform user data directory.
When the blob URI selects logfs and no database URI is supplied, startup
automatically selects `log:<blob>` and prints an explanation. An explicit
database URI, including `SEMANTIC_DB_URI`, takes precedence.

Select the database with `--db-uri` (or `SEMANTIC_DB_URI`):

| Database URI | Backend |
| --- | --- |
| `redb:PATH` | Local redb database |
| `logfs:PATH` | Direct local logfs database |
| `log:<blob>` | Log database sharing the already-opened blob object store |

Local paths may be absolute or relative to the working directory. They are
literal paths, with no URL decoding.

`--blob-uri` (or `SEMANTIC_BLOB_URI`) uses objstore's URI parser. The server
registers filesystem (`fs`) and logfs (`logfs`) object-store providers. CLI
values override the corresponding environment variables.

To put both the database and blobs in one logfs file:

```sh
semantic server \
  --blob-uri 'logfs:///srv/semantic/store.log?allow_create=true'
```

The parent directory must exist. `allow_create=true` allows objstore to create
the logfs file. Quote `log:<blob>` to prevent shell redirection.

Logfs blob storage prompts for a password at startup. Terminal input is hidden;
redirected stdin can provide one password line. An empty line opens without
password protection and prints an explicit message. EOF without a line is an
error. Passwords are passed to the object-store configuration in memory and are
never added to the URI or logged. The prompt's value takes precedence over any
`key` in the URI.

The `objstore_logfs` crate from the logfs repository provides the object store.
Logfs v3 stores its random encryption salts in the log file; Semantic does not
create a sidecar settings file. The default Argon2id profile is `standard`;
`profile=low-memory` may be specified in the blob URI and must also be supplied
when reopening a file created with that profile. Startup does not automatically
re-encrypt an existing unencrypted store.

Shared mode opens the physical object store once, then uses `db/default/wal/v1/`
for database events and `blob/default/` for uploaded contents. Reopening the
shared database through the scope API reuses its existing database instance,
so there is only one WAL writer. `OpenExisting` requires an existing WAL.
Switching an existing installation to shared mode does not migrate its data;
direct `logfs:PATH` and shared mode have different storage layouts.

The shared database inherits the selected object-store provider's durability
and consistency guarantees. The pinned objstore logfs provider uses buffered
writes and does not implement conditional puts; the server ensures one WAL
writer and isolates its keys, but shared mode does not provide the direct
logfs database's durable-commit guarantees on power loss. Filesystem objstore
also lacks the guarantees required for a production WAL.

For separate storage:

```sh
semantic server \
  --db-uri 'redb:/srv/semantic/database.redb' \
  --blob-uri 'fs:///srv/semantic/blobs'
```

Database backends implement `semantic_app::DbBackend`, providing a typed
`Config`, `parse_uri`, and `open_config`. Its blanket `DbProvider` implementation
adapts these to the application's existing scheme registry; dispatch splits
at the first `:` and leaves the remainder to the backend. Existing custom
`DbProvider` implementations remain supported.

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
