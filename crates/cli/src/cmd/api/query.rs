use std::io::Read as _;
use std::path::PathBuf;

use clap::ValueEnum;
use semantic_data::query::Query;
use semantic_data::value::serde::typed::{TypedRef, TypedValue};
use semantic_data::value::{FromValue, IntoValue};
use semantic_data::value::{Object, Value};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, OutputArgs};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum QueryFormat {
    /// A query AST encoded as typed Semantic Value JSON.
    Ast,
    Sql,
    Prql,
}

impl QueryFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ast => "ast",
            Self::Sql => "sql",
            Self::Prql => "prql",
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct Args {
    #[command(flatten)]
    pub source: QuerySource,

    /// Query format; AST input uses typed Semantic Value JSON.
    #[arg(long, value_enum, default_value = "sql")]
    pub format: QueryFormat,

    /// Parameter bindings as a JSON object, e.g. '{"id":"item-1"}'.
    #[arg(long, value_name = "JSON")]
    pub params: Option<String>,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

#[derive(Debug, clap::Args)]
pub struct QuerySource {
    /// Query input; when omitted, read UTF-8 from standard input.
    #[arg(value_name = "QUERY", conflicts_with = "file")]
    pub query: Option<String>,

    /// Read the query as UTF-8 from this file instead of QUERY or standard input.
    #[arg(long, short = 'f', value_name = "PATH")]
    pub file: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct ParseSqlArgs {
    #[command(flatten)]
    pub source: QuerySource,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let query = read_query(&args.source)?;
    let mut payload = query_payload(&query, args.format, args.params.as_deref())?;
    args.client.insert_scope(&mut payload);
    let response = args
        .client
        .rpc_client()
        .invoke_value("semantic.db.query", Value::Object(payload))
        .await?;
    args.output.print(&response)
}

pub(super) fn query_payload(
    query: &str,
    format: QueryFormat,
    params: Option<&str>,
) -> Result<Object, CliError> {
    let mut payload = Object::new();
    if matches!(format, QueryFormat::Ast) {
        let ast: TypedValue = serde_json::from_str(query).map_err(|source| CliError::Json {
            source_name: "query AST".into(),
            source,
        })?;
        let ast =
            Query::from_value(ast.0).map_err(|error| CliError::InvalidInput(error.to_string()))?;
        payload.insert("query", ast.into_value());
    } else {
        payload.insert("query", Value::String(query.into()));
        payload.insert("format", Value::String(format.as_str().into()));
    }
    if let Some(params) = params {
        let params: Value = serde_json::from_str(params).map_err(|source| CliError::Json {
            source_name: "parameter bindings".into(),
            source,
        })?;
        if !matches!(params, Value::Object(_)) {
            return Err(CliError::InvalidInput(
                "parameter bindings must be a JSON object".into(),
            ));
        }
        payload.insert("params", params);
    }
    Ok(payload)
}

/// Emit the same lossless encoding consumed by `query --format ast`.
pub async fn parse_sql(args: ParseSqlArgs) -> Result<(), CliError> {
    let mut payload = Object::new();
    payload.insert("query", read_query(&args.source)?);
    args.client.insert_scope(&mut payload);
    let response = args
        .client
        .rpc_client()
        .invoke_value("semantic.db.query.parse_sql", Value::Object(payload))
        .await?;
    let output = encode_ast(&response, args.output.pretty)?;
    println!("{output}");
    Ok(())
}

fn encode_ast(ast: &Value, pretty: bool) -> Result<String, CliError> {
    let result = if pretty {
        serde_json::to_string_pretty(&TypedRef(ast))
    } else {
        serde_json::to_string(&TypedRef(ast))
    };
    result.map_err(|source| CliError::Json {
        source_name: "query AST response".into(),
        source,
    })
}

pub(super) fn read_query(args: &QuerySource) -> std::result::Result<String, CliError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::query::{Expr, Operand, QueryField, SelectQuery};

    #[test]
    fn typed_ast_output_can_be_submitted_without_losing_literal_types() {
        let mut select = SelectQuery::new().with_collection("items");
        select.projection = vec![QueryField {
            expr: Box::new(Expr::Operand(Operand::Literal(Value::I128(i128::MAX)))),
            alias: Some("value".into()),
            wildcard: None,
        }];
        let ast = Query::Select(select).into_value();
        for pretty in [false, true] {
            let encoded = encode_ast(&ast, pretty).unwrap();
            let payload =
                query_payload(&encoded, QueryFormat::Ast, Some(r#"{"id":"item-1"}"#)).unwrap();
            assert_eq!(payload.get("query"), Some(&ast));
            assert!(payload.get("format").is_none());
            assert!(matches!(payload.get("params"), Some(Value::Object(_))));
        }
    }

    #[test]
    fn text_queries_keep_format_and_validate_bindings() {
        let payload = query_payload("from items", QueryFormat::Prql, None).unwrap();
        assert_eq!(
            payload.get("query").and_then(Value::as_str),
            Some("from items")
        );
        assert_eq!(payload.get("format").and_then(Value::as_str), Some("prql"));
        assert!(query_payload("SELECT * FROM items", QueryFormat::Sql, Some("[]")).is_err());
    }
}
