use std::{
    fmt::Write as _,
    io::{self, BufReader, BufWriter, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
};

use anyhow::{anyhow, bail, Context};
use factordb::{
    prelude::{AttrMapExt, DataMap, Db, Expr, Patch, Select},
    AnyError,
};

use semantic_core::{
    api::{ApiError, Job, JobId},
    base::{AttrBlobUriWeb, AttrDuration, AttrVideoHasSound, TypedFile, UniversalHash, Video},
};
use tokio::{io::AsyncBufReadExt, task::JoinHandle};

use crate::{blobstore::DynBlobStore, jobs::JobManager};

// pub fn video_mime_supports_browser(mime: &str) -> bool {
//     match mime {
//         "video/mp4" | "video/webm" => true,
//         _ => false,
//     }
// }

pub fn optimise_file_data(data: Vec<u8>) -> (Vec<u8>, UniversalHash, Option<UniversalHash>) {
    use sha2::Digest;

    let mime_guess = infer::get(&data);
    let raw_hash = sha2::Sha256::digest(&data);
    let hash = semantic_core::base::UniversalHash::new(
        semantic_core::base::UniversalHash::SHA256,
        &format!("{:x}", raw_hash),
    );
    match mime_guess {
        Some(t) if t.mime_type().starts_with("image/") => {
            tracing::trace!("starting media optimisation");
            match crate::util::media::optimize_image_data(&data) {
                Ok(new_data) => {
                    tracing::trace!(old_size=%data.len(), new_size=new_data.len(), "optimised image data");
                    let new_hash_raw = sha2::Sha256::digest(&new_data);
                    let new_hash = semantic_core::base::UniversalHash::new(
                        semantic_core::base::UniversalHash::SHA256,
                        &format!("{:x}", new_hash_raw),
                    );

                    (new_data, hash, Some(new_hash))
                }
                Err(err) => {
                    tracing::warn!(?err, "Failed to optimize image data");
                    (data, hash, None)
                }
            }
        }
        _ => (data, hash, None),
    }
}

pub fn optimize_image_data(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    let kind = infer::get(data).context("Could not determine mime type")?;
    match kind.mime_type() {
        "image/jpeg" => {
            // Try to use jpegtran binary
            mozjpeg_jpegtran(data)
        }
        "image/png" => oxipng(data),
        other => Err(anyhow!("Unsupported mime type: {}", other)),
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
        .map_err(|err| anyhow!("Reader failed {:?}", err))??;

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
        .map_err(|err| anyhow!("Reader failed {:?}", err))??;

    let status = child.wait()?;
    if !status.success() {
        bail!("oxipng failed");
    }

    Ok(output)
}

fn spawn_ffmpeg_output_monitor(
    jobs: &JobManager,
    progress_path: &PathBuf,
    frame_count: Option<u64>,
    step: &str,
    job_id: JobId,
) -> JoinHandle<()> {
    let jobs = jobs.clone();
    let path = progress_path.clone();
    let step = step.to_string();
    tokio::task::spawn(async move {
        let f = tokio::fs::File::open(&path).await.unwrap();

        let reader = tokio::io::BufReader::new(f);
        let mut lines = reader.lines();

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let count_raw = match line.split_once('=') {
                        Some((a, b)) if a == "frame" => b,
                        _ => continue,
                    };

                    if let Ok(frame) = count_raw.parse::<u64>() {
                        let mut msg = format!("Frame {frame}");

                        let progress_percent = if let Some(count) = &frame_count {
                            write!(&mut msg, "/{count}").unwrap();
                            let percent =
                                (((frame as f64) / (*count as f64)) * 100.0).floor() as u8;
                            Some(percent)
                        } else {
                            None
                        };

                        let res = jobs.job_update(
                            job_id,
                            semantic_core::api::JobStatus::Running {
                                step: Some(step.clone()),
                                progress_percent,
                                progress_message: Some(msg),
                            },
                        );
                        if res.is_err() {
                            // Stop because job seems to have disappeared.
                            break;
                        }
                    }
                }
                Ok(None) => {
                    // Wait a bit for more output.
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
                Err(error) => {
                    tracing::warn!(path=%path.display(), %error, "ffmpeg output monitor has failed");
                }
            }
        }
    })
}

#[derive(Clone, Debug)]
pub struct VideoInfo {
    pub duration: std::time::Duration,
    pub has_audio: bool,
}

#[derive(Clone)]
pub struct SharedBinarData(Arc<Vec<u8>>);

impl SharedBinarData {
    pub fn new(data: Vec<u8>) -> Self {
        Self(Arc::new(data))
    }

    pub fn try_into_owned(self) -> Option<Vec<u8>> {
        Arc::try_unwrap(self.0).ok()
    }
}

impl AsRef<[u8]> for SharedBinarData {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

// TODO: detect sound presence with minimum decible level
// currently just checks if an audio stream is present, but videos often have
// an audio stream that contains no hearable audio.
pub async fn analyze_video<R>(reader: R) -> Result<VideoInfo, anyhow::Error>
where
    R: std::io::Read + Send + 'static,
{
    let out = tokio::task::spawn_blocking(move || -> Result<ffprobe::FfProbe, anyhow::Error> {
        let mut cmd = std::process::Command::new("ffprobe");
        cmd.args(&[
            "-v",
            "quiet",
            // "-count_frames",
            "-show_format",
            "-show_streams",
            "-print_format",
            "json",
        ]);
        cmd.arg("-").stdin(Stdio::piped()).stdout(Stdio::piped());

        let mut proc = cmd.spawn().context("could not start ffprobe")?;
        let mut stdin = proc.stdin.take().unwrap();
        let mut stdout = proc.stdout.take().unwrap();
        let mut reader = reader;

        let res = std::io::copy(&mut reader, &mut stdin);
        match res {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => {
                // Ignore broken pipe errors because ffprobe often only needs
                // part of a video (unless count_frames is specified)
            }
            Err(err) => {
                bail!("could not write video to ffprobe: {err}");
            }
        }
        stdin.flush()?;
        std::mem::drop(stdin);

        let mut buf = Vec::new();
        stdout
            .read_to_end(&mut buf)
            .context("Could not read ffprobe output")?;

        let status = proc.wait().context("ffprobe failed")?;
        if !status.success() {
            bail!("ffprobe failed with status {status}");
        }

        let info: ffprobe::FfProbe =
            serde_json::from_slice(&buf).context("could not parse ffprobe json output")?;

        Ok(info)
    })
    .await??;

    let video_stream = out
        .streams
        .iter()
        .find(|s| s.codec_type.clone().unwrap_or_default() == "video")
        .ok_or_else(|| anyhow::anyhow!("could not detect video stream"))?;
    let audio_stream = out
        .streams
        .iter()
        .find(|s| s.codec_type.clone().unwrap_or_default() == "audio");

    let duration = out
        .format
        .duration
        .as_ref()
        .or(video_stream.duration.as_ref())
        .and_then(|d| d.parse::<f64>().ok())
        .map(|secs| std::time::Duration::from_secs(secs as u64))
        .ok_or_else(|| anyhow!("could not determine video duration"))?;
    let has_audio = audio_stream.is_some();

    Ok(VideoInfo {
        duration,
        has_audio,
    })
}

pub async fn analyze_files(db: Db, blob: DynBlobStore, force: bool) -> Result<(), anyhow::Error> {
    let span = tracing::debug_span!("media file analysis");
    let _guard = span.enter();

    tracing::trace!("loading files for media analysis...");

    // FIXME: pagination...
    // TODO: images, audio files, ...
    let files = db
        .select(Select::new().with_filter(Expr::is_entity::<Video>()))
        .await?;

    let total = files.items.len();
    span.record("count", &total);

    tracing::info!(file_count=%total, "starting analysis");
    for (index, item) in files.items.into_iter().enumerate() {
        let file_id = item.data.get_id().unwrap();
        let typed = match TypedFile::from_map(item.data) {
            Ok(t) => t,
            Err(error) => {
                tracing::trace!(%file_id, ?error, "could not analyse file");
                continue;
            }
        };

        let res = match typed {
            TypedFile::Video(video) => video_analyze_and_update(&db, &blob, video, force)
                .await
                .map(|opt| opt.map(TypedFile::Video)),
            TypedFile::Image(_) => todo!(),
            TypedFile::File(_) => todo!(),
        };

        span.record("complete", &(index + 1));
        match res {
            Ok(Some(_)) => {
                tracing::debug!(%file_id, "file analyzed ({}/{})", index + 1, total);
            }
            Ok(None) => {
                tracing::trace!(%file_id, "file analysis skipped ({}/{})", index + 1, total);
            }
            Err(error) => {
                tracing::warn!(%file_id, ?error, "file analysis failed ({}/{})", index + 1, total);
            }
        }
    }

    tracing::info!("file analysis complete");

    Ok(())
}

async fn video_analyze_and_update(
    db: &Db,
    blob: &DynBlobStore,
    video: Video,
    force: bool,
) -> Result<Option<Video>, anyhow::Error> {
    use factordb::prelude::{AttributeDescriptor, EntityContainer};

    if video.duration.is_some() && !force {
        // If duration is already set, assume that video was already analysed
        // and there is nothing to do. Overwritten by force argument.
        return Ok(None);
    }

    let blob_uri = video
        .file
        .blob_uri
        .clone()
        .ok_or_else(|| anyhow!("Video does not have a blob"))?;
    let reader = blob
        .get_std_reader(&blob_uri)
        .await
        .context("Could not obtaing blob reader for video")?;

    let info = analyze_video(reader)
        .await
        .context("could not analyse video")?;

    let patch = Patch::new()
        .replace(AttrDuration::QUALIFIED_NAME, info.duration.as_secs())
        .replace(AttrVideoHasSound::QUALIFIED_NAME, info.has_audio);

    db.patch(video.id(), patch)
        .await
        .context("could not persist video information to db")?;

    Ok(Some(video))
}

async fn try_optimise_video(
    db: &Db,
    store: DynBlobStore,
    job_manager: JobManager,
    video: Video,
    tmp_dir: PathBuf,
    job: Job,
) -> Result<String, anyhow::Error> {
    let step_fetch = "fetch video".to_string();
    let step_pass1 = "encode (pass 1)".to_string();
    let step_pass2 = "encode (pass 2)".to_string();
    let step_store = "store video".to_string();

    job_manager.job_add_steps(
        job.id,
        vec![
            step_fetch.clone(),
            step_pass1.clone(),
            step_pass2.clone(),
            step_store.clone(),
        ],
    )?;

    job_manager.job_update(
        job.id,
        semantic_core::api::JobStatus::Running {
            step: Some(step_fetch.clone()),
            progress_message: None,
            progress_percent: None,
        },
    )?;

    let filename = video
        .file
        .filename
        .ok_or_else(|| anyhow!("Video does not have a filename"))?;
    let blob_path = video
        .file
        .blob_uri
        .ok_or_else(|| anyhow!("Video does not have an attached blob"))?;

    // Save file to tmp dir.
    let id = uuid::Uuid::new_v4();
    let dir = tmp_dir.join(id.to_string());
    tokio::fs::create_dir_all(&dir).await?;

    let ext = PathBuf::from(&filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();

    let input_path = dir.join(format!("input{ext}"));

    let blob_info = store
        .get_meta(&blob_path)
        .await?
        .ok_or_else(|| anyhow!("Blob path not found: '{blob_path}"))?;
    let mut reader = store.get_std_reader(&blob_path).await?;
    let read_path = input_path.clone();

    // Write file out to disk and run ffprobe.
    let meta = tokio::task::spawn_blocking(move || -> Result<ffprobe::FfProbe, anyhow::Error> {
        {
            tracing::trace!(path=?read_path, "writing temporary video file");
            let mut file = std::fs::File::create(&read_path)?;
            std::io::copy(&mut reader, &mut file)?;
            file.flush()?;

            let size_actual = std::fs::metadata(&read_path)?.len() as u64;
            let size_expected = blob_info.size;

            if size_actual as u64 != size_expected {
                bail!("Could not write file to disk at {blob_path}: Expected size {size_expected} , actual is {size_actual}")
            }

            std::mem::drop(file);
        }

        let meta = ffprobe::ConfigBuilder::new()
            .count_frames(true)
            .run(&read_path)
            .map_err(|err| err)
            .context("Could not run ffprobe")?;
        Ok(meta)
    })
    .await??;

    // Try to figure out appropriate webm quality.
    // Use recommended CRF (constant frame rate) settings from Google
    // See https://developers.google.com/media/vp9/settings/vod

    // Try to find video stream.

    let video_streams = meta
        .streams
        .into_iter()
        .filter_map(|stream| {
            if stream.codec_type.unwrap_or_default() != "video" {
                return None;
            }
            let width = stream.width?;
            let height = stream.height?;
            let frames = stream.nb_read_frames.and_then(|x| x.parse::<u64>().ok());
            Some((width, height, frames))
        })
        .collect::<Vec<_>>();

    let (_width, height, frame_count) = match video_streams.len() {
        0 => bail!("Could not detect any video streams"),
        1 => video_streams.into_iter().next().unwrap(),
        x if x > 1 => bail!("Detected multiple video streams, cannot determine primary"),
        _other => bail!("Invalid video stream count"),
    };

    let (crf, _tiles, _threads, speed_first, speed_second) = match height {
        x if x <= 240 => (37, 1, 2, 4, 1),
        x if x <= 360 => (36, 2, 4, 4, 1),
        x if x <= 480 => (33, 1, 4, 4, 1),
        x if x <= 720 => (32, 2, 8, 4, 2),
        x if x <= 1080 => (31, 2, 8, 4, 2),
        x if x <= 1440 => (24, 3, 16, 4, 2),
        x if x <= 2160 => (15, 3, 16, 4, 2),
        other => bail!("Invalid video height {other}"),
    };

    let progress_path = dir.join("progress.txt");
    // Make sure progress file exists.
    {
        tokio::fs::File::create(&progress_path).await?
    };

    let common_args: Vec<String> = vec![
        // Input file
        "-i".to_string(),
        // TODO: no unwrap
        input_path.to_str().unwrap().to_string(),
        // Don't ask if output file should be overwritten.
        "-y".to_string(),
        "-progress".to_string(),
        // TODO: no unwrap
        progress_path.to_str().unwrap().to_string(),
        // Auto-detect bitrate
        "-b:v".to_string(),
        "0".to_string(),
        // quality (constant rate factor)
        "-crf".to_string(),
        crf.to_string(),
    ];

    let output_path = dir.join(format!("out{ext}")).with_extension("webm");

    // Run first pass.

    tracing::trace!(job_id=%job.id, frames=?frame_count, "Starting video conversion");
    job_manager.job_update(
        job.id,
        semantic_core::api::JobStatus::Running {
            step: Some(step_pass1.clone()),
            progress_message: None,
            progress_percent: None,
        },
    )?;
    let mut proc = tokio::process::Command::new("ffmpeg")
        .current_dir(&dir)
        .args(&common_args)
        .args(&["-pass", "1"])
        .args(&["-quality", "good"])
        .args(&["-speed", &speed_first.to_string()])
        // Auto-select streams
        .arg("-an")
        .args(&["-f", "webm"])
        .arg(&output_path)
        .spawn()
        .context("Could not spawn ffmpeg")?;

    let progress_handle = spawn_ffmpeg_output_monitor(
        &job_manager,
        &progress_path,
        frame_count,
        &step_pass1,
        job.id,
    );
    let res = proc.wait().await.context("ffmpeg failed")?;
    progress_handle.abort();

    if !res.success() {
        bail!("FFMPEG failed");
    }

    // Run second pass.

    // Truncate progress log to not have any garbage.
    {
        tokio::fs::OpenOptions::new()
            .write(true)
            .open(&progress_path)
            .await?
            .set_len(0)
            .await?;
    }

    job_manager.job_update(
        job.id,
        semantic_core::api::JobStatus::Running {
            step: Some(step_pass2.clone()),
            progress_message: None,
            progress_percent: None,
        },
    )?;
    let mut proc = tokio::process::Command::new("ffmpeg")
        .current_dir(&dir)
        .args(&common_args)
        .args(&["-pass", "2"])
        .args(&["-quality", "good"])
        .args(&["-speed", &speed_second.to_string()])
        .arg(&output_path)
        .spawn()
        .context("Could not spawn ffmpeg")?;

    let progress_handle = spawn_ffmpeg_output_monitor(
        &job_manager,
        &progress_path,
        frame_count,
        &step_pass2,
        job.id,
    );
    let res = proc.wait().await.context("ffmpeg failed")?;
    progress_handle.abort();
    if !res.success() {
        bail!("FFMPEG failed");
    }

    let mut filename = PathBuf::from(&filename);
    filename.set_extension("webm");
    let converted_blob_path = format!(
        "files/optimised/{}/{}",
        video.file.id,
        filename.to_string_lossy()
    );

    let mut writer = store.put_std_writer(&converted_blob_path).await?;

    job_manager.job_update(
        job.id,
        semantic_core::api::JobStatus::Running {
            step: Some(step_store.clone()),
            progress_message: None,
            progress_percent: None,
        },
    )?;

    // Upload file to blob store.
    let converted_file_size = tokio::task::spawn_blocking(move || -> Result<u64, anyhow::Error> {
        let mut f = std::fs::File::open(&output_path)?;
        let size = f.metadata()?.len() as u64;
        std::io::copy(&mut f, &mut writer)?;

        writer.flush()?;

        Ok(size)
    })
    .await??;

    // Ensure blob was correctly upbloaded.
    let converted_blob_info = store
        .get_meta(&converted_blob_path)
        .await?
        .ok_or_else(|| anyhow!("Could not retrieve metadata for new converted blob"))?;
    if converted_file_size != converted_blob_info.size {
        store.remove(&converted_blob_path).await.ok();
        bail!(
            "Blob upload failed: expected blob size to be {converted_file_size}, but it is {}",
            converted_blob_info.size
        );
    }

    // Update db entity with web blob path.
    let mut map = DataMap::new();
    map.insert_attr::<AttrBlobUriWeb>(converted_blob_path.clone());
    db.merge(video.file.id, map).await?;

    job_manager.job_update(
        job.id,
        semantic_core::api::JobStatus::Finished {
            result: Ok("Video converted".to_string()),
        },
    )?;

    // Delete tmp dir.
    if let Err(error) = tokio::fs::remove_dir_all(&dir).await {
        tracing::warn!(path=%dir.display(), %error, "could not delete video conversion temp directory");
    }

    Ok(converted_blob_path)
}

pub async fn optimise_video(
    db: Db,
    store: DynBlobStore,
    job_manager: JobManager,
    video: Video,
    tmp_dir: PathBuf,
    job: Job,
) -> Result<String, anyhow::Error> {
    let res =
        try_optimise_video(&db, store, job_manager.clone(), video, tmp_dir, job.clone()).await;

    let status = match &res {
        Ok(_) => semantic_core::api::JobStatus::Finished {
            result: Ok("Video was converted".to_string()),
        },
        Err(err) => semantic_core::api::JobStatus::Finished {
            result: Err(ApiError {
                message: err.to_string(),
                code: None,
                details: None,
            }),
        },
    };

    if job_manager.job_update(job.id, status).is_err() {
        tracing::warn!(job_id =%job.id, "Could not update video encoding job status: job has disappeared");
    }

    res
}
