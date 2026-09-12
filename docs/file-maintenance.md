# Native file maintenance

`FileService::delete` removes metadata and atomically records cleanup intent. It
does not delete bytes. Removing a domain link alone never creates cleanup intent.
Ordinary uploads are create-only; duplicate IDs return `file_already_exists`/409.
The bytes live under immutable hash/UUID locators, so losing uploads leave
reclaimable orphans without changing existing files.

Destructive maintenance is opt-in and currently supports a dedicated local
filesystem store through `semantic_app::file_maintenance`. Generic stores,
existing/shared stores, old scope-alias cleanup intents and custom locators retain
their bytes. There is no automatic GC job or HTTP maintenance endpoint.

Initialize a new empty store with a stable owner, the complete inventory of
database identities, and positive `retention_seconds`:

```rust,ignore
let store = ManagedFileStore::initialize(
    Path::new("/srv/semantic/managed-files"),
    "my-app".into(),
    BTreeSet::from(["/srv/semantic/main.redb".into()]),
    86_400,
)?;
let app = SemanticApp::builder()
    .with_default_scope(scope.clone(), db)
    .with_default_file_store(scope, store.into_store())
    .build()?;
```

On subsequent starts use `ManagedFileStore::open(root, owner)`. The durable
registry identifies the owner and every database that may publish into this
store, including databases for closed or retired scopes. Registry/retention
changes and adoption of populated stores are intentionally not exposed. Keep
the registry and lease files with the store; do not remove or replace them.
The canonical filesystem location identifies the physical store in cleanup
records, so aliases resolve consistently.

An OS lease remains held by the App's object-store handle and all its clones.
Another managed App or maintenance process cannot open that store concurrently.
Normal filesystem App configuration is rejected for managed directories; use
the managed handle. Independent software/direct filesystem access is outside
this ownership contract: a store with such users is not exclusively managed.

For maintenance, stop every writer to every registered database, including
importers, other Apps, and code holding DB handles independently. Drop all App
and store handles. Keep those writers stopped until maintenance is dropped.
The store lease excludes App publication, but does not lock arbitrary database
handles. Provide exactly the database identities from the persisted inventory,
mapped to their actual open databases:

```rust,ignore
let maintenance = FileMaintenance::open(root, "my-app", databases)?;
let preview = maintenance.run(MaintenanceOptions {
    dry_run: true,
    sweep_orphans: true,
    max_entries: 100,
}).await?;
// Review the preview before repeating with dry_run: false.
```

Each pass scans every collection of every registered database before deleting
anything. Any stored file locator protects its bytes, including legacy file
metadata and file subclasses. An inaccessible database aborts maintenance;
omitting it from the supplied inventory is rejected. `max_entries` bounds
cleanup attempts plus orphan intents per pass, not the complete reference scan.
Dry-run changes neither bytes nor cleanup records.

After retention, eligible unreferenced native locators are deleted before their
cleanup intents. Missing bytes count as success, including after a crash between
those two operations. Failed deletions persist `attempts`, `last_error` and the
next retry time, using `min(3600, 2^min(attempts,12))` seconds. Restarting
maintenance preserves that schedule. Republication into any registered database
protects the bytes when the next pass rechecks references.

The optional orphan sweep only visits native immutable hash/UUID and temporary
upload keys; it does not follow symlinks. Candidates must already be older than
retention and have no live metadata. Sweeping first creates durable cleanup
intent, giving each discovered orphan another full retention interval before a
later pass may delete it. A failed queue write leaves the bytes intact.
