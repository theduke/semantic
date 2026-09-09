mod cmd;

use clap::{Args, Parser, Subcommand};
use semantic_data::schema::DbOpenMode;
use semantic_db_core::Db;
use thiserror::Error;

pub use cmd::query::QueryFormat;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] semantic_db_core::DbError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
}

#[derive(Debug, Clone, Args)]
pub struct CommonArgs {
    #[arg(long, value_name = "DB_URI", env = "DB_URI")]
    pub db_uri: String,
}

#[derive(Debug, Parser)]
#[command(name = "db_cli")]
#[command(about = "Database command line interface")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Query(cmd::query::QueryArgs),
    Delete(cmd::delete::DeleteArgs),
    Insert(cmd::insert::InsertArgs),
    Repl(cmd::repl::ReplArgs),
}

pub async fn run() -> std::result::Result<(), CliError> {
    let cli = Cli::parse();
    match cli.command {
        Command::Query(args) => cmd::query::run(args).await,
        Command::Delete(args) => cmd::delete::run(args).await,
        Command::Insert(args) => cmd::insert::run(args).await,
        Command::Repl(args) => cmd::repl::run(args).await,
    }
}

pub(crate) fn open_db(db_uri: &str) -> std::result::Result<Db, CliError> {
    let (scheme, rest) = db_uri
        .split_once(':')
        .ok_or_else(|| CliError::Message(format!("invalid db uri '{db_uri}'")))?;

    if rest.starts_with("//") {
        return Err(CliError::Message(format!(
            "use '{scheme}:PATH' for a local database URI"
        )));
    }

    match scheme {
        "redb" => {
            if rest.is_empty() {
                return Err(CliError::Message(format!(
                    "invalid redb uri '{db_uri}': missing database path"
                )));
            }
            let backend = semantic_db_redb::open_backend(rest, DbOpenMode::AutoCreate)?;
            Ok(Db::new(backend))
        }
        "logfs" => {
            if rest.is_empty() {
                return Err(CliError::Message(format!(
                    "invalid logfs uri '{db_uri}': missing database path"
                )));
            }
            let backend = semantic_db_log::open_backend(rest, DbOpenMode::AutoCreate)?;
            Ok(Db::new(backend))
        }
        _ => Err(CliError::Message(format!(
            "unsupported db uri scheme '{scheme}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn opens_logfs_uri() {
        let dir = tempfile::tempdir().unwrap();
        let uri = format!("logfs:{}", dir.path().join("database.log").display());
        super::open_db(&uri).unwrap();
    }

    #[test]
    fn rejects_empty_logfs_path() {
        assert!(super::open_db("logfs:").is_err());
    }

    #[test]
    fn rejects_database_url_syntax() {
        for uri in ["redb://relative.db", "logfs:///absolute.log"] {
            let error = super::open_db(uri).err().expect("invalid database URI");
            assert!(error.to_string().contains(":PATH"));
        }
    }
}
