use semantic_data::value::{Object, Value};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, CollectionArgs, ConfirmationArgs, OutputArgs};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Record IDs.
    #[arg(required = true, num_args = 1.., value_name = "ID")]
    pub ids: Vec<String>,

    #[command(flatten)]
    pub collection: CollectionArgs,

    #[command(flatten)]
    pub confirmation: ConfirmationArgs,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let target = match &args.collection.collection {
        Some(collection) => format!("{} record(s) in {collection}", args.ids.len()),
        None => format!("{} record(s)", args.ids.len()),
    };
    args.confirmation.require(target)?;

    let mut operation = Object::new();
    operation.insert("kind", Value::String("delete_by_ids".to_string()));
    operation.insert(
        "ids",
        Value::List(args.ids.into_iter().map(Value::String).collect()),
    );
    args.collection.insert_collection(&mut operation);

    let mut payload = Object::new();
    payload.insert("operations", Value::List(vec![Value::Object(operation)]));
    args.client.insert_scope(&mut payload);
    let response = args
        .client
        .rpc_client()
        .invoke_value("semantic.db.batch", Value::Object(payload))
        .await?;
    args.output.print(&response)
}
