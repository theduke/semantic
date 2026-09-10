use crate::{
    CliError,
    cmd::shared::{ApiClientArgs, OutputArgs},
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
    /// List URL importers in priority order, including unsupported/unavailable reasons.
    Candidates { url: String },
    /// Fetch without saving, printing incremental content events and a terminal summary.
    Fetch { url: String },
    /// Start an import and return its job ID. Inspect progress with `api jobs get`.
    Start {
        url: String,
        /// Explicit plugin ID, as returned by candidates.
        #[arg(long, requires = "export")]
        plugin: Option<String>,
        /// Explicit export name, as returned by candidates.
        #[arg(long, requires = "plugin")]
        export: Option<String>,
        /// Reject a selection if this activation generation has changed.
        #[arg(long, requires = "plugin")]
        generation: Option<u64>,
    },
}
pub async fn run(args: Args) -> Result<(), CliError> {
    let mut payload = Object::new();
    if let Command::Fetch { url } = &args.command {
        use futures::StreamExt;
        use semantic_rpc::interface::{
            InvocationArgument, InvocationOutput, StreamEvent, ValidatedInvocation,
        };
        payload.insert("url", url.clone());
        args.client.insert_scope(&mut payload);
        let rpc = args.client.rpc_client();
        let output = rpc
            .invoke_interface(ValidatedInvocation {
                export: "application".into(),
                method: "fetch_source".into(),
                arguments: vec![InvocationArgument::Value(Value::Object(payload))],
            })
            .await?;
        let InvocationOutput::Stream(mut stream) = output else {
            return Err(CliError::InvalidInput(
                "expected fetched content stream".into(),
            ));
        };
        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::Item(value) => args.output.print(&value)?,
                StreamEvent::End(summary) => {
                    let mut terminal = Object::new();
                    terminal.insert("end", summary.unwrap_or(Value::Null));
                    args.output.print(&Value::Object(terminal))?;
                    return Ok(());
                }
            }
        }
        return Err(CliError::InvalidInput(
            "fetch ended without terminal summary".into(),
        ));
    }
    let command = match args.command {
        Command::Fetch { .. } => unreachable!("fetch handled above"),
        Command::Candidates { url } => {
            payload.insert("url", url);
            "semantic.import.candidates"
        }
        Command::Start {
            url,
            plugin,
            export,
            generation,
        } => {
            payload.insert("url", url);
            if let Some(plugin) = plugin {
                payload.insert("plugin_id", plugin);
            }
            if let Some(export) = export {
                payload.insert("export", export);
            }
            if let Some(generation) = generation {
                payload.insert("generation", generation);
            }
            "semantic.import.start_source"
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
