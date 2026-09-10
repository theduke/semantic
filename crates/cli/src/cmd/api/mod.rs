mod apply;
mod catalog;
mod delete;
mod file;
mod get;
mod import;
mod jobs;
mod package;
mod plugin;
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
    /// Discover URL importers and start imports.
    Import(import::Args),
    /// Inspect and configure scope plugins.
    Plugin(plugin::Args),
    /// Inspect, cancel, and clear scope jobs.
    Jobs(jobs::Args),
    /// Execute a SQL or PRQL query and print the API result as JSON.
    Query(query::Args),

    /// Fetch one record by ID from a collection.
    Get(get::Args),

    /// Permanently delete one or more records by ID.
    Delete(delete::Args),

    /// Fetch the database catalog, including known schema metadata.
    Catalog(catalog::Args),

    /// Create or update schema packages.
    Package(package::Args),

    /// Run operations on previously uploaded files.
    File(file::Args),

    /// Apply database operations from a JSON batch.
    ///
    /// INPUT may be a batch object or a bare array of operations. Use '-' or
    /// omit INPUT to read JSON from standard input. Bare arrays are wrapped in
    /// the API's batch object automatically.
    ///
    /// Batch operations may be destructive and are submitted without a
    /// confirmation prompt.
    Apply(apply::Args),

    /// Upload files, optionally traversing local directories.
    Upload(upload::Args),
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    match args.command {
        SubCmd::Import(args) => import::run(args).await,
        SubCmd::Plugin(args) => plugin::run(args).await,
        SubCmd::Jobs(args) => jobs::run(args).await,
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
