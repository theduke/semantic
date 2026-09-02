use std::io::Read as _;
use std::path::PathBuf;

use clap::ValueEnum;
use semantic_data::value::{Object, Value};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, OutputArgs};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum QueryFormat {
    Sql,
    Prql,
}

impl QueryFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sql => "sql",
            Self::Prql => "prql",
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Query text; when omitted, read the query as UTF-8 from standard input.
    #[arg(value_name = "QUERY", conflicts_with = "file")]
    pub query: Option<String>,

    /// Read the query as UTF-8 from this file instead of QUERY or standard input.
    #[arg(long, short = 'f', value_name = "PATH")]
    pub file: Option<PathBuf>,

    /// Language used to parse the query.
    #[arg(long, value_enum, default_value = "sql")]
    pub format: QueryFormat,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let query = read_query(&args)?;
    let mut payload = Object::new();
    payload.insert("query", Value::String(query));
    payload.insert("format", Value::String(args.format.as_str().to_string()));
    args.client.insert_scope(&mut payload);
    let response = args
        .client
        .rpc_client()
        .invoke_value("semantic.db.query", Value::Object(payload))
        .await?;
    args.output.print(&response)
}

fn read_query(args: &Args) -> std::result::Result<String, CliError> {
    if let Some(query) = &args.query {
        return Ok(query.clone());
    }
    if let Some(path) = &args.file {
        return std::fs::read_to_string(path).map_err(|source| CliError::Io {
            action: "read",
            path: path.display().to_string(),
            source,
        });
    }
    let mut query = String::new();
    std::io::stdin()
        .read_to_string(&mut query)
        .map_err(|source| CliError::Io {
            action: "read",
            path: "standard input".to_string(),
            source,
        })?;
    Ok(query)
}
