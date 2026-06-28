use semantic_data::bundles::directory;
use semantic_data::schema::{Meta, Migration, MigrationDdlOperation, MigrationOperation};

use crate::{bundle::MODULE_NAME, schema::common};

pub const INIT_MIGRATION_NAME: &str = "001_init";

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
    migrations
}
