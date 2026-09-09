pub mod bundle;
pub mod directory_query;
pub mod migrations;
pub mod schema;

pub use bundle::{MODULE_NAME, PACKAGE_NAME, package, root_module};

#[cfg(test)]
mod tests {
    use semantic_data::schema::{MigrationDdlOperation, MigrationOperation};

    use crate::{
        bundle, migrations,
        schema::{common, notes, web_bookmark},
    };

    #[test]
    fn package_has_expected_structure() {
        let package = bundle::package();

        assert_eq!(package.name, bundle::PACKAGE_NAME);
        assert_eq!(package.root.name, bundle::MODULE_NAME);
        assert!(package.modules.is_empty());
        assert_eq!(package.migrations.len(), 4);
        assert_eq!(package.migrations[0].name, migrations::INIT_MIGRATION_NAME);
        assert_eq!(package.migrations[2].name, migrations::NOTES_MIGRATION_NAME);

        assert!(
            package.migrations[0]
                .operations
                .iter()
                .all(|operation| !matches!(
                    operation,
                    MigrationOperation::Ddl(MigrationDdlOperation::UpsertCollection { .. })
                ))
        );

        assert!(package.root.classes.contains_key(common::person::CLASS_ID));
        assert!(package.root.classes.contains_key(notes::CLASS_ID));
        assert!(package.root.classes.contains_key(web_bookmark::CLASS_ID));
        assert_eq!(
            package.migrations[3].name,
            migrations::WEB_BOOKMARK_MIGRATION_NAME
        );
        assert!(
            package
                .root
                .attributes
                .contains_key(common::person::ATTR_PARENT)
        );
        assert_eq!(common::person::ATTR_PARENT, "semantic:parent");

        for attribute in common::person::attributes() {
            assert!(package.root.attributes.contains_key(&attribute.id));
        }
        for attribute in notes::attributes() {
            assert!(package.root.attributes.contains_key(&attribute.id));
        }
    }

    #[test]
    fn classes_reference_root_attributes() {
        let root = bundle::root_module();
        let core = semantic_db_core::fresh_catalog_with_core_schema().unwrap();

        for class in root.classes.values() {
            for attribute in class.attributes.values() {
                assert!(
                    root.attributes.contains_key(&attribute.attribute.id)
                        || core.attribute_id(&attribute.attribute.id).is_some()
                        || attribute.attribute.id.starts_with("semantic:relation:"),
                    "missing root attribute {} referenced by class {}",
                    attribute.attribute.id,
                    class.id
                );
            }
        }
    }

    #[test]
    fn migrations_validate_against_package_schema() {
        semantic_db_core::validate_package_migrations(&bundle::package()).unwrap();
    }

    #[test]
    fn bookmark_references_core_url_without_redeclaring_it() {
        use semantic_data::attr::ATTR_URL;

        let package = bundle::package();
        assert!(!package.root.attributes.contains_key(ATTR_URL));
        let bookmark = &package.root.classes[web_bookmark::CLASS_ID];
        assert_eq!(bookmark.attributes["url"].attribute.id, ATTR_URL);
        assert!(bookmark.attributes["url"].required);
        assert!(
            package
                .migrations
                .iter()
                .flat_map(|migration| &migration.operations)
                .all(|operation| {
                    !matches!(operation, MigrationOperation::Ddl(MigrationDdlOperation::UpsertIndex { field, .. }) if field == ATTR_URL)
                        && !matches!(operation, MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute }) if attribute.id == ATTR_URL)
                })
        );
    }
}
