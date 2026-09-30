use semantic_data::schema::Migration;
use sha2::{Digest, Sha256};

fn migrations() -> Vec<(&'static str, Vec<Migration>)> {
    vec![
        ("core", semantic_db_core::core_schema_migrations()),
        ("base", semantic_base::package().migrations),
        ("files", semantic_data::filestore::package().migrations),
        ("jobs", semantic_data::jobs::package().migrations),
        ("plugins", semantic_data::plugin::package().migrations),
        ("comments", semantic_base::comments::package().migrations),
    ]
}

#[test]
fn historical_migrations_remain_unchanged() {
    let expected = include_str!("listing_migrations.sha256");
    let mut fingerprints = String::new();
    for (package, migrations) in migrations() {
        for migration in migrations {
            let key = format!("{package} {} {} ", migration.module, migration.name);
            if !expected.lines().any(|line| line.starts_with(&key)) {
                continue; // Newly added forward migrations have no historical fingerprint.
            }
            let encoded = facet_json::to_string(&migration).unwrap();
            fingerprints.push_str(&format!(
                "{package} {} {} {:x}\n",
                migration.module,
                migration.name,
                Sha256::digest(encoded.as_bytes())
            ));
        }
    }
    assert_eq!(fingerprints, expected);
}

#[test]
fn forward_migrations_match_current_packages() {
    for package in [
        semantic_base::package(),
        semantic_data::filestore::package(),
        semantic_data::jobs::package(),
        semantic_data::plugin::package(),
        semantic_base::comments::package(),
    ] {
        semantic_db_core::validate_package_migrations(&package).unwrap();
    }
}

#[tokio::test]
async fn packages_upgrade_from_before_listing_metadata() {
    let db = semantic_db_core::Db::new(semantic_db_core::embedded::EmbeddedBackend::new(
        semantic_db_kv::open_memory().unwrap(),
    ));
    for package in [
        semantic_base::package(),
        semantic_data::filestore::package(),
        semantic_base::comments::package(),
    ] {
        let mut previous = package.clone();
        assert!(
            previous
                .migrations
                .pop()
                .unwrap()
                .name
                .ends_with("include_in_listings")
        );
        for class in previous.root.classes.values_mut() {
            class.include_in_ui_listings = None;
        }
        db.upsert_package(previous).await.unwrap();
        db.upsert_package(package.clone()).await.unwrap();
        db.upsert_package(package).await.unwrap();
    }
}
