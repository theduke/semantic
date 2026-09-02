use semantic_data::value::{Object, Value};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, CollectionArgs, OutputArgs};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// ID of the record to fetch.
    #[arg(value_name = "ID")]
    pub id: String,

    #[command(flatten)]
    pub collection: CollectionArgs,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let mut payload = Object::new();
    payload.insert("id", Value::String(args.id));
    args.collection.insert_collection(&mut payload);
    args.client.insert_scope(&mut payload);
    let response = args
        .client
        .rpc_client()
        .invoke_value("semantic.db.get", Value::Object(payload))
        .await?;
    args.output.print(&response)
}
