use semantic_data::value::{Object, Value};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, OutputArgs};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let mut payload = Object::new();
    args.client.insert_scope(&mut payload);
    let response = args
        .client
        .rpc_client()
        .invoke_value("semantic.db.catalog", Value::Object(payload))
        .await?;
    args.output.print(&response)
}
