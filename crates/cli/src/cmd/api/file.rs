use clap::Subcommand;
use semantic_data::value::{Object, Value};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, OutputArgs};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub command: SubCmd,
}

#[derive(Debug, Subcommand)]
pub enum SubCmd {
    /// Analyze a previously uploaded file and persist its metadata.
    Analyze(AnalyzeArgs),
}

#[derive(Debug, clap::Args)]
pub struct AnalyzeArgs {
    /// Persisted file record ID.
    pub id: String,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    match args.command {
        SubCmd::Analyze(args) => analyze(args).await,
    }
}

async fn analyze(args: AnalyzeArgs) -> std::result::Result<(), CliError> {
    let mut payload = Object::new();
    payload.insert("id", Value::String(args.id));
    args.client.insert_scope(&mut payload);
    let response = args
        .client
        .rpc_client()
        .invoke_value("semantic.file.analyze", Value::Object(payload))
        .await?;
    args.output.print(&response)
}
