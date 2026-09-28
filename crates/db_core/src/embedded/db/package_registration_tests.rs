//! Registration certificate invariants and differential replay regressions.

use futures::FutureExt as _;
use semantic_data::schema::{
    MigrationDdlOperation, attribute::attribute_type::AttributeType, class::class_ref::ClassRef,
    core::meta::Meta, primitives::string_type::StringType,
};

use super::tests::simple_schema_package;
use super::*;
use crate::embedded::storage::{CountingEntityStorage, EntityStorage};

#[test]
fn unchanged_filestore_registration_after_reopen_does_no_storage_work() {
    use std::sync::atomic::Ordering;

    let (storage, counts) = CountingEntityStorage::new();
    let mut db = EmbeddedDb::new(storage);
    let package = semantic_data::filestore::package();
    db.upsert_package(package.clone()).unwrap();
    let (_, storage) = db.into_parts();
    let mut reopened = EmbeddedDb::open(storage).unwrap();
    reopened
        .transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: "unrelated.registration_check".into(),
                name: "registration_check".into(),
                ty: Type {
                    kind: TypeKind::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    constraints: vec![],
                    annotations: vec![],
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        }))
        .unwrap();
    let revision = reopened.storage().current_revision().unwrap();
    let catalog_version = reopened.catalog.snapshot().version;
    let full_attempts = reopened.full_registration_attempts;
    let mut changes = reopened.subscribe_changes(crate::ChangeSubscriptionOptions::default());
    counts.reset();
    crate::catalog::take_storage_snapshot_count();
    registration_proof::take_replay_count();

    let outcome = reopened.upsert_package(package).unwrap();
    assert!(outcome.executed_migrations.is_empty());
    assert_eq!(reopened.storage().current_revision().unwrap(), revision);
    assert_eq!(reopened.catalog.snapshot().version, catalog_version);
    assert_eq!(reopened.full_registration_attempts, full_attempts);
    assert_eq!(crate::catalog::take_storage_snapshot_count(), 0);
    assert_eq!(registration_proof::take_replay_count(), 0);
    assert_eq!(counts.collection_scans(), 0);
    assert_eq!(counts.index_entry_scans(), 0);
    assert_eq!(counts.index_value_scans.load(Ordering::Relaxed), 0);
    assert_eq!(counts.collection_counts(), 0);
    assert_eq!(counts.entity_gets(), 0);
    assert_eq!(counts.commit_calls.load(Ordering::Relaxed), 0);
    assert_eq!(counts.submitted_ops.load(Ordering::Relaxed), 0);
    assert!(changes.next().now_or_never().is_none());
}

#[test]
fn reopened_invalid_foreign_class_preserves_registration_error() {
    let mut db = EmbeddedDb::in_memory();
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    let mut catalog = (*db.catalog()).clone();
    let mut class = catalog
        .class_by_lid(catalog.class_id("shared.test.note").unwrap())
        .unwrap()
        .class
        .clone();
    class.id = "unrelated.invalid_inheritance".into();
    class.inherits = Some(ClassRef {
        id: class.id.clone(),
    });
    catalog.upsert_class(class).unwrap();
    db.replace_catalog_snapshot(catalog);
    // A collection-only operation persists the supplied catalog without
    // validating every unrelated class graph, as does catalog loading.
    db.create_collection("persist_catalog", CollectionKind::Untyped)
        .unwrap();
    let (_, storage) = db.into_parts();
    let mut reopened = EmbeddedDb::open(storage).unwrap();
    assert!(!is_certified(&reopened, &package));
    let normalized = normalize_package_definition(&package).unwrap();
    assert!(
        reopened
            .apply_package_update(
                &reopened.catalog(),
                reopened.storage().current_revision().unwrap(),
                &normalized,
            )
            .is_err()
    );
    assert!(reopened.upsert_package(package).is_err());
}

#[test]
fn collection_kind_history_preserves_full_reconciliation() {
    use semantic_data::schema::{MigrationCollectionKind, MigrationIntegrityMode};

    let mut db = EmbeddedDb::in_memory();
    let mut package = simple_schema_package("Collection kind changes.");
    for kind in [
        MigrationCollectionKind::Schema,
        MigrationCollectionKind::Untyped,
    ] {
        package.migrations[0]
            .operations
            .push(MigrationOperation::Ddl(
                MigrationDdlOperation::UpsertCollection {
                    name: "changing_kind".into(),
                    kind,
                    integrity_mode: MigrationIntegrityMode::Permissive,
                },
            ));
    }
    db.upsert_package(package.clone()).unwrap();
    assert!(!is_certified(&db, &package));
    let normalized = normalize_package_definition(&package).unwrap();
    let installed = db.catalog();
    let (reconciled, before, after, executed) = db
        .apply_package_update(
            &installed,
            db.storage().current_revision().unwrap(),
            &normalized,
        )
        .unwrap();
    assert!(before.is_empty() && after.is_empty() && executed.is_empty());
    assert_ne!(
        installed.to_storage_snapshot(),
        reconciled.to_storage_snapshot()
    );

    db.upsert_package(package).unwrap();
    assert!(
        db.catalog().to_storage_snapshot() == reconciled.to_storage_snapshot(),
        "registration must preserve the full replay's collection field identities"
    );
}

#[test]
fn unchanged_package_repairs_unrelated_collection_projection() {
    let mut db = EmbeddedDb::in_memory();
    let package = semantic_data::filestore::package();
    db.upsert_package(package.clone()).unwrap();
    let mut catalog = (*db.catalog()).clone();
    catalog.upsert_attribute(AttributeType {
        id: "unrelated.low_level".into(),
        name: "low_level".into(),
        ty: Type {
            kind: TypeKind::String(StringType {
                format: None,
                normalization: None,
            }),
            constraints: vec![],
            annotations: vec![],
        },
        constraints: vec![],
        meta: Meta::default(),
    });
    let attribute_id = catalog
        .attribute_by_id("unrelated.low_level")
        .unwrap()
        .attribute
        .id
        .clone();
    assert!(
        catalog
            .collection_by_name(DEFAULT_COLLECTION)
            .unwrap()
            .field_id(&attribute_id)
            .is_none()
    );
    db.replace_catalog_snapshot(catalog);

    let outcome = db.upsert_package(package).unwrap();
    assert!(outcome.executed_migrations.is_empty());
    assert!(
        db.catalog()
            .collection_by_name(DEFAULT_COLLECTION)
            .unwrap()
            .field_id(&attribute_id)
            .is_some(),
        "registration must preserve reconciliation of unrelated collection projections"
    );
}

#[test]
fn unchanged_package_repairs_stale_owned_class_projection() {
    let mut db = EmbeddedDb::in_memory();
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    let mut stale = (*db.catalog()).clone();
    stale.remove_class_projection_for_test("shared.test.note");
    db.replace_catalog_snapshot(stale);
    db.upsert_package(package).unwrap();
    assert!(db.catalog().class_id("shared.test.note").is_some());
}

#[test]
fn unchanged_package_repairs_wrapper_only_typedef_drift() {
    let mut db = EmbeddedDb::in_memory();
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    let live = db.catalog();
    let type_name = live
        .type_def_by_name("shared.test.note")
        .unwrap()
        .type_def
        .name
        .clone();
    let mut snapshot = db.catalog().to_storage_snapshot();
    snapshot
        .type_defs
        .iter_mut()
        .find(|stored| stored.type_def.name == type_name)
        .unwrap()
        .type_def
        .meta
        .description = Some("wrapper drift".into());
    let drifted = Catalog::from_storage_snapshot(snapshot).unwrap();
    assert_eq!(
        drifted
            .class_id("shared.test.note")
            .and_then(|id| drifted.class_by_lid(id))
            .unwrap()
            .class,
        live.class_id("shared.test.note")
            .and_then(|id| live.class_by_lid(id))
            .unwrap()
            .class,
    );
    db.replace_catalog_snapshot(drifted);
    db.upsert_package(package).unwrap();
    assert_eq!(
        db.catalog()
            .type_def_by_name("shared.test.note")
            .unwrap()
            .type_def,
        live.type_def_by_name("shared.test.note").unwrap().type_def,
    );
}

#[test]
fn unchanged_registration_retries_revision_fence_conflict() {
    use std::sync::atomic::Ordering;

    let (storage, counts) = CountingEntityStorage::new();
    let mut db = EmbeddedDb::new(storage);
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    let revision = db.storage().current_revision().unwrap();
    counts.reset();
    counts.inject_revision_conflicts.store(1, Ordering::Relaxed);
    let full_attempts = db.full_registration_attempts;

    assert!(
        db.upsert_package(package)
            .unwrap()
            .executed_migrations
            .is_empty()
    );
    assert_eq!(counts.inject_revision_conflicts.load(Ordering::Relaxed), 0);
    assert_eq!(db.storage().current_revision().unwrap(), revision);
    assert_eq!(counts.commit_calls.load(Ordering::Relaxed), 0);
    assert_eq!(db.full_registration_attempts, full_attempts);
}

fn is_certified<S: EntityStorage>(db: &EmbeddedDb<S>, package: &Package) -> bool {
    let package = normalize_package_definition(package).unwrap();
    db.registration_proofs.permits(
        &db.catalog.snapshot(),
        &package.name,
        &registration_proof::digest(&package).unwrap(),
    )
}

#[test]
fn user_rows_with_catalog_metadata_ids_cannot_supply_certificates() {
    let mut db = EmbeddedDb::in_memory();
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    let schema = db
        .catalog()
        .collection_by_name(CORE_CATALOG_SCHEMA_COLLECTION)
        .unwrap()
        .lid;
    let mut impersonator = db
        .storage
        .get_entity(schema, "semantic:entry:meta/__catalog_meta__")
        .unwrap()
        .unwrap();
    impersonator.collection = db
        .catalog()
        .collection_by_name(DEFAULT_COLLECTION)
        .unwrap()
        .lid
        .0;
    let mut changed = (*db.catalog()).clone();
    changed.delete_class("shared.test.note");
    let mut ops = catalog_write_ops(&db.storage, &changed).unwrap();
    // Data operations follow the catalog prelude in package commits. A user
    // entity with the same ID must not replace the freshly computed evidence.
    ops.push(StorageWriteOp::PutEntity(impersonator));
    let proofs = proofs_from_write_ops(&changed, &ops);
    let snapshot = crate::catalog::CatalogSnapshot {
        version: 0,
        catalog: Arc::new(changed),
    };
    let bound = BoundRegistrationProofs::new(&snapshot, proofs);
    let package = normalize_package_definition(&package).unwrap();
    assert!(!bound.permits(
        &snapshot,
        &package.name,
        &registration_proof::digest(&package).unwrap()
    ));
}

#[test]
fn implicit_core_startup_repairs_do_not_inherit_loaded_certificates() {
    let mut db = EmbeddedDb::in_memory();
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    db.transact_ddl(DdlBatch::new().with_op(DdlOperation::DeleteCollection {
        name: DEFAULT_COLLECTION.into(),
    }))
    .unwrap();
    assert!(is_certified(&db, &package));
    let (_, storage) = db.into_parts();
    let mut reopened = EmbeddedDb::open(storage).unwrap();
    assert!(
        reopened
            .catalog()
            .collection_by_name(DEFAULT_COLLECTION)
            .is_some()
    );
    assert!(!is_certified(&reopened, &package));
    reopened.upsert_package(package).unwrap();
    assert_eq!(reopened.full_registration_attempts, 1);
}

#[test]
fn generated_collection_histories_match_full_replay_before_and_after_reopen() {
    use semantic_data::schema::{MigrationCollectionKind, MigrationIntegrityMode};

    let mut accepted = 0;
    let mut declined = 0;
    for history in 0..8 {
        let mut package = simple_schema_package("Generated collection history.");
        for step in 0..3 {
            let kind = if history & (1 << step) == 0 {
                MigrationCollectionKind::Schema
            } else {
                MigrationCollectionKind::Untyped
            };
            package.migrations[0]
                .operations
                .push(MigrationOperation::Ddl(
                    MigrationDdlOperation::UpsertCollection {
                        name: "generated".into(),
                        kind,
                        integrity_mode: MigrationIntegrityMode::Permissive,
                    },
                ));
        }
        let mut db = EmbeddedDb::in_memory();
        db.upsert_package(package.clone()).unwrap();
        let normalized = normalize_package_definition(&package).unwrap();
        for _ in 0..2 {
            let before = db.catalog().to_storage_snapshot();
            let (oracle, data_before, data_after, executed) = db
                .apply_package_update(
                    &db.catalog(),
                    db.storage().current_revision().unwrap(),
                    &normalized,
                )
                .unwrap();
            assert!(data_before.is_empty() && data_after.is_empty() && executed.is_empty());
            let expected = oracle.to_storage_snapshot();
            let certified = is_certified(&db, &package);
            if certified {
                accepted += 1;
                assert_eq!(before, expected);
            } else {
                declined += 1;
            }
            let full_attempts = db.full_registration_attempts;
            db.upsert_package(package.clone()).unwrap();
            assert_eq!(db.catalog().to_storage_snapshot(), expected);
            assert_eq!(
                db.full_registration_attempts - full_attempts,
                usize::from(!certified)
            );
            let (_, storage) = db.into_parts();
            db = EmbeddedDb::open(storage).unwrap();
        }
    }
    assert!(accepted > 0 && declined > 0);
}

#[test]
fn catalog_replacement_during_fast_fence_retries_and_repairs() {
    let (storage, counts) = CountingEntityStorage::new();
    let mut db = EmbeddedDb::new(storage);
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    assert!(is_certified(&db, &package));
    let mut changed = (*db.catalog()).clone();
    changed.delete_class("shared.test.note");
    *counts.replace_catalog_at_fence.lock().unwrap() = Some((db.shared_catalog().clone(), changed));
    let full_attempts = db.full_registration_attempts;
    db.upsert_package(package.clone()).unwrap();
    assert_eq!(db.full_registration_attempts, full_attempts + 1);
    assert!(db.catalog().class_id("shared.test.note").is_some());
    assert!(is_certified(&db, &package));
}

#[test]
fn certificates_and_new_migrations_never_reexecute_applied_data_operations() {
    let mut db = EmbeddedDb::in_memory();
    let mut package = simple_schema_package("Historical data.");
    let historical = [("id".to_string(), Value::String("historical".into()))]
        .into_iter()
        .collect();
    package.migrations[0]
        .operations
        .push(MigrationOperation::Insert {
            collection: DEFAULT_COLLECTION.into(),
            id: "historical".into(),
            object: historical,
        });
    db.upsert_package(package.clone()).unwrap();
    assert!(is_certified(&db, &package));
    let collection = db
        .catalog()
        .collection_by_name(DEFAULT_COLLECTION)
        .unwrap()
        .lid;
    db.transact(Batch::new().with_op(BatchOperation::DeleteById {
        collection: DEFAULT_COLLECTION.into(),
        id: "historical".into(),
    }))
    .unwrap();
    db.upsert_package(package.clone()).unwrap();
    assert!(
        db.storage
            .get_entity(collection, "historical")
            .unwrap()
            .is_none()
    );
    package.migrations.push(Migration {
        module: "test".into(),
        name: "002_next".into(),
        description: None,
        operations: vec![MigrationOperation::Insert {
            collection: DEFAULT_COLLECTION.into(),
            id: "new".into(),
            object: [("id".to_string(), Value::String("new".into()))]
                .into_iter()
                .collect(),
        }],
        meta: Meta::default(),
    });
    let outcome = db.upsert_package(package.clone()).unwrap();
    assert_eq!(outcome.executed_migrations.len(), 1);
    assert!(
        db.storage
            .get_entity(collection, "historical")
            .unwrap()
            .is_none()
    );
    assert!(db.storage.get_entity(collection, "new").unwrap().is_some());
    assert!(is_certified(&db, &package));
}

#[test]
fn missing_malformed_and_obsolete_certificates_fall_back_without_mutation() {
    for replacement in [None, Some("invalid"), Some("version"), Some("digest")] {
        let mut db = EmbeddedDb::in_memory();
        let package = semantic_data::filestore::package();
        db.upsert_package(package.clone()).unwrap();
        let collection = db
            .catalog()
            .collection_by_name(CORE_CATALOG_SCHEMA_COLLECTION)
            .unwrap()
            .lid;
        let mut meta = db
            .storage
            .get_entity(collection, "semantic:entry:meta/__catalog_meta__")
            .unwrap()
            .unwrap();
        let original = meta
            .object
            .get(registration_proof::FIELD)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        match replacement {
            None => {
                meta.object.remove(registration_proof::FIELD);
            }
            Some("invalid") => {
                meta.object
                    .insert(registration_proof::FIELD, Value::String("not json".into()));
            }
            Some("version") => {
                meta.object.insert(
                    registration_proof::FIELD,
                    Value::String(original.replace("\"version\":1", "\"version\":999")),
                );
            }
            Some("digest") => {
                meta.object.insert(
                    registration_proof::FIELD,
                    Value::String(
                        original.replace("catalog_digest\":\"", "catalog_digest\":\"wrong"),
                    ),
                );
            }
            _ => unreachable!(),
        }
        db.storage
            .apply_batch(&[StorageWriteOp::PutEntity(meta)])
            .unwrap();
        let (_, storage) = db.into_parts();
        let mut reopened = EmbeddedDb::open(storage).unwrap();
        assert!(!is_certified(&reopened, &package), "{replacement:?}");
        let revision = reopened.storage().current_revision().unwrap();
        reopened.upsert_package(package).unwrap();
        assert_eq!(reopened.full_registration_attempts, 1);
        assert_eq!(reopened.storage().current_revision().unwrap(), revision);
    }
}

#[test]
fn stale_persisted_certificate_cannot_hide_catalog_drift() {
    let mut db = EmbeddedDb::in_memory();
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    let collection = db
        .catalog()
        .collection_by_name(CORE_CATALOG_SCHEMA_COLLECTION)
        .unwrap()
        .lid;
    let meta = db
        .storage
        .get_entity(collection, "semantic:entry:meta/__catalog_meta__")
        .unwrap()
        .unwrap();
    let old_proof = meta.object.get(registration_proof::FIELD).unwrap().clone();
    let mut changed = (*db.catalog()).clone();
    changed.delete_class("shared.test.note");
    let mut ops = catalog_write_ops(&db.storage, &changed).unwrap();
    for op in &mut ops {
        if let StorageWriteOp::PutEntity(entity) = op
            && entity.id == meta.id
        {
            entity
                .object
                .insert(registration_proof::FIELD, old_proof.clone());
        }
    }
    db.storage.apply_batch(&ops).unwrap();
    let (_, storage) = db.into_parts();
    let mut reopened = EmbeddedDb::open(storage).unwrap();
    assert!(!is_certified(&reopened, &package));
    assert!(reopened.catalog().class_id("shared.test.note").is_none());
    reopened.upsert_package(package.clone()).unwrap();
    assert!(reopened.catalog().class_id("shared.test.note").is_some());
    assert!(is_certified(&reopened, &package));
}

#[test]
fn forward_core_migration_certifies_legacy_catalog_without_replaying_data() {
    let mut db = EmbeddedDb::in_memory();
    let package = semantic_data::filestore::package();
    db.upsert_package(package.clone()).unwrap();
    let mut snapshot = db.catalog().to_storage_snapshot();
    snapshot.applied_migrations.retain(|applied| {
        applied.applied.migration.name != crate::ddl::REGISTRATION_PROOFS_MIGRATION
    });
    let mut legacy = Catalog::from_storage_snapshot(snapshot).unwrap();
    legacy.delete_attribute("semantic:db:registration_proofs");
    db.storage
        .apply_batch(&catalog_write_ops(&db.storage, &legacy).unwrap())
        .unwrap();
    let (_, storage) = db.into_parts();
    let mut reopened = EmbeddedDb::open(storage).unwrap();
    assert!(is_certified(&reopened, &package));
    let revision = reopened.storage().current_revision().unwrap();
    reopened.upsert_package(package).unwrap();
    assert_eq!(reopened.full_registration_attempts, 0);
    assert_eq!(reopened.storage().current_revision().unwrap(), revision);
}

#[test]
fn data_writes_preserve_certificates_and_external_catalog_cas_invalidates_them() {
    let mut db = EmbeddedDb::in_memory();
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    assert!(is_certified(&db, &package));
    db.transact(
        Batch::new().with_op(BatchOperation::Upsert {
            collection: DEFAULT_COLLECTION.into(),
            id: "unrelated".into(),
            object: [("id".to_string(), Value::String("unrelated".into()))]
                .into_iter()
                .collect(),
        }),
    )
    .unwrap();
    let full_attempts = db.full_registration_attempts;
    db.upsert_package(package.clone()).unwrap();
    assert_eq!(db.full_registration_attempts, full_attempts);

    // Public SharedCatalog replacement is available independently of EmbeddedDb.
    // Even reinstalling the same Arc must invalidate the version-bound proof.
    let snapshot = db.catalog.snapshot();
    db.shared_catalog()
        .compare_and_swap_arc(snapshot.version, snapshot.catalog)
        .unwrap();
    assert!(!is_certified(&db, &package));
    let revision = db.storage().current_revision().unwrap();
    db.upsert_package(package).unwrap();
    assert_eq!(db.full_registration_attempts, full_attempts + 1);
    assert_eq!(db.storage().current_revision().unwrap(), revision);
}

#[test]
fn failed_catalog_commit_does_not_install_candidate_certificates() {
    use std::sync::atomic::Ordering;

    let (storage, counts) = CountingEntityStorage::new();
    let mut db = EmbeddedDb::new(storage);
    let original = simple_schema_package("Original migration.");
    db.upsert_package(original.clone()).unwrap();
    let mut updated = original.clone();
    updated.meta.description = Some("changed package metadata".into());
    let before = db.catalog().to_storage_snapshot();
    let revision = db.storage().current_revision().unwrap();
    let mut changes = db.subscribe_changes(crate::ChangeSubscriptionOptions::default());
    counts.inject_conflicts.store(4, Ordering::Relaxed);
    assert!(matches!(
        db.upsert_package(updated.clone()),
        Err(DbError::TransactionConflict(_))
    ));
    assert_eq!(db.catalog().to_storage_snapshot(), before);
    assert_eq!(db.storage().current_revision().unwrap(), revision);
    assert!(is_certified(&db, &original));
    assert!(!is_certified(&db, &updated));
    assert!(changes.next().now_or_never().is_none());

    let (_, storage) = db.into_parts();
    let reopened = EmbeddedDb::open(storage).unwrap();
    assert!(is_certified(&reopened, &original));
    assert!(!is_certified(&reopened, &updated));
}

#[test]
fn certified_registration_exhausts_revision_conflicts_without_writes() {
    use std::sync::atomic::Ordering;

    let (storage, counts) = CountingEntityStorage::new();
    let mut db = EmbeddedDb::new(storage);
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    counts.reset();
    counts.inject_revision_conflicts.store(4, Ordering::Relaxed);
    let full_attempts = db.full_registration_attempts;
    assert!(matches!(
        db.upsert_package(package),
        Err(DbError::TransactionConflict(_))
    ));
    assert_eq!(db.full_registration_attempts, full_attempts);
    assert_eq!(counts.inject_revision_conflicts.load(Ordering::Relaxed), 0);
    assert_eq!(counts.commit_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn failed_repair_preserves_catalog_revision_and_change_feed() {
    let mut db = EmbeddedDb::in_memory();
    let package = simple_schema_package("Original migration.");
    db.upsert_package(package.clone()).unwrap();
    db.activate_validation().unwrap();
    let class_id = db
        .catalog()
        .class_by_lid(db.catalog().class_id("shared.test.note").unwrap())
        .unwrap()
        .class
        .id
        .clone();
    let attribute_id = db
        .catalog()
        .attribute_by_id("shared.test.title")
        .unwrap()
        .attribute
        .id
        .clone();
    db.transact_ddl(DdlBatch::new().with_op(DdlOperation::DeleteClass {
        id: "shared.test.note".into(),
    }))
    .unwrap();
    assert!(!is_certified(&db, &package));
    let collection = db
        .catalog()
        .collection_by_name(DEFAULT_COLLECTION)
        .unwrap()
        .lid;
    // Controlled invalid legacy data: reintroducing the class must still run
    // ordinary row validation and atomically reject this wrong-typed attribute.
    db.storage
        .apply_batch(&[StorageWriteOp::PutEntity(StoredEntity {
            collection: collection.0,
            kind: StoredEntityKind::Class,
            id: "invalid".into(),
            object: [
                ("id".into(), Value::String("invalid".into())),
                ("type".into(), Value::String(class_id)),
                (attribute_id, Value::Bool(true)),
            ]
            .into_iter()
            .collect(),
        })])
        .unwrap();
    let revision = db.storage().current_revision().unwrap();
    let before = db.catalog().to_storage_snapshot();
    let mut changes = db.subscribe_changes(crate::ChangeSubscriptionOptions::default());
    assert!(db.upsert_package(package.clone()).is_err());
    assert_eq!(db.storage().current_revision().unwrap(), revision);
    assert_eq!(db.catalog().to_storage_snapshot(), before);
    assert!(!is_certified(&db, &package));
    assert!(changes.next().now_or_never().is_none());
}

#[test]
fn unchanged_filestore_has_identical_reconciliation_snapshot() {
    let mut db = EmbeddedDb::in_memory();
    let package = semantic_data::filestore::package();
    db.upsert_package(package.clone()).unwrap();
    let normalized = normalize_package_definition(&package).unwrap();
    let installed = db.catalog();
    let revision = db.storage().current_revision().unwrap();
    let (reconciled, before, after, executed) = db
        .apply_package_update(&installed, revision, &normalized)
        .unwrap();
    assert!(before.is_empty());
    assert!(after.is_empty());
    assert!(executed.is_empty());
    assert_eq!(
        reconciled.to_storage_snapshot(),
        installed.to_storage_snapshot()
    );
}

#[test]
fn unchanged_filestore_repairs_missing_index_then_preserves_revision() {
    let mut db = EmbeddedDb::in_memory();
    let package = semantic_data::filestore::package();
    db.upsert_package(package.clone()).unwrap();
    let (collection, name) = package
        .migrations
        .iter()
        .flat_map(|migration| &migration.operations)
        .find_map(|operation| match operation {
            MigrationOperation::Ddl(
                semantic_data::schema::MigrationDdlOperation::UpsertIndex {
                    collection, name, ..
                },
            ) => Some((collection.clone(), name.clone())),
            _ => None,
        })
        .unwrap();
    db.transact_ddl(DdlBatch::new().with_op(DdlOperation::DeleteIndex {
        name: name.clone(),
        collection: collection.clone(),
    }))
    .unwrap();
    db.upsert_package(package.clone()).unwrap();
    assert!(
        db.catalog()
            .indexes_for_collection(db.catalog().collection_by_name(&collection).unwrap().lid)
            .any(|index| index.schema.name == name)
    );
    let revision = db.storage().current_revision().unwrap();
    db.upsert_package(package).unwrap();
    assert_eq!(db.storage().current_revision().unwrap(), revision);
}
