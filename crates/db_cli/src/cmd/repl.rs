use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;

use clap::Args;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

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

    if !io::stdin().is_terminal() {
        return run_non_interactive(&db, args.query_format, args.format).await;
    }

    let mut editor = DefaultEditor::new()
        .map_err(|err| CliError::Message(format!("failed to initialize REPL editor: {err}")))?;
    let history_path = history_path();
    if let Some(path) = history_path.as_deref() {
        // Missing history is expected on first run.
        let _ = editor.load_history(path);
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

                let _ = editor.add_history_entry(query);

                match execute_text_or_explain(
                    &db,
                    args.query_format.into(),
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

    if let Some(path) = history_path.as_deref() {
        let _ = editor.save_history(path);
    }

    Ok(())
}

async fn run_non_interactive(
    db: &semantic_db_core::Db,
    query_format: QueryFormat,
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

        match execute_text_or_explain(db, query_format.into(), output_format, query.to_string())
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

fn history_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        let mut path = PathBuf::from(home);
        path.push(".semantic_db_cli_history");
        path
    })
}
