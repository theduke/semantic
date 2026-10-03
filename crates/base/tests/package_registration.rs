use std::{
    io::Write,
    sync::{Arc, Mutex},
};

use semantic_data::schema::{MigrationDdlOperation, MigrationOperation};
use semantic_db_core::embedded::EmbeddedDb;
use semantic_db_core::{DdlBatch, DdlOperation};

#[test]
fn historical_default_package_migrations_remain_exact() {
    use sha2::{Digest, Sha256};
    // Captured from the complete nine-migration histories at 51d747bb, before
    // appending the shared-ownership migrations. Include every field and DDL.
    for (package, expected) in [
        (
            semantic_base::package(),
            "f40311e1296952875fc442d141e98101c81ebc83bd7b30a5d27bff3cce2d076d",
        ),
        (
            semantic_data::filestore::package(),
            "6bfa172aea6da1f6a6db368c10b1d56da6cbffc508b29f95a634418f48e9e548",
        ),
    ] {
        let encoded = facet_json::to_string(&package.migrations[..9].to_vec()).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(encoded.as_bytes())),
            expected,
            "{}",
            package.name
        );
        assert_eq!(
            package.migrations[9],
            semantic_data::bundles::shared::migration_v1()
        );
    }
}

#[test]
fn legacy_default_packages_upgrade_shared_ownership_once() {
    let packages = [
        semantic_base::package(),
        semantic_data::filestore::package(),
    ];
    let mut db = semantic_db_kv::open_memory().unwrap();
    let shared_ownership = semantic_data::bundles::shared::migration_v1();
    let legacy_count = |package: &semantic_data::schema::Package| {
        package
            .migrations
            .iter()
            .position(|migration| migration == &shared_ownership)
            .expect("shared ownership migration follows the historical package")
    };
    for current in &packages {
        let mut legacy = current.clone();
        legacy.modules.clear();
        legacy.migrations.truncate(legacy_count(current));
        if current.name == semantic_base::PACKAGE_NAME {
            legacy
                .root
                .attributes
                .remove(semantic_base::content::ATTR_MAIN_CONTENT);
        }
        for operation in legacy
            .migrations
            .iter()
            .flat_map(|migration| &migration.operations)
        {
            // Reconstruct the historical snapshot from retained migrations;
            // current class metadata may include later forward migrations.
            if let MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }) = operation
            {
                legacy.root.classes.insert(class.id.clone(), class.clone());
            }
            if let MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute }) =
                operation
                && semantic_data::bundles::shared::ATTRIBUTE_IDS.contains(&attribute.id.as_str())
            {
                legacy
                    .root
                    .attributes
                    .insert(attribute.id.clone(), attribute.clone());
            }
        }
        db.upsert_package(legacy).unwrap();
    }
    let (_, storage) = db.into_parts();
    let mut db = EmbeddedDb::open(storage).unwrap();
    for current in &packages {
        let outcome = db.upsert_package(current.clone()).unwrap();
        let expected = current.migrations[legacy_count(current)..].to_vec();
        assert_eq!(
            outcome
                .executed_migrations
                .iter()
                .map(|entry| entry.migration.clone())
                .collect::<Vec<_>>(),
            expected
        );
    }
    let (_, storage) = db.into_parts();
    let mut db = EmbeddedDb::open(storage).unwrap();
    let revision = db.storage().current_revision().unwrap();
    for current in &packages {
        assert!(
            db.upsert_package(current.clone())
                .unwrap()
                .executed_migrations
                .is_empty()
        );
    }
    assert_eq!(db.storage().current_revision().unwrap(), revision);
    let catalog = db.catalog();
    for attribute in semantic_data::bundles::shared::ATTRIBUTE_IDS {
        let definition = &catalog.type_def_by_name(attribute).unwrap().type_def;
        assert_eq!(
            definition.module.as_deref(),
            Some(semantic_data::bundles::shared::MODULE_NAME)
        );
    }
}

#[derive(Clone)]
struct TraceWriter(Arc<Mutex<Vec<u8>>>);

impl Write for TraceWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn default_package_startup_converges_in_both_orders_and_after_reopen() {
    use futures::{FutureExt, StreamExt};
    for reverse in [false, true] {
        let mut packages = [
            semantic_base::package(),
            semantic_data::filestore::package(),
        ];
        if reverse {
            packages.reverse();
        }
        let mut db = semantic_db_kv::open_memory().unwrap();
        for package in &packages {
            db.upsert_package(package.clone()).unwrap();
        }
        for _ in 0..3 {
            let (_, storage) = db.into_parts();
            db = EmbeddedDb::open(storage).unwrap();
            let revision = db.storage().current_revision().unwrap();
            let catalog_version = db.shared_catalog().snapshot().version;
            let mut changes = db.subscribe_changes(Default::default());
            let captured = Arc::new(Mutex::new(Vec::new()));
            let writer = TraceWriter(Arc::clone(&captured));
            let subscriber = tracing_subscriber::fmt()
                .with_ansi(false)
                .without_time()
                .with_max_level(tracing::Level::DEBUG)
                .with_writer(move || writer.clone())
                .finish();
            tracing::subscriber::with_default(subscriber, || {
                for package in &packages {
                    assert!(
                        db.upsert_package(package.clone())
                            .unwrap()
                            .executed_migrations
                            .is_empty()
                    );
                }
            });
            let events = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
            assert_eq!(
                db.storage().current_revision().unwrap(),
                revision,
                "{events}"
            );
            assert_eq!(db.shared_catalog().snapshot().version, catalog_version);
            assert!(changes.next().now_or_never().is_none());
            assert_eq!(
                events
                    .matches("registration_path=\"unchanged_certified\"")
                    .count(),
                2,
                "{events}"
            );
            assert!(!events.contains("phase=\"reconciliation\""), "{events}");
            assert!(!events.contains("phase=\"full_validation\""), "{events}");
        }
    }
}

#[test]
fn base_registration_repairs_missing_relationship() {
    let mut db = semantic_db_kv::open_memory().unwrap();
    let package = semantic_base::package();
    let relationship_id = package
        .migrations
        .iter()
        .flat_map(|migration| &migration.operations)
        .find_map(|operation| match operation {
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertRelationship { relationship }) => {
                Some(relationship.id.clone())
            }
            _ => None,
        })
        .unwrap();
    db.upsert_package(package.clone()).unwrap();
    db.transact_ddl(DdlBatch::new().with_op(DdlOperation::DeleteRelationship {
        id: relationship_id.clone(),
    }))
    .unwrap();
    assert!(db.catalog().relationship_by_id(&relationship_id).is_none());

    db.upsert_package(package.clone()).unwrap();
    assert!(db.catalog().relationship_by_id(&relationship_id).is_some());
    let (_, storage) = db.into_parts();
    let mut reopened = EmbeddedDb::open(storage).unwrap();
    let revision = reopened.storage().current_revision().unwrap();
    reopened.upsert_package(package).unwrap();
    assert_eq!(reopened.storage().current_revision().unwrap(), revision);
}
