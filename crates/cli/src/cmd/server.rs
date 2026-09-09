use std::path::PathBuf;

use crate::CliError;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Database URI: redb:PATH, logfs:PATH, or log:<blob>.
    ///
    /// log:<blob> shares the configured blob store with the log database.
    /// Defaults to a redb database at <data-dir>/db/default.
    #[arg(long, env = "SEMANTIC_DB_URI", value_name = "URI")]
    pub db_uri: Option<String>,

    /// Blob-store URI used for uploaded file contents.
    ///
    /// Defaults to a file URI for <data-dir>/blob/default.
    #[arg(long, env = "SEMANTIC_BLOB_URI", value_name = "URI")]
    pub blob_uri: Option<String>,

    /// Exact socket address to bind, overriding --interface and --port.
    #[arg(long, value_name = "ADDRESS")]
    pub bind: Option<String>,

    /// Base directory for persistent database and blob data.
    ///
    /// Overrides SEMANTIC_DATA_DIR; defaults to the platform user data directory
    /// (for example, ~/.local/share/semantic on Linux).
    #[arg(long, value_name = "PATH")]
    pub data_dir: Option<PathBuf>,

    /// Directory used for temporary data, overriding SEMANTIC_TEMP_DIR.
    #[arg(long, value_name = "PATH")]
    pub temp_dir: Option<PathBuf>,

    /// Enable automatic media analysis, overriding a disabled environment setting.
    ///
    /// This behavior is enabled by default and can be configured with
    /// SEMANTIC_AUTO_ANALYZE_MEDIA.
    #[arg(long)]
    pub auto_analyze_media: bool,

    /// Interface or host on which to listen.
    ///
    /// Overrides SEMANTIC_INTERFACE; defaults to 127.0.0.1.
    #[arg(long, value_name = "HOST")]
    pub interface: Option<String>,

    /// TCP port on which to listen.
    ///
    /// Overrides SEMANTIC_PORT; defaults to 8888.
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let app_config = app_config(&args);
    let server_config = server_config(&args)?;
    let db_uri = match args.db_uri {
        Some(uri) => uri,
        None => {
            let path = app_config.default_db_path();
            let path = path.to_str().ok_or_else(|| {
                CliError::InvalidInput("database URI requires a UTF-8 path".into())
            })?;
            format!("redb:{path}")
        }
    };
    let blob_uri = match args.blob_uri {
        Some(uri) => uri,
        None => app_config
            .default_blob_uri()
            .map_err(CliError::DefaultBlobUri)?,
    };
    let bind = args.bind.unwrap_or_else(|| server_config.bind_address());

    let server = semantic_server::SemanticServer::from_uris(db_uri, blob_uri, app_config)
        .await?
        .with_config(server_config);
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|source| CliError::Bind {
            address: bind.clone(),
            source,
        })?;
    eprintln!("semantic server listening on http://{bind}");
    server.serve(listener).await?;
    Ok(())
}

fn app_config(args: &Args) -> semantic_app::AppConfig {
    let mut config = semantic_app::AppConfig::from_env();
    if let Some(data_dir) = &args.data_dir {
        config.data_dir = Some(data_dir.clone());
    }
    if let Some(temp_dir) = &args.temp_dir {
        config.temp_dir = Some(temp_dir.clone());
    }
    if args.auto_analyze_media {
        config.auto_analyze_media = true;
    }
    config
}

fn server_config(args: &Args) -> std::result::Result<semantic_server::ServerConfig, CliError> {
    let mut config = semantic_server::ServerConfig::from_env().map_err(CliError::ServerConfig)?;
    if let Some(interface) = &args.interface {
        config.interface.clone_from(interface);
    }
    if let Some(port) = args.port {
        config.port = port;
    }
    Ok(config)
}
