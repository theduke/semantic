//! Physical writes per entity write, measured inside the write transaction.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use semantic_data::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassType, EntityRef, Meta, Type, TypeKind,
};
use semantic_db_core::catalog::IntegrityMode;
use semantic_db_core::embedded::EmbeddedDb;
use semantic_db_core::{
    Batch, BatchOperation, BatchReturn, DdlBatch, DdlCollectionKind, DdlOperation,
};

use super::*;
use crate::keys;
use semantic_data::schema::{IndexSchema as DataIndexSchema, KeyPath};
use semantic_db_core::catalog::IndexSchema;

const COLLECTION: &str = "articles";
const CLASS: &str = "test:article";
const AUTHOR: &str = "test:author";
const EDITOR: &str = "test:editor";
/// The equality indexes created by the test.
const INDEXED: [&str; 3] = ["headline", "status", "category"];
/// Built-in equality indexes of every polymorphic collection.
const BUILTIN_INDEXES: [&str; 2] = ["id", "type"];
/// Index entries of one article row.
const ROW_ENTRIES: usize = BUILTIN_INDEXES.len() + INDEXED.len();
/// Index entries of one reverse reference row (`id` and `target`).
const REFERENCE_ENTRIES: usize = 2;

/// Key accesses of one write transaction.
#[derive(Debug, Default, Clone)]
struct TxnAccess {
    gets: Vec<Vec<u8>>,
    puts: Vec<Vec<u8>>,
    deletes: Vec<Vec<u8>>,
}

impl TxnAccess {
    fn puts_with_tag(&self, tag: u8) -> usize {
        self.puts.iter().filter(|key| key[0] == tag).count()
    }

    fn gets_with_tag(&self, tag: u8) -> usize {
        self.gets.iter().filter(|key| key[0] == tag).count()
    }

    fn deletes_with_tag(&self, tag: u8) -> usize {
        self.deletes.iter().filter(|key| key[0] == tag).count()
    }
}

/// Counts the `get`/`put`/`delete` calls made through a write transaction.
struct CountingTxn<'a> {
    inner: &'a mut dyn KvWriteTxn,
    gets: RefCell<Vec<Vec<u8>>>,
    access: TxnAccess,
}

impl KvWriteTxn for CountingTxn<'_> {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        self.gets.borrow_mut().push(key.to_vec());
        self.inner.get(key)
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, DbError> {
        self.inner.scan_prefix(prefix)
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), DbError> {
        self.access.puts.push(key.to_vec());
        self.inner.put(key, value)
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.access.deletes.push(key.to_vec());
        self.inner.delete(key)
    }
}

#[derive(Debug)]
struct CountingEngine {
    inner: MemoryKvEngine,
    /// Accesses of every write transaction since the last `take`.
    txns: Arc<Mutex<Vec<TxnAccess>>>,
}

impl KvEngine for CountingEngine {
    type PrefixScan = <MemoryKvEngine as KvEngine>::PrefixScan;

    fn begin_read(&self) -> Result<Box<dyn KvReadTxn + '_>, DbError> {
        self.inner.begin_read()
    }

    fn write_with<F>(
        &mut self,
        expected_revision: Option<u64>,
        f: F,
    ) -> Result<StorageCommitOutcome, DbError>
    where
        F: FnOnce(&mut dyn KvWriteTxn) -> Result<(), DbError>,
    {
        let txns = self.txns.clone();
        self.inner.write_with(expected_revision, |txn| {
            let mut counting = CountingTxn {
                inner: txn,
                gets: RefCell::default(),
                access: TxnAccess::default(),
            };
            let result = f(&mut counting);
            let mut access = counting.access;
            access.gets = counting.gets.into_inner();
            txns.lock().unwrap().push(access);
            result
        })
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        self.inner.get(key)
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), DbError> {
        self.inner.put(key, value)
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.inner.delete(key)
    }

    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<Self::PrefixScan, DbError> {
        self.inner.scan_prefix_stream(prefix)
    }

    fn scan_range_stream(
        &self,
        start: Vec<u8>,
        end: Option<Vec<u8>>,
    ) -> Result<BoxKvPrefixScan, DbError> {
        self.inner.scan_range_stream(start, end)
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        self.inner.tx_capabilities()
    }

    fn current_revision(&self) -> Result<Option<u64>, DbError> {
        self.inner.current_revision()
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> Result<(), DbError> {
        self.inner.write_batch(ops)
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        self.inner.write_batch_conditional(ops, expected_revision)
    }
}

type Db = EmbeddedDb<EntityStore<CountingEngine>>;

fn ref_attribute(id: &str, name: &str) -> DdlOperation {
    DdlOperation::UpsertAttribute {
        attribute: AttributeType {
            id: id.into(),
            name: name.into(),
            ty: Type::new(TypeKind::Ref(EntityRef::new(CLASS))),
            constraints: vec![],
            meta: Meta::default(),
        },
    }
}

fn class_attribute(id: &str) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef { id: id.into() },
        required: false,
        ui_order: None,
        computed: None,
        constraints: vec![],
        meta: Meta::default(),
    }
}

/// A collection with three equality indexes (besides the built-in `id` and
/// `type` indexes) and an article class with two references.
fn setup() -> (Db, Arc<Mutex<Vec<TxnAccess>>>) {
    setup_with_path_index(false)
}

/// [`setup`], optionally with the automatic path index, which indexes
/// every field.
fn setup_with_path_index(path_index: bool) -> (Db, Arc<Mutex<Vec<TxnAccess>>>) {
    let txns = Arc::new(Mutex::new(Vec::new()));
    let mut db = EmbeddedDb::open(EntityStore::new(CountingEngine {
        inner: MemoryKvEngine::new(),
        txns: txns.clone(),
    }))
    .unwrap();
    db.set_auto_index_enabled(path_index).unwrap();
    let mut ddl = DdlBatch::new().with_op(DdlOperation::UpsertCollection {
        name: COLLECTION.into(),
        kind: DdlCollectionKind::Polymorphic,
        integrity_mode: IntegrityMode::Permissive,
    });
    for field in INDEXED {
        ddl = ddl.with_op(DdlOperation::UpsertIndex {
            name: format!("by_{field}"),
            collection: COLLECTION.into(),
            field: field.into(),
            unique: false,
            kind: IndexKind::Equality,
            extra_fields: Vec::new(),
            predicate: None,
        });
    }
    ddl = ddl
        .with_op(ref_attribute(AUTHOR, "author"))
        .with_op(ref_attribute(EDITOR, "editor"))
        .with_op(DdlOperation::UpsertClass {
            class: ClassType {
                id: CLASS.into(),
                name: "Article".into(),
                inherits: None,
                extends: vec![],
                strict_schema: false,
                creatable_in_ui: None,
                attributes: BTreeMap::from([
                    ("author".into(), class_attribute(AUTHOR)),
                    ("editor".into(), class_attribute(EDITOR)),
                ]),
                constraints: vec![],
                meta: Meta::default(),
            },
        });
    db.transact_ddl(ddl).unwrap();
    let collection = db.catalog().collection_by_name(COLLECTION).unwrap().lid;
    assert_eq!(
        db.catalog()
            .indexes_for_collection(collection)
            .map(|index| index.canonical_field.clone())
            .collect::<Vec<_>>(),
        BUILTIN_INDEXES
            .iter()
            .chain(path_index.then_some(&"__path__"))
            .chain(&INDEXED)
            .copied()
            .collect::<Vec<_>>()
    );
    for target in ["alice", "bob", "carol"] {
        upsert(&mut db, article(target, None));
    }
    upsert(&mut db, article("post", Some(("alice", "bob"))));
    txns.lock().unwrap().clear();
    (db, txns)
}

fn article(id: &str, refs: Option<(&str, &str)>) -> Object {
    let mut object = Object::new();
    object.insert("id", id.to_string());
    object.insert("type", CLASS.to_string());
    object.insert("headline", format!("headline of {id}"));
    object.insert("status", "draft".to_string());
    object.insert("category", "news".to_string());
    object.insert("body", "text".to_string());
    if let Some((author, editor)) = refs {
        object.insert(AUTHOR, author.to_string());
        object.insert(EDITOR, editor.to_string());
    }
    object
}

fn upsert(db: &mut Db, object: Object) {
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    db.execute_batch_returning(
        Batch::new().with_op(BatchOperation::Upsert {
            collection: COLLECTION.into(),
            id,
            object,
        }),
        BatchReturn::Stats,
    )
    .unwrap();
}

/// Upsert `object` and return the accesses of its single write transaction.
fn measure(db: &mut Db, txns: &Mutex<Vec<TxnAccess>>, object: Object) -> TxnAccess {
    upsert(db, object);
    let mut txns = std::mem::take(&mut *txns.lock().unwrap());
    assert_eq!(txns.len(), 1, "one write transaction per write: {txns:?}");
    txns.pop().unwrap()
}

fn references_lid(db: &Db) -> LocalCollectionId {
    db.catalog()
        .collection_by_name("__semantic.reverse_references")
        .unwrap()
        .lid
}

fn entity_writes(keys: &[Vec<u8>], collection: LocalCollectionId) -> usize {
    keys.iter()
        .filter(|key| parse_entity_key(key).is_some_and(|(lid, _)| lid == collection))
        .count()
}

/// The maintained counters must equal the actual key counts.
fn assert_counters_match_keys(db: &Db) {
    let store = db.storage();
    for (lid, _) in db.catalog().collections() {
        assert_eq!(
            store.collection_row_count(lid).unwrap(),
            Some(store.collection_keys(lid).unwrap().len() as u64),
            "row counter of collection {lid:?}"
        );
    }
    for (lid, _) in db.catalog().indexes() {
        assert_eq!(
            store.index_entry_count(lid).unwrap(),
            Some(store.index_keys(lid).unwrap().len() as u64),
            "entry counter of index {lid:?}"
        );
    }
}

#[test]
fn updating_a_plain_field_writes_only_the_payload() {
    let (mut db, txns) = setup();
    let mut object = article("post", Some(("alice", "bob")));
    object.insert("body", "edited".to_string());

    let access = measure(&mut db, &txns, object);

    assert_eq!(access.puts.len(), 1, "{access:?}");
    assert_eq!(access.puts_with_tag(keys::TAG_ENTITY), 1);
    assert!(access.deletes.is_empty(), "{access:?}");
    // The payload comparison is the only read.
    assert_eq!(access.gets.len(), 1, "{access:?}");
    assert_counters_match_keys(&db);
}

#[test]
fn updating_a_field_replaces_only_its_path_index_entry() {
    let (mut db, txns) = setup_with_path_index(true);
    let mut object = article("post", Some(("alice", "bob")));
    object.insert("body", "edited".to_string());

    let access = measure(&mut db, &txns, object);

    // The payload plus the one changed path entry of the path index.
    assert_eq!(access.puts_with_tag(keys::TAG_ENTITY), 1, "{access:?}");
    assert_eq!(access.puts_with_tag(keys::TAG_INDEX), 1, "{access:?}");
    assert_eq!(access.deletes_with_tag(keys::TAG_INDEX), 1, "{access:?}");
    assert_eq!(access.puts.len(), 2, "{access:?}");
    assert_eq!(access.deletes.len(), 1, "{access:?}");
    assert_eq!(access.gets.len(), 1, "{access:?}");
    assert_counters_match_keys(&db);
}

#[test]
fn updating_an_indexed_field_writes_one_entry_delta() {
    let (mut db, txns) = setup();
    let mut object = article("post", Some(("alice", "bob")));
    object.insert("status", "published".to_string());

    let access = measure(&mut db, &txns, object);

    assert_eq!(access.puts_with_tag(keys::TAG_ENTITY), 1, "{access:?}");
    assert_eq!(access.puts_with_tag(keys::TAG_INDEX), 1, "{access:?}");
    assert_eq!(access.deletes_with_tag(keys::TAG_INDEX), 1, "{access:?}");
    // The entry count of the index is unchanged, so no counter is written.
    assert_eq!(access.puts.len(), 2, "{access:?}");
    assert_eq!(access.deletes.len(), 1, "{access:?}");
    // Entry states are known from the old object; only the payload is read.
    assert_eq!(access.gets.len(), 1, "{access:?}");
    let collection = db.catalog().collection_by_name(COLLECTION).unwrap().lid;
    let index = db
        .catalog()
        .find_equality_index(collection, "status")
        .unwrap()
        .lid;
    let ids = |value: &str| {
        db.storage()
            .scan_index_value(index, None, &Value::String(value.into()))
            .unwrap()
    };
    assert_eq!(ids("published"), vec!["post".to_string()]);
    assert!(!ids("draft").contains(&"post".to_string()));
    assert_counters_match_keys(&db);
}

#[test]
fn writing_an_identical_object_writes_nothing() {
    let (mut db, txns) = setup();
    let revision = db.storage().current_revision().unwrap();

    let access = measure(&mut db, &txns, article("post", Some(("alice", "bob"))));

    assert!(access.puts.is_empty(), "{access:?}");
    assert!(access.deletes.is_empty(), "{access:?}");
    assert!(access.gets.is_empty(), "{access:?}");
    assert_eq!(db.storage().current_revision().unwrap(), revision);
}

#[test]
fn changing_one_reference_touches_only_its_reverse_reference() {
    let (mut db, txns) = setup();
    let references = references_lid(&db);

    let access = measure(&mut db, &txns, article("post", Some(("alice", "carol"))));

    let collection = db.catalog().collection_by_name(COLLECTION).unwrap().lid;
    // Payload, plus the changed reverse reference row and its entries
    // replaced; the unchanged reference is not touched.
    assert_eq!(entity_writes(&access.puts, collection), 1, "{access:?}");
    assert_eq!(entity_writes(&access.puts, references), 1, "{access:?}");
    assert_eq!(entity_writes(&access.deletes, references), 1, "{access:?}");
    assert_eq!(
        access.puts_with_tag(keys::TAG_INDEX),
        REFERENCE_ENTRIES,
        "{access:?}"
    );
    assert_eq!(
        access.deletes_with_tag(keys::TAG_INDEX),
        REFERENCE_ENTRIES,
        "{access:?}"
    );
    assert_eq!(access.puts.len(), 2 + REFERENCE_ENTRIES, "{access:?}");
    assert_eq!(access.deletes.len(), 1 + REFERENCE_ENTRIES, "{access:?}");
    // Only the payload and the two reverse reference rows are compared.
    assert_eq!(access.gets_with_tag(keys::TAG_ENTITY), 3, "{access:?}");
    assert_eq!(access.gets.len(), 3, "{access:?}");
    assert_counters_match_keys(&db);
}

#[test]
fn inserting_writes_payload_entries_references_and_counters() {
    let (mut db, txns) = setup();
    let references = references_lid(&db);

    let access = measure(&mut db, &txns, article("fresh", Some(("alice", "bob"))));

    let collection = db.catalog().collection_by_name(COLLECTION).unwrap().lid;
    assert_eq!(entity_writes(&access.puts, collection), 1, "{access:?}");
    assert_eq!(entity_writes(&access.puts, references), 2, "{access:?}");
    let entries = ROW_ENTRIES + 2 * REFERENCE_ENTRIES;
    assert_eq!(access.puts_with_tag(keys::TAG_INDEX), entries, "{access:?}");
    // Counters: rows of both collections, the row's indexes and the reverse
    // reference indexes.
    let counters = 2 + ROW_ENTRIES + REFERENCE_ENTRIES;
    assert_eq!(
        access.puts_with_tag(keys::TAG_STATS),
        counters,
        "{access:?}"
    );
    assert_eq!(access.puts.len(), 3 + entries + counters, "{access:?}");
    assert!(access.deletes.is_empty(), "{access:?}");
    // New entity keys are compared; new entries are known to be absent.
    assert_eq!(access.gets_with_tag(keys::TAG_ENTITY), 3, "{access:?}");
    assert_eq!(access.gets_with_tag(keys::TAG_INDEX), 0, "{access:?}");
    assert_counters_match_keys(&db);
}

#[test]
fn deleting_removes_known_entries_and_keeps_counters_exact() {
    let (mut db, txns) = setup();
    db.execute_batch(Batch::new().with_op(BatchOperation::DeleteById {
        collection: COLLECTION.into(),
        id: "post".into(),
    }))
    .unwrap();
    let access = txns.lock().unwrap().pop().unwrap();
    assert_eq!(
        access.deletes_with_tag(keys::TAG_INDEX),
        ROW_ENTRIES + 2 * REFERENCE_ENTRIES,
        "{access:?}"
    );
    assert_eq!(access.gets_with_tag(keys::TAG_INDEX), 0, "{access:?}");
    assert_counters_match_keys(&db);
}

fn kind_index() -> IndexSchema {
    IndexSchema {
        lid: LocalIndexId(3),
        schema: DataIndexSchema {
            id: "items.by_kind".to_string(),
            name: "by_kind".to_string(),
            kind: IndexKind::Equality,
            collection: "items".to_string(),
            key_path: KeyPath {
                segments: vec!["kind".to_string()],
            },
            unique: false,
            extra_key_paths: Vec::new(),
            predicate: None,
        },
        collection: LocalCollectionId(7),
        canonical_field: "kind".to_string(),
        field_id: None,
        attr_id: None,
    }
}

fn kind(value: &str) -> Object {
    let mut object = Object::new();
    object.insert("kind", Value::String(value.to_string()));
    object
}

fn reindex(old: Option<&str>, new: Option<&str>) -> StorageWriteOp {
    StorageWriteOp::ReindexEntity {
        index: kind_index(),
        entity_id: "one".to_string(),
        old: old.map(kind),
        new: new.map(kind),
    }
}

/// Stats-maintaining store with an initialized, empty `by_kind` index.
fn kind_store() -> EntityStore<MemoryKvEngine> {
    let mut store = EntityStore::new(MemoryKvEngine::new());
    store.prepare_open().unwrap();
    store
        .apply_batch(&[StorageWriteOp::ResetIndex(kind_index().lid)])
        .unwrap();
    store
}

fn commit(store: &mut EntityStore<MemoryKvEngine>, ops: &[StorageWriteOp]) {
    let revision = store.current_revision().unwrap();
    assert!(matches!(
        store.apply_batch_conditional(ops, revision).unwrap(),
        StorageCommitOutcome::Committed { .. }
    ));
}

fn assert_kind_entries(store: &EntityStore<MemoryKvEngine>, expected: usize) {
    let index = kind_index().lid;
    assert_eq!(store.index_keys(index).unwrap().len(), expected);
    assert_eq!(
        store.index_entry_count(index).unwrap(),
        Some(expected as u64)
    );
}

#[test]
fn reindex_writes_only_the_entry_difference() {
    let mut store = kind_store();
    commit(&mut store, &[reindex(None, Some("a"))]);
    assert_kind_entries(&store, 1);
    commit(&mut store, &[reindex(Some("a"), Some("b"))]);
    assert_kind_entries(&store, 1);
    assert_eq!(
        store
            .scan_index_value(kind_index().lid, None, &Value::String("b".into()))
            .unwrap(),
        vec!["one".to_string()]
    );
    let revision = store.current_revision().unwrap();
    commit(&mut store, &[reindex(Some("b"), Some("b"))]);
    assert_eq!(store.current_revision().unwrap(), revision);
    commit(&mut store, &[reindex(Some("b"), None)]);
    assert_kind_entries(&store, 0);
}

#[test]
fn reindex_previous_states_are_not_trusted_after_resets_or_repeated_keys() {
    // The old object's entry never existed: a reset in the same batch makes
    // the lowering compare instead of trusting `old`.
    let mut store = kind_store();
    commit(
        &mut store,
        &[
            StorageWriteOp::ResetIndex(kind_index().lid),
            reindex(Some("a"), Some("b")),
        ],
    );
    assert_kind_entries(&store, 1);

    // A key another operation of the batch also writes is compared too.
    let mut store = kind_store();
    commit(
        &mut store,
        &[
            StorageWriteOp::IndexEntity {
                index: kind_index(),
                entity_id: "one".to_string(),
                object: kind("a"),
            },
            reindex(None, Some("a")),
        ],
    );
    assert_kind_entries(&store, 1);

    // Unconditional batches never trust `old`.
    let mut store = kind_store();
    store.apply_batch(&[reindex(Some("a"), Some("b"))]).unwrap();
    assert_kind_entries(&store, 1);
}
