//! Registration invariants and differential regressions for the proposed shortcut.

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
    let mut changes = reopened.subscribe_changes(crate::ChangeSubscriptionOptions::default());
    counts.reset();

    let outcome = reopened.upsert_package(package).unwrap();
    assert!(outcome.executed_migrations.is_empty());
    assert_eq!(reopened.storage().current_revision().unwrap(), revision);
    assert_eq!(reopened.catalog.snapshot().version, catalog_version);
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

    assert!(
        db.upsert_package(package)
            .unwrap()
            .executed_migrations
            .is_empty()
    );
    assert_eq!(counts.inject_revision_conflicts.load(Ordering::Relaxed), 0);
    assert_eq!(db.storage().current_revision().unwrap(), revision);
    assert_eq!(counts.commit_calls.load(Ordering::Relaxed), 0);
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
