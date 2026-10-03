use crate::{
    embedded::{EmbeddedDb, MemoryEntityStorage},
    validate_package_migrations,
};
use semantic_data::{bundles::query as query_package, query::semantic::definitions};

#[test]
fn query_ast_package_installs_reopens_and_is_idempotent() {
    let package = query_package::package();
    validate_package_migrations(&package).unwrap();
    let mut db = EmbeddedDb::new(MemoryEntityStorage::new());
    let outcome = db.upsert_package(package.clone()).unwrap();
    assert_eq!(outcome.executed_migrations.len(), 1);
    let catalog = db.catalog();
    for name in definitions().keys() {
        let definition = catalog.type_def_by_name(name).unwrap();
        assert!(
            definition.data.data_type().is_some(),
            "{name} must resolve to a data type"
        );
    }
    drop(catalog);
    assert!(
        db.upsert_package(package.clone())
            .unwrap()
            .executed_migrations
            .is_empty()
    );
    let (_, storage) = db.into_parts();
    let mut reopened = EmbeddedDb::open(storage).unwrap();
    assert!(
        reopened
            .upsert_package(package)
            .unwrap()
            .executed_migrations
            .is_empty()
    );
    assert!(
        reopened
            .catalog()
            .type_def_by_name("semantic:query:Query")
            .is_some()
    );
}
