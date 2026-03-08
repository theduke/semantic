use clap::{Args, ValueEnum};
use semantic_db_core::{Db, TextQueryFormat};

use crate::{CliError, CommonArgs, open_db};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum QueryFormat {
    Sql,
    Prql,
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
    #[arg(long, value_enum, default_value_t = QueryFormat::Sql)]
    pub format: QueryFormat,
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
    format: TextQueryFormat,
    query: String,
) -> std::result::Result<String, CliError> {
    let json = match parse_query_action(query)? {
        QueryAction::Execute(query) => {
            let result = db.query_text(format, query).await?;
            facet_json::to_string(&result).map_err(|err| CliError::Message(err.to_string()))?
        }
        QueryAction::Explain(query) => {
            let plan = db.plan_query_text(format, query).await?;
            facet_json::to_string(&plan).map_err(|err| CliError::Message(err.to_string()))?
        }
    };

    Ok(json)
}

pub async fn run(args: QueryArgs) -> std::result::Result<(), CliError> {
    let db = open_db(&args.common.db_uri)?;
    let json = execute_text_or_explain(&db, args.format.into(), args.query).await?;
    println!("{json}");
    Ok(())
}
