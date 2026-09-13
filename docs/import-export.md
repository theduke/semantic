# Database import and export

`semantic db export` and `semantic db import` operate directly on local storage;
they do not contact a running Semantic server. A database already opened by a
server may therefore be locked and must normally be taken offline first.

## Formats

The default format is JSONL. Each nonblank line is one versioned entity envelope:

```json
{"version":1,"collection":"entities","id":"example","object":{"object":{"id":{"string":"example"}}}}
```

Values use Semantic's typed JSON encoding, preserving numeric widths, bytes,
UUIDs, temporal values, maps, objects, and variants. Internal catalog and derived
index collections are omitted. The destination must already have any compatible
custom packages and collections installed.

The `tar` format contains `entities.jsonl` followed by
`blobs/<lowercase-sha256>`. Only blobs referenced by an exported entity's
`semantic:filestore:file:content_hash_sha256` attribute are included, and blob
content is deduplicated by hash. Entity filestore locators are preserved exactly.
Archives are uncompressed; external compression can be used in a pipeline.

Both formats stream records. Tar operations use private temporary disk staging
to validate hashes and archive completeness without retaining all entities or
blob bytes in memory.

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

Imports are upsert/merge operations; entities and blobs absent from the input are
left untouched. Foreign-key existence and target-class validation is disabled
for imports by default so records may arrive in any order. All shape, type,
identity, uniqueness, and other constraints remain active, and ordinary later
writes still use foreign-key validation. Pass `--validate-foreign-keys` to enable
per-batch checks. Disabling the checks permits dangling references; import does
not run an implicit whole-database validation pass afterward.

The default batch size is 1,000 and the default maximum JSONL record size is 64
MiB. Earlier batches remain committed when a later record or batch fails. Tar
imports validate the entire archive before publishing blobs or entities; a later
blob publication or database failure may leave an unreferenced blob, which can be
handled by the normal retention cleanup.

Transfer imports require the backend's bounded point-batch path and fail before a
batch is mutated if that path is unavailable; they never silently switch to the
dataset-returning collection materialization path. Redb provides the intended
end-to-end bounded-memory behavior. Log-backed databases retain their ordinary
in-memory database state, although transfer parsing and blob handling remain
streamed and bounded.

Imports currently reject a batch that would change an indexed (transitive)
relationship. Maintaining those closures atomically requires a disk-backed graph
work area that is not part of the v1 implementation; silently materializing the
relationship graph would violate the import memory contract. Any earlier batches
remain committed, as with other batch failures.
