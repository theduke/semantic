use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, Subcommand};
use semantic_app::transfer::{
    DEFAULT_BATCH_SIZE, DEFAULT_MAX_RECORD_BYTES, ExportOptions, ImportFormat, ImportOptions,
    TransferFormat,
};
use semantic_app::{AppRequestContext, Principal};
use semantic_data::schema::DbOpenMode;

use crate::CliError;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Database URI: redb:PATH, logfs:PATH, or log:<blob>.
    #[arg(long, env = "SEMANTIC_DB_URI", value_name = "URI")]
    pub db_uri: Option<String>,

    /// Blob-store URI used by full tar transfers.
    #[arg(long, env = "SEMANTIC_BLOB_URI", value_name = "URI")]
    pub blob_uri: Option<String>,

    /// Base directory for default local database and blob storage.
    #[arg(long, env = "SEMANTIC_DATA_DIR", value_name = "PATH")]
    pub data_dir: Option<PathBuf>,

    /// Directory used for transfer staging and other temporary files.
    #[arg(long, env = "SEMANTIC_TEMP_DIR", value_name = "PATH")]
    pub temp_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: SubCmd,
}

#[derive(Debug, Subcommand)]
pub enum SubCmd {
    /// Export portable entities as JSONL, or entities and referenced blobs as tar.
    Export(ExportArgs),
    /// Upsert a JSONL or tar transfer into the local database.
    Import(ImportArgs),
}

#[derive(Debug, ClapArgs)]
pub struct ExportArgs {
    /// Output path, or '-' for stdout. Existing files are never overwritten.
    pub output: PathBuf,

    /// Transfer format (default: jsonl).
    #[arg(long)]
    pub format: Option<TransferFormat>,

    /// Alias for '--format tar'.
    #[arg(long)]
    pub full: bool,
}

#[derive(Debug, ClapArgs)]
pub struct ImportArgs {
    /// Input path, or '-' for stdin.
    pub input: PathBuf,

    /// Input format. Auto detects JSONL versus tar from the content.
    #[arg(long, default_value = "auto")]
    pub format: ImportFormat,

    /// Number of entity upserts committed per batch.
    #[arg(long, default_value_t = DEFAULT_BATCH_SIZE, value_parser = positive_usize)]
    pub batch_size: usize,

    /// Validate referenced entities during each batch (disabled by default).
    #[arg(long)]
    pub validate_foreign_keys: bool,

    /// Maximum number of bytes accepted for one JSONL record.
    #[arg(long, default_value_t = DEFAULT_MAX_RECORD_BYTES, value_parser = positive_usize)]
    pub max_record_bytes: usize,
}

pub async fn run(args: Args) -> Result<(), CliError> {
    if matches!(
        &args.command,
        SubCmd::Export(ExportArgs {
            full: true,
            format: Some(TransferFormat::Jsonl),
            ..
        })
    ) {
        return Err(CliError::InvalidInput(
            "--full conflicts with '--format jsonl'".into(),
        ));
    }
    let mode = match &args.command {
        SubCmd::Export(_) => DbOpenMode::OpenExisting,
        SubCmd::Import(_) => DbOpenMode::AutoCreate,
    };
    let mut app_config = semantic_app::AppConfig::from_env();
    if let Some(data_dir) = &args.data_dir {
        app_config.data_dir = Some(data_dir.clone());
    }
    if let Some(temp_dir) = &args.temp_dir {
        app_config.temp_dir = Some(temp_dir.clone());
    }
    let storage = semantic_app::storage::resolve_storage(
        &app_config,
        semantic_app::storage::StorageConfig {
            db_uri: args.db_uri,
            blob_uri: args.blob_uri,
            mode,
            ..Default::default()
        },
    )?;
    if storage.db_uri_selection() == semantic_app::storage::DbUriSelection::SharedBlob {
        eprintln!(
            "No database URI specified; selected 'log:<blob>' because the blob store uses logfs."
        );
    }
    let blob_password = semantic_server::prompt_blob_password(storage.blob_uri())
        .map_err(|error| CliError::InvalidInput(format!("read logfs password: {error}")))?;
    let app =
        semantic_app::storage::open_app(app_config, storage.with_blob_password(blob_password))
            .await?;
    let context = AppRequestContext {
        app,
        principal: Principal::system(),
        session: None,
        request_scope: None,
    };
    let db = context.resolve_db(None).await?;

    match args.command {
        SubCmd::Export(export_args) => {
            let format = if export_args.full {
                TransferFormat::Tar
            } else {
                export_args.format.unwrap_or_default()
            };
            let blob_store = if format == TransferFormat::Tar {
                Some(context.default_file_store(None).await?)
            } else {
                None
            };
            let options = ExportOptions {
                format,
                temp_dir: args.temp_dir,
            };
            let stats = if is_stdio(&export_args.output) {
                let mut stdout = tokio::io::stdout();
                semantic_app::transfer::export(db.as_ref(), blob_store, &mut stdout, &options)
                    .await?
            } else {
                export_path(db.as_ref(), blob_store, &export_args.output, &options).await?
            };
            eprintln!(
                "exported {} entities, {} blobs ({} bytes)",
                stats.entities, stats.blobs, stats.blob_bytes
            );
        }
        SubCmd::Import(import_args) => {
            let options = ImportOptions {
                format: import_args.format,
                batch_size: import_args.batch_size,
                write_settings: semantic_db_core::WriteSettings {
                    validate_foreign_keys: import_args.validate_foreign_keys,
                },
                max_record_bytes: import_args.max_record_bytes,
                temp_dir: args.temp_dir,
            };
            let blob_store = if import_args.format == ImportFormat::Jsonl {
                None
            } else {
                Some(context.default_file_store(None).await?)
            };
            let stats = if is_stdio(&import_args.input) {
                let mut stdin = tokio::io::stdin();
                semantic_app::transfer::import(
                    db.as_ref(),
                    blob_store,
                    &mut stdin,
                    "stdin",
                    &options,
                )
                .await?
            } else {
                let mut input =
                    tokio::fs::File::open(&import_args.input)
                        .await
                        .map_err(|source| CliError::Io {
                            action: "open",
                            path: import_args.input.display().to_string(),
                            source,
                        })?;
                semantic_app::transfer::import(
                    db.as_ref(),
                    blob_store,
                    &mut input,
                    &import_args.input.display().to_string(),
                    &options,
                )
                .await?
            };
            eprintln!(
                "imported {} entities in {} batches; restored {} blobs ({} bytes)",
                stats.entities, stats.batches, stats.blobs, stats.blob_bytes
            );
        }
    }
    context.app.shutdown().await?;
    Ok(())
}

async fn export_path(
    db: &dyn semantic_app::SemanticDb,
    blob_store: Option<objstore::DynObjStore>,
    output: &Path,
    options: &ExportOptions,
) -> Result<semantic_app::transfer::TransferStats, CliError> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".semantic-export-")
        .tempfile_in(parent)
        .map_err(|source| CliError::Io {
            action: "create temporary output in",
            path: parent.display().to_string(),
            source,
        })?;
    let std_file = temporary.reopen().map_err(|source| CliError::Io {
        action: "open temporary output in",
        path: parent.display().to_string(),
        source,
    })?;
    let mut file = tokio::fs::File::from_std(std_file);
    let stats = semantic_app::transfer::export(db, blob_store, &mut file, options).await?;
    file.sync_all().await.map_err(|source| CliError::Io {
        action: "sync",
        path: output.display().to_string(),
        source,
    })?;
    drop(file);
    temporary
        .persist_noclobber(output)
        .map_err(|error| CliError::Io {
            action: "publish without overwriting",
            path: output.display().to_string(),
            source: error.error,
        })?;
    Ok(stats)
}

fn is_stdio(path: &Path) -> bool {
    path.as_os_str() == "-"
}

fn positive_usize(value: &str) -> Result<usize, String> {
    let value = value
        .parse::<usize>()
        .map_err(|error| format!("invalid positive integer: {error}"))?;
    if value == 0 {
        return Err("value must be greater than zero".into());
    }
    Ok(value)
}
