use std::{
    io::{BufReader, BufWriter, Read, Write},
    process::{Command, Stdio},
};

use anyhow::{bail, Context};
use factordb::AnyError;
use futures::{FutureExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub fn video_mime_supports_browser(mime: &str) -> bool {
    match mime {
        "video/mp4" | "video/webm" => true,
        _ => false,
    }
}

pub fn build_file_web_blob_uri(file: &semantic_core::base::File, extension: &str) -> String {
    format!("__converted/web/{}.{extension}", file.id)
}

pub fn optimize_image_data(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    let kind = infer::get(data).context("Could not determine mime type")?;
    match kind.mime_type() {
        "image/jpeg" => {
            // Try to use jpegtran binary
            mozjpeg_jpegtran(data)
        }
        "image/png" => oxipng(data),
        other => Err(anyhow::anyhow!("Unsupported mime type: {}", other)),
    }
}

/// Losslessly optimize pngs with oxipng as an embedded library.
#[cfg(feature = "media-optimizers")]
fn oxipng(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    oxipng::optimize_from_memory(data, &oxipng::Options::from_preset(4))
        .context("Could not optimize png with oxipng")
}

/// Losslessly optimize pngs with oxipng as a CLI tool.
#[cfg(not(feature = "media-optimizers"))]
fn oxipng(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    let mut child = Command::new("oxipng")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(&["-o", "2", "-"])
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .context("Empty stdin")
        .map(BufWriter::new)?;
    let mut stdout = child
        .stdout
        .take()
        .context("Empty stdout")
        .map(BufReader::new)?;

    let reader = std::thread::spawn(move || -> Result<Vec<u8>, anyhow::Error> {
        let mut buffer = Vec::new();
        stdout.read_to_end(&mut buffer)?;
        Ok(buffer)
    });

    stdin.write_all(data)?;
    std::mem::drop(stdin);

    let output = reader
        .join()
        .map_err(|err| anyhow::anyhow!("Reader failed {:?}", err))??;

    let status = child.wait()?;
    if !status.success() {
        bail!("oxipng failed");
    }

    Ok(output)
}

// /// Losslessly optimize jpegs with mozjpeg as a CLI tool.
// #[cfg(feature = "media-optimizers")]
// fn mozjpeg_jpegtran(data: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
//     std::panic::catch_unwind(|| -> Result<Vec<u8>, anyhow::Error> {
//         let decomp = mozjpeg::Decompress::new_mem(data)?;

//         let mut compress = mozjpeg::Compress::new(decomp.color_space());
//         compress.set_raw_data_in(true);
//         compress.set_size(decomp.width(), decomp.height());
//         compress.set_optimize_scans(true);
//         compress.set_optimize_coding(true);
//         compress.set_mem_dest();

//         let mut bitmaps: Vec<_> = decomp
//             .components()
//             .iter()
//             .map(|_c| Vec::with_capacity(decomp.width() * decomp.height()))
//             .collect();

//         let mut decomp = decomp.raw()?;
//         {
//             let mut bitmap_refs: Vec<_> = bitmaps.iter_mut().collect();
//             decomp.read_raw_data(&mut bitmap_refs);
//             if !decomp.finish_decompress() {
//                 bail!("Could not decompress image")
//             }
//         }

//         let bitmap_refs: Vec<_> = bitmaps.iter().map(|v| v.as_slice()).collect();
//         if !compress.write_raw_data(&bitmap_refs) {
//             bail!("Could not compress image");
//         }

//         compress.finish_compress();

//         let data = compress
//             .data_to_vec()
//             .map_err(|_| anyhow!("Could not compress image"))?;

//         Ok(data)
//     })
//     .map_err(|_err| anyhow!("mozjpeg failed"))?
// }

/// Losslessly optimize jpegs with mozjpeg as a CLI tool.
// #[cfg(not(feature = "media-optimizers"))]
fn mozjpeg_jpegtran(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    let mut child = Command::new("jpegtran")
        .arg("-optimize")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .context("Empty stdin")
        .map(BufWriter::new)?;
    let mut stdout = child
        .stdout
        .take()
        .context("Empty stdout")
        .map(BufReader::new)?;

    let reader = std::thread::spawn(move || -> Result<Vec<u8>, anyhow::Error> {
        let mut buffer = Vec::new();
        stdout.read_to_end(&mut buffer)?;
        Ok(buffer)
    });

    stdin.write_all(data)?;
    std::mem::drop(stdin);

    let output = reader
        .join()
        .map_err(|err| anyhow::anyhow!("Reader failed {:?}", err))??;

    let status = child.wait()?;
    if !status.success() {
        bail!("oxipng failed");
    }

    Ok(output)
}

pub async fn ffmpeg_convert_video(
    input: impl futures::Stream<Item = Result<Vec<u8>, AnyError>> + Unpin + Send + 'static,
) -> Result<hyper::Body, AnyError> {
    let mut proc = tokio::process::Command::new("ffmpeg")
        .args(&[
            // Less verbose output.
            // Only show errors.
            "-hide_banner",
            "-loglevel",
            "error",
            // read from stdin
            "-i",
            "pipe:",
            // Convert to libvpx-vp9
            "-c:v",
            "libvpx-vp9",
            // Constant rate factor.
            // See https://trac.ffmpeg.org/wiki/Encode/VP9 - Constant Quality
            // for background info.
            "-crf",
            "30",
            // Bitrate. 0 means auto, but this must be specified or the webm encode
            // will pick a very low default.
            "-b:v",
            "0",
            // Set output format.
            "-f",
            "webm",
            // Write to stdoud.
            "pipe:",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = proc.stdout.take().unwrap();

    // Start a task that writes the stream to ffmpeg.
    // TODO: log errors when the sender task fails.
    // TODO: investigate if custom termination of the sender task is required.
    let writer_handle = tokio::task::spawn(async move {
        let mut input = input;
        let mut stdin = proc.stdin.take().unwrap();

        while let Some(res) = input.next().await {
            let chunk = res?;
            stdin.write_all(&chunk).await?;
        }

        stdin.flush().await?;
        std::mem::drop(stdin);

        match proc.wait().await {
            Ok(status) => {
                if !status.success() {
                    let mut error_msg = String::new();
                    if proc
                        .stderr
                        .take()
                        .unwrap()
                        .read_to_string(&mut error_msg)
                        .await
                        .is_err()
                    {
                        error_msg = "Unknown error".to_string();
                    }

                    Err(AnyError::msg(error_msg))
                } else {
                    Ok(())
                }
            }
            Err(err) => {
                let mut error_msg = String::new();
                if proc
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut error_msg)
                    .await
                    .is_err()
                {
                    error_msg = "Unknown error".to_string();
                }
                Err(err).context(format!("ffmpeg failed: {}", error_msg))
            }
        }
    });

    // Read the first chunk right now.
    // This ensures that ffmpeg has started converting the video and won't
    // fail immediately.
    let mut stdout_stream = tokio_util::io::ReaderStream::new(stdout).fuse();
    let mut writer_join = writer_handle.fuse();

    let first_chunk = futures::select_biased! {

    join_res = &mut writer_join => {
            join_res.context("ffmpeg stdin writer task failed")??;
            stdout_stream.next().await.ok_or_else(|| AnyError::msg("ffmpeg did not return any chunks"))??
    }
    chunk_res = stdout_stream.next() => {
            match chunk_res {
                None => {
                    bail!("ffmpeg returned empty output");
                }
                Some(res) => {
                    res?
                }
            }
        }

    };

    let (sender, body) = hyper::Body::channel();

    // Start a task that reads the ffmpeg stdout and writes it to the hyper body.
    tokio::task::spawn(async move {
        let mut sender = sender;

        // Now send the first chunk.
        if let Err(error) = sender.send_data(first_chunk).await {
            tracing::warn!(?error, "could not send video chunk to hyper body");
            return;
        }

        loop {
            futures::select! {
                join_res = &mut writer_join => {
                    let res = join_res.map_err(AnyError::from).and_then(|x| x);
                    if let Err(error) = res {
                        tracing::warn!(?error, "ffmpeg failed");
                        sender.abort();
                        break;
                    }
                }
                chunk_res = stdout_stream.next() => {
                    match chunk_res {
                        None => {
                            break;
                        }
                        Some(Ok(chunk)) => {
                            if let Err(error) = sender.send_data(chunk).await {
                                tracing::warn!(?error, "could not send video chunk to hyper body");
                                sender.abort();
                                break;
                            }
                        }
                        Some(Err(error)) => {
                            tracing::warn!(?error, "ffmpeg stdout read failed");
                            sender.abort();
                            break;
                        }

                    }

                }
            }
        }
    });

    Ok(body)
}
