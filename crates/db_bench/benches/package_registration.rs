//! Baseline healthy registration of base/filestore on empty databases.
//! Setup and open are outside the timed call; these fixtures do not establish
//! scaling with entity count, unrelated schema, or end-to-end startup latency.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use semantic_data::schema::{DbOpenMode, Package};
use semantic_db_kv::EntityStore;
use semantic_db_redb::{RedbDatabase, RedbKvEngine};

fn register_memory(c: &mut Criterion, name: &str, package: Package) {
    let mut db = semantic_db_kv::open_memory().expect("memory database");
    db.upsert_package(package.clone()).expect("seed package");
    let revision = db.storage().current_revision().expect("storage revision");
    c.bench_function(name, |bench| {
        bench.iter(|| {
            black_box(
                db.upsert_package(black_box(package.clone()))
                    .expect("unchanged registration"),
            )
        })
    });
    assert_eq!(db.storage().current_revision().unwrap(), revision);
}

fn register_redb_reopened(c: &mut Criterion, name: &str, package: Package) {
    let directory = tempfile::tempdir().expect("database directory");
    let path = directory.path().join("registration.redb");
    {
        let engine = RedbKvEngine::open(&path, DbOpenMode::AutoCreate).expect("create redb");
        let mut db = RedbDatabase::open(EntityStore::new(engine)).expect("open database");
        db.upsert_package(package.clone()).expect("seed package");
    }
    c.bench_function(name, |bench| {
        bench.iter_custom(|iterations| {
            let mut registration_time = Duration::ZERO;
            for _ in 0..iterations {
                let engine =
                    RedbKvEngine::open(&path, DbOpenMode::OpenExisting).expect("reopen redb");
                let mut db = RedbDatabase::open(EntityStore::new(engine)).expect("reopen database");
                let revision = db.storage().current_revision().expect("storage revision");
                let started = Instant::now();
                black_box(
                    db.upsert_package(black_box(package.clone()))
                        .expect("unchanged registration"),
                );
                registration_time += started.elapsed();
                assert_eq!(db.storage().current_revision().unwrap(), revision);
            }
            registration_time
        })
    });
}

fn package_registration(c: &mut Criterion) {
    let base = semantic_base::package();
    let filestore = semantic_data::filestore::package();
    register_memory(c, "package_registration/base/memory_warm", base.clone());
    register_memory(
        c,
        "package_registration/filestore/memory_warm",
        filestore.clone(),
    );
    register_redb_reopened(c, "package_registration/base/redb_reopened", base);
    register_redb_reopened(c, "package_registration/filestore/redb_reopened", filestore);
}

criterion_group!(benches, package_registration);
criterion_main!(benches);
