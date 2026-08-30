pub mod cmd;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    Fuse(#[from] semantic_fuse::FuseError),

    #[error("invalid server configuration: {0}")]
    ServerConfig(String),

    #[error("failed to build default semantic blob store URI: {0}")]
    DefaultBlobUri(String),

    #[error("failed to create semantic server database directory {path}: {source}")]
    CreateDatabaseDirectory {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    #[error("failed to bind semantic server to {address}: {source}")]
    Bind {
        address: String,
        source: std::io::Error,
    },

    #[error(transparent)]
    Server(#[from] semantic_server::ServerError),
}

pub async fn run(args: cmd::Args) -> std::result::Result<(), CliError> {
    match args.command {
        cmd::SubCmd::Fuse(args) => cmd::fuse::run(args).await,
        cmd::SubCmd::Server(args) => cmd::server::run(args).await,
    }
}
