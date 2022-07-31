mod archive;
mod client;
mod cmd_upload;
mod db;

use anyhow::anyhow;
use factordb::AnyError;
use semantic_core::{
    api::{self, DbConfig},
    base::SemanticBasePlugin,
    plugin::PluginDescriptor,
};
use std::{io::Write, path::PathBuf, sync::Arc};

use semantic::{
    app::{self, AppConfig},
    server,
};

/// Semantic CLI
#[derive(clap::Parser)]
struct CliArgs {
    #[clap(subcommand)]
    command: CliCommand,
}

#[derive(clap::Subcommand)]
enum CliCommand {
    Server(CommandServer),
    /// Database related commands.
    #[clap(subcommand)]
    Db(db::DbCmd),
    #[clap(subcommand)]
    Client(client::ClientCommand),
    ImportFiles(CommandImportFiles),
    GenerateTypescript(GenerateTypescript),
    #[clap(subcommand)]
    Archive(archive::ArchiveCmd),
    Upload(cmd_upload::CmdUpload),
}

#[derive(clap::Parser)]
struct GenerateTypescript {}

impl GenerateTypescript {
    fn run(self) {
        let builtin = factordb::schema::builtin::builtin_db_schema();
        let base = SemanticBasePlugin::new().schema().db.unwrap();

        let schema = builtin.merge(base);

        dbg!(&schema);

        let ts = factor_tools::typescript::schema_to_typescript(&schema, None).unwrap();

        write!(std::io::stdout(), "{}", ts).unwrap();
    }
}

#[derive(clap::Parser, Clone)]
struct BackendOptions {
    /// Path for the database.
    #[clap(short = 'p', long, env = "SEMANTIC_DATA_PATH")]
    data_path: Option<String>,
    /// Fore creation of a new database.
    /// Will fail if the database already exists.
    #[clap(long)]
    create: bool,
    /// The password.
    // TODO: use anonymizing wrapper?
    #[clap(long, short, env = "SEMANTIC_KEY")]
    key: Option<String>,
    /// Number of key iterations used for encryption key derivation.
    /// This can normally remain unchanged.
    #[clap(long, env = "SEMANTIC_KEY_ITERATIONS")]
    key_iterations: Option<u32>,
    /// Use a custom encryption salt.
    // TODO: use anonymizing wrapper?
    #[clap(long, env = "SEMANTIC_SALT")]
    salt: Option<String>,
    /// Binary offset in the storage file.
    /// Either a number of bytes, or a parsable pretty byte number like "300mb".
    #[clap(long)]
    offset: Option<String>,

    /// The number of key changes after which a full key index is written.
    #[clap(long)]
    full_index_write_interval: Option<u64>,
}

impl BackendOptions {
    fn build_backend_config(&self) -> Result<api::BackendConfig, AnyError> {
        let offset = if let Some(off) = &self.offset {
            let size = off
                .parse::<bytesize::ByteSize>()
                .map_err(|err| anyhow!("Invalid offset: {err}"))?;
            Some(size.0)
        } else {
            None
        };

        let db = DbConfig::Crypto(api::BackendCryptoConfig {
            offset,
            data_path: self.data_path.clone(),
            key: self.key.clone().expect("Must specify --key"),
            full_index_write_interval: self.full_index_write_interval,
            raw: false,
            key_iterations: self.key_iterations,
            salt: self.salt.clone(),
        });

        let c = api::BackendConfig {
            db,
            // TODO: make configurable.
            idle_timeout: None,
        };

        Ok(c)
    }
}

#[derive(clap::Parser, Clone)]
struct ClientOptions {
    /// Server address.
    ///
    /// Defaults to "http://localhost:3000".
    #[clap(short = 'a', long)]
    address: Option<url::Url>,
}

impl ClientOptions {
    fn build_client(&self) -> client::ReqwestApiClient {
        let endpoint = self
            .address
            .clone()
            .unwrap_or_else(|| "http://localhost:3000".parse().unwrap());
        client::ReqwestApiClient::new(client::ReqwestExecutor::new(endpoint))
    }
}

#[derive(clap::Parser, Clone)]
struct AppOptions {
    #[clap(flatten)]
    backend: BackendOptions,

    /// Start without a backend.
    /// A backend will have to be initialized via the UI.
    #[clap(short)]
    no_backend: bool,

    /// Directory used for storing temporary data.
    ///
    /// If not specified then only memory will be used.
    #[clap(long, env = "SEMANTIC_TMP_DIR")]
    tmp_dir: Option<PathBuf>,

    /// Key for API token generation.
    /// This should remain the same across server restarts.
    /// Otherwise existing tokens will be invalidated.
    // TODO: this should only be on server config...
    token_key: Option<String>,
}

impl AppOptions {
    fn build(&self) -> Result<AppConfig, AnyError> {
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

pub struct DenoOptions {}

#[derive(clap::Parser)]
/// Import files into a semantic database.
struct CommandImportFiles {
    #[clap(flatten)]
    backend: BackendOptions,
    paths: Vec<std::path::PathBuf>,
}

/// Run the semantic server.
#[derive(clap::Parser)]
struct CommandServer {
    #[clap(flatten)]
    app: AppOptions,

    /// The server interface to listen on.
    /// eg: `0.0.0.0:3000`
    #[clap(long, env = "SEMANTIC_ADDRESS")]
    address: Option<String>,
}

impl CommandServer {
    fn run(self) {
        let app_config = self.app.build().unwrap();
        let config = server::ServerConfig {
            // Enable authentication when no backend is provided.
            require_auth: app_config.backend.is_none(),
            app: app_config,
            address: self.address.unwrap_or(format!("127.0.0.1:3000")),
        };

        let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
        rt.block_on(server::run_server(config, rt.handle().clone()))
            .expect("Server failed");
    }
}

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        #[cfg(not(debug_assertions))]
        let default = "semantic=info";

        #[cfg(debug_assertions)]
        let default =
            "semantic=trace,logfs=trace,factordb=debug,factor_engine=debug,semantic_core=trace";

        std::env::set_var("RUST_LOG", default);
    }

    // Initialize logger.
    // TODO: tracing-tree disabled until it supports tracing_subscriber 0.3
    // let subscriber =
    //     tracing_subscriber::Registry::default().with(tracing_tree::HierarchicalLayer::new(2));
    // tracing::subscriber::set_global_default(subscriber).unwrap();
    tracing_subscriber::fmt::init();

    let args = <CliArgs as clap::Parser>::parse();

    match args.command {
        CliCommand::Server(cmd) => {
            cmd.run();
        }
        CliCommand::GenerateTypescript(cmd) => {
            cmd.run();
        }
        CliCommand::Archive(cmd) => {
            cmd.run();
        }
        CliCommand::ImportFiles(args) => {
            let data_dir = app::App::default_data_dir().unwrap();

            let backend_config = args.backend.build_backend_config().unwrap();

            let app_config = app::AppConfig {
                backend: Some(backend_config),
                token_key: "".to_string(),
                deno: Some(app::DenoConfig {
                    data_dir: data_dir.join("deno"),
                    plugin_dir: None,
                }),
                tmp_dir: None,
            };

            let meta = api::FileImportMetadata {
                collection_id: None,
                tags: Vec::new(),
                url: None,
                parent: None,
            };

            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let handle = rt.handle().clone();
            rt.block_on(async move {
                let app = semantic::app::App::build(app_config, handle).await?;

                app.import_files(
                    args.paths,
                    meta,
                    Arc::new(|path, _file| {
                        tracing::info!(?path, "Imported file");
                    }),
                )
                .await?;

                Result::<(), AnyError>::Ok(())
            })
            .expect("Export failed");

            tracing::info!("All paths imported");
        }
        CliCommand::Upload(cmd) => {
            cmd.run();
        }
        CliCommand::Db(cmd) => {
            cmd.run();
        }
        CliCommand::Client(cmd) => {
            cmd.run();
        }
    }
}
