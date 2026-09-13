# Database import and export

`semantic db export` and `semantic db import` operate directly on local storage;
they do not contact a running Semantic server. A database already opened by a
server may therefore be locked and must normally be taken offline first.

The default export is entities-only JSONL. Use `--full` or `--format tar` to
include referenced blobs in an uncompressed tar archive. Import detects either
format from its content by default. Both support streaming through `-` for
stdin/stdout and emit regular entities before relations. The
[export format reference](formats/exports.md) specifies the record encoding,
archive layout, blob rules, ordering, staging, and compatibility requirements.

## Examples

```sh
semantic db --db-uri redb:/data/main.redb export entities.jsonl
semantic db --db-uri redb:/data/main.redb --blob-uri file:///data/blobs \
  export --full backup.tar
semantic db --db-uri redb:/data/restore.redb import entities.jsonl
semantic db --db-uri redb:/data/restore.redb --blob-uri file:///data/blobs \
  import backup.tar --batch-size 1000
semantic db --data-dir /data export --format tar -
semantic db --data-dir /data import --format tar -
```

Imports upsert into existing data, defaulting to 1,000 entities per batch and a
64 MiB maximum record size. Adjust these with `--batch-size` and
`--max-record-bytes`. Foreign-key checks are disabled by default; enable them
with `--validate-foreign-keys`. Earlier batches remain committed on later failure.
See [import behavior](formats/exports.md#import-behavior-and-compatibility) for
validation and partial-import semantics.

Use `semantic db --temp-dir PATH` to select temporary staging storage. Both
formats defer relations on disk during export, and tar transfers additionally
stage archive and blob data.

Bounded imports currently cannot change indexed/transitive relationships.
Relations-last ordering does not remove this restriction; see the
[indexed relationship limitation](formats/exports.md#indexed-relationship-limitation).
