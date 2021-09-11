use semantic_core::api;
use structopt::StructOpt;

use semantic::app;

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        #[cfg(not(debug_assertions))]
        let default = "semantic=info";
        #[cfg(debug_assertions)]
        let default = "semantic=trace,logfs=trace,factordb=debug,semantic_core=trace";

        std::env::set_var("RUST_LOG", default);
    }
    tracing_subscriber::fmt::init();

    let args = CliArgs::from_args();

    match args.command {
        CliCommand::Server(subargs) => {
            let db_config = if subargs.no_backend {
                None
            } else {
                Some(semantic_core::api::DbConfig::Crypto(
                    semantic_core::api::BackendCryptoConfig {
                        data_path: subargs.data_path,
                        key: subargs.key.expect("Must specify --key"),
                    },
                ))
            };

            let backend_config = db_config.map(|db| api::BackendConfig {
                db,
                // TODO: make configurable
                idle_timeout: None,
            });

            let config = app::AppConfig {
                backend: backend_config,
                token_key: subargs.token_key.unwrap_or_else(app::App::random_token_key),
                server: Some(app::ServerConfig {
                    interface: subargs.interface.unwrap_or(format!("127.0.0.1:3000")),
                }),
            };

            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let app = rt
                .block_on(app::App::build(config, rt.handle().clone()))
                .expect("Could not build app");
            rt.block_on(app.run_server()).expect("Server failed");
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
}

/// Run the semantic server backend.
#[derive(StructOpt)]
struct CommandServer {
    #[structopt(long)]
    data_path: Option<String>,
    #[structopt(long, short)]
    key: Option<String>,
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
