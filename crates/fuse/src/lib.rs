//! FUSE projection for one semantic database scope.
//!
//! The projection provides `entities/`, `tree/`, `files/`, and `blobs/` views.
//! Supported mutations are entity-document replacement/creation, new streamed
//! file uploads below a directory in `tree/`, and moving existing tree entries
//! between directories. Directory creation/deletion and replacement of an
//! existing raw file are intentionally rejected for now. Mount separate
//! instances to browse several scopes at the same time.

mod filesystem;
mod layout;
mod runtime;

use std::path::Path;

use semantic_rpc::RpcClient;
use thiserror::Error;

pub use filesystem::SemanticFilesystem;
pub use layout::{EntityFormat, MountConfig};

#[derive(Debug, Error)]
pub enum FuseError {
    #[error("RPC error: {0}")]
    Rpc(String),
    #[error("FUSE mount failed: {0}")]
    Mount(#[from] std::io::Error),
    #[error("FUSE worker stopped unexpectedly")]
    WorkerStopped,
}

/// Mount a semantic database and serve it until the filesystem is unmounted.
///
/// Fuser invokes synchronous callbacks on its worker thread. Those callbacks
/// use the calling Tokio runtime handle to wait for native `RpcClient` futures.
/// Upload bodies are the exception: they run as independent runtime tasks so
/// sequential FUSE writes can feed a bounded stream with real backpressure.
pub async fn mount(
    client: RpcClient,
    mountpoint: impl AsRef<Path>,
    config: MountConfig,
) -> std::result::Result<(), FuseError> {
    let snapshot = layout::load_snapshot(&client, &config)
        .await
        .map_err(FuseError::Rpc)?;
    let bridge = runtime::RuntimeBridge::new(client, tokio::runtime::Handle::current());
    let filesystem = SemanticFilesystem::from_snapshot(bridge, snapshot, config.clone());
    let mountpoint = mountpoint.as_ref().to_path_buf();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("semantic-fuse".to_string())
        .spawn(move || {
            let options = filesystem::mount_options(&config);
            let result = fuser::mount(filesystem, mountpoint, &options);
            let _ = done_tx.send(result);
        })
        .map_err(FuseError::Mount)?;

    done_rx
        .await
        .map_err(|_| FuseError::WorkerStopped)?
        .map_err(FuseError::Mount)
}
