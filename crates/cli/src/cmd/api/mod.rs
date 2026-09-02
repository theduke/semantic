mod apply;
mod catalog;
mod delete;
mod file;
mod get;
mod package;
mod query;
mod upload;

use clap::Subcommand;

use crate::CliError;

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub command: SubCmd,
}

#[derive(Debug, Subcommand)]
pub enum SubCmd {
    /// Execute a SQL or PRQL query.
    Query(query::Args),

    /// Get a record by ID.
    Get(get::Args),

    /// Delete a record by ID.
    Delete(delete::Args),

    /// Fetch the database catalog.
    Catalog(catalog::Args),

    /// Manage schema packages.
    Package(package::Args),

    /// Work with persisted files.
    File(file::Args),

    /// Apply a JSON batch of database operations.
    Apply(apply::Args),

    /// Upload one or more files.
    Upload(upload::Args),
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    match args.command {
        SubCmd::Query(args) => query::run(args).await,
        SubCmd::Get(args) => get::run(args).await,
        SubCmd::Delete(args) => delete::run(args).await,
        SubCmd::Catalog(args) => catalog::run(args).await,
        SubCmd::Package(args) => package::run(args).await,
        SubCmd::File(args) => file::run(args).await,
        SubCmd::Apply(args) => apply::run(args).await,
        SubCmd::Upload(args) => upload::run(args).await,
    }
}
