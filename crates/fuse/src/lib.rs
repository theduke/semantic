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

use std::{future::Future, io, path::Path};

#[cfg(target_os = "linux")]
use std::{ffi::OsString, path::PathBuf, process::Command};

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
    mount_with_shutdown(client, mountpoint, config, std::future::pending()).await
}

/// Mount a semantic database until it is unmounted or `shutdown` completes.
///
/// Completing `shutdown` explicitly unmounts the FUSE session and waits for
/// its worker thread to finish. Before mounting, an inaccessible stale Linux
/// FUSE mount is detached automatically.
pub async fn mount_with_shutdown<F>(
    client: RpcClient,
    mountpoint: impl AsRef<Path>,
    config: MountConfig,
    shutdown: F,
) -> std::result::Result<(), FuseError>
where
    F: Future<Output = ()>,
{
    let mountpoint = mountpoint.as_ref().to_path_buf();
    recover_stale_mount(&mountpoint).map_err(FuseError::Mount)?;
    let snapshot = layout::load_snapshot(&client, &config)
        .await
        .map_err(FuseError::Rpc)?;
    let bridge = runtime::RuntimeBridge::new(client, tokio::runtime::Handle::current());
    let filesystem = SemanticFilesystem::from_snapshot(bridge, snapshot, config.clone());
    let options = filesystem::mount_options(&config);
    let mut session =
        fuser::Session::new(filesystem, mountpoint, &options).map_err(FuseError::Mount)?;
    let mut unmounter = session.unmount_callable();
    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("semantic-fuse".to_string())
        .spawn(move || {
            let result = session.run();
            let _ = done_tx.send(result);
        })
        .map_err(FuseError::Mount)?;

    tokio::pin!(shutdown);
    tokio::select! {
        result = &mut done_rx => worker_result(result),
        () = &mut shutdown => {
            tokio::task::spawn_blocking(move || unmounter.unmount())
                .await
                .map_err(|error| FuseError::Mount(io::Error::other(error.to_string())))?
                .map_err(FuseError::Mount)?;
            worker_result(done_rx.await)
        }
    }
}

fn worker_result(
    result: std::result::Result<io::Result<()>, tokio::sync::oneshot::error::RecvError>,
) -> std::result::Result<(), FuseError> {
    result
        .map_err(|_| FuseError::WorkerStopped)?
        .map_err(FuseError::Mount)
}

fn recover_stale_mount(mountpoint: &Path) -> io::Result<()> {
    let Err(error) = std::fs::metadata(mountpoint) else {
        return Ok(());
    };
    if !is_stale_mount_error(&error) {
        return Ok(());
    }

    detach_stale_mount(mountpoint)?;
    std::fs::metadata(mountpoint).map(|_| ()).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "stale FUSE mount at '{}' was detached, but the mountpoint is still inaccessible: {error}",
                mountpoint.display()
            ),
        )
    })
}

fn is_stale_mount_error(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ENOTCONN)
}

fn detach_stale_mount(mountpoint: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let mut helpers = Vec::<OsString>::new();
        if let Some(path) = std::env::var_os("FUSERMOUNT_PATH") {
            helpers.push(path);
        }
        helpers.extend([OsString::from("fusermount3"), OsString::from("fusermount")]);

        let mut failures = Vec::new();
        for helper in helpers {
            match Command::new(&helper)
                .args(["-u", "-q", "-z", "--"])
                .arg(mountpoint)
                .output()
            {
                Ok(output) if output.status.success() => return Ok(()),
                Ok(output) => failures.push(format!(
                    "{} exited with {}: {}",
                    PathBuf::from(&helper).display(),
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                )),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!(
                    "{} could not be started: {error}",
                    PathBuf::from(&helper).display()
                )),
            }
        }

        return Err(io::Error::other(format!(
            "failed to detach stale FUSE mount at '{}'; {}",
            mountpoint.display(),
            if failures.is_empty() {
                "neither fusermount3 nor fusermount was found".to_string()
            } else {
                failures.join("; ")
            }
        )));
    }

    #[cfg(not(target_os = "linux"))]
    Err(io::Error::other(format!(
        "automatic recovery of stale FUSE mount '{}' is only supported on Linux",
        mountpoint.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::is_stale_mount_error;

    #[test]
    fn recognizes_disconnected_fuse_mounts_only() {
        assert!(is_stale_mount_error(&std::io::Error::from_raw_os_error(
            libc::ENOTCONN
        )));
        assert!(!is_stale_mount_error(&std::io::Error::from_raw_os_error(
            libc::ENOENT
        )));
    }
}
