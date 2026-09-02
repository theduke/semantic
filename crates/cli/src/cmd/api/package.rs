use clap::Subcommand;
use semantic_data::value::{Object, Value};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, FileInputArgs, OutputArgs};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub command: SubCmd,
}

#[derive(Debug, Subcommand)]
pub enum SubCmd {
    /// Create or update a package from a facet-json document.
    Upsert(UpsertArgs),
}

#[derive(Debug, clap::Args)]
pub struct UpsertArgs {
    #[command(flatten)]
    pub input: FileInputArgs,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    match args.command {
        SubCmd::Upsert(args) => upsert(args).await,
    }
}

async fn upsert(args: UpsertArgs) -> std::result::Result<(), CliError> {
    let package = args.input.read()?;
    let mut payload = Object::new();
    payload.insert("format", Value::String("facet-json".to_string()));
    payload.insert("package", Value::String(package));
    args.client.insert_scope(&mut payload);
    let response = args
        .client
        .rpc_client()
        .invoke_value("semantic.db.package.upsert", Value::Object(payload))
        .await?;
    args.output.print(&response)
}
