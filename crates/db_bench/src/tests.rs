//! Runs every workload once on a tiny data set, so the benchmark code is
//! compiled and exercised by `cargo test`.

use std::sync::Arc;

use semantic_db_core::QueryPlan;
use semantic_db_redb::RedbDurability;

use super::*;
use crate::schema::{INDEX_CODE, INDEX_KIND_CREATED, INDEX_SCORE, INDEX_SEARCH};

const SIZE: u64 = 100;

/// Items plus their owners.
fn entities() -> usize {
    (SIZE + data::owner_count(SIZE)) as usize
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn assert_index(plan: &QueryPlan, index: &str, ordered: bool) {
    match plan {
        QueryPlan::IndexLookup { index_name, .. } if !ordered => assert_eq!(index_name, index),
        QueryPlan::IndexRange {
            index_name,
            ordered: actual,
            ..
        } => {
            assert_eq!(index_name, index, "{plan:?}");
            assert!(*actual || !ordered, "{plan:?} must serve the ordering");
        }
        QueryPlan::TextSearch { index_name, .. } => assert_eq!(index_name, index),
        other => panic!("expected a scan of {index}, got {other:?}"),
    }
}

async fn check_plans(fixture: &Fixture) {
    for (workload, index, ordered) in [
        (ReadWorkload::EqSelect, INDEX_CODE, false),
        (ReadWorkload::RangeLimit, INDEX_SCORE, false),
        (ReadWorkload::OrderedScan, INDEX_SCORE, true),
        (ReadWorkload::KeysetPage, INDEX_SCORE, true),
        (ReadWorkload::FullText, INDEX_SEARCH, false),
        (ReadWorkload::Feed, INDEX_KIND_CREATED, true),
    ] {
        let plan = fixture.plan(workload, 3).await.unwrap();
        assert_index(&plan, index, ordered);
    }
    for workload in [ReadWorkload::FullScan, ReadWorkload::TopN] {
        let plan = fixture.plan(workload, 0).await.unwrap();
        assert!(matches!(plan, QueryPlan::FullScan { .. }), "{plan:?}");
    }
}

async fn run_all(fixture: Fixture) {
    for workload in ReadWorkload::ALL {
        for n in 0..3 {
            let rows = fixture.read(workload, n).await.unwrap();
            let expected = match workload {
                ReadWorkload::PointGet | ReadWorkload::Join | ReadWorkload::Feed => Some(1),
                ReadWorkload::EqSelect => Some(data::ROWS_PER_CODE as usize),
                ReadWorkload::RangeLimit => Some((SIZE / 10) as usize),
                ReadWorkload::OrderedScan | ReadWorkload::KeysetPage => Some(PAGE),
                ReadWorkload::Count => Some(entities()),
                ReadWorkload::TopN => Some(10),
                ReadWorkload::FullText | ReadWorkload::FullScan => None,
            };
            if let Some(expected) = expected {
                assert_eq!(rows, expected, "{workload:?} #{n}");
            }
        }
    }

    for n in 0..3 {
        assert_eq!(fixture.update_by_kind(n).await.unwrap(), 1);
        assert_eq!(fixture.update_by_id(n).await.unwrap(), 1);
        for path in [WritePath::Dataset, WritePath::Stats] {
            fixture.delete_by_id_timed(n, path).await.unwrap();
        }
        fixture.delete_by_kind_timed(n).await.unwrap();
        fixture.transaction(n).await.unwrap();
    }
    for len in [1, 100] {
        for path in [WritePath::Dataset, WritePath::Stats] {
            fixture.insert_batch_timed(len, path).await.unwrap();
        }
    }
    let fixture = Arc::new(fixture);
    fixture.concurrent_reads(8, 4).await.unwrap();

    // Mutating workloads restore the data set.
    assert_eq!(
        fixture.read(ReadWorkload::Count, 0).await.unwrap(),
        entities()
    );
    let restored = fixture
        .db()
        .get(schema::COLLECTION, data::item_id(0))
        .await
        .unwrap();
    assert!(restored.is_some());
}

#[test]
fn memory_workloads() {
    runtime().block_on(async {
        run_all(Fixture::seeded(Engine::Memory, SIZE).await.unwrap()).await;
    });
}

#[test]
fn redb_workloads() {
    runtime().block_on(async {
        let fixture = Fixture::seeded(Engine::Redb, SIZE).await.unwrap();
        let fixture = fixture
            .reopen_redb(RedbDurability::Immediate)
            .await
            .unwrap();
        fixture
            .insert_batch_timed(10, WritePath::Stats)
            .await
            .unwrap();
        run_all(fixture).await;
    });
}

/// The planner picks full scans for tiny collections, so plans are checked
/// on a larger one.
#[test]
fn workloads_use_indexes() {
    runtime().block_on(async {
        let fixture = Fixture::seeded(Engine::Memory, 2000).await.unwrap();
        check_plans(&fixture).await;
        let explain = fixture.explain(ReadWorkload::Join, 0).await.unwrap();
        let physical = format!("{:?}", explain.physical);
        assert!(physical.contains("IndexNestedLoop"), "{physical}");
        assert!(physical.contains(INDEX_KIND_CREATED), "{physical}");
    });
}

#[test]
fn selection_from_env_values() {
    let default = BenchSelection::parse(None, false, None).unwrap();
    assert_eq!(default.sizes, DEFAULT_SIZES.to_vec());
    assert_eq!(default.engines, Engine::ALL.to_vec());

    let custom = BenchSelection::parse(Some("100, 10_000"), true, Some("redb")).unwrap();
    assert_eq!(custom.sizes, vec![100, 10_000, LARGE_SIZE]);
    assert_eq!(custom.engines, vec![Engine::Redb]);

    assert!(BenchSelection::parse(Some("10"), false, None).is_err());
    assert!(BenchSelection::parse(None, false, Some("sqlite")).is_err());
}
