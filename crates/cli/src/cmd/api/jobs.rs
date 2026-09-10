use crate::{
    CliError,
    cmd::shared::{ApiClientArgs, OutputArgs},
};
use semantic_data::{Object, Value};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
    #[command(flatten)]
    pub client: ApiClientArgs,
    #[command(flatten)]
    pub output: OutputArgs,
}
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// List operational job history (runtime inputs and outputs are not stored).
    List {
        #[arg(long)]
        status: Vec<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    Get {
        id: String,
    },
    Cancel {
        id: String,
    },
    /// Delete all currently completed job records. Domain data remains intact.
    ClearCompleted,
    Kinds,
}
pub async fn run(args: Args) -> Result<(), CliError> {
    let mut payload = Object::new();
    let command = match args.command {
        Command::List {
            status,
            kind,
            limit,
        } => {
            payload.insert(
                "statuses",
                Value::List(status.into_iter().map(Value::String).collect()),
            );
            if let Some(kind) = kind {
                payload.insert("kind", kind);
            }
            payload.insert("limit", limit as u64);
            "semantic.jobs.list"
        }
        Command::Get { id } => {
            payload.insert("id", id);
            "semantic.jobs.get"
        }
        Command::Cancel { id } => {
            payload.insert("id", id);
            "semantic.jobs.cancel"
        }
        Command::ClearCompleted => "semantic.jobs.clear_completed",
        Command::Kinds => "semantic.jobs.kinds",
    };
    args.client.insert_scope(&mut payload);
    let result = args
        .client
        .rpc_client()
        .invoke_value(command, Value::Object(payload))
        .await?;
    args.output.print(&result)
}
