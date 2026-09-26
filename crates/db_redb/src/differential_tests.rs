//! Randomised differential and concurrency tests: the redb engine against
//! the in-memory engine and the reference model of `semantic_db_test` (see
//! `docs/testing.md`).

use semantic_data::schema::DbOpenMode;
use semantic_db_core::Db;
use semantic_db_core::embedded::EmbeddedBackend;
use semantic_db_test::concurrency::{ConcurrencyConfig, run_concurrency};
use semantic_db_test::differential::{DiffConfig, DiffTarget, run_differential};
use semantic_db_test::rng::seeds_from_env;

use crate::open_backend;

fn memory() -> Db {
    Db::new(EmbeddedBackend::new(semantic_db_kv::open_memory().unwrap()))
}

/// A reopenable redb target in `dir`.
fn redb(dir: &tempfile::TempDir) -> DiffTarget {
    let path = dir.path().join("differential.redb");
    DiffTarget::reopenable("redb", move || {
        Db::new(open_backend(&path, DbOpenMode::AutoCreate).unwrap())
    })
}

async fn differential(seeds: &[u64], steps: usize) {
    for seed in seeds_from_env(seeds) {
        let dir = tempfile::tempdir().unwrap();
        run_differential(
            DiffConfig::new(seed, steps),
            vec![DiffTarget::new("memory", memory()), redb(&dir)],
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_and_redb_agree_on_random_operations() {
    differential(&[1, 2, 3], 300).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "expensive: many seeds and long operation sequences"]
async fn memory_and_redb_agree_on_many_long_random_runs() {
    differential(&(100..110).collect::<Vec<_>>(), 1_000).await;
}

async fn concurrency(db: Db, seeds: &[u64]) {
    for seed in seeds_from_env(seeds) {
        run_concurrency(&db, ConcurrencyConfig::new(seed)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_invariants_hold_on_memory() {
    for seed in seeds_from_env(&[11]) {
        run_concurrency(&memory(), ConcurrencyConfig::new(seed)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_invariants_hold_on_redb() {
    for seed in seeds_from_env(&[12]) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(
            open_backend(dir.path().join("concurrency.redb"), DbOpenMode::AutoCreate).unwrap(),
        );
        run_concurrency(&db, ConcurrencyConfig::new(seed)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "expensive: repeated concurrency runs"]
async fn concurrency_invariants_hold_on_many_runs() {
    for round in 0..10 {
        concurrency(memory(), &[1_000 + round]).await;
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(
            open_backend(dir.path().join("concurrency.redb"), DbOpenMode::AutoCreate).unwrap(),
        );
        concurrency(db, &[2_000 + round]).await;
    }
}

/// Creating a unique index over rows that already hold duplicate values.
async fn unique_index_over_duplicates_is_rejected_on(db: &Db) {
    use semantic_data::query::{Batch, BatchOperation};
    use semantic_data::value::{Object, Value};
    use semantic_db_core::catalog::IntegrityMode;
    use semantic_db_core::{DdlBatch, DdlCollectionKind, DdlOperation};

    db.execute_ddl(DdlBatch::new().with_op(DdlOperation::UpsertCollection {
        name: "dup_items".into(),
        kind: DdlCollectionKind::Polymorphic,
        integrity_mode: IntegrityMode::Permissive,
    }))
    .await
    .unwrap();
    let mut batch = Batch::new();
    for id in ["one", "two"] {
        let mut object = Object::new();
        object.insert("id", Value::String(id.into()));
        object.insert("nick", Value::String("same".into()));
        batch = batch.with_op(BatchOperation::Upsert {
            collection: "dup_items".into(),
            id: id.into(),
            object,
        });
    }
    db.execute_batch(batch).await.unwrap();
    let unique = DdlBatch::new().with_op(DdlOperation::UpsertIndex {
        name: "dup_items_nick".into(),
        collection: "dup_items".into(),
        field: "nick".into(),
        unique: true,
        kind: Default::default(),
        extra_fields: Vec::new(),
        predicate: None,
        analyzer: Default::default(),
    });
    assert!(
        db.execute_ddl(unique).await.is_err(),
        "a unique index was built over duplicate values"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "bug: DDL builds a unique index over existing duplicate values without an error \
            (EmbeddedDb::backfill_indexes never checks uniqueness); later writes only probe \
            changed rows, so the duplicates persist under a `unique` index"]
async fn unique_index_over_duplicates_is_rejected() {
    unique_index_over_duplicates_is_rejected_on(&memory()).await;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::new(open_backend(dir.path().join("unique.redb"), DbOpenMode::AutoCreate).unwrap());
    unique_index_over_duplicates_is_rejected_on(&db).await;
}
