use std::path::PathBuf;

use clap::{Args, Subcommand};
use semantic_db_core::{
    DEFAULT_REWRITE_BATCH_SIZE, Db, ReindexTarget, VerifyOptions, format_storage_stats,
};

use crate::{CliError, CommonArgs, open_db};

#[derive(Debug, Clone, Args)]
pub struct MaintenanceArgs {
    #[clap(flatten)]
    pub common: CommonArgs,
    #[command(subcommand)]
    pub command: MaintenanceCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum MaintenanceCommand {
    /// Rebuild index entries from the stored rows (all indexes by default).
    Reindex {
        /// Only rebuild the indexes of this collection.
        #[arg(long)]
        collection: Option<String>,
        /// Only rebuild this index of `--collection`.
        #[arg(long, requires = "collection")]
        index: Option<String>,
    },
    /// Check indexes, derived data, counters and payloads without writing.
    /// Exits with an error when problems are found.
    Verify(VerifyArgs),
    /// Verify, rebuild what the problems call for, and verify again.
    Repair(VerifyArgs),
    /// Compact the storage file (redb). Fails while other readers are open.
    Compact,
    /// Print physical storage statistics.
    Stats,
    /// Rewrite stored payloads in the current format.
    RewritePayloads {
        /// Rows rewritten per write transaction.
        #[arg(long, default_value_t = DEFAULT_REWRITE_BATCH_SIZE, value_parser = positive_usize)]
        batch_size: usize,
    },
    /// Write a consistent copy of the database to a new file (redb).
    Backup {
        /// Path of the backup; must not exist.
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Args)]
pub struct VerifyArgs {
    /// Skip the index entry checks.
    #[arg(long)]
    pub skip_indexes: bool,
    /// Skip the reverse reference checks.
    #[arg(long)]
    pub skip_reverse_references: bool,
    /// Skip the relationship contributor, count and edge checks.
    #[arg(long)]
    pub skip_relationship_edges: bool,
    /// Skip the row and index entry counter checks.
    #[arg(long)]
    pub skip_stats: bool,
    /// Skip the physical storage integrity check.
    #[arg(long)]
    pub skip_storage_integrity: bool,
    /// Skip decoding every payload.
    #[arg(long)]
    pub skip_payloads: bool,
    /// Maximum number of listed problems.
    #[arg(long, default_value_t = VerifyOptions::DEFAULT_MAX_PROBLEMS)]
    pub max_problems: usize,
}

impl VerifyArgs {
    pub fn options(&self) -> VerifyOptions {
        VerifyOptions {
            check_indexes: !self.skip_indexes,
            check_reverse_references: !self.skip_reverse_references,
            check_relationship_edges: !self.skip_relationship_edges,
            check_stats: !self.skip_stats,
            check_storage_integrity: !self.skip_storage_integrity,
            check_payloads: !self.skip_payloads,
            max_problems: self.max_problems,
        }
    }
}

pub async fn run(args: MaintenanceArgs) -> std::result::Result<(), CliError> {
    let db = open_db(&args.common.db_uri)?;
    println!("{}", execute(&db, args.command).await?);
    Ok(())
}

/// Run `command` and render its report.
pub(crate) async fn execute(
    db: &Db,
    command: MaintenanceCommand,
) -> std::result::Result<String, CliError> {
    Ok(match command {
        MaintenanceCommand::Reindex { collection, index } => {
            let target = match (collection, index) {
                (Some(collection), Some(name)) => ReindexTarget::Index { collection, name },
                (Some(collection), None) => ReindexTarget::Collection(collection),
                _ => ReindexTarget::All,
            };
            db.reindex(target).await?.to_string()
        }
        MaintenanceCommand::Verify(args) => {
            let report = db.verify(args.options()).await?;
            if !report.is_ok() {
                return Err(CliError::Message(report.to_string()));
            }
            report.to_string()
        }
        MaintenanceCommand::Repair(args) => {
            let report = db.repair(args.options()).await?;
            if !report.after.is_ok() {
                return Err(CliError::Message(report.to_string()));
            }
            report.to_string()
        }
        MaintenanceCommand::Compact => db.compact_storage().await?.to_string(),
        MaintenanceCommand::Stats => format_storage_stats(&db.storage_stats().await?),
        MaintenanceCommand::RewritePayloads { batch_size } => {
            db.rewrite_payloads(batch_size).await?.to_string()
        }
        MaintenanceCommand::Backup { path } => db.backup(path).await?.to_string(),
    })
}

fn positive_usize(value: &str) -> std::result::Result<usize, String> {
    match value.parse::<usize>() {
        Ok(0) => Err("value must be greater than zero".into()),
        Ok(value) => Ok(value),
        Err(error) => Err(format!("invalid positive integer: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;
    use crate::{Cli, Command};

    fn parse(args: &[&str]) -> MaintenanceArgs {
        let cli = Cli::try_parse_from(
            ["db_cli", "maintenance", "--db-uri", "redb:db.redb"]
                .iter()
                .chain(args),
        )
        .unwrap();
        match cli.command {
            Command::Maintenance(args) => args,
            other => panic!("maintenance command expected, got {other:?}"),
        }
    }

    #[test]
    fn parses_maintenance_commands() {
        assert!(matches!(
            parse(&["reindex"]).command,
            MaintenanceCommand::Reindex {
                collection: None,
                index: None
            }
        ));
        assert!(matches!(
            parse(&["reindex", "--collection", "notes", "--index", "by_title"]).command,
            MaintenanceCommand::Reindex { collection: Some(c), index: Some(i) }
                if c == "notes" && i == "by_title"
        ));
        let MaintenanceCommand::Verify(verify) =
            parse(&["verify", "--skip-payloads", "--max-problems", "5"]).command
        else {
            panic!("verify expected")
        };
        let options = verify.options();
        assert!(!options.check_payloads && options.check_indexes);
        assert_eq!(options.max_problems, 5);
        assert!(matches!(
            parse(&["rewrite-payloads", "--batch-size", "10"]).command,
            MaintenanceCommand::RewritePayloads { batch_size: 10 }
        ));
        assert!(matches!(
            parse(&["backup", "copy.redb"]).command,
            MaintenanceCommand::Backup { path } if path == PathBuf::from("copy.redb")
        ));
        for command in ["repair", "compact", "stats"] {
            parse(&[command]);
        }
    }

    #[test]
    fn rejects_invalid_arguments() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(
                ["db_cli", "maintenance", "--db-uri", "redb:db.redb"]
                    .iter()
                    .chain(args),
            )
        };
        assert!(parse(&["reindex", "--index", "by_title"]).is_err());
        assert!(parse(&["rewrite-payloads", "--batch-size", "0"]).is_err());
        assert!(parse(&["backup"]).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runs_maintenance_on_a_redb_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = open_db(&format!("redb:{}", dir.path().join("db.redb").display())).unwrap();
        let verify = VerifyArgs {
            skip_indexes: false,
            skip_reverse_references: false,
            skip_relationship_edges: false,
            skip_stats: false,
            skip_storage_integrity: false,
            skip_payloads: false,
            max_problems: 10,
        };
        let output = execute(&db, MaintenanceCommand::Verify(verify.clone()))
            .await
            .unwrap();
        assert!(output.ends_with("0 problems"), "{output}");
        let output = execute(
            &db,
            MaintenanceCommand::Reindex {
                collection: None,
                index: None,
            },
        )
        .await
        .unwrap();
        assert!(output.contains("rebuilt"), "{output}");
        execute(&db, MaintenanceCommand::Repair(verify))
            .await
            .unwrap();
        let output = execute(&db, MaintenanceCommand::Stats).await.unwrap();
        assert!(output.starts_with("file size:"), "{output}");
        execute(&db, MaintenanceCommand::Compact).await.unwrap();
        execute(&db, MaintenanceCommand::RewritePayloads { batch_size: 100 })
            .await
            .unwrap();
        let backup = dir.path().join("copy.redb");
        let output = execute(
            &db,
            MaintenanceCommand::Backup {
                path: backup.clone(),
            },
        )
        .await
        .unwrap();
        assert!(output.starts_with("backed up revision"), "{output}");
        assert!(backup.exists());
    }
}
