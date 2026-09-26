//! Entity payload formats on a redb file: legacy (version 1) rows, lazy
//! upgrades, field-name dictionary growth across reopen, and the explicit
//! rewrite to the compact format.

use std::path::Path;

use semantic_data::schema::DbOpenMode;
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::LocalCollectionId;
use semantic_db_core::embedded::{EntityStorage, StorageWriteOp, StoredEntity, StoredEntityKind};
use semantic_db_kv::keys;
use semantic_db_kv::{
    ENTITY_FORMAT_VERSION_V1_MSGPACK, ENTITY_FORMAT_VERSION_V2_COMPACT, EntityPayloadFormat,
    EntityStore,
};

use crate::RedbKvEngine;

type Store = EntityStore<RedbKvEngine>;

const ITEMS: LocalCollectionId = LocalCollectionId(3);

fn open(path: &Path, format: EntityPayloadFormat) -> Store {
    let engine = RedbKvEngine::open(path, DbOpenMode::AutoCreate).unwrap();
    let mut store = EntityStore::with_payload_format(engine, format);
    store.prepare_open().unwrap();
    store
}

fn entity(id: &str, fields: &[(&str, Value)]) -> StoredEntity {
    let mut object = Object::new();
    object.insert("id", Value::String(id.to_string()));
    for (name, value) in fields {
        object.insert(*name, value.clone());
    }
    StoredEntity {
        id: id.to_string(),
        collection: ITEMS.0,
        kind: StoredEntityKind::Record,
        object,
    }
}

fn put(store: &mut Store, entities: &[StoredEntity]) {
    let ops = entities
        .iter()
        .cloned()
        .map(StorageWriteOp::PutEntity)
        .collect::<Vec<_>>();
    store.apply_batch(&ops).unwrap();
}

fn version_of(store: &Store, id: &str) -> u16 {
    let payload = store
        .get_raw(&keys::entity_key(ITEMS, id))
        .unwrap()
        .unwrap();
    u16::from_le_bytes([payload[0], payload[1]])
}

fn dictionary(store: &Store) -> Vec<String> {
    store
        .scan_raw_prefix(&keys::field_dict_prefix(ITEMS))
        .unwrap()
        .into_iter()
        .map(|(_, name)| String::from_utf8(name).unwrap())
        .collect()
}

fn rows(store: &Store) -> Vec<StoredEntity> {
    let direct = store.scan_collection(ITEMS).unwrap();
    let owned = store
        .owned_snapshot()
        .unwrap()
        .expect("redb provides owned snapshots")
        .scan_collection_stream(ITEMS)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(direct, owned);
    direct
}

#[test]
fn redb_entity_formats_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let a = entity("a", &[("semantic:title", Value::String("A".into()))]);
    let b = entity("b", &[("semantic:title", Value::String("B".into()))]);
    // Written by code predating the compact format.
    {
        let mut store = open(&path, EntityPayloadFormat::SelfContained);
        put(&mut store, &[a.clone(), b.clone()]);
    }

    let b = entity(
        "b",
        &[
            ("semantic:title", Value::String("B2".into())),
            (
                "semantic:tags",
                Value::List(vec![Value::String("x".into())]),
            ),
        ],
    );
    let c = entity("c", &[("semantic:title", Value::String("C".into()))]);
    {
        let mut store = open(&path, EntityPayloadFormat::Compact);
        assert_eq!(
            rows(&store),
            [
                a.clone(),
                entity("b", &[("semantic:title", Value::String("B".into()))])
            ]
        );
        let snapshot = store.owned_snapshot().unwrap().unwrap();
        put(&mut store, &[b.clone(), c.clone()]);
        assert_eq!(version_of(&store, "a"), ENTITY_FORMAT_VERSION_V1_MSGPACK);
        assert_eq!(version_of(&store, "b"), ENTITY_FORMAT_VERSION_V2_COMPACT);
        assert_eq!(dictionary(&store), ["semantic:tags", "semantic:title"]);
        // The snapshot keeps observing its own rows.
        assert_eq!(snapshot.get_entity(ITEMS, "c").unwrap(), None);
        assert_eq!(
            snapshot
                .get_entity(ITEMS, "b")
                .unwrap()
                .unwrap()
                .object
                .get("semantic:title"),
            Some(&Value::String("B".into()))
        );
        assert_eq!(rows(&store), [a.clone(), b.clone(), c.clone()]);
    }

    let d = entity(
        "d",
        &[
            ("semantic:title", Value::String("D".into())),
            ("semantic:created_at", Value::I64(1)),
        ],
    );
    {
        let mut store = open(&path, EntityPayloadFormat::Compact);
        put(&mut store, std::slice::from_ref(&d));
        assert_eq!(
            dictionary(&store),
            ["semantic:tags", "semantic:title", "semantic:created_at"]
        );
        assert_eq!(store.rewrite_all_entities_to_current_format().unwrap(), 1);
        assert_eq!(store.rewrite_all_entities_to_current_format().unwrap(), 0);
    }

    let store = open(&path, EntityPayloadFormat::Compact);
    for id in ["a", "b", "c", "d"] {
        assert_eq!(version_of(&store, id), ENTITY_FORMAT_VERSION_V2_COMPACT);
    }
    assert_eq!(rows(&store), [a, b, c, d]);
    assert_eq!(store.collection_row_count(ITEMS).unwrap(), Some(4));
}
