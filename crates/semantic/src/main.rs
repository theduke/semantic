use semantic_core::{api, base::SemanticBasePlugin, plugin::PluginDescriptor};
use std::{io::Write, path::PathBuf};
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

            let db_config = if subargs.no_backend {
                None
            } else {
                Some(semantic_core::api::DbConfig::Crypto(
                    semantic_core::api::BackendCryptoConfig {
                        data_path: subargs.data_path,
                        key: subargs.key.expect("Must specify --key"),
                        raw: false,
                        key_iterations: subargs.key_iterations,
                        salt: subargs.salt,
                    },
                ))
            };

            let backend_config = db_config.map(|db| api::BackendConfig {
                db,
                // TODO: make configurable
                idle_timeout: None,
            });

            let app_config = app::AppConfig {
                backend: backend_config,
                token_key: subargs.token_key.unwrap_or_else(app::App::random_token_key),
                deno: Some(app::DenoConfig {
                    data_dir: data_dir.join("deno"),
                    plugin_dir: subargs.deno_plugin_dir.map(PathBuf::from),
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
}

#[derive(StructOpt)]
struct GenerateTypescript {}

/// Run the semantic server backend.
#[derive(StructOpt)]
struct CommandServer {
    #[structopt(long)]
    data_path: Option<String>,
    #[structopt(long, short)]
    key: Option<String>,
    #[structopt(long)]
    key_iterations: Option<u32>,
    #[structopt(long)]
    salt: Option<String>,

    #[structopt(long)]
    deno_plugin_dir: Option<String>,

    #[structopt(long)]
    no_backend: bool,
    /// The interface to listen on.
    /// eg: `0.0.0.0:3000`
    #[structopt(long)]
    interface: Option<String>,
    /// The key used for JWT token encryption.
    #[structopt(long)]
    token_key: Option<String>,
}

/// Run a semantic UI inside webkit.
#[cfg(feature = "webkit")]
#[derive(StructOpt)]
#[structopt(about = "Semantic CLI")]
struct CommandWebkit {}
