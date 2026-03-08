use std::io::{self, BufRead, Write};

use clap::Args;

use crate::cmd::query::{QueryFormat, execute_text_or_explain};
use crate::{CliError, CommonArgs, open_db};

#[derive(Debug, Clone, Args)]
pub struct ReplArgs {
    #[clap(flatten)]
    pub common: CommonArgs,
    #[arg(long, value_enum, default_value_t = QueryFormat::Sql)]
    pub format: QueryFormat,
}

pub async fn run(args: ReplArgs) -> std::result::Result<(), CliError> {
    let db = open_db(&args.common.db_uri)?;
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

        match execute_text_or_explain(&db, args.format.into(), query.to_string()).await {
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
