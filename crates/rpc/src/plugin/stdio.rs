//! Content-Length framed JSON on portable asynchronous byte streams.
use super::{ProviderConnection, error, negotiate};
use crate::interface::{ImplementationDescriptor, InvocationError, session::Session};
use semantic_data::value::Value;
use semantic_rpc_core::interface_protocol::InterfaceMessage;
use std::{collections::BTreeMap, path::PathBuf, process::Stdio, sync::Arc};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};

#[derive(Clone, Debug)]
pub struct StdioConfig {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
}

/// None means clean EOF between frames; partial headers/bodies are errors.
pub async fn read_frame(
    reader: &mut (impl AsyncBufRead + Unpin),
) -> Result<Option<Vec<u8>>, InvocationError> {
    let mut length = None;
    let mut started = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.map_err(error)? == 0 {
            return if started {
                Err(error("partial frame header"))
            } else {
                Ok(None)
            };
        }
        started = true;
        let line = line
            .strip_suffix("\r\n")
            .ok_or_else(|| error("frame headers require CRLF"))?;
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| error("invalid frame header"))?;
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(error("duplicate Content-Length"));
            }
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(error("invalid Content-Length"));
            }
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| error("Content-Length overflow"))?,
            );
        } else if !name.eq_ignore_ascii_case("content-type") {
            return Err(error("unexpected protocol header / stdout contamination"));
        }
    }
    let length = length.ok_or_else(|| error("missing Content-Length"))?;
    let mut body = Vec::new();
    body.try_reserve_exact(length).map_err(error)?;
    body.resize(length, 0);
    reader.read_exact(&mut body).await.map_err(error)?;
    Ok(Some(body))
}

pub async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    body: &[u8],
) -> Result<(), InvocationError> {
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await
        .map_err(error)?;
    writer.write_all(body).await.map_err(error)?;
    writer.flush().await.map_err(error)
}

/// SDK dispatcher for protocol-speaking executables; stdout is reserved for frames.
pub async fn serve<R, W, F, Fut>(
    reader: R,
    mut writer: W,
    implementation: Arc<dyn crate::interface::InterfaceImplementation>,
    revision: Option<String>,
    configure: F,
) -> Result<(), InvocationError>
where
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
    F: FnOnce(Value) -> Fut,
    Fut: std::future::Future<Output = Result<(), InvocationError>>,
{
    let mut reader = BufReader::new(reader);
    let (input, incoming) = mpsc::unbounded_channel();
    let (outgoing, mut output) = mpsc::unbounded_channel::<InterfaceMessage>();
    let writer_errors = input.clone();
    let reader = tokio::spawn(async move {
        loop {
            let result = match read_frame(&mut reader).await {
                Ok(Some(body)) => serde_json::from_slice(&body).map_err(error),
                Ok(None) => Err(error("host closed stdin")),
                Err(error) => Err(error),
            };
            let terminal = result.is_err();
            if input.send(result).is_err() || terminal {
                break;
            }
        }
    });
    let writer = tokio::spawn(async move {
        while let Some(message) = output.recv().await {
            let result = match serde_json::to_vec(&message) {
                Ok(bytes) => write_frame(&mut writer, &bytes).await,
                Err(e) => Err(error(e)),
            };
            if let Err(error) = result {
                let _ = writer_errors.send(Err(error));
                break;
            }
        }
        let _ = writer.shutdown().await;
    });
    let result = super::accept(incoming, outgoing, implementation, revision, configure).await;
    if let Ok(session) = &result {
        session.closed().await;
    }
    reader.abort();
    writer.await.map_err(error)?;
    result.map(|_| ())
}

pub async fn connect(
    config: StdioConfig,
    exports: Vec<ImplementationDescriptor>,
    revision: Option<String>,
    configuration: Value,
) -> Result<ProviderConnection, InvocationError> {
    connect_with_cancellation(
        config,
        exports,
        revision,
        configuration,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
}

pub async fn connect_with_cancellation(
    config: StdioConfig,
    exports: Vec<ImplementationDescriptor>,
    revision: Option<String>,
    configuration: Value,
    cancellation: tokio_util::sync::CancellationToken,
) -> Result<ProviderConnection, InvocationError> {
    if cancellation.is_cancelled() {
        return Err(InvocationError::new(
            "cancelled",
            "Plugin startup cancelled",
        ));
    }
    let mut command = Command::new(&config.program);
    command
        .args(&config.args)
        .envs(&config.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = config.cwd {
        command.current_dir(cwd);
    }
    let mut child = command
        .spawn()
        .map_err(|e| InvocationError::new("provider_unavailable", e.to_string()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| error("missing child stdin"))?;
    let mut stdout = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| error("missing child stdout"))?,
    );
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| error("missing child stderr"))?;
    let stderr_task = tokio::spawn(async move {
        let mut bytes = [0u8; 4096];
        loop {
            match stderr.read(&mut bytes).await {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    tracing::debug!(target: "semantic_plugin", stderr = %String::from_utf8_lossy(&bytes[..count]), "plugin stderr")
                }
            }
        }
    });
    let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<InterfaceMessage>();
    let (incoming_tx, mut incoming) = mpsc::unbounded_channel();
    // This task is the sole child owner from spawn through explicit wait. Dropping
    // a startup waiter or connection handle cannot discard direct-child reaping.
    let exit_tx = incoming_tx.clone();
    let reaper = tokio::spawn(async move {
        let result = child.wait().await.map_err(error);
        let message = match &result {
            Ok(status) => format!("plugin exited with {status}"),
            Err(error) => error.to_string(),
        };
        let _ = exit_tx.send(Err(error(message)));
        result
    });
    let read_tx = incoming_tx.clone();
    let reader = tokio::spawn(async move {
        loop {
            let result = match read_frame(&mut stdout).await {
                Ok(Some(body)) => serde_json::from_slice(&body).map_err(error),
                Ok(None) => Err(error("plugin stdout closed")),
                Err(error) => Err(error),
            };
            let terminal = result.is_err();
            if read_tx.send(result).is_err() || terminal {
                break;
            }
        }
    });
    let writer = tokio::spawn(async move {
        while let Some(message) = outgoing_rx.recv().await {
            let result = match serde_json::to_vec(&message) {
                Ok(bytes) => write_frame(&mut stdin, &bytes).await,
                Err(e) => Err(error(e)),
            };
            if let Err(error) = result {
                let _ = incoming_tx.send(Err(error));
                break;
            }
        }
        let _ = stdin.shutdown().await;
    });
    let result = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(InvocationError::new("cancelled", "Plugin startup cancelled")),
        result = negotiate(&mut incoming, &outgoing, &exports, revision, configuration) => result,
    };
    if let Err(failure) = result {
        drop(outgoing);
        let _ = writer.await;
        // Failed startup follows cooperative shutdown too; no implicit deadline
        // or forced escalation is introduced for a noncooperative executable.
        let _ = reaper.await;
        reader.abort();
        stderr_task.abort();
        return Err(failure);
    }
    let session = Session::start(incoming, outgoing, None, exports);
    Ok(ProviderConnection {
        implementation: Arc::new(session.clone()),
        session,
        cleanup: Box::pin(async move {
            writer.await.map_err(error)?;
            let reaped = reaper.await.map_err(error)?;
            reader.abort();
            stderr_task.abort();
            // The session already reports a child crash as provider failure.
            // A nonzero exit does not prevent successfully reaped resources
            // from being released for an explicit restart or removal.
            reaped.map(|_| ())
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn framing_and_partial_eof() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, "hé\nllo".as_bytes()).await.unwrap();
        write_frame(&mut bytes, b"{}").await.unwrap();
        let mut reader = bytes.as_slice();
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap(),
            "hé\nllo".as_bytes()
        );
        assert_eq!(read_frame(&mut reader).await.unwrap().unwrap(), b"{}");
        assert!(read_frame(&mut reader).await.unwrap().is_none());
        for bad in [
            "Content-Length: 9\r\n\r\nabc",
            "Content-Length: 1\r\nContent-Length: 1\r\n\r\nx",
            "hello\n",
            "\r\n",
            "Content-Length: -1\r\n\r\n",
            "Content-Length: 999999999999999999999999999\r\n\r\n",
        ] {
            assert!(read_frame(&mut bad.as_bytes()).await.is_err(), "{bad:?}");
        }
    }
}
