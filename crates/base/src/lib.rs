pub mod bundle;
pub mod migrations;
pub mod schema;

pub use bundle::{MODULE_NAME, PACKAGE_NAME, package, root_module};

#[cfg(test)]
mod tests {
    use semantic_data::schema::{MigrationDdlOperation, MigrationOperation};

    use crate::{bundle, migrations, schema::common};

    #[test]
    fn package_has_expected_structure() {
        let package = bundle::package();

        assert_eq!(package.name, bundle::PACKAGE_NAME);
        assert_eq!(package.root.name, bundle::MODULE_NAME);
        assert!(package.modules.is_empty());
        assert_eq!(package.migrations.len(), 1);
        assert_eq!(package.migrations[0].name, migrations::INIT_MIGRATION_NAME);

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

        for attribute in common::person::attributes() {
            assert!(package.root.attributes.contains_key(&attribute.id));
        }
    }

    #[test]
    fn classes_reference_root_attributes_and_are_optional() {
        let root = bundle::root_module();

        for class in root.classes.values() {
            for attribute in class.attributes.values() {
                assert!(
                    root.attributes.contains_key(&attribute.attribute.id),
                    "missing root attribute {} referenced by class {}",
                    attribute.attribute.id,
                    class.id
                );
                assert!(!attribute.required);
            }
        }
    }

    #[test]
    fn migrations_validate_against_package_schema() {
        semantic_db_core::validate_package_migrations(&bundle::package()).unwrap();
    }
}
