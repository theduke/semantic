pub mod comments;
pub mod content;
mod domain_support;
#[cfg(test)]
mod domain_tests;
#[allow(dead_code)]
mod migration_support_v1;
pub mod tasks;
pub use comments::CommentsPackage;
pub use tasks::TasksPackage;
pub mod bundle;
pub mod directory_query;
pub mod labels;
pub mod migrations;
pub mod schema;
pub mod server;

pub use bundle::{MODULE_NAME, PACKAGE_NAME, package, root_module};

/// The built-in semantic schema package.
#[derive(Clone, Copy, Debug, Default)]
pub struct BasePackage;

impl<Ctx, E> semantic_rpc_core::RuntimePackage<Ctx, E> for BasePackage
where
    Ctx: labels::LabelContext,
    E: From<semantic_rpc_core::RpcError>,
{
    fn schema(&self) -> semantic_data::schema::Package {
        bundle::package()
    }

    fn commands(&self) -> Vec<Box<dyn semantic_rpc_core::DynCommand<Ctx, E>>> {
        labels::commands()
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::schema::{MigrationDdlOperation, MigrationOperation};

    use crate::{
        bundle, migrations,
        schema::{common, notes, web_bookmark},
    };

    #[test]
    fn package_has_expected_structure() {
        let package = crate::package();

        assert_eq!(package.name, bundle::PACKAGE_NAME);
        assert_eq!(package.root.name, bundle::MODULE_NAME);
        let shared = &package.modules[semantic_data::bundles::shared::MODULE_NAME];
        assert_eq!(package.modules.len(), 1);
        assert_eq!(package.migrations.len(), 13);
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
            shared
                .attributes
                .contains_key(semantic_data::attr::ATTR_PARENT)
        );
        assert_eq!(semantic_data::attr::ATTR_PARENT, "semantic:parent");

        for attribute in common::person::attributes() {
            assert!(
                package.root.attributes.contains_key(&attribute.id)
                    || shared.attributes.contains_key(&attribute.id)
            );
        }
        for attribute in notes::attributes() {
            assert!(
                package.root.attributes.contains_key(&attribute.id)
                    || shared.attributes.contains_key(&attribute.id)
            );
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
    fn note_markdown_default_is_a_forward_migration() {
        let original = migrations::notes_migration();
        let MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class: old_class }) =
            original.operations.last().unwrap()
        else {
            panic!("expected original Note class");
        };
        assert_eq!(old_class.attributes["note_format"].default, None);
        assert_eq!(
            old_class.attributes["note_format"].attribute.id,
            notes::ATTR_NOTE_FORMAT
        );

        let updated = migrations::note_markdown_default_migration();
        assert_eq!(
            updated.name,
            migrations::NOTE_MARKDOWN_DEFAULT_MIGRATION_NAME
        );
        assert_eq!(
            updated.operations,
            vec![MigrationOperation::Ddl(
                MigrationDdlOperation::UpsertClass {
                    class: notes::class()
                }
            )]
        );
        assert_eq!(
            crate::package().migrations.last().unwrap().name,
            "013_include_in_listings"
        );
    }

    #[test]
    fn bookmark_title_migration_preserves_the_original_definition() {
        let original = migrations::web_bookmark_migration();
        let updated = migrations::web_bookmark_title_migration();
        assert_eq!(updated.name, migrations::WEB_BOOKMARK_TITLE_MIGRATION_NAME);
        let MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }) =
            &original.operations[0]
        else {
            panic!("expected bookmark class migration");
        };
        assert_eq!(class.meta.title.as_deref(), Some("Web Bookmark"));
        let mut expected = class.clone();
        expected.meta.title = Some("WebBookmark".to_string());
        assert_eq!(expected, web_bookmark::class());
        assert_eq!(
            updated.operations,
            vec![MigrationOperation::Ddl(
                MigrationDdlOperation::UpsertClass { class: expected }
            )]
        );
    }

    #[test]
    fn bookmark_references_core_url_without_redeclaring_it() {
        use semantic_data::attr::ATTR_URL;

        let package = crate::package();
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
