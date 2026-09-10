use crate::{
    CliError,
    cmd::shared::{ApiClientArgs, FileInputArgs, OutputArgs},
};
use semantic_data::{Object, Value};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
    #[command(flatten)]
    client: ApiClientArgs,
    #[command(flatten)]
    output: OutputArgs,
}
#[derive(Debug, clap::Subcommand)]
enum Command {
    /// List installed plugin descriptors and desired activation settings.
    List,
    /// Install or update a plugin using a JSON activation descriptor.
    Configure(FileInputArgs),
    /// Uninstall a plugin. Previously imported entities and files remain intact.
    Uninstall { id: String },
}
pub async fn run(args: Args) -> Result<(), CliError> {
    let mut payload = Object::new();
    let command = match args.command {
        Command::List => "semantic.plugin.list",
        Command::Configure(input) => {
            payload.insert("activation", input.read_json()?);
            "semantic.plugin.configure"
        }
        Command::Uninstall { id } => {
            payload.insert("id", id);
            "semantic.plugin.uninstall"
        }
    };
    args.client.insert_scope(&mut payload);
    let result = args
        .client
        .rpc_client()
        .invoke_value(command, Value::Object(payload))
        .await?;
    args.output.print(&result)
}
