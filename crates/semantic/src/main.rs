use anyhow::bail;
use factordb::AnyError;
use semantic_core::{
    api::{self, DbConfig},
    base::SemanticBasePlugin,
    plugin::PluginDescriptor,
};
use std::io::Write;
use structopt::StructOpt;

use semantic::{app, server};

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        #[cfg(not(debug_assertions))]
        let default = "semantic=info";
        #[cfg(debug_assertions)]
        let default = "semantic=trace,logfs=trace,factordb=debug,semantic_core=trace";

        std::env::set_var("RUST_LOG", default);
    }

    // Initialize logger.
    // TODO: tracing-tree disabled until it supports tracing_subscriber 0.3
    // let subscriber =
    //     tracing_subscriber::Registry::default().with(tracing_tree::HierarchicalLayer::new(2));
    // tracing::subscriber::set_global_default(subscriber).unwrap();
    tracing_subscriber::fmt::init();

    let args = CliArgs::from_args();

    match args.command {
        CliCommand::Server(subargs) => {
            let data_dir = app::App::default_data_dir().unwrap();

            let backend_config = if subargs.no_backend {
                None
            } else {
                Some(subargs.backend.build_backend_config().unwrap())
            };

            let app_config = app::AppConfig {
                backend: backend_config,
                token_key: subargs.token_key.unwrap_or_else(app::App::random_token_key),
                deno: Some(app::DenoConfig {
                    data_dir: data_dir.join("deno"),
                    plugin_dir: None,
                }),
            };
            let config = server::ServerConfig {
                // Enable authentication when no backend is provided.
                require_auth: app_config.backend.is_none(),
                app: app_config,
                address: subargs.address.unwrap_or(format!("127.0.0.1:3000")),
            };

            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            rt.block_on(server::run_server(config, rt.handle().clone()))
                .expect("Server failed");
        }
        CliCommand::GenerateTypescript(_) => {
            let builtin = factordb::schema::builtin::builtin_db_schema();
            let base = SemanticBasePlugin::new().schema().db.unwrap();

            let schema = builtin.merge(base);

            let ts = factor_tools::typescript::schema_to_typescript(&schema, None).unwrap();

            write!(std::io::stdout(), "{}", ts).unwrap();
        }
        #[cfg(feature = "webkit")]
        CliCommand::Gtk => {
            let config = app::AppConfig {
                backend: None,
                token_key: uuid::Uuid::new_v4().to_string(),
                server: None,
            };
            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let app = rt
                .block_on(app::App::build(config, rt.handle().clone()))
                .expect("Could not build app");
            app.run_webview_gtk().expect("Could not run GTK app");
        }
        CliCommand::Export { backend, path } => {
            let backend = backend.build_backend_config().unwrap();
            let app_config = semantic::app::AppConfig {
                backend: Some(backend),
                // TODO: this is useless. should be moved to server config.
                token_key: "xxx".to_string(),
                // No need for deno when exporting.
                deno: None,
            };

            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let handle = rt.handle().clone();
            rt.block_on(async move {
                let app = semantic::app::App::build(app_config, handle).await?;

                if let Some(path) = path {
                    let pathb = std::path::PathBuf::from(&path);
                    if pathb.is_dir() {
                        bail!("Given path is a directory: {path}");
                    } else if pathb.is_file() {
                        bail!("Given path already exists: {path}");
                    }
                    let f = std::fs::File::create(path)?;
                    let writer = std::io::BufWriter::new(f);

                    app.build_export(writer).await
                } else {
                    let writer = std::io::stdout();
                    app.build_export(writer).await
                }
            })
            .expect("Export failed");
        }
    }
}

/// Semantic CLI
#[derive(StructOpt)]
struct CliArgs {
    #[structopt(subcommand)]
    command: CliCommand,
}

#[derive(StructOpt)]
enum CliCommand {
    Server(CommandServer),
    #[cfg(feature = "webkit")]
    Webkit(CommandWebkit),
    GenerateTypescript(GenerateTypescript),
    /// Generate an archive that contains all data and blobs.
    Export {
        #[structopt(flatten)]
        backend: BackendOptions,

        /// Path where the export should be written.
        /// If not given, data is written to stdout.
        path: Option<String>,
    },
}

#[derive(StructOpt)]
struct GenerateTypescript {}

#[derive(StructOpt)]
struct BackendOptions {
    #[structopt(long, env = "SEMANTIC_DATA_PATH")]
    data_path: Option<String>,
    #[structopt(long, short, env = "SEMANTIC_KEY")]
    key: Option<String>,
    #[structopt(long, env = "SEMANTIC_KEY_ITERATIONS")]
    key_iterations: Option<u32>,
    #[structopt(long, env = "SEMANTIC_SALT")]
    salt: Option<String>,
}

impl BackendOptions {
    fn build_backend_config(self) -> Result<api::BackendConfig, AnyError> {
        let db = DbConfig::Crypto(api::BackendCryptoConfig {
            data_path: self.data_path,
            key: self.key.expect("Must specify --key"),
            raw: false,
            key_iterations: self.key_iterations,
            salt: self.salt,
        });

        let c = api::BackendConfig {
            db,
            // TODO: make configurable.
            idle_timeout: None,
        };

        Ok(c)
    }
}

pub struct DenoOptions {}

/// Run the semantic server backend.
#[derive(StructOpt)]
struct CommandServer {
    #[structopt(flatten)]
    backend: BackendOptions,

    /// Do not initialize a backend.
    /// The backend will have to be configured via the UI.
    #[structopt(long)]
    no_backend: bool,

    /// The server interface to listen on.
    /// eg: `0.0.0.0:3000`
    #[structopt(long, env = "SEMANTIC_ADDRESS")]
    address: Option<String>,

    /// The key used for JWT token encryption.
    #[structopt(long, env = "SEMANTIC_TOKEN_KEY")]
    token_key: Option<String>,
}

/// Run a semantic UI inside webkit.
#[cfg(feature = "webkit")]
#[derive(StructOpt)]
#[structopt(about = "Semantic CLI")]
struct CommandWebkit {}
