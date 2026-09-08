//! Stream-based media analysis primitives.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{Stream, StreamExt as _};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncWriteExt as _, BufWriter};
use tokio::process::Command;

pub mod mime;

pub type FileByteStream =
    Pin<Box<dyn Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static>>;

#[derive(Clone, Debug, Default)]
pub struct AnalyzerConfig {
    pub temp_dir: Option<PathBuf>,
}

impl AnalyzerConfig {
    pub fn with_temp_dir(temp_dir: impl Into<PathBuf>) -> Self {
        Self {
            temp_dir: Some(temp_dir.into()),
        }
    }
}

pub struct FileAnalysisInput {
    pub filename: Option<String>,
    pub declared_mime_type: Option<String>,
    pub stream: FileByteStream,
}

impl FileAnalysisInput {
    pub fn from_bytes(bytes: Bytes) -> Self {
        Self {
            filename: None,
            declared_mime_type: None,
            stream: Box::pin(futures_util::stream::once(async move { Ok(bytes) })),
        }
    }

    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    pub fn with_declared_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.declared_mime_type = Some(mime_type.into());
        self
    }

    pub async fn detect_mime_type(
        mut self,
    ) -> std::result::Result<(Option<String>, Self), MediaAnalysisError> {
        const MAX_SNIFF_BYTES: usize = 8 * 1024;

        let mut buffered_chunks = Vec::new();
        let mut sniffed_bytes = Vec::with_capacity(MAX_SNIFF_BYTES);
        let mut detected = None;
        while sniffed_bytes.len() < MAX_SNIFF_BYTES {
            let Some(chunk) = self.stream.next().await else {
                break;
            };
            let chunk = chunk?;
            let remaining = MAX_SNIFF_BYTES - sniffed_bytes.len();
            sniffed_bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            buffered_chunks.push(chunk);
            detected = mime::detect_bytes(&sniffed_bytes);
            if detected.is_some() {
                break;
            }
        }
        self.stream = Box::pin(
            futures_util::stream::iter(buffered_chunks.into_iter().map(Ok)).chain(self.stream),
        );

        Ok((
            detected.or_else(|| mime::detect_bytes(&sniffed_bytes)),
            self,
        ))
    }

    pub async fn read_to_bytes(mut self) -> std::result::Result<Bytes, MediaAnalysisError> {
        let mut out = Vec::new();
        while let Some(chunk) = self.stream.next().await {
            out.extend_from_slice(&chunk?);
        }
        Ok(Bytes::from(out))
    }

    pub async fn write_to_temp_file(
        mut self,
        temp_dir: &Path,
    ) -> std::result::Result<TempMediaFile, MediaAnalysisError> {
        tokio::fs::create_dir_all(temp_dir).await?;
        let path = temp_dir.join(temp_file_name(self.filename.as_deref()));
        let mut file = tokio::fs::File::create(&path).await?;
        while let Some(chunk) = self.stream.next().await {
            file.write_all(&chunk?).await?;
        }
        file.flush().await?;
        Ok(TempMediaFile { path })
    }
}

#[derive(Debug)]
pub struct TempMediaFile {
    path: PathBuf,
}

impl TempMediaFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempMediaFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn temp_file_name(source_filename: Option<&str>) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let extension = source_filename.and_then(filename_extension);
    match extension {
        Some(extension) => format!("semantic-media-{}-{nanos}.{extension}", std::process::id()),
        None => format!("semantic-media-{}-{nanos}.tmp", std::process::id()),
    }
}

fn filename_extension(filename: &str) -> Option<&str> {
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(safe_extension)
}

fn safe_extension(extension: &str) -> Option<&str> {
    let extension = extension.trim().trim_start_matches('.');
    if !extension.is_empty()
        && extension.len() <= 16
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        Some(extension)
    } else {
        None
    }
}

#[async_trait]
pub trait FileAnalyzer: Send + Sync {
    fn name(&self) -> &'static str;

    async fn analyze(
        &self,
        input: FileAnalysisInput,
    ) -> std::result::Result<Option<FileAnalysis>, MediaAnalysisError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dimensions {
    pub width: u64,
    pub height: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageAnalysis {
    pub materialized_mime_type: Option<String>,
    pub dimensions: Dimensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoAnalysis {
    pub materialized_mime_type: Option<String>,
    pub duration: Duration,
    pub has_audio: bool,
    pub dimensions: Option<Dimensions>,
    pub frames_per_second: Option<f64>,
    pub frame_count: Option<u64>,
    pub bitrate: Option<u64>,
    pub video_bitrate: Option<u64>,
    pub audio_bitrate: Option<u64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u64>,
    pub audio_sample_rate: Option<u64>,
    pub container_format: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioAnalysis {
    pub materialized_mime_type: Option<String>,
    pub duration: Duration,
    pub bitrate: Option<u64>,
    pub audio_bitrate: Option<u64>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u64>,
    pub audio_sample_rate: Option<u64>,
    pub container_format: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileAnalysis {
    Image(ImageAnalysis),
    Video(VideoAnalysis),
    Audio(AudioAnalysis),
}

impl FileAnalysis {
    pub fn materialized_mime_type(&self) -> Option<&str> {
        match self {
            Self::Image(image) => image.materialized_mime_type.as_deref(),
            Self::Video(video) => video.materialized_mime_type.as_deref(),
            Self::Audio(audio) => audio.materialized_mime_type.as_deref(),
        }
    }

    pub fn dimensions(&self) -> Option<&Dimensions> {
        match self {
            Self::Image(image) => Some(&image.dimensions),
            Self::Video(video) => video.dimensions.as_ref(),
            Self::Audio(_) => None,
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        match self {
            Self::Image(_) => None,
            Self::Video(video) => Some(video.duration),
            Self::Audio(audio) => Some(audio.duration),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MediaAnalysisError {
    #[error("media analysis requires a configured temporary directory")]
    TempDirRequired,

    #[error("could not detect a {kind} stream")]
    MissingStream { kind: &'static str },

    #[error("could not determine {kind} duration")]
    MissingDuration { kind: &'static str },

    #[error("media stream IO failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("blocking media analysis task failed: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[cfg(feature = "image")]
    #[error("image analysis failed: {0}")]
    Image(#[from] image::ImageError),

    #[error("ffprobe failed: {0}")]
    Ffprobe(#[from] ffprobe::FfProbeError),
}

#[cfg(feature = "image")]
#[derive(Clone, Debug, Default)]
pub struct ImageAnalyzer;

#[cfg(feature = "image")]
impl ImageAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "image")]
#[async_trait]
impl FileAnalyzer for ImageAnalyzer {
    fn name(&self) -> &'static str {
        "image-rs"
    }

    async fn analyze(
        &self,
        input: FileAnalysisInput,
    ) -> std::result::Result<Option<FileAnalysis>, MediaAnalysisError> {
        let declared_mime_type = input.declared_mime_type.clone();
        let bytes = input.read_to_bytes().await?;
        let mime_analysis = mime::analyze_bytes(bytes.as_ref(), declared_mime_type.as_deref());
        if !mime::is_image(mime_analysis.best_effort()) {
            return Ok(None);
        }
        let mime_type = mime_analysis.best_effort().map(ToOwned::to_owned);

        tokio::task::spawn_blocking(move || {
            let reader =
                image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
            let dimensions = reader.into_dimensions()?;
            Ok(Some(FileAnalysis::Image(ImageAnalysis {
                materialized_mime_type: mime_type,
                dimensions: Dimensions {
                    width: dimensions.0.into(),
                    height: dimensions.1.into(),
                },
            })))
        })
        .await?
    }
}

#[derive(Clone, Debug, Default)]
pub struct FfprobeAnalyzer {
    config: AnalyzerConfig,
    ffprobe_bin: Option<PathBuf>,
}

impl FfprobeAnalyzer {
    pub fn new(config: AnalyzerConfig) -> Self {
        Self {
            config,
            ffprobe_bin: None,
        }
    }

    pub fn with_ffprobe_bin(mut self, ffprobe_bin: impl Into<PathBuf>) -> Self {
        self.ffprobe_bin = Some(ffprobe_bin.into());
        self
    }
}

#[async_trait]
impl FileAnalyzer for FfprobeAnalyzer {
    fn name(&self) -> &'static str {
        "ffprobe"
    }

    async fn analyze(
        &self,
        input: FileAnalysisInput,
    ) -> std::result::Result<Option<FileAnalysis>, MediaAnalysisError> {
        let declared_mime_type = input.declared_mime_type.clone();
        if declared_mime_type.is_some()
            && !mime::is_video(declared_mime_type.as_deref())
            && !mime::is_audio(declared_mime_type.as_deref())
        {
            return Ok(None);
        }

        let probe = match self.config.temp_dir.as_deref() {
            Some(temp_dir) => {
                let temp_file = input.write_to_temp_file(temp_dir).await?;
                let path = temp_file.path().to_path_buf();
                let ffprobe_bin = self.ffprobe_bin.clone();

                tokio::task::spawn_blocking(move || {
                    let mut builder = ffprobe::Config::builder();
                    if let Some(ffprobe_bin) = ffprobe_bin {
                        builder = builder.ffprobe_bin(ffprobe_bin);
                    }
                    builder.run(path)
                })
                .await??
            }
            None => ffprobe_stream(input, self.ffprobe_bin.as_deref()).await?,
        };

        let has_audio = has_stream(&probe, "audio");
        let has_video = has_stream(&probe, "video");
        if mime::is_audio(declared_mime_type.as_deref()) && has_audio {
            // Prefer audio for audio containers that also expose cover art as a video stream.
            Ok(Some(FileAnalysis::Audio(audio_analysis(
                probe,
                declared_mime_type.as_deref(),
            )?)))
        } else if has_video {
            Ok(Some(FileAnalysis::Video(video_analysis(
                probe,
                declared_mime_type.as_deref(),
            )?)))
        } else if has_audio {
            // Header-only MIME detection identifies containers such as WebM as video even
            // when they contain audio only. The probed streams are authoritative here.
            Ok(Some(FileAnalysis::Audio(audio_analysis(
                probe,
                declared_mime_type.as_deref(),
            )?)))
        } else {
            Ok(None)
        }
    }
}

async fn ffprobe_stream(
    mut input: FileAnalysisInput,
    ffprobe_bin: Option<&Path>,
) -> std::result::Result<ffprobe::FfProbe, MediaAnalysisError> {
    let input_format = ffprobe_input_format(
        input.declared_mime_type.as_deref(),
        input.filename.as_deref(),
    );
    let mut command = Command::new(ffprobe_bin.unwrap_or_else(|| Path::new("ffprobe")));
    command
        .args([
            "-v",
            "quiet",
            "-show_format",
            "-show_streams",
            "-print_format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(input_format) = input_format {
        command.args(["-f", input_format]);
    }
    command.arg("pipe:0");

    let mut child = command.spawn().map_err(ffprobe::FfProbeError::Io)?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("could not open ffprobe stdin"))?;
    let writer = tokio::spawn(async move {
        let mut stdin = BufWriter::new(stdin);
        while let Some(chunk) = input.stream.next().await {
            stdin.write_all(&chunk?).await?;
        }
        stdin.shutdown().await?;
        Ok::<(), MediaAnalysisError>(())
    });

    let output = child
        .wait_with_output()
        .await
        .map_err(ffprobe::FfProbeError::Io)?;
    let writer_result = writer.await?;
    if !output.status.success() {
        return Err(ffprobe::FfProbeError::Status(output).into());
    }
    writer_result?;

    serde_json::from_slice(&output.stdout)
        .map_err(ffprobe::FfProbeError::Deserialize)
        .map_err(MediaAnalysisError::from)
}

fn ffprobe_input_format(mime_type: Option<&str>, filename: Option<&str>) -> Option<&'static str> {
    let mime_type = mime::normalize_declared(mime_type);
    match mime_type.as_deref() {
        Some("video/mp4" | "audio/mp4" | "video/quicktime" | "video/3gpp" | "audio/3gpp") => {
            Some("mov")
        }
        Some("video/webm" | "audio/webm") => Some("matroska,webm"),
        Some("video/x-matroska" | "audio/x-matroska") => Some("matroska"),
        Some("video/ogg" | "audio/ogg" | "application/ogg") => Some("ogg"),
        Some("video/x-msvideo") => Some("avi"),
        Some("video/x-flv") => Some("flv"),
        Some("video/mp2t") => Some("mpegts"),
        Some("audio/mpeg") => Some("mp3"),
        Some("audio/wav" | "audio/x-wav") => Some("wav"),
        Some("audio/flac") => Some("flac"),
        Some("audio/aac") => Some("aac"),
        Some(_) => None,
        None => ffprobe_input_format_from_filename(filename),
    }
}

fn ffprobe_input_format_from_filename(filename: Option<&str>) -> Option<&'static str> {
    match filename
        .and_then(filename_extension)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4" | "m4a" | "m4v" | "mov" | "3gp" | "3g2") => Some("mov"),
        Some("webm") => Some("matroska,webm"),
        Some("mkv" | "mka") => Some("matroska"),
        Some("ogg" | "ogv" | "oga") => Some("ogg"),
        Some("avi") => Some("avi"),
        Some("flv") => Some("flv"),
        Some("ts" | "mts" | "m2ts") => Some("mpegts"),
        Some("mp3") => Some("mp3"),
        Some("wav") => Some("wav"),
        Some("flac") => Some("flac"),
        Some("aac") => Some("aac"),
        _ => None,
    }
}

fn has_stream(probe: &ffprobe::FfProbe, kind: &'static str) -> bool {
    probe
        .streams
        .iter()
        .any(|stream| stream.codec_type.as_deref() == Some(kind))
}

fn video_analysis(
    probe: ffprobe::FfProbe,
    input_mime_type: Option<&str>,
) -> std::result::Result<VideoAnalysis, MediaAnalysisError> {
    let video_stream = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or(MediaAnalysisError::MissingStream { kind: "video" })?;
    let audio_stream = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));

    let duration = probe
        .format
        .duration
        .as_ref()
        .or(video_stream.duration.as_ref())
        .and_then(|duration| duration.parse::<f64>().ok())
        .map(duration_from_secs_f64)
        .ok_or(MediaAnalysisError::MissingDuration { kind: "video" })?;

    let mime_type = materialized_mime_type(
        MediaKind::Video,
        input_mime_type,
        Some(probe.format.format_name.as_str()),
    );
    Ok(VideoAnalysis {
        materialized_mime_type: mime_type,
        duration,
        has_audio: audio_stream.is_some(),
        dimensions: dimensions(video_stream),
        frames_per_second: frames_per_second(video_stream),
        frame_count: parse_u64(video_stream.nb_frames.as_deref()),
        bitrate: parse_u64(probe.format.bit_rate.as_deref()),
        video_bitrate: parse_u64(video_stream.bit_rate.as_deref()),
        audio_bitrate: audio_stream.and_then(|stream| parse_u64(stream.bit_rate.as_deref())),
        video_codec: video_stream.codec_name.clone(),
        audio_codec: audio_stream.and_then(|stream| stream.codec_name.clone()),
        audio_channels: audio_stream.and_then(|stream| stream.channels?.try_into().ok()),
        audio_sample_rate: audio_stream.and_then(|stream| parse_u64(stream.sample_rate.as_deref())),
        container_format: Some(probe.format.format_name).filter(|value| !value.is_empty()),
    })
}

fn audio_analysis(
    probe: ffprobe::FfProbe,
    input_mime_type: Option<&str>,
) -> std::result::Result<AudioAnalysis, MediaAnalysisError> {
    let audio_stream = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"))
        .ok_or(MediaAnalysisError::MissingStream { kind: "audio" })?;

    let duration = probe
        .format
        .duration
        .as_ref()
        .or(audio_stream.duration.as_ref())
        .and_then(|duration| duration.parse::<f64>().ok())
        .map(duration_from_secs_f64)
        .ok_or(MediaAnalysisError::MissingDuration { kind: "audio" })?;

    let mime_type = materialized_mime_type(
        MediaKind::Audio,
        input_mime_type,
        Some(probe.format.format_name.as_str()),
    );
    Ok(AudioAnalysis {
        materialized_mime_type: mime_type,
        duration,
        bitrate: parse_u64(probe.format.bit_rate.as_deref()),
        audio_bitrate: parse_u64(audio_stream.bit_rate.as_deref()),
        audio_codec: audio_stream.codec_name.clone(),
        audio_channels: audio_stream
            .channels
            .and_then(|channels| channels.try_into().ok()),
        audio_sample_rate: parse_u64(audio_stream.sample_rate.as_deref()),
        container_format: Some(probe.format.format_name).filter(|value| !value.is_empty()),
    })
}

#[derive(Clone, Copy)]
enum MediaKind {
    Audio,
    Video,
}

fn materialized_mime_type(
    kind: MediaKind,
    input_mime_type: Option<&str>,
    container_format: Option<&str>,
) -> Option<String> {
    let input_mime_type = mime::normalize_declared(input_mime_type);
    let input_matches_kind = match kind {
        MediaKind::Audio => mime::is_audio(input_mime_type.as_deref()),
        MediaKind::Video => mime::is_video(input_mime_type.as_deref()),
    };
    if input_matches_kind {
        return input_mime_type;
    }

    let formats = container_format?.split(',').collect::<Vec<_>>();
    let contains = |format: &str| formats.contains(&format);
    match kind {
        MediaKind::Audio if contains("webm") => Some("audio/webm".to_string()),
        MediaKind::Audio if contains("matroska") => Some("audio/x-matroska".to_string()),
        MediaKind::Audio if contains("mp3") => Some("audio/mpeg".to_string()),
        MediaKind::Audio if contains("ogg") => Some("audio/ogg".to_string()),
        MediaKind::Audio if contains("wav") => Some("audio/wav".to_string()),
        MediaKind::Audio if contains("flac") => Some("audio/flac".to_string()),
        MediaKind::Audio if contains("aac") => Some("audio/aac".to_string()),
        MediaKind::Audio if contains("mp4") || contains("m4a") => Some("audio/mp4".to_string()),
        MediaKind::Video if contains("webm") => Some("video/webm".to_string()),
        MediaKind::Video if contains("matroska") => Some("video/x-matroska".to_string()),
        MediaKind::Video if contains("mp4") => Some("video/mp4".to_string()),
        _ => None,
    }
}

fn dimensions(stream: &ffprobe::Stream) -> Option<Dimensions> {
    let width = stream.width.and_then(|width| width.try_into().ok());
    let height = stream.height.and_then(|height| height.try_into().ok());
    match (width, height) {
        (Some(width), Some(height)) => Some(Dimensions { width, height }),
        _ => None,
    }
}

fn duration_from_secs_f64(secs: f64) -> Duration {
    if secs.is_finite() && secs > 0.0 {
        Duration::from_secs_f64(secs)
    } else {
        Duration::ZERO
    }
}

fn frames_per_second(stream: &ffprobe::Stream) -> Option<f64> {
    parse_rational(&stream.avg_frame_rate).or_else(|| parse_rational(&stream.r_frame_rate))
}

fn parse_rational(value: &str) -> Option<f64> {
    let value = value.trim();
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator = numerator.trim().parse::<f64>().ok()?;
        let denominator = denominator.trim().parse::<f64>().ok()?;
        if denominator == 0.0 {
            return None;
        }
        let out = numerator / denominator;
        return (out.is_finite() && out > 0.0).then_some(out);
    }
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn parse_u64(value: Option<&str>) -> Option<u64> {
    value?.trim().parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_FAKE_FFPROBE_ID: AtomicU64 = AtomicU64::new(0);

    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5,
        0x1c, 0x0c, 0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xfa,
        0xcf, 0x00, 0x00, 0x02, 0x07, 0x01, 0x02, 0x9a, 0x1c, 0x31, 0x71, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn mime_detection_prefers_detected_bytes() {
        let analysis =
            mime::analyze_bytes(PNG_1X1, Some("application/octet-stream; charset=utf-8"));

        assert_eq!(
            analysis.declared.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(analysis.detected.as_deref(), Some("image/png"));
        assert_eq!(analysis.best_effort(), Some("image/png"));
    }

    #[test]
    fn temp_file_name_preserves_safe_source_extension() {
        assert!(temp_file_name(Some("clip.mp4")).ends_with(".mp4"));
        assert!(temp_file_name(Some("clip.tar.gz")).ends_with(".gz"));
        assert!(temp_file_name(Some("clip.bad-extension")).ends_with(".tmp"));
        assert!(temp_file_name(None).ends_with(".tmp"));
    }

    #[tokio::test]
    async fn stream_detection_preserves_all_input_bytes() {
        let chunks = vec![
            Ok(Bytes::copy_from_slice(&PNG_1X1[..4])),
            Ok(Bytes::copy_from_slice(&PNG_1X1[4..16])),
            Ok(Bytes::copy_from_slice(&PNG_1X1[16..])),
        ];
        let input = FileAnalysisInput {
            filename: None,
            declared_mime_type: None,
            stream: Box::pin(futures_util::stream::iter(chunks)),
        };

        let (detected, input) = input.detect_mime_type().await.unwrap();

        assert_eq!(detected.as_deref(), Some("image/png"));
        assert_eq!(input.read_to_bytes().await.unwrap().as_ref(), PNG_1X1);
    }

    #[cfg(feature = "image")]
    #[tokio::test]
    async fn image_analyzer_reads_dimensions_from_stream() {
        let input = FileAnalysisInput::from_bytes(Bytes::from_static(PNG_1X1));

        let analysis = ImageAnalyzer::new().analyze(input).await.unwrap().unwrap();

        assert_eq!(
            analysis,
            FileAnalysis::Image(ImageAnalysis {
                materialized_mime_type: Some("image/png".to_string()),
                dimensions: Dimensions {
                    width: 1,
                    height: 1,
                },
            })
        );
    }

    #[test]
    fn ffprobe_input_format_prefers_mime_type_then_filename() {
        assert_eq!(
            ffprobe_input_format(Some("video/mp4; codecs=avc1"), Some("clip.webm")),
            Some("mov")
        );
        assert_eq!(
            ffprobe_input_format(None, Some("clip.WEBM")),
            Some("matroska,webm")
        );
        assert_eq!(ffprobe_input_format(Some("video/mpeg"), None), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ffprobe_analyzer_streams_to_stdin_without_temp_dir() {
        let script = fake_ffprobe(
            "case \" $* \" in *\" -f mov \"*) ;; *) exit 41 ;; esac\n\
             [ \"$(cat)\" = \"video bytes\" ] || exit 42",
        );

        let input = FileAnalysisInput::from_bytes(Bytes::from_static(b"video bytes"))
            .with_filename("clip.mp4")
            .with_declared_mime_type("video/mp4");
        let analysis = FfprobeAnalyzer::new(AnalyzerConfig::default())
            .with_ffprobe_bin(script.path())
            .analyze(input)
            .await
            .unwrap();

        assert_eq!(
            analysis,
            Some(FileAnalysis::Video(
                video_analysis(fake_video_probe(), Some("video/mp4")).unwrap()
            ))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ffprobe_uses_audio_stream_for_audio_only_webm() {
        let probe = fake_audio_probe();
        let script = fake_ffprobe_with_probe(
            "case \" $* \" in *\" -f matroska,webm \"*) ;; *) exit 41 ;; esac\n\
             [ \"$(cat)\" = \"audio bytes\" ] || exit 42",
            &probe,
        );
        let expected = FileAnalysis::Audio(audio_analysis(probe, Some("video/webm")).unwrap());
        let input = FileAnalysisInput::from_bytes(Bytes::from_static(b"audio bytes"))
            .with_filename("recording.webm")
            .with_declared_mime_type("video/webm");

        let analysis = FfprobeAnalyzer::new(AnalyzerConfig::default())
            .with_ffprobe_bin(script.path())
            .analyze(input)
            .await
            .unwrap();

        assert_eq!(analysis, Some(expected));
        assert_eq!(
            analysis
                .as_ref()
                .and_then(FileAnalysis::materialized_mime_type),
            Some("audio/webm")
        );
        assert_eq!(
            analysis.and_then(|analysis| analysis.duration()),
            Some(Duration::from_millis(120))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ffprobe_analyzer_uses_temp_file_when_configured() {
        let script = fake_ffprobe(
            "case \" $* \" in *\" pipe:0 \"*) exit 41 ;; esac\n\
             for argument in \"$@\"; do input_path=$argument; done\n\
             [ \"$(cat \"$input_path\")\" = \"video bytes\" ] || exit 42",
        );
        let input = FileAnalysisInput::from_bytes(Bytes::from_static(b"video bytes"))
            .with_filename("clip.mp4")
            .with_declared_mime_type("video/mp4");

        let analysis = FfprobeAnalyzer::new(AnalyzerConfig::with_temp_dir(std::env::temp_dir()))
            .with_ffprobe_bin(script.path())
            .analyze(input)
            .await
            .unwrap();

        assert_eq!(
            analysis,
            Some(FileAnalysis::Video(
                video_analysis(fake_video_probe(), Some("video/mp4")).unwrap()
            ))
        );
    }

    #[cfg(unix)]
    fn fake_ffprobe(assertions: &str) -> TempMediaFile {
        fake_ffprobe_with_probe(assertions, &fake_video_probe())
    }

    #[cfg(unix)]
    fn fake_ffprobe_with_probe(assertions: &str, probe: &ffprobe::FfProbe) -> TempMediaFile {
        use std::os::unix::fs::PermissionsExt as _;

        let probe_json = serde_json::to_string(probe).unwrap();
        let script = TempMediaFile {
            path: std::env::temp_dir().join(format!(
                "semantic-media-fake-ffprobe-{}-{}",
                std::process::id(),
                NEXT_FAKE_FFPROBE_ID.fetch_add(1, Ordering::Relaxed)
            )),
        };
        std::fs::write(
            script.path(),
            format!("#!/bin/sh\n{assertions}\nprintf '%s' '{probe_json}'\n"),
        )
        .unwrap();
        std::fs::set_permissions(script.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        script
    }

    #[cfg(unix)]
    fn fake_video_probe() -> ffprobe::FfProbe {
        ffprobe::FfProbe {
            streams: vec![ffprobe::Stream {
                codec_type: Some("video".to_string()),
                codec_name: Some("h264".to_string()),
                width: Some(320),
                height: Some(240),
                avg_frame_rate: "24/1".to_string(),
                r_frame_rate: "24/1".to_string(),
                duration: Some("2.5".to_string()),
                ..ffprobe::Stream::default()
            }],
            format: ffprobe::Format {
                duration: Some("2.5".to_string()),
                format_name: "mov,mp4,m4a,3gp,3g2,mj2".to_string(),
                ..ffprobe::Format::default()
            },
        }
    }

    #[cfg(unix)]
    fn fake_audio_probe() -> ffprobe::FfProbe {
        ffprobe::FfProbe {
            streams: vec![ffprobe::Stream {
                codec_type: Some("audio".to_string()),
                codec_name: Some("opus".to_string()),
                duration: Some("0.12".to_string()),
                channels: Some(1),
                sample_rate: Some("48000".to_string()),
                ..ffprobe::Stream::default()
            }],
            format: ffprobe::Format {
                duration: Some("0.12".to_string()),
                format_name: "matroska,webm".to_string(),
                ..ffprobe::Format::default()
            },
        }
    }

    #[test]
    fn parses_ffprobe_rational_frame_rates() {
        assert_eq!(parse_rational("25/1"), Some(25.0));
        assert_eq!(parse_rational("30000/1001"), Some(30000.0 / 1001.0));
        assert_eq!(parse_rational("0/0"), None);
        assert_eq!(parse_rational("not-a-rate"), None);
    }

    #[test]
    fn video_analysis_maps_ffprobe_metadata() {
        let mut video_stream = ffprobe::Stream {
            codec_type: Some("video".to_string()),
            codec_name: Some("h264".to_string()),
            width: Some(1920),
            height: Some(1080),
            avg_frame_rate: "30000/1001".to_string(),
            r_frame_rate: "30/1".to_string(),
            nb_frames: Some("42".to_string()),
            bit_rate: Some("4000000".to_string()),
            ..ffprobe::Stream::default()
        };
        video_stream.duration = Some("12.5".to_string());
        let audio_stream = ffprobe::Stream {
            codec_type: Some("audio".to_string()),
            codec_name: Some("aac".to_string()),
            bit_rate: Some("128000".to_string()),
            channels: Some(2),
            sample_rate: Some("48000".to_string()),
            ..ffprobe::Stream::default()
        };
        let probe = ffprobe::FfProbe {
            streams: vec![video_stream, audio_stream],
            format: ffprobe::Format {
                duration: Some("12.5".to_string()),
                bit_rate: Some("4128000".to_string()),
                format_name: "mov,mp4,m4a,3gp,3g2,mj2".to_string(),
                ..ffprobe::Format::default()
            },
        };

        let analysis = video_analysis(probe, Some("video/mp4")).expect("video analysis");

        assert_eq!(analysis.duration, Duration::from_millis(12_500));
        assert_eq!(
            analysis.dimensions,
            Some(Dimensions {
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(analysis.frames_per_second, Some(30000.0 / 1001.0));
        assert_eq!(analysis.frame_count, Some(42));
        assert_eq!(analysis.bitrate, Some(4_128_000));
        assert_eq!(analysis.video_bitrate, Some(4_000_000));
        assert_eq!(analysis.audio_bitrate, Some(128_000));
        assert_eq!(analysis.video_codec.as_deref(), Some("h264"));
        assert_eq!(analysis.audio_codec.as_deref(), Some("aac"));
        assert_eq!(analysis.audio_channels, Some(2));
        assert_eq!(analysis.audio_sample_rate, Some(48_000));
        assert_eq!(
            analysis.container_format.as_deref(),
            Some("mov,mp4,m4a,3gp,3g2,mj2")
        );
    }
}
