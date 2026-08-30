use std::path::PathBuf;

use crate::CliError;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Path to the local redb database.
    #[arg(long)]
    pub db: Option<PathBuf>,

    /// URI of the blob store used for file contents.
    #[arg(long)]
    pub blob_uri: Option<String>,

    /// Socket address to bind, overriding --interface and --port.
    #[arg(long)]
    pub bind: Option<String>,

    /// Directory used for persistent semantic data.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Directory used for temporary semantic data.
    #[arg(long)]
    pub temp_dir: Option<PathBuf>,

    /// Enable automatic media analysis.
    #[arg(long)]
    pub auto_analyze_media: bool,

    /// Network interface on which to listen.
    #[arg(long)]
    pub interface: Option<String>,

    /// Network port on which to listen.
    #[arg(long)]
    pub port: Option<u16>,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let app_config = app_config(&args);
    let server_config = server_config(&args)?;
    let db_path = args.db.unwrap_or_else(|| app_config.default_db_path());
    let blob_uri = match args.blob_uri {
        Some(uri) => uri,
        None => app_config
            .default_blob_uri()
            .map_err(CliError::DefaultBlobUri)?,
    };
    let bind = args.bind.unwrap_or_else(|| server_config.bind_address());

    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| CliError::CreateDatabaseDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let server = semantic_server::SemanticServer::local_redb_with_app_config_and_blob_store(
        db_path, app_config, blob_uri,
    )?
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
