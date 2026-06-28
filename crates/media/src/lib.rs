//! Stream-based media analysis primitives.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{Stream, StreamExt as _};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt as _;

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
    pub dimensions: Dimensions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoAnalysis {
    pub duration: Duration,
    pub has_audio: bool,
    pub dimensions: Option<Dimensions>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioAnalysis {
    pub duration: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileAnalysis {
    Image(ImageAnalysis),
    Video(VideoAnalysis),
    Audio(AudioAnalysis),
}

impl FileAnalysis {
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

        tokio::task::spawn_blocking(move || {
            let reader =
                image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
            let dimensions = reader.into_dimensions()?;
            Ok(Some(FileAnalysis::Image(ImageAnalysis {
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

        let temp_dir = self
            .config
            .temp_dir
            .as_deref()
            .ok_or(MediaAnalysisError::TempDirRequired)?;
        let temp_file = input.write_to_temp_file(temp_dir).await?;
        let path = temp_file.path().to_path_buf();
        let ffprobe_bin = self.ffprobe_bin.clone();

        let probe = tokio::task::spawn_blocking(move || {
            let mut builder = ffprobe::Config::builder();
            if let Some(ffprobe_bin) = ffprobe_bin {
                builder = builder.ffprobe_bin(ffprobe_bin);
            }
            builder.run(path)
        })
        .await??;

        if mime::is_video(declared_mime_type.as_deref()) {
            Ok(Some(FileAnalysis::Video(video_analysis(probe)?)))
        } else if mime::is_audio(declared_mime_type.as_deref()) {
            Ok(Some(FileAnalysis::Audio(audio_analysis(probe)?)))
        } else if has_stream(&probe, "video") {
            Ok(Some(FileAnalysis::Video(video_analysis(probe)?)))
        } else if has_stream(&probe, "audio") {
            Ok(Some(FileAnalysis::Audio(audio_analysis(probe)?)))
        } else {
            Ok(None)
        }
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

    Ok(VideoAnalysis {
        duration,
        has_audio: audio_stream.is_some(),
        dimensions: dimensions(video_stream),
    })
}

fn audio_analysis(
    probe: ffprobe::FfProbe,
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

    Ok(AudioAnalysis { duration })
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[cfg(feature = "image")]
    #[tokio::test]
    async fn image_analyzer_reads_dimensions_from_stream() {
        let input = FileAnalysisInput::from_bytes(Bytes::from_static(PNG_1X1));

        let analysis = ImageAnalyzer::new().analyze(input).await.unwrap().unwrap();

        assert_eq!(
            analysis,
            FileAnalysis::Image(ImageAnalysis {
                dimensions: Dimensions {
                    width: 1,
                    height: 1,
                },
            })
        );
    }

    #[tokio::test]
    async fn ffprobe_analyzer_requires_temp_dir_before_writing() {
        let input = FileAnalysisInput::from_bytes(Bytes::from_static(b"not a video"))
            .with_declared_mime_type("video/mp4");

        let error = FfprobeAnalyzer::new(AnalyzerConfig::default())
            .analyze(input)
            .await
            .unwrap_err();

        assert!(matches!(error, MediaAnalysisError::TempDirRequired));
    }
}
