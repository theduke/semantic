use clap::{Args, ValueEnum};
use semantic_data::query::{QueryInput, TextQueryFormat};
use semantic_data::value::serde::FlatValueRef;
use semantic_data::value::{Object, ValueRef};
use semantic_db_core::Db;
use serde::Serialize;

use crate::{CliError, CommonArgs, open_db};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum QueryFormat {
    Sql,
    Prql,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    JsonTyped,
    JsonLinesUntyped,
}

impl From<QueryFormat> for TextQueryFormat {
    fn from(value: QueryFormat) -> Self {
        match value {
            QueryFormat::Sql => TextQueryFormat::Sql,
            QueryFormat::Prql => TextQueryFormat::Prql,
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct QueryArgs {
    #[clap(flatten)]
    pub common: CommonArgs,
    #[arg(long = "query-format", short = 'l', value_enum, default_value_t = QueryFormat::Sql)]
    pub query_format: QueryFormat,
    #[arg(long, value_enum, default_value_t = OutputFormat::JsonLinesUntyped)]
    pub format: OutputFormat,
    #[arg(value_name = "QUERY")]
    pub query: String,
}

enum QueryAction {
    Execute(String),
    Explain(String),
}

fn parse_query_action(query: String) -> std::result::Result<QueryAction, CliError> {
    let trimmed = query.trim_start();
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or_default();
    if first.eq_ignore_ascii_case("EXPLAIN") {
        let explain_query = parts.next().unwrap_or_default().trim();
        if explain_query.is_empty() {
            return Err(CliError::Message(
                "EXPLAIN requires a query to explain".to_string(),
            ));
        }
        return Ok(QueryAction::Explain(explain_query.to_string()));
    }

    Ok(QueryAction::Execute(query))
}

pub(crate) async fn execute_text_or_explain(
    db: &Db,
    query_format: TextQueryFormat,
    output_format: OutputFormat,
    query: String,
) -> std::result::Result<String, CliError> {
    let output = match parse_query_action(query)? {
        QueryAction::Execute(query) => {
            let result = db
                .query(QueryInput::Text {
                    format: query_format,
                    query,
                })
                .await?;
            format_query_result(&result, output_format)?
        }
        QueryAction::Explain(query) => {
            let plan = db
                .plan(QueryInput::Text {
                    format: query_format,
                    query,
                })
                .await?;
            format_plan(&plan, output_format)?
        }
    };

    Ok(output)
}

pub async fn run(args: QueryArgs) -> std::result::Result<(), CliError> {
    let db = open_db(&args.common.db_uri)?;
    let json =
        execute_text_or_explain(&db, args.query_format.into(), args.format, args.query).await?;
    println!("{json}");
    Ok(())
}

fn format_query_result(
    result: &semantic_db_core::QueryResult,
    format: OutputFormat,
) -> std::result::Result<String, CliError> {
    match format {
        OutputFormat::JsonTyped => {
            facet_json::to_string(result).map_err(|err| CliError::Message(err.to_string()))
        }
        OutputFormat::JsonLinesUntyped => format_query_result_json_lines_untyped(result),
    }
}

fn format_plan(
    plan: &semantic_db_core::QueryPlan,
    format: OutputFormat,
) -> std::result::Result<String, CliError> {
    match format {
        OutputFormat::JsonTyped | OutputFormat::JsonLinesUntyped => {
            facet_json::to_string(plan).map_err(|err| CliError::Message(err.to_string()))
        }
    }
}

fn format_query_result_json_lines_untyped(
    result: &semantic_db_core::QueryResult,
) -> std::result::Result<String, CliError> {
    match result {
        semantic_db_core::QueryResult::Select(rows) => {
            let mut out = String::new();
            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&serde_json::to_string(&UntypedObjectRef(row))?);
            }
            Ok(out)
        }
        _ => facet_json::to_string(result).map_err(|err| CliError::Message(err.to_string())),
    }
}

#[derive(Debug, Clone, Copy)]
struct UntypedObjectRef<'a>(&'a Object);

impl Serialize for UntypedObjectRef<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (field, value) in self.0.iter() {
            map.serialize_entry(field, &FlatValueRef(ValueRef::Ref(value)))?;
        }
        map.end()
    }
}
