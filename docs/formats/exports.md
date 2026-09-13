# Database export formats

Semantic supports two logical export formats: JSONL for entities, and an
uncompressed tar archive for entities plus referenced blobs. These are portable
entity transfers, not physical database snapshots: internal catalog and derived
index collections are omitted. Install compatible custom packages and
collections in the destination before importing.

See [database import and export](../import-export.md) for local CLI examples.

## JSONL

A JSONL export contains one UTF-8 JSON entity envelope per line, with no header or
trailer. Export writes LF line endings; import also accepts CRLF, blank lines, and
a final record without a trailing newline. An empty file represents no entities.

```json
{"version":1,"collection":"entities","id":"example","object":{"object":{"id":{"string":"example"}}}}
```

The envelope fields are:

| Field | Meaning |
| --- | --- |
| `version` | Record format version; the importer currently accepts only `1`. |
| `collection` | Nonempty destination collection name. |
| `id` | Nonempty entity ID, identifying the entity within its collection. |
| `object` | The entity as a typed object value, including a string `id` equal to the envelope ID. |

`object` uses Semantic's [typed value JSON encoding](../../crates/data/src/value/serde/typed.rs),
not ordinary untagged JSON. For example, a string is `{"string":"example"}` and
an object is `{"object":{...}}`. The encoding preserves numeric widths, bytes,
UUIDs, temporal values, maps, objects, and variants. Export rejects non-finite
floating-point values rather than silently changing their representation.

JSONL carries entity attributes, including blob hashes and locators, but no blob
contents. Importing JSONL does not fetch, publish, or validate referenced blobs.

### Entity ordering

The export format invariant is that all regular entities precede all relation
entities, both in standalone JSONL and in the JSONL inside tar. Relations include
`semantic:relation`, its inherited and extension classes, and rows contributing
to declared external relationships.
Embedded relationships remain on their regular owner entities. Scan order is
preserved within each group; there is no global ID sort or dependency sort.

Consumers that transform, split, or recombine exports should preserve this
ordering so relations retain their regular endpoints earlier in the stream. The
importer does not reject a file solely for violating this ordering, but reordered
input loses that guarantee and can fail when foreign-key validation is enabled.
This is a limited ordering guarantee, not a solution for arbitrary dependencies
or for the [indexed relationship limitation](#indexed-relationship-limitation).

The exporter performs one entity scan, writing regular entities immediately and
spooling relation records to a private temporary file. It then streams that file
after the regular records. Memory does not grow with the number of deferred
relations, but temporary disk usage does. `semantic db --temp-dir PATH` selects
the staging location for either export format.

## Tar

The tar format contains the same JSONL plus raw blob contents:

```text
entities.jsonl
blobs/<64-character-lowercase-sha256>
blobs/<64-character-lowercase-sha256>
...
```

Export writes `entities.jsonl` first, followed by one regular file per distinct
referenced blob hash. No separate manifest is needed: entities supply the blob
references. An explicit `blobs/` directory entry is optional. Import accepts the
entries in any order, but requires exactly one `entities.jsonl` and rejects
duplicate blob entries, unknown paths, and non-regular entries other than the
optional blob directory. Symlinks and hard links are not accepted.

### Blob references

An entity references blob contents through
`semantic:filestore:file:content_hash_sha256`. Only hashes referenced by exported
entities are included; unreferenced objects in the source store are omitted.
Hashes must contain exactly 64 lowercase hexadecimal characters. A hash-bearing
entity must also have a nonempty
`semantic:filestore:file:filestore_locator` identifying its source and destination
object-store key. Locators are preserved exactly and must not contain NUL bytes.

The short attribute names `content_hash_sha256`, `filestore_locator`, and
`mime_type` are also accepted. If both the qualified and short name are present,
their string values must agree. The MIME attribute's qualified name is
`semantic:filestore:file:mime_type`.

Multiple entities or locators may reference the same hash, which is archived
once. A locator cannot reference conflicting hashes. Export streams and hashes
blob content and fails if no referenced locator supplies the blob or if read
content disagrees with its declared hash.

Import verifies each blob's SHA-256 against its archive filename and checks that
the archive contains every referenced hash and no unreferenced blobs. It restores
contents at all referenced locators, using MIME metadata when supplied by the
entities. Existing destination content is reused if its hash matches; a locator
containing different content causes an error.

### Streaming and staging

Tar export and import use temporary disk files and a disk-backed blob-reference
index. Export stages the JSONL, builds the archive while staging one source blob
at a time, and then streams the archive to the output. Import stages the incoming
archive and validates its entries, JSONL, hashes, and reference completeness
before publishing blobs or importing entities. Neither operation loads the whole
archive, entity set, or blob set into memory. Temporary disk space must accommodate
the staged archive and working files; bounded memory does not imply immediate
output or immediate database writes.

Archives are uncompressed. Compression can be applied externally; decompress
before import, since the importer accepts JSONL and tar rather than compressed
containers.

## Import behavior and compatibility

Import defaults to content-based format detection, including for stdin. It reads
and replays a bounded prefix of at most 512 bytes, recognizing JSON objects and
tar headers without requiring a seekable input. Filenames and extensions do not
select the format. `--format jsonl` and `--format tar` override autodetection;
`--format auto` is the default.

Records are processed as upserts in input order, with a configurable
`--batch-size` defaulting to 1,000. `--max-record-bytes` defaults to 64 MiB and
bounds each JSONL record read, for both formats. Both values must be positive.
Parsing holds a bounded record and batch rather than the full entity set.
Imports merge into the destination; data absent from the input is not deleted.
Earlier batches remain committed if a later record or batch fails. Tar's archive
validation happens before publication, but a later blob-store or database failure
can still leave published blobs or committed batches; the whole import is not
one transaction.

Foreign-key existence and target-class validation is disabled by default through
`ImportOptions.write_settings.validate_foreign_keys = false`. This allows records
to arrive without a full dependency ordering. `--validate-foreign-keys` enables
per-batch validation; other shape, type, identity, and uniqueness constraints
remain active. Disabling foreign-key checks permits dangling references and does
not trigger a whole-database validation pass afterward. Regular database writes
still use their normal validation defaults.

Imports require the backend's bounded point-batch path and reject a batch before
mutation if that path is unavailable. Redb provides the intended end-to-end
bounded-memory behavior. Log-backed databases retain their ordinary in-memory
database state even though transfer parsing and blob handling are streamed.

### Indexed relationship limitation

The bounded import path currently rejects batches that change an indexed
(transitive) relationship contribution. Atomic closure maintenance needs a
disk-backed graph work area that is not implemented; loading the relationship
graph into memory would violate the import memory contract. Earlier batches can
already have committed when this rejection occurs.

Exporting relations last makes their regular endpoints available first, but does
not remove this limitation: closure maintenance depends on the relationship
graph regardless of entity ordering. The ordering also does not guarantee
foreign-key dependencies between regular entities or between relations.
