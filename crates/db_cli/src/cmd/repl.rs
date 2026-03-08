use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;

use clap::Args;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use semantic_data::query::TextQueryFormat;

use crate::cmd::query::{OutputFormat, QueryFormat, execute_text_or_explain};
use crate::{CliError, CommonArgs, open_db};

#[derive(Debug, Clone, Args)]
pub struct ReplArgs {
    #[clap(flatten)]
    pub common: CommonArgs,
    #[arg(long = "query-format", value_enum, default_value_t = QueryFormat::Sql)]
    pub query_format: QueryFormat,
    #[arg(long, value_enum, default_value_t = OutputFormat::JsonLinesUntyped)]
    pub format: OutputFormat,
}

pub async fn run(args: ReplArgs) -> std::result::Result<(), CliError> {
    let db = open_db(&args.common.db_uri)?;
    let mut query_format = args.query_format;

    if !io::stdin().is_terminal() {
        return run_non_interactive(&db, &mut query_format, args.format).await;
    }

    let mut editor = DefaultEditor::new()
        .map_err(|err| CliError::Message(format!("failed to initialize REPL editor: {err}")))?;
    let history_path = Some(history_path()?);

    // Missing history is expected on first run.
    if let Some(path) = history_path.as_deref()
        && path.exists()
        && let Err(err) = editor.load_history(path)
    {
        tracing::debug!(
            "failed to load REPL history from '{}': {err}",
            path.display()
        );
    }

    loop {
        match editor.readline("db> ") {
            Ok(line) => {
                let query = line.trim();
                if query.is_empty() {
                    continue;
                }
                if matches!(query, "exit" | "quit" | ":q" | ":quit") {
                    break;
                }
                if let Some(message) = handle_syntax_command(query, &mut query_format)? {
                    println!("{message}");
                    continue;
                }

                let _ = editor.add_history_entry(query);

                match execute_text_or_explain(
                    &db,
                    query_format.into(),
                    args.format,
                    query.to_string(),
                )
                .await
                {
                    Ok(json) => {
                        println!("{json}");
                    }
                    Err(err) => {
                        eprintln!("{err}");
                    }
                }
            }
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(err) => {
                return Err(CliError::Message(format!("failed to read input: {err}")));
            }
        }
    }

    if let Some(path) = &history_path
        && let Err(err) = editor.save_history(path)
    {
        tracing::debug!("failed to save REPL history to '{}': {err}", path.display());
    }

    Ok(())
}

async fn run_non_interactive(
    db: &semantic_db_core::Db,
    query_format: &mut QueryFormat,
    output_format: OutputFormat,
) -> std::result::Result<(), CliError> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut stdout = io::stdout();

    loop {
        write!(stdout, "db> ")?;
        stdout.flush()?;

        let Some(line) = lines.next() else {
            break;
        };
        let line = line?;
        let query = line.trim();
        if query.is_empty() {
            continue;
        }
        if matches!(query, "exit" | "quit" | ":q" | ":quit") {
            break;
        }
        if let Some(message) = handle_syntax_command(query, query_format)? {
            println!("{message}");
            continue;
        }

        match execute_text_or_explain(db, (*query_format).into(), output_format, query.to_string())
            .await
        {
            Ok(json) => {
                println!("{json}");
            }
            Err(err) => {
                eprintln!("{err}");
            }
        }
    }

    Ok(())
}

fn history_path() -> Result<PathBuf, std::io::Error> {
    let home = std::env::home_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "home directory not found")
    })?;
    let semdir = home.join(".local/share/semantic");

    std::fs::create_dir_all(&semdir)?;

    Ok(semdir.join("repl_history"))
}

fn handle_syntax_command(
    query: &str,
    query_format: &mut QueryFormat,
) -> std::result::Result<Option<String>, CliError> {
    if !query.starts_with("/syntax") {
        return Ok(None);
    }

    let mut parts = query.split_whitespace();
    let command = parts.next().unwrap_or_default();
    if command != "/syntax" {
        return Ok(None);
    }

    let Some(requested) = parts.next() else {
        return Ok(Some(format_syntax_help(*query_format)));
    };
    if parts.next().is_some() {
        return Err(CliError::Message("usage: /syntax [prql|sql]".to_string()));
    }

    let next = match requested.to_ascii_lowercase().as_str() {
        "sql" => QueryFormat::Sql,
        "prql" => QueryFormat::Prql,
        _ => {
            return Err(CliError::Message("usage: /syntax [prql|sql]".to_string()));
        }
    };
    *query_format = next;

    Ok(Some(format!(
        "syntax set to {}",
        format_query_format(*query_format)
    )))
}

fn format_query_format(query_format: QueryFormat) -> &'static str {
    match TextQueryFormat::from(query_format) {
        TextQueryFormat::Sql => "sql",
        TextQueryFormat::Prql => "prql",
    }
}

fn format_syntax_help(query_format: QueryFormat) -> String {
    let current = format_query_format(query_format);
    let other = match query_format {
        QueryFormat::Sql => "prql",
        QueryFormat::Prql => "sql",
    };
    format!("current syntax: {current} (available: {current}, {other})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_command_reports_current_value() {
        let mut query_format = QueryFormat::Sql;
        let response = handle_syntax_command("/syntax", &mut query_format).unwrap();
        assert_eq!(
            response.as_deref(),
            Some("current syntax: sql (available: sql, prql)")
        );
        assert_eq!(query_format, QueryFormat::Sql);
    }

    #[test]
    fn syntax_command_switches_value() {
        let mut query_format = QueryFormat::Sql;
        let response = handle_syntax_command("/syntax prql", &mut query_format).unwrap();
        assert_eq!(response.as_deref(), Some("syntax set to prql"));
        assert_eq!(query_format, QueryFormat::Prql);
    }

    #[test]
    fn syntax_command_is_ignored_for_other_commands() {
        let mut query_format = QueryFormat::Sql;
        let response = handle_syntax_command("/syntaxx", &mut query_format).unwrap();
        assert_eq!(response, None);
        assert_eq!(query_format, QueryFormat::Sql);
    }

    #[test]
    fn syntax_command_errors_for_invalid_value() {
        let mut query_format = QueryFormat::Sql;
        let error = handle_syntax_command("/syntax foo", &mut query_format).unwrap_err();
        assert_eq!(error.to_string(), "usage: /syntax [prql|sql]".to_string());
        assert_eq!(query_format, QueryFormat::Sql);
    }
}
