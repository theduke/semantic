//! Randomised differential and concurrency tests of the logfs engine
//! against the in-memory engine and the reference model of
//! `semantic_db_test` (see `docs/testing.md`).

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

#[tokio::test(flavor = "multi_thread")]
async fn memory_and_logfs_agree_on_random_operations() {
    for seed in seeds_from_env(&[21]) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("differential.log");
        let logfs = DiffTarget::reopenable("logfs", move || {
            Db::new(open_backend(&path, DbOpenMode::AutoCreate).unwrap())
        });
        run_differential(
            DiffConfig::new(seed, 200),
            vec![DiffTarget::new("memory", memory()), logfs],
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_invariants_hold_on_logfs() {
    for seed in seeds_from_env(&[22]) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(
            open_backend(dir.path().join("concurrency.log"), DbOpenMode::AutoCreate).unwrap(),
        );
        run_concurrency(&db, ConcurrencyConfig::new(seed)).await;
    }
}
