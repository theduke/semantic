//! Healthy registration versus entity count and unrelated schema size.
//! Setup and open are outside the timed call. Reopen fixtures also report the
//! observed mean open time separately, so loading work remains visible.
//! Both default packages are installed in every fixture to catch cross-package
//! reconciliation churn as well as isolated no-op performance.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use semantic_data::schema::{AttributeType, ClassType, Meta, StringType, Type, TypeKind};
use semantic_data::schema::{DbOpenMode, Package};
use semantic_data::value::{Object, Value};
use semantic_db_core::embedded::{EmbeddedDb, EntityStorage};
use semantic_db_core::{Batch, BatchOperation, DEFAULT_COLLECTION, DdlBatch, DdlOperation};
use semantic_db_kv::EntityStore;
use semantic_db_redb::{RedbDatabase, RedbKvEngine};

#[derive(Clone, Copy, Default)]
struct Scale {
    rows: usize,
    attributes: usize,
    classes: usize,
}

fn seed_scale<S: EntityStorage>(db: &mut EmbeddedDb<S>, scale: Scale) {
    if scale.attributes > 0 || scale.classes > 0 {
        let mut ddl = DdlBatch::new();
        for index in 0..scale.attributes {
            ddl = ddl.with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: format!("unrelated.attribute_{index}"),
                    name: format!("attribute_{index}"),
                    ty: Type::new(TypeKind::String(StringType {
                        format: None,
                        normalization: None,
                    })),
                    constraints: Vec::new(),
                    meta: Meta::default(),
                },
            });
        }
        for index in 0..scale.classes {
            ddl = ddl.with_op(DdlOperation::UpsertClass {
                class: ClassType {
                    id: format!("unrelated.class_{index}"),
                    name: format!("Class{index}"),
                    inherits: None,
                    extends: Vec::new(),
                    strict_schema: false,
                    creatable_in_ui: None,
                    include_in_ui_listings: None,
                    attributes: Default::default(),
                    constraints: Vec::new(),
                    meta: Meta::default(),
                },
            });
        }
        db.transact_ddl(ddl).expect("seed unrelated schema");
    }
    for offset in (0..scale.rows).step_by(1000) {
        let mut batch = Batch::new();
        for index in offset..(offset + 1000).min(scale.rows) {
            let id = format!("row_{index}");
            let mut object = Object::default();
            object.insert("id", Value::String(id.clone()));
            batch = batch.with_op(BatchOperation::Upsert {
                collection: DEFAULT_COLLECTION.into(),
                id,
                object,
            });
        }
        db.transact(batch).expect("seed unrelated rows");
    }
}

fn register_memory(c: &mut Criterion, name: &str, package: Package, scale: Scale) {
    let mut db = semantic_db_kv::open_memory().expect("memory database");
    db.upsert_package(package.clone()).expect("seed package");
    db.upsert_package(other_default_package(&package))
        .expect("seed other default package");
    seed_scale(&mut db, scale);
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

fn register_redb_reopened(c: &mut Criterion, name: &str, package: Package, scale: Scale) {
    let directory = tempfile::tempdir().expect("database directory");
    let path = directory.path().join("registration.redb");
    {
        let engine = RedbKvEngine::open(&path, DbOpenMode::AutoCreate).expect("create redb");
        let mut db = RedbDatabase::open(EntityStore::new(engine)).expect("open database");
        db.upsert_package(package.clone()).expect("seed package");
        db.upsert_package(other_default_package(&package))
            .expect("seed other default package");
        seed_scale(&mut db, scale);
    }
    let mut open_time = Duration::ZERO;
    let mut opens = 0;
    c.bench_function(name, |bench| {
        bench.iter_custom(|iterations| {
            let mut registration_time = Duration::ZERO;
            for _ in 0..iterations {
                let open_started = Instant::now();
                let engine =
                    RedbKvEngine::open(&path, DbOpenMode::OpenExisting).expect("reopen redb");
                let mut db = RedbDatabase::open(EntityStore::new(engine)).expect("reopen database");
                open_time += open_started.elapsed();
                opens += 1;
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
    if opens > 0 {
        eprintln!(
            "{name}: mean open {:?} across {opens} opens",
            open_time / opens
        );
    }
}

fn other_default_package(package: &Package) -> Package {
    if package.name == semantic_base::PACKAGE_NAME {
        semantic_data::filestore::package()
    } else {
        semantic_base::package()
    }
}

fn package_registration(c: &mut Criterion) {
    let base = semantic_base::package();
    let filestore = semantic_data::filestore::package();
    register_memory(
        c,
        "package_registration/base/memory_warm",
        base.clone(),
        Scale::default(),
    );
    register_memory(
        c,
        "package_registration/filestore/memory_warm",
        filestore.clone(),
        Scale::default(),
    );
    register_redb_reopened(
        c,
        "package_registration/base/redb_reopened",
        base.clone(),
        Scale::default(),
    );
    register_redb_reopened(
        c,
        "package_registration/filestore/redb_reopened",
        filestore,
        Scale::default(),
    );
    for (label, scale) in [
        (
            "rows_1000",
            Scale {
                rows: 1000,
                attributes: 0,
                classes: 0,
            },
        ),
        (
            "rows_100000",
            Scale {
                rows: 100000,
                attributes: 0,
                classes: 0,
            },
        ),
        (
            "attributes_100",
            Scale {
                rows: 0,
                attributes: 100,
                classes: 0,
            },
        ),
        (
            "attributes_1000",
            Scale {
                rows: 0,
                attributes: 1000,
                classes: 0,
            },
        ),
        (
            "classes_1000",
            Scale {
                classes: 1000,
                ..Scale::default()
            },
        ),
    ] {
        register_memory(
            c,
            &format!("package_registration/base/memory_{label}"),
            base.clone(),
            scale,
        );
        register_redb_reopened(
            c,
            &format!("package_registration/base/reopened_{label}"),
            base.clone(),
            scale,
        );
    }
}

criterion_group!(benches, package_registration);
criterion_main!(benches);
