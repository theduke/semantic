use clap::ValueEnum;
use semantic_data::{Object, Value, value::IntoValue, vdb::VdbSchemaRequest};

use super::query::{self, QueryFormat, QuerySource};
use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, OutputArgs};

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// List virtual collections available in the scope.
    List(ListArgs),
    /// Show the schema exposed by a virtual collection.
    Schema(SchemaArgs),
    /// Explain a SELECT over local and virtual collections without scanning rows.
    Explain(ExplainArgs),
}

#[derive(Debug, clap::Args)]
struct ListArgs {
    #[command(flatten)]
    client: ApiClientArgs,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, clap::Args)]
struct SchemaArgs {
    /// Virtual collection name.
    name: String,
    #[command(flatten)]
    client: ApiClientArgs,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ExplainFormat {
    Sql,
    /// A query AST encoded as typed Semantic Value JSON.
    Ast,
}

#[derive(Debug, clap::Args)]
struct ExplainArgs {
    #[command(flatten)]
    source: QuerySource,
    /// Query format; AST input uses typed Semantic Value JSON.
    #[arg(long, value_enum, default_value = "sql")]
    format: ExplainFormat,
    /// Parameter bindings as a JSON object, e.g. '{"id":"item-1"}'.
    #[arg(long, value_name = "JSON")]
    params: Option<String>,
    #[command(flatten)]
    client: ApiClientArgs,
    #[command(flatten)]
    output: OutputArgs,
}

pub async fn run(args: Args) -> Result<(), CliError> {
    let (command, mut payload, client, output) = match args.command {
        Command::List(args) => ("semantic.vdb.list", Object::new(), args.client, args.output),
        Command::Schema(args) => {
            let Value::Object(payload) = VdbSchemaRequest {
                scope_id: args.client.scope.clone(),
                name: args.name,
            }
            .into_value() else {
                unreachable!("schema request is a record")
            };
            ("semantic.vdb.schema", payload, args.client, args.output)
        }
        Command::Explain(args) => {
            let format = match args.format {
                ExplainFormat::Sql => QueryFormat::Sql,
                ExplainFormat::Ast => QueryFormat::Ast,
            };
            let mut payload = query::query_payload(
                &query::read_query(&args.source)?,
                format,
                args.params.as_deref(),
            )?;
            // The VDB command accepts SQL text or a typed AST directly.
            payload.remove("format");
            ("semantic.vdb.explain", payload, args.client, args.output)
        }
    };
    client.insert_scope(&mut payload);
    let response = client
        .rpc_client()
        .invoke_value(command, Value::Object(payload))
        .await?;
    output.print(&response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn schema_and_explain_parse_scope_output_and_query_options() {
        let parsed = crate::cmd::Args::try_parse_from([
            "semantic", "api", "vdb", "schema", "fx", "--scope", "main", "--pretty",
        ])
        .unwrap();
        let crate::cmd::SubCmd::Api(api) = parsed.command else {
            panic!("api")
        };
        let super::super::SubCmd::Vdb(args) = api.command else {
            panic!("vdb")
        };
        let Command::Schema(args) = args.command else {
            panic!("schema")
        };
        assert_eq!(args.name, "fx");
        assert_eq!(args.client.scope.as_deref(), Some("main"));
        assert!(args.output.pretty);

        let parsed = crate::cmd::Args::try_parse_from([
            "semantic",
            "api",
            "vdb",
            "explain",
            "SELECT * FROM fx WHERE id = :id",
            "--params",
            r#"{"id":"item-1"}"#,
            "--format",
            "sql",
            "--pretty",
        ])
        .unwrap();
        let crate::cmd::SubCmd::Api(api) = parsed.command else {
            panic!("api")
        };
        let super::super::SubCmd::Vdb(args) = api.command else {
            panic!("vdb")
        };
        let Command::Explain(args) = args.command else {
            panic!("explain")
        };
        assert!(matches!(args.format, ExplainFormat::Sql));
        assert_eq!(args.params.as_deref(), Some(r#"{"id":"item-1"}"#));
        assert!(args.output.pretty);
    }

    #[test]
    fn explain_rejects_conflicting_input_and_unsupported_format() {
        for arguments in [
            vec!["SELECT * FROM fx", "--file", "query.sql"],
            vec!["SELECT * FROM fx", "--format", "prql"],
        ] {
            assert!(
                crate::cmd::Args::try_parse_from(
                    ["semantic", "api", "vdb", "explain"]
                        .into_iter()
                        .chain(arguments)
                )
                .is_err()
            );
        }
    }
}
