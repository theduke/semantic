use std::path::PathBuf;

use crate::{CliError, cmd::shared::ApiClientArgs};
use semantic_fuse::{EntityFormat, MountConfig};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Existing local directory on which to mount the semantic filesystem.
    #[arg(value_name = "MOUNTPOINT")]
    pub mountpoint: PathBuf,

    #[command(flatten)]
    pub client: ApiClientArgs,

    /// Format used to expose entity documents: `json` or `yaml`.
    #[arg(long, default_value = "json")]
    pub format: EntityFormat,

    /// Allow users other than the mounting user to access the mount.
    #[arg(long)]
    pub allow_other: bool,

    /// Mount read-only and reject mutations through the filesystem.
    #[arg(long)]
    pub read_only: bool,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    let client = args.client.rpc_client();
    let config = MountConfig {
        scope_id: args.client.scope,
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
