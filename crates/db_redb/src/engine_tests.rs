//! Tests of the redb table layout, options, legacy table migration and
//! maintenance operations.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use redb::{ReadableTable, TableDefinition};
use semantic_data::schema::{DbOpenMode, IndexKind};
use semantic_data::value::{Object, Value};
use semantic_db_core::StorageErrorKind;
use semantic_db_core::catalog::{CollectionKind, LocalIndexId};
use semantic_db_core::embedded::EntityStorage;
use semantic_db_kv::keys::{self, TAG_ENTITY, TAG_INDEX, TAG_INDEX_MARKER, TAG_META, TAG_STATS};
use semantic_db_kv::{EntityStore, KvEngine, KvReadTxn, KvWriteOp};

use crate::tables::{REVISION_KEY, TableSetup, prepare_tables};
use crate::{RedbDatabase, RedbDurability, RedbKvEngine, RedbOptions, RedbTable};

type Entries = Vec<(Vec<u8>, Vec<u8>)>;

const LEGACY_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("kv");

fn put(key: &[u8], value: &[u8]) -> KvWriteOp {
    KvWriteOp::Put {
        key: key.to_vec(),
        value: value.to_vec(),
    }
}

/// Keys of every physical table of the database at `path`.
fn physical_keys(db: &redb::Database) -> BTreeMap<&'static str, Vec<Vec<u8>>> {
    let txn = db.begin_read().unwrap();
    RedbTable::ALL
        .into_iter()
        .map(|table| {
            let keys = txn
                .open_table(TableDefinition::<&[u8], &[u8]>::new(table.name()))
                .unwrap()
                .iter()
                .unwrap()
                .map(|item| item.unwrap().0.value().to_vec())
                .collect();
            (table.name(), keys)
        })
        .collect()
}

fn legacy_table_exists(db: &redb::Database) -> bool {
    match db.begin_read().unwrap().open_table(LEGACY_TABLE) {
        Ok(_) => true,
        Err(redb::TableError::TableDoesNotExist(_)) => false,
        Err(err) => panic!("{err}"),
    }
}

#[test]
fn keys_are_stored_in_the_table_of_their_key_space() {
    let dir = tempfile::tempdir().unwrap();
    let meta = keys::layout_version_key();
    let entity = vec![TAG_ENTITY, 1, 1, b'x'];
    let index = vec![TAG_INDEX, 1, 2, b'v'];
    let marker = vec![TAG_INDEX_MARKER, 1, 2];
    let stats = vec![TAG_STATS, 1];
    let zero = vec![0, 1];
    let legacy = b"c/1/e/x".to_vec();
    let mut engine = RedbKvEngine::open(dir.path().join("db"), DbOpenMode::AutoCreate).unwrap();
    let all = [&meta, &entity, &index, &marker, &stats, &zero, &legacy];
    engine.write_batch(&all.map(|key| put(key, b"v"))).unwrap();

    let tables = physical_keys(&engine.db);
    assert_eq!(tables["meta"], [meta.clone(), REVISION_KEY.to_vec()]);
    assert_eq!(tables["entities"], [entity.clone()]);
    assert_eq!(tables["indexes"], [index.clone(), marker.clone()]);
    assert_eq!(
        tables["other"],
        [zero.clone(), stats.clone(), legacy.clone()]
    );

    // The logical key space is unchanged: point reads route by tag, scans
    // concatenate tables in key order and never expose the revision key.
    for key in all {
        assert_eq!(engine.get(key).unwrap(), Some(b"v".to_vec()));
    }
    let mut expected = all.map(|key| key.clone()).to_vec();
    expected.sort();
    let scanned = |scan: semantic_db_kv::BoxKvPrefixScan| {
        scan.map(|item| item.unwrap().0).collect::<Vec<_>>()
    };
    assert_eq!(
        scanned(engine.scan_range_stream(vec![], None).unwrap()),
        expected
    );
    assert_eq!(
        scanned(
            engine
                .scan_range_stream(entity.clone(), Some(stats.clone()))
                .unwrap()
        ),
        [entity.clone(), index.clone(), marker.clone()]
    );
    engine
        .write_with(None, |txn| {
            assert_eq!(
                txn.scan_prefix(&[])?
                    .into_iter()
                    .map(|(key, _)| key)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(txn.scan_prefix(&[TAG_META])?.len(), 1);
            Ok(())
        })
        .unwrap();

    let before = engine.current_revision().unwrap();
    engine.delete(&index).unwrap();
    assert_eq!(engine.get(&index).unwrap(), None);
    assert_eq!(physical_keys(&engine.db)["indexes"], [marker]);
    assert_eq!(
        engine.current_revision().unwrap(),
        before.map(|revision| revision + 1)
    );
}

#[test]
fn non_durable_commits_survive_reopen() {
    for durability in [RedbDurability::None, RedbDurability::Eventual] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let options = RedbOptions::default()
            .with_durability(durability)
            .with_cache_size(4 * 1024 * 1024)
            .with_quick_repair(true);
        let revision = {
            let mut engine =
                RedbKvEngine::open_with_options(&path, DbOpenMode::AutoCreate, options.clone())
                    .unwrap();
            assert_eq!(engine.options(), &options);
            for index in 0..10u8 {
                engine.put(vec![TAG_ENTITY, index], vec![index]).unwrap();
            }
            engine.current_revision().unwrap()
        };
        let engine = RedbKvEngine::open(&path, DbOpenMode::OpenExisting).unwrap();
        assert_eq!(engine.current_revision().unwrap(), revision);
        assert_eq!(
            engine.get(&[TAG_ENTITY, 9]).unwrap(),
            Some(vec![9]),
            "{durability:?}"
        );
    }
}

#[test]
fn open_errors_keep_the_redb_error_as_source() {
    use std::error::Error as _;

    let dir = tempfile::tempdir().unwrap();
    let err = RedbKvEngine::open(dir.path().join("missing"), DbOpenMode::OpenExisting).unwrap_err();
    assert_eq!(err.storage_kind(), Some(StorageErrorKind::Io), "{err}");
    let source = err
        .source()
        .and_then(|storage| storage.source())
        .expect("redb error source");
    assert!(
        matches!(
            source.downcast_ref::<redb::Error>(),
            Some(redb::Error::Io(io)) if io.kind() == std::io::ErrorKind::NotFound
        ),
        "{source:?}"
    );
}

#[test]
fn compaction_and_integrity_check_succeed_after_deletes() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = RedbKvEngine::open(dir.path().join("db"), DbOpenMode::AutoCreate).unwrap();
    let key = |index: u32| {
        let mut key = vec![TAG_ENTITY];
        key.extend_from_slice(&index.to_be_bytes());
        key
    };
    let ops = (0..2_000)
        .map(|index| put(&key(index), &[7; 1024]))
        .collect::<Vec<_>>();
    engine.write_batch(&ops).unwrap();
    let deletes = (1..2_000)
        .map(|index| KvWriteOp::Delete { key: key(index) })
        .collect::<Vec<_>>();
    engine.write_batch(&deletes).unwrap();

    // Maintenance refuses to run while reads are open.
    {
        let read = engine.read_txn().unwrap();
        let scan = read.scan_prefix_stream(vec![TAG_ENTITY]).unwrap();
        drop(read);
        assert_eq!(
            engine.compact().unwrap_err().storage_kind(),
            Some(StorageErrorKind::InvalidState)
        );
        assert!(engine.check_integrity().is_err());
        assert_eq!(scan.count(), 1);
    }

    let before = engine.stats().unwrap();
    assert!(engine.compact().is_ok());
    assert!(engine.check_integrity().unwrap());
    let after = engine.stats().unwrap();
    assert!(after.file_size_bytes.unwrap() <= before.file_size_bytes.unwrap());
    assert_eq!(engine.get(&key(0)).unwrap(), Some(vec![7; 1024]));
    assert_eq!(engine.get(&key(1)).unwrap(), None);
}

#[test]
fn stats_report_table_entries_and_file_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = RedbKvEngine::open(dir.path().join("db"), DbOpenMode::AutoCreate).unwrap();
    engine
        .write_batch(&[
            put(&[TAG_ENTITY, 1], b"a"),
            put(&[TAG_ENTITY, 2], b"b"),
            put(&[TAG_INDEX, 1], b""),
        ])
        .unwrap();
    let stats = engine.stats().unwrap();
    let entries = stats
        .tables
        .iter()
        .map(|table| (table.name.as_str(), table.entries))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        entries,
        BTreeMap::from([("entities", 2), ("indexes", 1), ("meta", 1), ("other", 0)])
    );
    assert_eq!(stats.entries, Some(4));
    assert!(stats.file_size_bytes.unwrap() > 0);
    assert!(stats.allocated_bytes.unwrap() > 0);
    assert!(stats.stored_bytes.is_some());

    // The entity storage forwards maintenance to the engine.
    let mut store = EntityStore::new(engine);
    assert_eq!(store.storage_stats().unwrap(), stats);
    assert!(store.check_storage_integrity().unwrap());
    assert!(store.compact_storage().is_ok());
}

/// Entries of a memory database with an indexed `items` collection, and its
/// path-equality indexes.
fn seeded_entries() -> (Entries, BTreeSet<LocalIndexId>) {
    let mut db = semantic_db_kv::open_memory().unwrap();
    let items = db
        .create_collection("items", CollectionKind::Untyped)
        .unwrap();
    db.create_index("by_kind", items, "kind", false).unwrap();
    for (id, kind) in [("one", "music"), ("two", "video"), ("three", "music")] {
        let mut object = Object::new();
        object.insert("id", Value::String(id.into()));
        object.insert("kind", Value::String(kind.into()));
        db.insert("items", id, object).unwrap();
    }
    let path_indexes = db
        .catalog()
        .indexes()
        .filter(|(_, index)| index.schema.kind == IndexKind::PathEquality)
        .map(|(lid, _)| lid)
        .collect();
    let (_, store) = db.into_parts();
    (store.scan_raw_prefix(&[]).unwrap(), path_indexes)
}

/// Entries of an indexed database re-encoded in the legacy textual layout.
pub(crate) fn legacy_layout_entries() -> Entries {
    let (entries, path_indexes) = seeded_entries();
    semantic_db_kv::keys::legacy::downgrade_entries(entries, &path_indexes).unwrap()
}

/// Write `entries` and `revision` in the single-table layout of earlier
/// engine versions.
fn write_legacy_table(path: &Path, entries: &Entries, revision: u64) {
    let db = redb::Database::create(path).unwrap();
    let txn = db.begin_write().unwrap();
    {
        let mut table = txn.open_table(LEGACY_TABLE).unwrap();
        for (key, value) in entries {
            table.insert(key.as_slice(), value.as_slice()).unwrap();
        }
        table
            .insert(
                b"__semantic/revision".as_slice(),
                revision.to_be_bytes().as_slice(),
            )
            .unwrap();
    }
    txn.commit().unwrap();
}

/// Open the database and check the seeded rows and index; returns the
/// storage revision.
fn open_and_verify(path: &Path) -> Option<u64> {
    let engine = RedbKvEngine::open(path, DbOpenMode::OpenExisting).unwrap();
    let db = RedbDatabase::open(EntityStore::new(engine)).unwrap();
    assert_eq!(
        db.get("items", "two").unwrap().unwrap().object.get("kind"),
        Some(&Value::String("video".into()))
    );
    let catalog = db.catalog();
    let items = catalog.collection_by_name("items").unwrap().lid;
    let by_kind = catalog.find_equality_index(items, "kind").unwrap().lid;
    let (_, store) = db.into_parts();
    assert!(!store.index_needs_rebuild(by_kind).unwrap());
    assert_eq!(
        store
            .scan_index_value(by_kind, None, &Value::String("music".into()))
            .unwrap(),
        ["one", "three"]
    );
    assert_eq!(
        store.layout_version().unwrap(),
        Some(semantic_db_kv::LAYOUT_VERSION_CURRENT)
    );
    assert!(store.scan_raw_prefix(b"c/").unwrap().is_empty());
    assert!(store.scan_raw_prefix(b"i/").unwrap().is_empty());
    store.current_revision().unwrap()
}

#[test]
fn legacy_single_table_databases_are_split_on_open() {
    // Binary-layout databases only need the table split; legacy textual
    // databases are then migrated by the entity store's layout migration,
    // which reads the moved keys through the regular engine API.
    let (current, _) = seeded_entries();
    for (entries, layout_migrated) in [(current, false), (legacy_layout_entries(), true)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        write_legacy_table(&path, &entries, 7);

        // The table split keeps the revision; only the layout migration
        // (and the index rebuild it triggers) advances it.
        let revision = open_and_verify(&path).unwrap();
        if layout_migrated {
            assert!(revision > 7);
        } else {
            assert_eq!(revision, 7);
        }

        // Reopening neither migrates nor writes.
        {
            let db = redb::Database::open(&path).unwrap();
            assert!(!legacy_table_exists(&db));
            // Only the maintained stats counters live in the `other` table.
            assert!(
                physical_keys(&db)["other"]
                    .iter()
                    .all(|key| key.first() == Some(&TAG_STATS))
            );
            assert_eq!(prepare_tables(&db).unwrap(), TableSetup::UpToDate);
        }
        assert_eq!(open_and_verify(&path), Some(revision));
    }
}

#[test]
fn prepare_tables_creates_tables_once() {
    let dir = tempfile::tempdir().unwrap();
    let db = redb::Database::create(dir.path().join("db")).unwrap();
    assert_eq!(prepare_tables(&db).unwrap(), TableSetup::Created);
    assert_eq!(prepare_tables(&db).unwrap(), TableSetup::UpToDate);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    write_legacy_table(&path, &vec![(vec![TAG_ENTITY, 1], vec![1])], 3);
    let db = redb::Database::open(&path).unwrap();
    assert_eq!(
        prepare_tables(&db).unwrap(),
        TableSetup::MigratedLegacy { rows: 1 }
    );
    assert!(!legacy_table_exists(&db));
    assert_eq!(physical_keys(&db)["entities"], [vec![TAG_ENTITY, 1]]);
    assert_eq!(prepare_tables(&db).unwrap(), TableSetup::UpToDate);
    drop(db);
    let engine = RedbKvEngine::open(&path, DbOpenMode::OpenExisting).unwrap();
    assert_eq!(engine.current_revision().unwrap(), Some(3));
}
