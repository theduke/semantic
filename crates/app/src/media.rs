use std::io;
use std::path::PathBuf;
use std::time::Duration as StdDuration;

use bytes::Bytes;
use futures_util::StreamExt as _;
use ordered_float::OrderedFloat;
use semantic_data::filestore::{
    ATTR_FILE_MEDIA_AUDIO_BITRATE, ATTR_FILE_MEDIA_AUDIO_CHANNELS, ATTR_FILE_MEDIA_AUDIO_CODEC,
    ATTR_FILE_MEDIA_AUDIO_SAMPLE_RATE, ATTR_FILE_MEDIA_BITRATE, ATTR_FILE_MEDIA_CONTAINER_FORMAT,
    ATTR_FILE_MEDIA_DURATION, ATTR_FILE_MEDIA_HAS_AUDIO, ATTR_FILE_MEDIA_PIXEL_HEIGHT,
    ATTR_FILE_MEDIA_PIXEL_WIDTH, ATTR_FILE_MEDIA_VIDEO_BITRATE, ATTR_FILE_MEDIA_VIDEO_CODEC,
    ATTR_FILE_MEDIA_VIDEO_FRAME_COUNT, ATTR_FILE_MEDIA_VIDEO_FRAMES_PER_SECOND,
};
use semantic_data::value::{Object, Value};
use semantic_db_core::{Batch, BatchOperation};
use semantic_media::{
    AnalyzerConfig, AudioAnalysis, FfprobeAnalyzer, FileAnalysis, FileAnalysisInput, FileAnalyzer,
    VideoAnalysis, mime,
};

use crate::{AppError, AppRequestContext, DbScopeId, FileByteStream};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaAnalysisConfig {
    pub temp_dir: Option<PathBuf>,
    pub auto_analyze_media: bool,
}

impl From<&crate::AppConfig> for MediaAnalysisConfig {
    fn from(value: &crate::AppConfig) -> Self {
        Self {
            temp_dir: value.temp_dir.clone(),
            auto_analyze_media: value.auto_analyze_media,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MediaAnalysisOutcome {
    pub id: String,
    pub collection: String,
    pub analyzed: bool,
    pub analysis_kind: Option<&'static str>,
    pub attributes: Object,
    pub object: Object,
}

#[derive(Clone, Debug, Default)]
pub struct MediaAnalysisService {
    config: MediaAnalysisConfig,
}

impl MediaAnalysisService {
    pub fn new(config: MediaAnalysisConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &MediaAnalysisConfig {
        &self.config
    }

    pub async fn analyze_created_bytes(
        &self,
        bytes: Bytes,
        filename: Option<&str>,
        declared_mime_type: Option<&str>,
    ) -> std::result::Result<Option<FileAnalysis>, AppError> {
        let media_mime_type = mime::analyze_bytes_owned(&bytes, declared_mime_type)
            .best_effort()
            .map(ToOwned::to_owned);
        let input = input_from_bytes(bytes, filename, media_mime_type.as_deref());
        self.analyze_input(input, media_mime_type.as_deref()).await
    }

    pub async fn analyze_stream(
        &self,
        stream: FileByteStream,
        filename: Option<&str>,
        declared_mime_type: Option<&str>,
    ) -> std::result::Result<Option<FileAnalysis>, AppError> {
        let input = input_from_stream(stream, filename, declared_mime_type);
        self.analyze_input(input, declared_mime_type).await
    }

    pub async fn analyze_persisted_file(
        &self,
        ctx: &AppRequestContext,
        scope_id: Option<DbScopeId>,
        id: String,
    ) -> std::result::Result<MediaAnalysisOutcome, AppError> {
        let read = ctx
            .app
            .files()
            .read(ctx, scope_id.clone(), id.clone())
            .await?;
        let filename = object_string(&read.record.object, "filename");
        let analysis = self
            .analyze_stream(read.stream, filename.as_deref(), read.mime_type.as_deref())
            .await?;
        let attributes = analysis
            .as_ref()
            .map(analysis_attributes)
            .unwrap_or_default();
        let analysis_kind = analysis.as_ref().map(analysis_kind);
        let mut object = read.record.object;
        for (name, value) in attributes.clone() {
            object.insert(name, value);
        }

        if !attributes.is_empty() {
            let db = ctx.resolve_db(scope_id).await?;
            db.execute_batch(Batch::new().with_op(BatchOperation::Upsert {
                collection: read.record.collection.clone(),
                id: read.record.id.clone(),
                object: object.clone(),
            }))
            .await?;
        }

        Ok(MediaAnalysisOutcome {
            id,
            collection: read.record.collection,
            analyzed: analysis.is_some(),
            analysis_kind,
            attributes,
            object,
        })
    }

    async fn analyze_input(
        &self,
        input: FileAnalysisInput,
        mime_type: Option<&str>,
    ) -> std::result::Result<Option<FileAnalysis>, AppError> {
        if mime::is_image(mime_type) {
            return semantic_media::ImageAnalyzer::new()
                .analyze(input)
                .await
                .map_err(AppError::from);
        }
        if mime::is_video(mime_type) || mime::is_audio(mime_type) || mime_type.is_none() {
            return FfprobeAnalyzer::new(AnalyzerConfig {
                temp_dir: self.config.temp_dir.clone(),
            })
            .analyze(input)
            .await
            .map_err(AppError::from);
        }
        Ok(None)
    }
}

pub fn merge_analysis_attributes(object: &mut Object, analysis: &FileAnalysis) {
    for (name, value) in analysis_attributes(analysis) {
        object.insert(name, value);
    }
}

pub fn analysis_attributes(analysis: &FileAnalysis) -> Object {
    let mut out = Object::new();
    if let Some(dimensions) = analysis.dimensions() {
        out.insert(ATTR_FILE_MEDIA_PIXEL_WIDTH, Value::U64(dimensions.width));
        out.insert(ATTR_FILE_MEDIA_PIXEL_HEIGHT, Value::U64(dimensions.height));
    }
    match analysis {
        FileAnalysis::Image(_) => {}
        FileAnalysis::Video(video) => insert_video_attributes(&mut out, video),
        FileAnalysis::Audio(audio) => insert_audio_attributes(&mut out, audio),
    }
    out
}

fn insert_video_attributes(out: &mut Object, video: &VideoAnalysis) {
    out.insert(ATTR_FILE_MEDIA_DURATION, duration_value(video.duration));
    out.insert(ATTR_FILE_MEDIA_HAS_AUDIO, Value::Bool(video.has_audio));
    insert_f64(
        out,
        ATTR_FILE_MEDIA_VIDEO_FRAMES_PER_SECOND,
        video.frames_per_second,
    );
    insert_u64(out, ATTR_FILE_MEDIA_VIDEO_FRAME_COUNT, video.frame_count);
    insert_u64(out, ATTR_FILE_MEDIA_BITRATE, video.bitrate);
    insert_u64(out, ATTR_FILE_MEDIA_VIDEO_BITRATE, video.video_bitrate);
    insert_u64(out, ATTR_FILE_MEDIA_AUDIO_BITRATE, video.audio_bitrate);
    insert_string(
        out,
        ATTR_FILE_MEDIA_VIDEO_CODEC,
        video.video_codec.as_deref(),
    );
    insert_string(
        out,
        ATTR_FILE_MEDIA_AUDIO_CODEC,
        video.audio_codec.as_deref(),
    );
    insert_u64(out, ATTR_FILE_MEDIA_AUDIO_CHANNELS, video.audio_channels);
    insert_u64(
        out,
        ATTR_FILE_MEDIA_AUDIO_SAMPLE_RATE,
        video.audio_sample_rate,
    );
    insert_string(
        out,
        ATTR_FILE_MEDIA_CONTAINER_FORMAT,
        video.container_format.as_deref(),
    );
}

fn insert_audio_attributes(out: &mut Object, audio: &AudioAnalysis) {
    out.insert(ATTR_FILE_MEDIA_DURATION, duration_value(audio.duration));
    insert_u64(out, ATTR_FILE_MEDIA_BITRATE, audio.bitrate);
    insert_u64(out, ATTR_FILE_MEDIA_AUDIO_BITRATE, audio.audio_bitrate);
    insert_string(
        out,
        ATTR_FILE_MEDIA_AUDIO_CODEC,
        audio.audio_codec.as_deref(),
    );
    insert_u64(out, ATTR_FILE_MEDIA_AUDIO_CHANNELS, audio.audio_channels);
    insert_u64(
        out,
        ATTR_FILE_MEDIA_AUDIO_SAMPLE_RATE,
        audio.audio_sample_rate,
    );
    insert_string(
        out,
        ATTR_FILE_MEDIA_CONTAINER_FORMAT,
        audio.container_format.as_deref(),
    );
}

fn duration_value(duration: StdDuration) -> Value {
    let seconds = i64::try_from(duration.as_secs()).unwrap_or(i64::MAX);
    let nanos = i32::try_from(duration.subsec_nanos()).unwrap_or(i32::MAX);
    Value::Duration(semantic_data::value::Duration::from(time::Duration::new(
        seconds, nanos,
    )))
}

fn insert_u64(out: &mut Object, name: &str, value: Option<u64>) {
    if let Some(value) = value {
        out.insert(name, Value::U64(value));
    }
}

fn insert_f64(out: &mut Object, name: &str, value: Option<f64>) {
    if let Some(value) = value.filter(|value| value.is_finite()) {
        out.insert(name, Value::F64(OrderedFloat(value)));
    }
}

fn insert_string(out: &mut Object, name: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        out.insert(name, Value::String(value.to_string()));
    }
}

fn analysis_kind(analysis: &FileAnalysis) -> &'static str {
    match analysis {
        FileAnalysis::Image(_) => "image",
        FileAnalysis::Video(_) => "video",
        FileAnalysis::Audio(_) => "audio",
    }
}

fn input_from_bytes(
    bytes: Bytes,
    filename: Option<&str>,
    declared_mime_type: Option<&str>,
) -> FileAnalysisInput {
    apply_input_metadata(
        FileAnalysisInput::from_bytes(bytes),
        filename,
        declared_mime_type,
    )
}

fn input_from_stream(
    stream: FileByteStream,
    filename: Option<&str>,
    declared_mime_type: Option<&str>,
) -> FileAnalysisInput {
    let stream = stream.map(|chunk| chunk.map_err(|err| io::Error::other(err.to_string())));
    apply_input_metadata(
        FileAnalysisInput {
            filename: None,
            declared_mime_type: None,
            stream: Box::pin(stream),
        },
        filename,
        declared_mime_type,
    )
}

fn apply_input_metadata(
    mut input: FileAnalysisInput,
    filename: Option<&str>,
    declared_mime_type: Option<&str>,
) -> FileAnalysisInput {
    if let Some(filename) = filename {
        input = input.with_filename(filename);
    }
    if let Some(mime_type) = declared_mime_type {
        input = input.with_declared_mime_type(mime_type);
    }
    input
}

fn object_string(object: &Object, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}
