//! The shared randomised differential and concurrency tests on the
//! in-memory engine, with and without MVCC snapshots.

use semantic_db_core::Db;
use semantic_db_core::embedded::EmbeddedBackend;
use semantic_db_test::concurrency::{ConcurrencyConfig, run_concurrency};
use semantic_db_test::differential::{DiffConfig, DiffTarget, run_differential};
use semantic_db_test::rng::seeds_from_env;

fn memory() -> Db {
    Db::new(EmbeddedBackend::new(crate::open_memory().unwrap()))
}

fn memory_mvcc() -> Db {
    Db::new(EmbeddedBackend::new(crate::open_memory_mvcc().unwrap()))
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_engines_agree_on_random_operations() {
    for seed in seeds_from_env(&[31]) {
        run_differential(
            DiffConfig::new(seed, 200),
            vec![
                DiffTarget::new("memory", memory()),
                DiffTarget::new("memory-mvcc", memory_mvcc()),
            ],
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_invariants_hold_on_memory_mvcc() {
    for seed in seeds_from_env(&[32]) {
        run_concurrency(&memory_mvcc(), ConcurrencyConfig::new(seed)).await;
    }
}
