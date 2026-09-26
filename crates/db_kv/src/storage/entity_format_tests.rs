//! Entity payload formats: legacy (version 1) rows, lazy and explicit
//! upgrades to the compact format, and field-name dictionary maintenance.

use semantic_data::value::{Object, Value};
use semantic_db_core::SelectQuery;
use semantic_db_core::catalog::{CollectionKind, LocalCollectionId};
use semantic_db_core::embedded::{
    EmbeddedDb, EntityStorage, StorageCommitOutcome, StorageWriteOp, StoredEntity, StoredEntityKind,
};

use super::entity_codec::{
    ENTITY_FORMAT_VERSION_V1_MSGPACK, ENTITY_FORMAT_VERSION_V2_COMPACT, EntityPayloadFormat,
    encode_entity, payload_version,
};
use super::{EntityStore, KvEngine, MemoryKvEngine};
use crate::keys;

type Store = EntityStore<MemoryKvEngine>;

const ITEMS: LocalCollectionId = LocalCollectionId(7);
const OTHER: LocalCollectionId = LocalCollectionId(8);

fn entity(collection: LocalCollectionId, id: &str, fields: &[(&str, Value)]) -> StoredEntity {
    let mut object = Object::new();
    object.insert("id", Value::String(id.to_string()));
    for (name, value) in fields {
        object.insert(*name, value.clone());
    }
    StoredEntity {
        id: id.to_string(),
        collection: collection.0,
        kind: StoredEntityKind::Untyped,
        object,
    }
}

fn put(store: &mut Store, entities: &[StoredEntity]) {
    store
        .apply_batch(
            &entities
                .iter()
                .cloned()
                .map(StorageWriteOp::PutEntity)
                .collect::<Vec<_>>(),
        )
        .unwrap();
}

/// An empty store with maintained stats counters.
fn new_store(format: EntityPayloadFormat) -> Store {
    let mut store = EntityStore::with_payload_format(MemoryKvEngine::new(), format);
    store.ensure_stats().unwrap();
    store
}

/// Reopen the store over the same engine state with a cold dictionary
/// cache.
fn reopen(store: Store, format: EntityPayloadFormat) -> Store {
    EntityStore::with_payload_format(store.into_inner(), format)
}

fn version_of(store: &Store, collection: LocalCollectionId, id: &str) -> u16 {
    let payload = store
        .get_raw(&keys::entity_key(collection, id))
        .unwrap()
        .unwrap();
    payload_version(&payload).unwrap()
}

fn dictionary(store: &Store, collection: LocalCollectionId) -> Vec<String> {
    store
        .scan_raw_prefix(&keys::field_dict_prefix(collection))
        .unwrap()
        .into_iter()
        .enumerate()
        .map(|(position, (key, name))| {
            assert_eq!(
                keys::parse_field_dict_key(&key),
                Some((collection, position as u32))
            );
            String::from_utf8(name).unwrap()
        })
        .collect()
}

/// All rows of `collection`, read through the store, a borrowed snapshot and
/// an owned snapshot, which must agree.
fn scan_all(store: &Store, collection: LocalCollectionId) -> Vec<StoredEntity> {
    let direct = store.scan_collection(collection).unwrap();
    let borrowed = EntityStorage::snapshot(store)
        .unwrap()
        .scan_collection_stream(collection)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let owned = EntityStorage::owned_snapshot(store)
        .unwrap()
        .unwrap()
        .scan_collection_stream(collection)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(direct, borrowed);
    assert_eq!(direct, owned);
    for row in &direct {
        let snapshot = EntityStorage::snapshot(store).unwrap();
        assert_eq!(
            snapshot.get_entity(collection, &row.id).unwrap().as_ref(),
            Some(row)
        );
        assert_eq!(
            store.get_entity(collection, &row.id).unwrap().as_ref(),
            Some(row)
        );
    }
    direct
}

#[test]
fn compact_format_is_the_default_and_uses_a_dictionary() {
    let mut store = new_store(EntityPayloadFormat::Compact);
    assert_eq!(store.payload_format(), EntityPayloadFormat::Compact);
    let row = entity(ITEMS, "a", &[("semantic:title", Value::String("A".into()))]);
    put(&mut store, std::slice::from_ref(&row));

    assert_eq!(
        version_of(&store, ITEMS, "a"),
        ENTITY_FORMAT_VERSION_V2_COMPACT
    );
    // The id field is derived from the key and not part of the dictionary.
    assert_eq!(dictionary(&store, ITEMS), ["semantic:title"]);
    assert_eq!(scan_all(&store, ITEMS), [row]);
    assert_eq!(store.collection_row_count(ITEMS).unwrap(), Some(1));
}

#[test]
fn mixed_v1_and_v2_rows_in_one_collection() {
    let mut store = new_store(EntityPayloadFormat::SelfContained);
    let a = entity(ITEMS, "a", &[("kind", Value::String("music".into()))]);
    let b = entity(ITEMS, "b", &[("kind", Value::String("video".into()))]);
    put(&mut store, &[a.clone(), b.clone()]);
    assert!(dictionary(&store, ITEMS).is_empty());

    let mut store = reopen(store, EntityPayloadFormat::Compact);
    let c = entity(
        ITEMS,
        "c",
        &[("kind", Value::String("text".into())), ("n", Value::I64(3))],
    );
    let b = entity(
        ITEMS,
        "b",
        &[
            ("kind", Value::String("video".into())),
            ("n", Value::I64(1)),
        ],
    );
    put(&mut store, &[b.clone(), c.clone()]);

    // Rows are upgraded lazily, when they are written.
    assert_eq!(
        version_of(&store, ITEMS, "a"),
        ENTITY_FORMAT_VERSION_V1_MSGPACK
    );
    assert_eq!(
        version_of(&store, ITEMS, "b"),
        ENTITY_FORMAT_VERSION_V2_COMPACT
    );
    assert_eq!(
        version_of(&store, ITEMS, "c"),
        ENTITY_FORMAT_VERSION_V2_COMPACT
    );
    assert_eq!(scan_all(&store, ITEMS), [a, b, c]);
    assert_eq!(store.collection_row_count(ITEMS).unwrap(), Some(3));

    // Rewriting an unchanged v1 row upgrades it too.
    let a = store.get_entity(ITEMS, "a").unwrap().unwrap();
    put(&mut store, std::slice::from_ref(&a));
    assert_eq!(
        version_of(&store, ITEMS, "a"),
        ENTITY_FORMAT_VERSION_V2_COMPACT
    );
    assert_eq!(store.get_entity(ITEMS, "a").unwrap(), Some(a));
}

#[test]
fn dictionary_grows_across_transactions_and_reopen() {
    let mut store = new_store(EntityPayloadFormat::Compact);
    let first = entity(ITEMS, "a", &[("x", Value::I64(1))]);
    put(&mut store, std::slice::from_ref(&first));
    let second = entity(ITEMS, "b", &[("x", Value::I64(2)), ("y", Value::I64(3))]);
    let other = entity(OTHER, "o", &[("y", Value::Null)]);
    put(&mut store, &[second.clone(), other.clone()]);
    assert_eq!(dictionary(&store, ITEMS), ["x", "y"]);
    assert_eq!(dictionary(&store, OTHER), ["y"]);

    let mut store = reopen(store, EntityPayloadFormat::Compact);
    let mut nested = Object::new();
    nested.insert("inner", Value::Bool(true));
    let third = entity(
        ITEMS,
        "c",
        &[("z", Value::Object(nested)), ("x", Value::I64(4))],
    );
    put(&mut store, std::slice::from_ref(&third));
    // Existing ids are kept; new names are appended.
    assert_eq!(dictionary(&store, ITEMS), ["x", "y", "z", "inner"]);

    let store = reopen(store, EntityPayloadFormat::Compact);
    assert_eq!(scan_all(&store, ITEMS), [first, second, third]);
    assert_eq!(scan_all(&store, OTHER), [other]);
}

#[test]
fn snapshots_decode_with_the_dictionary_matching_their_rows() {
    let mut store = new_store(EntityPayloadFormat::Compact);
    let old = entity(ITEMS, "a", &[("old", Value::I64(1))]);
    put(&mut store, std::slice::from_ref(&old));
    // A cold cache: the snapshots load dictionaries through their handles.
    let mut store = reopen(store, EntityPayloadFormat::Compact);
    let before = EntityStorage::owned_snapshot(&store).unwrap().unwrap();

    let new = entity(
        ITEMS,
        "b",
        &[("new", Value::I64(2)), ("old", Value::I64(3))],
    );
    put(&mut store, std::slice::from_ref(&new));

    let rows = before
        .scan_collection_stream(ITEMS)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows, std::slice::from_ref(&old));
    assert_eq!(before.get_entity(ITEMS, "b").unwrap(), None);
    assert_eq!(scan_all(&store, ITEMS), [old, new]);

    // Snapshots opened after a dictionary was cached do not see later rows
    // (and their names); new handles do.
    let after = EntityStorage::owned_snapshot(&store).unwrap().unwrap();
    let newest = entity(ITEMS, "c", &[("newest", Value::I64(4))]);
    put(&mut store, std::slice::from_ref(&newest));
    assert_eq!(after.get_entity(ITEMS, "c").unwrap(), None);
    assert_eq!(
        EntityStorage::snapshot(&store)
            .unwrap()
            .get_entity(ITEMS, "c")
            .unwrap(),
        Some(newest)
    );
}

#[test]
fn rewrite_all_entities_upgrades_legacy_rows() {
    let mut store = new_store(EntityPayloadFormat::SelfContained);
    let rows = [
        entity(ITEMS, "a", &[("kind", Value::String("music".into()))]),
        entity(ITEMS, "b", &[("kind", Value::String("video".into()))]),
        entity(OTHER, "o", &[("title", Value::String("other".into()))]),
    ];
    put(&mut store, &rows);
    // Already in the store's (self-contained) format.
    assert_eq!(store.rewrite_all_entities_to_current_format().unwrap(), 0);

    let mut store = reopen(store, EntityPayloadFormat::Compact);
    let revision = store.current_revision().unwrap();
    assert_eq!(store.rewrite_all_entities_to_current_format().unwrap(), 3);
    assert_ne!(store.current_revision().unwrap(), revision);
    for row in &rows {
        assert_eq!(
            version_of(&store, LocalCollectionId(row.collection), &row.id),
            ENTITY_FORMAT_VERSION_V2_COMPACT
        );
    }
    assert_eq!(dictionary(&store, ITEMS), ["kind"]);
    assert_eq!(dictionary(&store, OTHER), ["title"]);
    assert_eq!(scan_all(&store, ITEMS), rows[..2]);
    assert_eq!(scan_all(&store, OTHER), rows[2..]);
    assert_eq!(store.collection_row_count(ITEMS).unwrap(), Some(2));
    assert_eq!(store.collection_row_count(OTHER).unwrap(), Some(1));

    // Nothing left to do: the second run writes nothing.
    let revision = store.current_revision().unwrap();
    assert_eq!(store.rewrite_all_entities_to_current_format().unwrap(), 0);
    assert_eq!(store.current_revision().unwrap(), revision);

    // And back to the self-contained format.
    let mut store = reopen(store, EntityPayloadFormat::SelfContained);
    assert_eq!(store.rewrite_all_entities_to_current_format().unwrap(), 3);
    let payload = store
        .get_raw(&keys::entity_key(ITEMS, "a"))
        .unwrap()
        .unwrap();
    assert_eq!(payload, encode_entity(&rows[0]).unwrap());
}

#[test]
fn clearing_a_collection_keeps_its_dictionary() {
    let mut store = new_store(EntityPayloadFormat::Compact);
    put(&mut store, &[entity(ITEMS, "a", &[("x", Value::I64(1))])]);
    store
        .apply_batch(&[StorageWriteOp::ClearCollection(ITEMS)])
        .unwrap();
    assert!(scan_all(&store, ITEMS).is_empty());
    assert_eq!(dictionary(&store, ITEMS), ["x"]);

    let row = entity(ITEMS, "b", &[("y", Value::I64(2)), ("x", Value::I64(3))]);
    put(&mut store, std::slice::from_ref(&row));
    assert_eq!(dictionary(&store, ITEMS), ["x", "y"]);
    assert_eq!(scan_all(&store, ITEMS), [row]);
}

#[test]
fn conflicting_writes_do_not_allocate_dictionary_ids() {
    let mut store = new_store(EntityPayloadFormat::Compact);
    let stale = store
        .current_revision()
        .unwrap()
        .map(|revision| revision + 1);
    let outcome = store
        .apply_batch_conditional(
            &[StorageWriteOp::PutEntity(entity(
                ITEMS,
                "a",
                &[("lost", Value::I64(1))],
            ))],
            stale,
        )
        .unwrap();
    assert!(matches!(outcome, StorageCommitOutcome::Conflict { .. }));
    assert!(dictionary(&store, ITEMS).is_empty());

    let row = entity(ITEMS, "b", &[("kept", Value::I64(2))]);
    put(&mut store, std::slice::from_ref(&row));
    assert_eq!(dictionary(&store, ITEMS), ["kept"]);
    assert_eq!(scan_all(&store, ITEMS), [row]);
}

/// Replace every compact payload with its self-contained (version 1) form
/// and drop the dictionaries, like a database written before the compact
/// format existed.
fn downgrade_payloads(store: Store) -> Store {
    let mut store = reopen(store, EntityPayloadFormat::SelfContained);
    let rewritten = store.rewrite_all_entities_to_current_format().unwrap();
    assert!(rewritten > 0);
    let mut engine = store.into_inner();
    for (key, _) in engine
        .scan_prefix(&keys::meta_key(keys::META_FIELD_DICT))
        .unwrap()
    {
        engine.delete(&key).unwrap();
    }
    EntityStore::new(engine)
}

#[test]
fn database_written_with_v1_payloads_opens_and_upgrades() {
    let mut db = crate::open_memory().unwrap();
    db.create_collection("items", CollectionKind::Untyped)
        .unwrap();
    for (id, kind) in [("one", "music"), ("two", "video")] {
        let mut object = Object::new();
        object.insert("id", Value::String(id.to_string()));
        object.insert("kind", Value::String(kind.to_string()));
        db.insert("items", id, object).unwrap();
    }
    let (_, store) = db.into_parts();
    let store = downgrade_payloads(store);
    assert!(
        store
            .scan_raw_prefix(&[keys::TAG_ENTITY])
            .unwrap()
            .iter()
            .all(|(_, payload)| payload_version(payload).unwrap()
                == ENTITY_FORMAT_VERSION_V1_MSGPACK)
    );

    let mut db = EmbeddedDb::open(store).unwrap();
    let row = db.get("items", "two").unwrap().unwrap();
    assert_eq!(
        row.object.get("kind"),
        Some(&Value::String("video".to_string()))
    );
    let mut object = Object::new();
    object.insert("id", Value::String("three".to_string()));
    object.insert("kind", Value::String("text".to_string()));
    db.insert("items", "three", object).unwrap();
    let rows = db
        .select(SelectQuery::new().with_collection("items"))
        .unwrap();
    assert_eq!(rows.len(), 3);
    let items = db.catalog().collection_by_name("items").unwrap().lid;
    let (_, mut store) = db.into_parts();
    assert_eq!(
        version_of(&store, items, "one"),
        ENTITY_FORMAT_VERSION_V1_MSGPACK
    );
    assert_eq!(
        version_of(&store, items, "three"),
        ENTITY_FORMAT_VERSION_V2_COMPACT
    );

    assert!(store.rewrite_all_entities_to_current_format().unwrap() > 0);
    let db = EmbeddedDb::open(store).unwrap();
    let rows = db
        .select(SelectQuery::new().with_collection("items"))
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        db.get("items", "one").unwrap().unwrap().object.get("kind"),
        Some(&Value::String("music".to_string()))
    );
}
