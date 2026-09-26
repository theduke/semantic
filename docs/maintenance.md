# Database maintenance

The embedded database (`semantic_db_core::embedded::EmbeddedDb`) offers
maintenance operations to rebuild derived data, check consistency, reclaim
space, upgrade stored payloads and take backups. They are exposed through
`Backend`/`Db` (backends without them return an `Unsupported` storage error),
the app's `SemanticDb`, the `semantic.db.maintenance.*` RPC commands and
`db_cli maintenance`.

| Operation | `Db` method | RPC command | CLI |
|-----------|-------------|-------------|-----|
| Rebuild indexes | `reindex(ReindexTarget)` | `semantic.db.maintenance.reindex` | `db_cli maintenance reindex [--collection C [--index I]]` |
| Check consistency | `verify(VerifyOptions)` | `semantic.db.maintenance.verify` | `db_cli maintenance verify [--skip-...]` |
| Repair | `repair(VerifyOptions)` | `semantic.db.maintenance.repair` | `db_cli maintenance repair` |
| Compact storage | `compact_storage()` | `semantic.db.maintenance.compact` | `db_cli maintenance compact` |
| Storage statistics | `storage_stats()` | `semantic.db.maintenance.stats` | `db_cli maintenance stats` |
| Rewrite payloads | `rewrite_payloads(batch_size)` | `semantic.db.maintenance.rewrite_payloads` | `db_cli maintenance rewrite-payloads [--batch-size N]` |
| Backup | `backup(path)` | `semantic.db.maintenance.backup` | `db_cli maintenance backup PATH` |
| Snapshot export | `export_snapshot()` | - | - |

All RPC commands accept an optional `scope_id`. `backup` writes a file on the
server and requires a system principal. `db_cli` takes the database with
`--db-uri redb:PATH` or `logfs:PATH` (or `DB_URI`) before the subcommand,
e.g. `db_cli maintenance --db-uri redb:data.redb verify`; `verify` and
`repair` exit with an error when problems remain.

## Reindex

`ReindexTarget::Index { collection, name }`, `Collection(name)` or `All`.
Every index is rebuilt in its own write transaction (`ResetIndex` plus one
`IndexEntity` per row, streamed from a storage snapshot), so readers observe
either the old or the complete new index. `All` also rebuilds the reverse
references and the relationship contributors, counts and edges (including the
indexes of their internal collections) and recounts the stats counters.

Index entry counters are exact after a rebuild: the key-value store counts the
keys a reset or clear removes and uses that count, instead of the stored
counter, as the base of the new value.

Rebuilds commit with `ChangeSource::Maintenance` and no row changes; since
neither rows nor the catalog change, the change feed publishes nothing.

## Verify and repair

`verify` is read-only and runs on one storage snapshot without the writer
lock (only the optional physical integrity check needs exclusive access). It
streams every collection once and reports `VerifyProblem`s:

- index entries: entries a row derives that are missing (point lookups per
  entry) and stored entries no row derives (found by comparing entry counts
  and, on a mismatch, checking each stored entry against its row), plus
  indexes not marked as built;
- reverse references and relationship contributors: missing, differing and
  stale rows, with the same count-then-scan scheme;
- relationship counts and edges, derived from the direct `(source, target)`
  pairs tallied during the row scan (memory proportional to the distinct
  pairs, not the rows);
- maintained row and index entry counters against the actual counts;
- payloads that fail to decode, with their entity id;
- unique indexes holding the same key value for several rows
  (`duplicate_unique_value`, memory proportional to the rows of the checked
  collection). Creating a unique index over duplicates fails, but a rebuild
  on open (for example after a bug) builds the index anyway, logs a warning
  and leaves the duplicates for `verify` to report;
- the storage's physical integrity check (redb). It cannot run while read
  snapshots are open and is then reported under `skipped`; retry later.

Checks a storage cannot run (for example index key checks without raw index
keys) are listed under `skipped`. The indexes of the internal relationship
contributor and count collections are not checked: the write path stores
those rows without index maintenance. At most `max_problems` problems are
listed; `problem_count` counts all of them.

`repair` verifies, rebuilds the indexes with problems, the reverse references
and/or relationship data when they mismatch, recounts the counters when a
counter was wrong, and verifies again. Corrupt payloads, duplicate unique
values and physical damage cannot be repaired and remain in the final
report.

## Compaction and statistics

`compact_storage` wraps the storage compaction (redb `Database::compact`) and
reports the storage statistics before and after, and the freed file bytes.
redb cannot compact while read transactions are open (running queries,
exports, interactive transactions); the operation then fails with an
`InvalidState` storage error and should be retried once they finished.

## Payload rewrite

Entity payloads written in an older format (version 1 MessagePack) are
upgraded lazily on their next write. `rewrite_payloads(batch_size)` upgrades
all of them: candidates are found through a read snapshot (keeping only their
keys), and each batch of at most `batch_size` rows is rewritten in one write
transaction, resuming after the last examined key. Through the backend the
writer lock is taken per batch, so other writes interleave. Entity contents
do not change and nothing is published to the change feed.

## Backups and snapshot exports

`backup(path)` writes a consistent copy of a redb database to a new file:
redb has no online backup API, so every key-value pair of every engine table
(including the revision counter) is copied from one read transaction into a
fresh database with the same table layout, written next to the target and
moved into place when complete. The copy opens at the revision the backup
started at; writes committed afterwards are not included, and writers are not
blocked while it is written. Existing files are never overwritten.

`export_snapshot()` streams every portable entity at one revision and returns
that revision with the catalog (and its version) the entities were written
under; `scan_entities()` is the same stream without the metadata. For
portable JSONL/tar transfers see [import-export.md](import-export.md).
