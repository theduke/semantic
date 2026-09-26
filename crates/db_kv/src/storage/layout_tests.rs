//! Layout versioning and the legacy layout migration.

use std::collections::BTreeSet;

use semantic_data::query::BinaryOp;
use semantic_data::schema::IndexKind;
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::catalog::{CollectionKind, LocalIndexId};
use semantic_db_core::embedded::{EmbeddedDb, EntityStorage};
use semantic_db_core::{Expr, Operand, SelectQuery};

use super::layout::{LAYOUT_VERSION_CURRENT, LayoutMigration};
use super::{EntityStore, KvEngine, KvWriteOp, MemoryKvEngine};
use crate::keys::{self, legacy};

type Db = EmbeddedDb<EntityStore<MemoryKvEngine>>;

fn object(id: &str, kind: &str) -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String(id.to_string()));
    object.insert("kind", Value::String(kind.to_string()));
    object
}

/// Entries of a database with an indexed collection, in the legacy layout.
fn legacy_entries() -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut db = crate::open_memory().unwrap();
    let items = db
        .create_collection("items", CollectionKind::Untyped)
        .unwrap();
    db.create_index("by_kind", items, "kind", false).unwrap();
    for (id, kind) in [("one", "music"), ("two", "video"), ("three", "music")] {
        db.insert("items", id, object(id, kind)).unwrap();
    }
    let path_indexes = db
        .catalog()
        .indexes()
        .filter(|(_, index)| index.schema.kind == IndexKind::PathEquality)
        .map(|(lid, _)| lid)
        .collect::<BTreeSet<_>>();
    let (_, store) = db.into_parts();
    let entries = store.scan_raw_prefix(&[]).unwrap();
    legacy::downgrade_entries(entries, &path_indexes).unwrap()
}

fn legacy_engine() -> MemoryKvEngine {
    let entries = legacy_entries();
    assert!(entries.iter().any(|(key, _)| key.starts_with(b"c/")));
    assert!(
        entries
            .iter()
            .any(|(key, _)| key.starts_with(b"i/") && !key.ends_with(b"/fmt"))
    );
    assert!(entries.iter().any(|(key, _)| key.ends_with(b"/fmt")));
    assert!(
        entries
            .iter()
            .all(|(key, _)| key[0] >= keys::TAG_RESERVED_END)
    );
    let mut engine = MemoryKvEngine::new();
    engine
        .write_batch(
            &entries
                .into_iter()
                .map(|(key, value)| KvWriteOp::Put { key, value })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    engine
}

fn kind_is(kind: &str) -> Expr {
    Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
            "kind",
        ])))),
        right: Box::new(Expr::Operand(Operand::Literal(Value::String(
            kind.to_string(),
        )))),
    }
}

/// Assert the migrated database serves rows and index lookups, returning its
/// storage and the `by_kind` index.
fn assert_migrated(db: Db) -> (EntityStore<MemoryKvEngine>, LocalIndexId) {
    let row = db.get("items", "two").unwrap().unwrap();
    assert_eq!(
        row.object.get("kind"),
        Some(&Value::String("video".to_string()))
    );
    let rows = db
        .select(
            SelectQuery::new()
                .with_collection("items")
                .with_predicate(kind_is("music")),
        )
        .unwrap();
    let mut ids = rows
        .iter()
        .map(|row| row.get("id").cloned().unwrap())
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(
        ids,
        [
            Value::String("one".to_string()),
            Value::String("three".to_string())
        ]
    );

    let catalog = db.catalog();
    let items = catalog.collection_by_name("items").unwrap().lid;
    let by_kind = catalog.find_equality_index(items, "kind").unwrap().lid;
    let index_lids = catalog.indexes().map(|(lid, _)| lid).collect::<Vec<_>>();
    let (_, store) = db.into_parts();

    assert_eq!(
        store.layout_version().unwrap(),
        Some(LAYOUT_VERSION_CURRENT)
    );
    let raw = store.scan_raw_prefix(&[]).unwrap();
    assert!(
        raw.iter()
            .all(|(key, _)| !key.starts_with(legacy::ENTITY_SPACE)
                && !key.starts_with(legacy::INDEX_SPACE)),
        "legacy keys survived the migration"
    );
    for lid in index_lids {
        assert!(!store.index_needs_rebuild(lid).unwrap(), "{lid:?}");
    }
    assert_eq!(
        store
            .scan_index_value(by_kind, None, &Value::String("music".to_string()))
            .unwrap(),
        ["one", "three"]
    );
    (store, by_kind)
}

#[test]
fn legacy_database_is_migrated_once_on_open() {
    let db = EmbeddedDb::open(EntityStore::new(legacy_engine())).unwrap();
    let (store, _) = assert_migrated(db);
    let revision = store.current_revision().unwrap();

    let db = EmbeddedDb::open(store).unwrap();
    let (store, _) = assert_migrated(db);
    assert_eq!(store.current_revision().unwrap(), revision);
}

#[test]
fn migrate_layout_reports_rewritten_and_dropped_keys() {
    let entries = legacy_entries();
    let entities = entries
        .iter()
        .filter(|(key, _)| key.starts_with(b"c/"))
        .count();
    let indexes = entries
        .iter()
        .filter(|(key, _)| key.starts_with(b"i/"))
        .count();
    let mut store = EntityStore::new(legacy_engine());
    assert_eq!(store.layout_version().unwrap(), None);

    assert_eq!(
        store.migrate_layout().unwrap(),
        LayoutMigration::Migrated {
            entities,
            dropped_index_keys: indexes,
        }
    );
    let revision = store.current_revision().unwrap();
    assert_eq!(store.migrate_layout().unwrap(), LayoutMigration::UpToDate);
    assert_eq!(store.current_revision().unwrap(), revision);
}

#[test]
fn new_databases_are_created_at_the_current_version() {
    let mut store = EntityStore::new(MemoryKvEngine::new());
    assert_eq!(
        store.migrate_layout().unwrap(),
        LayoutMigration::Initialized
    );
    assert_eq!(
        store.layout_version().unwrap(),
        Some(LAYOUT_VERSION_CURRENT)
    );

    let db = crate::open_memory().unwrap();
    let (_, store) = db.into_parts();
    assert_eq!(
        store.layout_version().unwrap(),
        Some(LAYOUT_VERSION_CURRENT)
    );
}

#[test]
fn refuses_databases_with_a_newer_layout() {
    let mut engine = MemoryKvEngine::new();
    engine
        .put(
            keys::layout_version_key(),
            (LAYOUT_VERSION_CURRENT + 1).to_be_bytes().to_vec(),
        )
        .unwrap();
    let err = EmbeddedDb::open(EntityStore::new(engine)).unwrap_err();
    assert!(
        err.to_string().contains("newer than the supported"),
        "{err}"
    );
}

#[test]
fn rejects_unrecognized_legacy_entity_keys() {
    let mut engine = MemoryKvEngine::new();
    engine.put(b"c/not-a-key".to_vec(), Vec::new()).unwrap();
    let mut store = EntityStore::new(engine);
    assert!(store.migrate_layout().is_err());
    // Nothing was committed.
    assert_eq!(store.layout_version().unwrap(), None);
}
