use std::{
    io::Write,
    sync::{Arc, Mutex},
};

use semantic_data::schema::{MigrationDdlOperation, MigrationOperation};
use semantic_db_core::embedded::EmbeddedDb;
use semantic_db_core::{DdlBatch, DdlOperation};

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
fn unchanged_base_registration_after_reopen_reports_reconciliation() {
    let mut db = semantic_db_kv::open_memory().unwrap();
    let package = semantic_base::package();
    db.upsert_package(package.clone()).unwrap();
    let (_, storage) = db.into_parts();
    let mut reopened = EmbeddedDb::open(storage).unwrap();
    let revision = reopened.storage().current_revision().unwrap();

    let captured = Arc::new(Mutex::new(Vec::new()));
    let writer = TraceWriter(Arc::clone(&captured));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let outcome = reopened.upsert_package(package).unwrap();
        assert!(outcome.executed_migrations.is_empty());
    });

    assert_eq!(reopened.storage().current_revision().unwrap(), revision);
    let events = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    assert!(
        events.contains("registration_path=\"unchanged_reconciled\"")
            && events.contains("catalog_changed=false"),
        "{events}"
    );
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
