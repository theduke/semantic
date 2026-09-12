use super::*;
use semantic_data::schema::{RelationIndexingMode, RelationMode, RelationType};
use semantic_db_core::catalog::CollectionKind;
use semantic_db_core::embedded::EmbeddedDb;
use semantic_db_core::{Batch, BatchOperation};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct Probe {
    writes: Vec<KvWriteOp>,
    fail: bool,
    gets: usize,
    scans: Vec<Vec<u8>>,
}

#[derive(Debug)]
struct ObservedEngine {
    engine: MemoryKvEngine,
    probe: Arc<Mutex<Probe>>,
}

impl KvEngine for ObservedEngine {
    type PrefixScan = BoxKvPrefixScan;
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        self.probe.lock().unwrap().gets += 1;
        self.engine.get(key)
    }
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), DbError> {
        self.engine.put(key, value)
    }
    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.engine.delete(key)
    }
    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<Self::PrefixScan, DbError> {
        self.probe.lock().unwrap().scans.push(prefix.clone());
        self.engine.scan_prefix_stream(prefix)
    }
    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.engine.tx_capabilities()
    }
    fn current_revision(&self) -> Result<Option<u64>, DbError> {
        self.engine.current_revision()
    }
    fn write_batch(&mut self, ops: &[KvWriteOp]) -> Result<(), DbError> {
        let mut probe = self.probe.lock().unwrap();
        if probe.fail {
            return Err(DbError::Storage("injected commit failure".into()));
        }
        probe.writes = ops.to_vec();
        self.engine.write_batch(ops)
    }
    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        if revision != self.current_revision()? {
            return Ok(StorageCommitOutcome::Conflict {
                expected_revision: revision,
                actual_revision: self.current_revision()?,
            });
        }
        self.write_batch(ops)?;
        Ok(StorageCommitOutcome::Committed {
            revision: self.current_revision()?,
        })
    }
}

fn object(id: &str, name: &str) -> Object {
    let mut object = Object::new();
    object.insert("id", Value::String(id.into()));
    object.insert("name", Value::String(name.into()));
    object
}

fn setup() -> (EmbeddedDb<EntityStore<ObservedEngine>>, Arc<Mutex<Probe>>) {
    let probe = Arc::new(Mutex::new(Probe::default()));
    let mut db = EmbeddedDb::open(EntityStore::new(ObservedEngine {
        engine: MemoryKvEngine::new(),
        probe: probe.clone(),
    }))
    .unwrap();
    let collection = db
        .create_collection("items", CollectionKind::Polymorphic)
        .unwrap();
    db.create_index("by_name", collection, "name", true)
        .unwrap();
    (db, probe)
}

#[test]
fn compact_kv_append_has_constant_point_index_reads_and_writes() {
    use semantic_db_core::BatchReturn;
    let mut measurements = Vec::new();
    for size in [10, 1_000] {
        let (mut db, probe) = setup();
        db.execute_batch(Batch {
            operations: (0..size)
                .map(|i| {
                    let id = format!("row-{i}");
                    BatchOperation::Upsert {
                        collection: "items".into(),
                        object: object(&id, &id),
                        id,
                    }
                })
                .collect(),
        })
        .unwrap();
        *probe.lock().unwrap() = Probe::default();
        db.execute_batch_returning(
            Batch::new().with_op(BatchOperation::Create {
                collection: "items".into(),
                id: "new".into(),
                object: object("new", "new"),
            }),
            BatchReturn::Changes,
        )
        .unwrap();
        let probe = probe.lock().unwrap();
        assert!(
            !probe.scans.iter().any(|prefix| prefix.starts_with(b"c/")),
            "compact execution must not scan entity collections: {:?}",
            probe.scans
        );
        measurements.push((probe.gets, probe.scans.len(), probe.writes.len()));
    }
    assert_eq!(measurements[0], measurements[1]);
    assert!(measurements[0].0 < 20, "{:?}", measurements[0]);
}

#[test]
fn compact_kv_failed_commit_and_reopen_preserve_objects_indexes_and_contributors() {
    use semantic_db_core::BatchReturn;
    let (mut db, probe) = setup();
    db.upsert_relationship(RelationType {
        id: "compact-direct".into(),
        name: "compact-direct".into(),
        source_collection: "items".into(),
        mode: RelationMode::External,
        indexing_mode: RelationIndexingMode::Disabled,
        meta: Default::default(),
    })
    .unwrap();
    let relation_row = |id: &str| {
        let mut row = object(id, id);
        row.insert("from", "source".to_string());
        row.insert("to", "target".to_string());
        row
    };
    db.execute_batch_returning(
        Batch {
            operations: ["one", "two"]
                .into_iter()
                .map(|id| BatchOperation::Create {
                    collection: "items".into(),
                    id: id.into(),
                    object: relation_row(id),
                })
                .collect(),
        },
        BatchReturn::Stats,
    )
    .unwrap();
    let delete = |id: &str| BatchOperation::DeleteById {
        collection: "items".into(),
        id: id.into(),
    };
    db.execute_batch_returning(Batch::new().with_op(delete("one")), BatchReturn::Stats)
        .unwrap();
    let edges = db
        .catalog()
        .collection_by_name("__semantic.relationship_edges")
        .unwrap()
        .lid;
    assert_eq!(db.collection_rows(edges).unwrap().len(), 1);
    probe.lock().unwrap().fail = true;
    assert!(
        db.execute_batch_returning(Batch::new().with_op(delete("two")), BatchReturn::Changes)
            .is_err()
    );
    probe.lock().unwrap().fail = false;
    let (_, store) = db.into_parts();
    let mut db = EmbeddedDb::open(store).unwrap();
    assert!(db.get("items", "two").unwrap().is_some());
    assert_eq!(db.collection_rows(edges).unwrap().len(), 1);
    db.execute_batch_returning(Batch::new().with_op(delete("two")), BatchReturn::Changes)
        .unwrap();
    assert!(db.collection_rows(edges).unwrap().is_empty());
}

#[test]
fn incremental_write_counts_are_independent_of_collection_size() {
    let mut counts = Vec::new();
    for size in [10, 1_000] {
        let (mut db, probe) = setup();
        let mut batch = Batch::new();
        for i in 0..size {
            let id = format!("row-{i}");
            batch = batch.with_op(BatchOperation::Upsert {
                collection: "items".into(),
                id: id.clone(),
                object: object(&id, &id),
            });
        }
        db.execute_batch(batch).unwrap();
        db.insert("items", "row-0", object("row-0", "changed"))
            .unwrap();
        let writes = probe.lock().unwrap().writes.clone();
        let entity_writes = writes
            .iter()
            .filter(|op| match op {
                KvWriteOp::Put { key, .. } | KvWriteOp::Delete { key } => {
                    parse_entity_key(key).is_some()
                }
            })
            .count();
        assert_eq!(entity_writes, 1, "only the changed entity may be persisted");
        assert!(
            writes.len() <= 5,
            "one entity and two changed index key pairs: {writes:?}"
        );
        counts.push(writes.len());
        db.insert("items", "row-0", object("row-0", "changed"))
            .unwrap();
        assert!(
            probe.lock().unwrap().writes.is_empty(),
            "no-op must produce zero physical writes"
        );
    }
    assert_eq!(counts[0], counts[1]);
    eprintln!("incremental scalar edit writes at 10/1000 rows: {counts:?}");
}

#[test]
fn incremental_unique_swap_reopen_and_failed_commit() {
    let (mut db, probe) = setup();
    db.insert("items", "a", object("a", "first")).unwrap();
    db.insert("items", "b", object("b", "second")).unwrap();
    db.execute_batch(
        Batch::new()
            .with_op(BatchOperation::Upsert {
                collection: "items".into(),
                id: "a".into(),
                object: object("a", "second"),
            })
            .with_op(BatchOperation::Upsert {
                collection: "items".into(),
                id: "b".into(),
                object: object("b", "first"),
            }),
    )
    .unwrap();
    let (_, store) = db.into_parts();
    let mut db = EmbeddedDb::open(store).unwrap();
    let catalog = db.catalog();
    let index = catalog
        .indexes_for_collection(catalog.collection_by_name("items").unwrap().lid)
        .find(|index| index.schema.name == "by_name")
        .unwrap();
    let (_, store) = db.into_parts();
    assert_eq!(
        store
            .scan_index_value(index.lid, None, &Value::String("first".into()))
            .unwrap(),
        vec!["b"]
    );
    assert!(!store.index_needs_rebuild(index.lid).unwrap());
    db = EmbeddedDb::open(store).unwrap();
    probe.lock().unwrap().fail = true;
    assert!(db.insert("items", "a", object("a", "failed")).is_err());
    probe.lock().unwrap().fail = false;
    assert_eq!(
        db.get("items", "a")
            .unwrap()
            .unwrap()
            .object
            .get(&index.canonical_field),
        Some(&Value::String("second".into()))
    );
    db.delete("items", "a").unwrap();
    let (_, store) = db.into_parts();
    assert!(
        store
            .scan_index_value(index.lid, None, &Value::String("second".into()))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn incremental_duplicate_contributors_survive_reopen_and_failed_delete() {
    let (mut db, probe) = setup();
    db.upsert_relationship(RelationType {
        id: "membership".into(),
        name: "membership".into(),
        source_collection: "items".into(),
        mode: RelationMode::External,
        indexing_mode: RelationIndexingMode::Enabled,
        meta: Default::default(),
    })
    .unwrap();
    for id in ["a", "b"] {
        let mut row = object(id, id);
        row.insert("from", Value::String("source".into()));
        row.insert("to", Value::String("target".into()));
        db.insert("items", id, row).unwrap();
    }
    let (_, store) = db.into_parts();
    let mut db = EmbeddedDb::open(store).unwrap();
    db.delete("items", "a").unwrap();
    let edge_collection = db
        .catalog()
        .collection_by_name("__semantic.relationship_edges")
        .unwrap()
        .lid;
    assert_eq!(db.collection_rows(edge_collection).unwrap().len(), 1);
    assert!(
        !probe.lock().unwrap().writes.iter().any(|op| match op {
            KvWriteOp::Put { key, .. } | KvWriteOp::Delete { key } =>
                parse_entity_key(key).is_some_and(|(collection, _)| collection == edge_collection),
        }),
        "removing one duplicate must not write any edge"
    );
    probe.lock().unwrap().fail = true;
    assert!(db.delete("items", "b").is_err());
    probe.lock().unwrap().fail = false;
    assert_eq!(db.collection_rows(edge_collection).unwrap().len(), 1);
    db.delete("items", "b").unwrap();
    assert!(db.collection_rows(edge_collection).unwrap().is_empty());
}

#[test]
fn incremental_reopen_backfills_legacy_contributors_atomically() {
    let (mut db, _) = setup();
    db.upsert_relationship(RelationType {
        id: "legacy".into(),
        name: "legacy".into(),
        source_collection: "items".into(),
        mode: RelationMode::External,
        indexing_mode: RelationIndexingMode::Disabled,
        meta: Default::default(),
    })
    .unwrap();
    for id in ["a", "b"] {
        let mut row = object(id, id);
        row.insert("from", Value::String("source".into()));
        row.insert("to", Value::String("target".into()));
        db.insert("items", id, row).unwrap();
    }
    let catalog = db.catalog();
    let contributors = catalog
        .collection_by_name("__semantic.relationship_contributors")
        .unwrap()
        .lid;
    let counts = catalog
        .collection_by_name("__semantic.relationship_counts")
        .unwrap()
        .lid;
    let edges = catalog
        .collection_by_name("__semantic.relationship_edges")
        .unwrap()
        .lid;
    let (_, mut store) = db.into_parts();
    store
        .apply_batch(&[
            StorageWriteOp::ClearCollection(contributors),
            StorageWriteOp::ClearCollection(counts),
        ])
        .unwrap();
    let mut db = EmbeddedDb::open(store).unwrap();
    assert_eq!(db.collection_rows(contributors).unwrap().len(), 2);
    assert_eq!(
        db.collection_rows(counts).unwrap().len(),
        2,
        "count and persisted completion marker"
    );
    db.delete("items", "a").unwrap();
    assert_eq!(db.collection_rows(edges).unwrap().len(), 1);
    db.delete("items", "b").unwrap();
    assert!(db.collection_rows(edges).unwrap().is_empty());
}

#[test]
fn incremental_transitive_depth_changes_leave_other_relations_untouched() {
    let (mut db, probe) = setup();
    for relation in ["chain", "unrelated"] {
        db.upsert_relationship(RelationType {
            id: relation.into(),
            name: relation.into(),
            source_collection: "items".into(),
            mode: RelationMode::External,
            indexing_mode: RelationIndexingMode::Enabled,
            meta: Default::default(),
        })
        .unwrap();
    }
    for (id, relation, source, target) in [
        ("first", "chain", "a", "b"),
        ("second", "chain", "b", "c"),
        ("shortcut", "chain", "a", "c"),
        ("other", "unrelated", "x", "y"),
    ] {
        let mut row = object(id, id);
        row.insert("relation", Value::String(relation.into()));
        row.insert("from", Value::String(source.into()));
        row.insert("to", Value::String(target.into()));
        db.insert("items", id, row).unwrap();
    }
    db.delete("items", "shortcut").unwrap();
    let edge_collection = db
        .catalog()
        .collection_by_name("__semantic.relationship_edges")
        .unwrap()
        .lid;
    let edge = db
        .collection_rows(edge_collection)
        .unwrap()
        .into_iter()
        .find(|row| row.id == "chain|a|c")
        .unwrap();
    assert_eq!(edge.object.get("depth"), Some(&Value::U64(2)));
    assert!(!probe.lock().unwrap().writes.iter().any(|operation| {
        match operation {
            KvWriteOp::Put { key, .. } | KvWriteOp::Delete { key } => parse_entity_key(key)
                .is_some_and(|(collection, id)| {
                    collection == edge_collection && id.starts_with("unrelated|")
                }),
        }
    }));
    db.delete("items", "second").unwrap();
    assert!(
        !db.collection_rows(edge_collection)
            .unwrap()
            .iter()
            .any(|row| row.id == "chain|a|c")
    );
}
