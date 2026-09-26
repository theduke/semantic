//! Random write sequences with new and known field names across several
//! collections keep every row decodable, through borrowed and owned
//! snapshots taken at any point, after reopening with a cold dictionary
//! cache, and with rows of both payload formats.

use std::collections::BTreeMap;
use std::sync::Arc;

use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::LocalCollectionId;
use semantic_db_core::embedded::{
    EntityReadSnapshot, EntityStorage, StorageWriteOp, StoredEntity, StoredEntityKind,
};

use super::payloads::{first_difference, to_v1_durations};
use super::{deep_value, rng};
use crate::storage::EntityPayloadFormat;
use crate::test_values::Rng;
use crate::{EntityStore, MemoryKvEngine};

const COLLECTIONS: [LocalCollectionId; 3] = [
    LocalCollectionId(1),
    LocalCollectionId(2),
    LocalCollectionId(300),
];

type Model = BTreeMap<(usize, String), Object>;

/// Field names: a slowly growing pool shared by all collections, with
/// nested objects using the same names.
struct Names(usize);

impl Names {
    fn pick(&mut self, rng: &mut Rng) -> String {
        if rng.below(6) == 0 {
            self.0 += 1;
        }
        format!("field-{}", rng.below(self.0 as u64 + 1))
    }

    fn object(&mut self, rng: &mut Rng, id: &str, depth: u32) -> Object {
        let mut object = Object::new();
        if depth == 0 {
            object.insert("id", Value::String(id.to_string()));
        }
        for _ in 0..rng.below(5) {
            let value = if depth < 2 && rng.below(4) == 0 {
                Value::Object(self.object(rng, id, depth + 1))
            } else {
                // Durations at millisecond precision survive both formats.
                to_v1_durations(if rng.below(8) == 0 {
                    deep_value(rng, 1)
                } else {
                    rng.value(1)
                })
            };
            object.insert(self.pick(rng), value);
        }
        object
    }
}

fn format(rng: &mut Rng) -> EntityPayloadFormat {
    if rng.below(4) == 0 {
        EntityPayloadFormat::SelfContained
    } else {
        EntityPayloadFormat::Compact
    }
}

fn expected_rows(model: &Model, collection: LocalCollectionId) -> Vec<(String, Object)> {
    model
        .iter()
        .filter(|((lid, _), _)| *lid == collection.0)
        .map(|((_, id), object)| (id.clone(), object.clone()))
        .collect()
}

fn assert_snapshot_matches(snapshot: &dyn EntityReadSnapshot, model: &Model, context: &str) {
    for collection in COLLECTIONS {
        let rows = snapshot
            .scan_collection(collection)
            .unwrap_or_else(|err| panic!("{context}: scan: {err}"))
            .into_iter()
            .map(|row| (row.id, row.object))
            .collect::<Vec<_>>();
        let expected = expected_rows(model, collection);
        if rows != expected {
            let ids = |rows: &[(String, Object)]| {
                rows.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>()
            };
            assert_eq!(
                ids(&rows),
                ids(&expected),
                "{context}: scan of {collection:?}"
            );
            for ((id, row), (_, object)) in rows.iter().zip(&expected) {
                assert!(
                    row == object,
                    "{context}: scan of {collection:?} row {id:?}: {}",
                    first_difference(&Value::Object(object.clone()), &Value::Object(row.clone()))
                );
            }
        }
        for (id, object) in &expected {
            let row = snapshot
                .get_entity(collection, id)
                .unwrap_or_else(|err| panic!("{context}: get {id:?}: {err}"))
                .unwrap_or_else(|| panic!("{context}: missing {id:?}"));
            assert!(
                &row.object == object,
                "{context}: get {id:?}: {}",
                first_difference(&Value::Object(object.clone()), &Value::Object(row.object))
            );
        }
    }
}

#[test]
fn random_writes_keep_every_row_decodable_through_all_snapshots() {
    for round in 0..3 {
        let (mut rng, _seed) = rng(0xD1C7_0000 + round);
        let mut store = EntityStore::with_payload_format(MemoryKvEngine::new(), format(&mut rng));
        store.ensure_stats().unwrap();
        let mut names = Names(2);
        let mut model = Model::new();
        // Snapshots with the model state they observed; checked again after
        // later writes allocated new field ids.
        let mut owned: Vec<(Arc<dyn EntityReadSnapshot>, Model)> = Vec::new();

        for step in 0..200 {
            let mut ops = Vec::new();
            let mut next = model.clone();
            for _ in 0..1 + rng.below(4) {
                let collection = COLLECTIONS[rng.below(3) as usize];
                let id = format!("row-{}", rng.below(20));
                match rng.below(10) {
                    0 | 1 => {
                        next.remove(&(collection.0, id.clone()));
                        ops.push(StorageWriteOp::DeleteEntity {
                            collection,
                            entity_id: id,
                        });
                    }
                    2 if rng.below(8) == 0 => {
                        next.retain(|(lid, _), _| *lid != collection.0);
                        ops.push(StorageWriteOp::ClearCollection(collection));
                    }
                    _ => {
                        let object = names.object(&mut rng, &id, 0);
                        next.insert((collection.0, id.clone()), object.clone());
                        ops.push(StorageWriteOp::PutEntity(StoredEntity {
                            id,
                            collection: collection.0,
                            kind: StoredEntityKind::Untyped,
                            object,
                        }));
                    }
                }
            }
            store.apply_batch(&ops).unwrap();
            model = next;

            let context = format!("round {round} step {step}");
            assert_snapshot_matches(store.snapshot().unwrap().as_ref(), &model, &context);
            if rng.below(8) == 0
                && let Some(snapshot) = store.owned_snapshot().unwrap()
            {
                owned.push((snapshot, model.clone()));
            }
            if rng.below(25) == 0 {
                // Cold dictionary cache, possibly switching payload formats.
                store = EntityStore::with_payload_format(store.into_inner(), format(&mut rng));
            }
        }
        assert!(!owned.is_empty());
        for (index, (snapshot, observed)) in owned.iter().enumerate() {
            assert_snapshot_matches(
                snapshot.as_ref(),
                observed,
                &format!("round {round} owned snapshot {index}"),
            );
        }
        let reopened = EntityStore::new(store.into_inner());
        assert_snapshot_matches(
            reopened.snapshot().unwrap().as_ref(),
            &model,
            &format!("round {round} reopened"),
        );
        // Dictionaries stay dense (loading checks it) and hold every name
        // once.
        for collection in COLLECTIONS {
            let dict = reopened.field_dictionary(collection, true).unwrap();
            let mut names = (0..dict.len() as u32)
                .map(|id| {
                    let key = crate::keys::field_dict_key(collection, id);
                    reopened.get_raw(&key).unwrap().unwrap()
                })
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), dict.len(), "duplicate dictionary entries");
        }
    }
}
