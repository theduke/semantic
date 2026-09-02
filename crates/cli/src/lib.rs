pub mod cmd;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    Rpc(#[from] semantic_rpc::RpcClientError),

    #[error("failed to {action} {path}: {source}")]
    Io {
        action: &'static str,
        path: String,
        source: std::io::Error,
    },

    #[error("invalid JSON in {source_name}: {source}")]
    Json {
        source_name: String,
        source: serde_json::Error,
    },

    #[error("{0}")]
    InvalidInput(String),

    #[error("confirmation required: pass --yes to delete '{target}'")]
    ConfirmationRequired { target: String },

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
        cmd::SubCmd::Api(args) => cmd::api::run(args).await,
        cmd::SubCmd::Fuse(args) => cmd::fuse::run(args).await,
        cmd::SubCmd::Server(args) => cmd::server::run(args).await,
    }
}
