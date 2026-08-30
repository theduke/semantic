use std::path::PathBuf;

use semantic_fuse::{EntityFormat, MountConfig};
use semantic_rpc::transport::http_client::HttpRpcClient;

use crate::CliError;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Directory on which to mount the semantic filesystem.
    pub mountpoint: PathBuf,

    /// Semantic HTTP RPC endpoint.
    #[arg(
        long,
        env = "SEMANTIC_RPC_URL",
        default_value = "http://127.0.0.1:8888/api/v1/rpc"
    )]
    pub rpc_url: String,

    /// Database scope exposed by this mount.
    #[arg(long)]
    pub scope: Option<String>,

    /// Entity document format: json or yaml.
    #[arg(long, default_value = "json")]
    pub format: EntityFormat,

    /// Allow users other than the mounting user to access the mount.
    #[arg(long)]
    pub allow_other: bool,

    /// Disable all mutations.
    #[arg(long)]
    pub read_only: bool,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let client = HttpRpcClient::new(args.rpc_url).into();
    let config = MountConfig {
        scope_id: args.scope,
        format: args.format,
        allow_other: args.allow_other,
        read_only: args.read_only,
        ..MountConfig::default()
    };

    semantic_fuse::mount_with_shutdown(client, args.mountpoint, config, shutdown_signal())
        .await
        .map_err(CliError::from)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
