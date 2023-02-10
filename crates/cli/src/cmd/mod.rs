use std::path::PathBuf;

use anyhow::{anyhow, Context};

use semantic::app::{self, AppConfig};
use semantic_core::api::{self, DbConfig};

pub mod archive;
pub mod client;
pub mod db;
pub mod generate_typescript;
pub mod import_files;
pub mod serve;
pub mod upload;

/// Semantic CLI
#[derive(clap::Parser)]
pub struct Args {
    #[command(subcommand)]
    command: SubCmd,
}

impl Args {
    pub fn run(self) -> Result<(), anyhow::Error> {
        match self.command {
            SubCmd::Serve(cmd) => CliCommand::run(cmd),
            SubCmd::GenerateTypescript(cmd) => cmd.run(),
            SubCmd::Archive(cmd) => cmd.run(),
            SubCmd::ImportFiles(args) => CliCommand::run(args),
            SubCmd::Upload(cmd) => CliCommand::run(cmd),
            SubCmd::Db(cmd) => cmd.run(),
            SubCmd::Client(cmd) => cmd.run(),
        }
    }
}

#[derive(clap::Subcommand)]
enum SubCmd {
    Serve(serve::CmdServe),
    /// Database related commands.
    #[command(subcommand)]
    Db(db::CmdDb),
    #[command(subcommand)]
    Client(client::CmdClient),
    ImportFiles(import_files::CmdImportFiles),
    GenerateTypescript(generate_typescript::CmdGenerateTypescript),
    #[command(subcommand)]
    Archive(archive::CmdArchive),
    Upload(upload::CmdUpload),
}

#[derive(clap::Parser, Clone)]
struct BackendOptions {
    /// Path for the database.
    #[arg(short = 'p', long, env = "SEMANTIC_DATA_PATH")]
    data_path: Option<String>,
    #[arg(long, env = "SEMANTIC_READONLY")]
    readonly: bool,
    /// Fore creation of a new database.
    /// Will fail if the database already exists.
    #[arg(long)]
    create: bool,
    /// The password.
    // TODO: use anonymizing wrapper?
    #[arg(long, short, env = "SEMANTIC_KEY")]
    key: Option<String>,
    /// Number of key iterations used for encryption key derivation.
    /// This can normally remain unchanged.
    #[arg(long, env = "SEMANTIC_KEY_ITERATIONS")]
    key_iterations: Option<u32>,
    /// Use a custom encryption salt.
    // TODO: use anonymizing wrapper?
    #[arg(long, env = "SEMANTIC_SALT")]
    salt: Option<String>,
    /// Binary offset in the storage file.
    /// Either a number of bytes, or a parsable pretty byte number like "300mb".
    #[arg(long)]
    offset: Option<String>,

    /// Use blobfs storage.
    /// (default is logfs)
    #[arg(long)]
    blobfs: bool,

    /// The number of key changes after which a full key index is written.
    #[arg(long)]
    full_index_write_interval: Option<u64>,
}

impl BackendOptions {
    fn build_backend_config(&self) -> Result<api::BackendConfig, anyhow::Error> {
        let offset = if let Some(off) = &self.offset {
            let size = off
                .parse::<bytesize::ByteSize>()
                .map_err(|err| anyhow!("Invalid offset: {err}"))?;
            Some(size.0)
        } else {
            None
        };

        let db = if self.blobfs {
            let path = self
                .data_path
                .clone()
                .ok_or_else(|| anyhow!("--data-path must be specified for blobfs"))?;
            let password = self
                .key
                .clone()
                .context("--key must be specified for blobfs")?;

            api::DbConfig::BlobFs(api::BlobFsConfig { path, password })
        } else {
            DbConfig::Crypto(api::BackendCryptoConfig {
                offset,
                data_path: self.data_path.clone(),
                key: self.key.clone().context("Must specify --key")?,
                full_index_write_interval: self.full_index_write_interval,
                raw: false,
                key_iterations: self.key_iterations,
                salt: self.salt.clone(),
                readonly: self.readonly,
            })
        };

        let c = api::BackendConfig {
            db,
            // TODO: make configurable.
            idle_timeout: None,
        };

        Ok(c)
    }
}

#[derive(clap::Parser, Clone)]
struct AppOptions {
    #[clap(flatten)]
    backend: BackendOptions,

    /// Start without a backend.
    /// A backend will have to be initialized via the UI.
    #[arg(short)]
    no_backend: bool,

    /// Directory used for storing temporary data.
    ///
    /// If not specified then only memory will be used.
    #[arg(long, env = "SEMANTIC_TMP_DIR")]
    tmp_dir: Option<PathBuf>,

    /// Key for API token generation.
    /// This should remain the same across server restarts.
    /// Otherwise existing tokens will be invalidated.
    // TODO: this should only be on server config...
    #[arg(long)]
    token_key: Option<String>,
}

impl AppOptions {
    fn build(&self) -> Result<AppConfig, anyhow::Error> {
        let data_dir = app::App::default_data_dir().unwrap();

        let backend_config = if self.no_backend {
            None
        } else {
            Some(self.backend.build_backend_config()?)
        };
        let token_key = self
            .token_key
            .clone()
            .unwrap_or_else(app::App::random_token_key);

        let app_config = app::AppConfig {
            backend: backend_config,
            token_key,
            deno: Some(app::DenoConfig {
                data_dir: data_dir.join("deno"),
                plugin_dir: None,
            }),
            tmp_dir: self.tmp_dir.clone(),
        };

        Ok(app_config)
    }
}

pub trait CliCommand {
    fn run(self) -> Result<(), anyhow::Error>;
}

pub trait AsyncCliCommand {
    async fn run(self) -> Result<(), anyhow::Error>;
}

impl<C> CliCommand for C
where
    C: AsyncCliCommand,
{
    fn run(self) -> Result<(), anyhow::Error> {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(self.run())
    }
}
