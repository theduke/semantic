use std::io::Write;

use anyhow::Context;

use semantic_core::{base::SemanticBasePlugin, plugin::PluginDescriptor};

use super::CliCommand;

#[derive(clap::Parser)]
pub struct CmdGenerateTypescript {}

impl CliCommand for CmdGenerateTypescript {
    fn run(self) -> Result<(), anyhow::Error> {
        let builtin = factdb::schema::builtin::builtin_db_schema();
        let base = SemanticBasePlugin::new()
            .schema()
            .db
            .context("Could not load db schema")?;

        let schema = builtin.merge(base);

        let ts = factor_tools::typescript::schema_to_typescript(&schema, None)?;

        write!(std::io::stdout(), "{}", ts).unwrap();

        Ok(())
    }
}
