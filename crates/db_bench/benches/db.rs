//! Criterion benchmarks of the embedded database; see `docs/benchmarks.md`.
//!
//! Benchmark ids are `<workload>/<engine>/<size>`, for example
//! `point_get/redb/10000`. Each (engine, size) database is seeded once and
//! shared by all workloads; mutating workloads restore the data outside
//! their measured sections.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::measurement::WallTime;
use criterion::{
    BenchmarkGroup, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
};
use semantic_db_bench::{BenchSelection, Engine, Fixture, ReadWorkload, WritePath};
use semantic_db_redb::RedbDurability;
use tokio::runtime::Runtime;

/// Concurrent reader tasks of the `concurrent_read` workload.
const READERS: u64 = 4;

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn id(fixture: &Fixture) -> BenchmarkId {
    BenchmarkId::new(fixture.engine().name(), fixture.size())
}

/// Workloads whose iterations take milliseconds run with the minimum
/// sample count.
fn heavy(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.sample_size(10);
}

fn sum(rt: &Runtime, iters: u64, mut run: impl FnMut(u64) -> Duration) -> Duration {
    let _guard = rt.enter();
    (0..iters).map(&mut run).sum()
}

fn reads(c: &mut Criterion, rt: &Runtime, fixture: &Fixture) {
    for workload in ReadWorkload::ALL {
        let mut group = c.benchmark_group(workload.name());
        if workload.scans_all_rows() || workload == ReadWorkload::Join {
            heavy(&mut group);
        }
        let mut n = 0;
        group.bench_function(id(fixture), |b| {
            b.iter(|| {
                n += 1;
                black_box(
                    rt.block_on(fixture.read(workload, n))
                        .unwrap_or_else(|err| panic!("{}: {err}", workload.name())),
                )
            })
        });
        group.finish();
    }
}

fn inserts(
    c: &mut Criterion,
    rt: &Runtime,
    fixture: &Fixture,
    name: &str,
    path: WritePath,
    batches: &[usize],
) {
    let mut group = c.benchmark_group(name);
    heavy(&mut group);
    for &len in batches {
        group.throughput(Throughput::Elements(len as u64));
        let id = BenchmarkId::new(
            format!("{}/batch_{len}", fixture.engine().name()),
            fixture.size(),
        );
        group.bench_function(id, |b| {
            b.iter_custom(|iters| {
                sum(rt, iters, |_| {
                    rt.block_on(fixture.insert_batch_timed(len, path))
                        .expect("insert")
                })
            })
        });
    }
    group.finish();
}

fn writes(c: &mut Criterion, rt: &Runtime, fixture: &Fixture) {
    let mut group = c.benchmark_group("update_by_kind");
    heavy(&mut group);
    let mut n = 0;
    group.bench_function(id(fixture), |b| {
        b.iter(|| {
            n += 1;
            black_box(rt.block_on(fixture.update_by_kind(n)).expect("update"))
        })
    });
    group.finish();

    let mut group = c.benchmark_group("update_by_id");
    let mut n = 0;
    group.bench_function(id(fixture), |b| {
        b.iter(|| {
            n += 1;
            black_box(rt.block_on(fixture.update_by_id(n)).expect("update"))
        })
    });
    group.finish();

    for (name, path) in [
        ("delete_by_id", WritePath::Dataset),
        ("delete_by_id_stats", WritePath::Stats),
    ] {
        let mut group = c.benchmark_group(name);
        if path == WritePath::Dataset {
            heavy(&mut group);
        }
        let mut n = 0;
        group.bench_function(id(fixture), |b| {
            b.iter_custom(|iters| {
                sum(rt, iters, |_| {
                    n += 1;
                    rt.block_on(fixture.delete_by_id_timed(n, path))
                        .expect("delete")
                })
            })
        });
        group.finish();
    }

    let mut group = c.benchmark_group("delete_by_kind");
    heavy(&mut group);
    let mut n = 0;
    group.bench_function(id(fixture), |b| {
        b.iter_custom(|iters| {
            sum(rt, iters, |_| {
                n += 1;
                rt.block_on(fixture.delete_by_kind_timed(n))
                    .expect("delete")
            })
        })
    });
    group.finish();

    let mut group = c.benchmark_group("transaction_10r_10w");
    let mut n = 0;
    group.bench_function(id(fixture), |b| {
        b.iter(|| {
            n += 1;
            rt.block_on(fixture.transaction(n)).expect("transaction")
        })
    });
    group.finish();
}

fn concurrent(c: &mut Criterion, rt: &Runtime, fixture: &Arc<Fixture>) {
    let mut group = c.benchmark_group("concurrent_read");
    heavy(&mut group);
    let id = BenchmarkId::new(
        format!("{}/{READERS}_readers_1_writer", fixture.engine().name()),
        fixture.size(),
    );
    group.bench_function(id, |b| {
        b.iter_custom(|iters| {
            rt.block_on(fixture.concurrent_reads(iters, READERS))
                .expect("concurrent reads")
        })
    });
    group.finish();
}

fn benches(c: &mut Criterion) {
    let selection = BenchSelection::from_env().expect("benchmark selection");
    let rt = runtime();
    for &size in &selection.sizes {
        for &engine in &selection.engines {
            eprintln!("seeding {} with {size} items", engine.name());
            let fixture = rt
                .block_on(Fixture::seeded(engine, size))
                .expect("seed fixture");
            reads(c, &rt, &fixture);
            inserts(c, &rt, &fixture, "insert", WritePath::Dataset, &[100, 1000]);
            inserts(
                c,
                &rt,
                &fixture,
                "insert_stats",
                WritePath::Stats,
                &[100, 1000],
            );
            writes(c, &rt, &fixture);
            let fixture = Arc::new(fixture);
            concurrent(c, &rt, &fixture);
            let fixture = Arc::into_inner(fixture).expect("no concurrent tasks left");
            if engine == Engine::Redb {
                // fsync on every commit; compare with `insert_stats`.
                let fixture = rt
                    .block_on(fixture.reopen_redb(RedbDurability::Immediate))
                    .expect("reopen redb");
                inserts(
                    c,
                    &rt,
                    &fixture,
                    "insert_immediate",
                    WritePath::Stats,
                    &[100],
                );
            }
        }
    }
}

criterion_group!(db, benches);
criterion_main!(db);
