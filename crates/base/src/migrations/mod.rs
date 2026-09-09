use semantic_data::bundles::directory;
use semantic_data::schema::{Meta, Migration, MigrationDdlOperation, MigrationOperation};

use crate::{
    bundle::MODULE_NAME,
    schema::{common, notes, web_bookmark},
};

pub const INIT_MIGRATION_NAME: &str = "001_init";
pub const NOTES_MIGRATION_NAME: &str = "003_notes";
pub const WEB_BOOKMARK_MIGRATION_NAME: &str = "004_web_bookmark";
pub const WEB_BOOKMARK_TITLE_MIGRATION_NAME: &str = "005_web_bookmark_title";

pub fn web_bookmark_migration() -> Migration {
    // Preserve the definition already recorded by existing databases.
    let mut class = web_bookmark::class();
    class.meta.title = Some("Web Bookmark".to_string());
    Migration {
        module: MODULE_NAME.to_string(),
        name: WEB_BOOKMARK_MIGRATION_NAME.to_string(),
        description: Some("Add web bookmarks.".to_string()),
        operations: vec![MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertClass { class },
        )],
        meta: Meta::default(),
    }
}

pub fn init() -> Migration {
    let mut operations = Vec::new();

    for attribute in common::person::attributes() {
        operations.push(MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertAttribute { attribute },
        ));
    }

    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertClass {
            class: common::person::class(),
        },
    ));

    Migration {
        module: MODULE_NAME.to_string(),
        name: INIT_MIGRATION_NAME.to_string(),
        description: Some("Initial semantic base schema.".to_string()),
        operations,
        meta: Meta::default(),
    }
}

pub fn all() -> Vec<Migration> {
    let mut migrations = vec![init()];
    migrations.extend(directory::migrations());
    migrations.push(notes_migration());
    migrations.push(web_bookmark_migration());
    migrations.push(web_bookmark_title_migration());
    migrations.push(creatable_in_ui_migration());
    migrations
}

pub fn web_bookmark_title_migration() -> Migration {
    Migration {
        module: MODULE_NAME.to_string(),
        name: WEB_BOOKMARK_TITLE_MIGRATION_NAME.to_string(),
        description: Some("Set the web bookmark display title to WebBookmark.".to_string()),
        operations: vec![MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertClass {
                class: web_bookmark::class(),
            },
        )],
        meta: Meta::default(),
    }
}

pub fn notes_migration() -> Migration {
    let mut operations = Vec::new();

    for attribute in notes::attributes() {
        operations.push(MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertAttribute { attribute },
        ));
    }

    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertClass {
            class: notes::class(),
        },
    ));

    Migration {
        module: MODULE_NAME.to_string(),
        name: NOTES_MIGRATION_NAME.to_string(),
        description: Some("Add note schema.".to_string()),
        operations,
        meta: Meta::default(),
    }
}

fn creatable_in_ui_migration() -> Migration {
    Migration {
        module: MODULE_NAME.to_string(),
        name: "008_creatable_in_ui".to_string(),
        description: Some(
            "Exclude directories and directory nodes from generic entity creators.".to_string(),
        ),
        operations: directory::classes()
            .into_iter()
            .map(|class| MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }))
            .collect(),
        meta: Meta::default(),
    }
}
