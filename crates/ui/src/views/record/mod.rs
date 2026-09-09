use dioxus::prelude::*;
use semantic_data::{
    attr::ATTR_TITLE,
    value::{Object, Value},
};
use semantic_rpc::file::{FileUploadContent, FileUploadRequest, FileUploadResponse};

#[cfg(not(target_arch = "wasm32"))]
use native::NativeRecorder as Recorder;
#[cfg(target_arch = "wasm32")]
use web::BrowserRecorder as Recorder;

#[derive(Clone, Debug, PartialEq)]
enum RecordingState {
    Idle,
    Preview,
    Starting,
    Recording,
    Pausing,
    Paused,
    Resuming,
    Stopping,
    Uploading,
    ReadyToUpload,
    Complete(FileUploadResponse),
}

mod page;
mod preview;
pub use page::RecordPage;

/// Capture modes share review and upload, while device access stays platform-specific.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CaptureMode {
    #[default]
    Audio,
    Photo,
    Video,
    Screen,
}

impl CaptureMode {
    const ALL: [Self; 4] = [Self::Audio, Self::Photo, Self::Video, Self::Screen];

    fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Photo => "Photo",
            Self::Video => "Camera video",
            Self::Screen => "Screen",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Audio => "Capture your voice",
            Self::Photo => "Take a webcam photo",
            Self::Video => "Record with your webcam",
            Self::Screen => "Record a tab, window, or screen",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Audio => "Audio recording",
            Self::Photo => "Webcam photo",
            Self::Video => "Camera recording",
            Self::Screen => "Screen recording",
        }
    }

    fn is_camera(self) -> bool {
        matches!(self, Self::Photo | Self::Video)
    }
}

fn normalized_level(decibels: f32) -> f32 {
    if !decibels.is_finite() {
        return 0.0;
    }
    ((decibels + 60.0) / 60.0).clamp(0.0, 1.0)
}

#[derive(Clone, Debug, PartialEq)]
struct CompletedRecording {
    bytes: bytes::Bytes,
    extension: &'static str,
    mime_type: String,
}

async fn save_recording(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    title: String,
    recording: CompletedRecording,
    mut completed: Signal<Option<CompletedRecording>>,
    mut state: Signal<RecordingState>,
    mut error: Signal<Option<String>>,
) {
    error.set(None);
    state.set(RecordingState::Uploading);
    match upload_recording(client, scope_id, title, recording).await {
        Ok(id) => {
            completed.set(None);
            state.set(RecordingState::Complete(id));
        }
        Err(message) => {
            state.set(RecordingState::ReadyToUpload);
            error.set(Some(message));
        }
    }
}

async fn upload_recording(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    title: String,
    recording: CompletedRecording,
) -> std::result::Result<FileUploadResponse, String> {
    let mut entity = Object::new();
    let title = title.trim();
    if !title.is_empty() {
        entity.insert(ATTR_TITLE, Value::String(title.to_string()));
    }
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let prefix = if recording.mime_type.starts_with("image/") {
        "photo"
    } else if recording.mime_type.starts_with("video/") {
        "video-recording"
    } else {
        "audio-recording"
    };
    let filename = format!("{prefix}-{timestamp}.{}", recording.extension);
    client
        .upload_file(
            FileUploadRequest {
                scope_id,
                id: None,
                filename: Some(filename),
                mime_type: Some(recording.mime_type),
                entity,
                content: FileUploadContent::Bytes(recording.bytes),
            },
            None,
        )
        .await
        .map_err(|error| format!("Recording finished, but upload failed: {error}"))
}

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(any(target_arch = "wasm32", test))]
fn browser_recording(
    bytes: Vec<u8>,
    recorder_mime: String,
    chunk_mime: Option<String>,
) -> Result<CompletedRecording, String> {
    if bytes.is_empty() {
        return Err("The browser returned an empty recording.".to_string());
    }
    let mime_type = if !recorder_mime.is_empty() {
        recorder_mime
    } else {
        chunk_mime
            .filter(|mime| !mime.is_empty())
            .unwrap_or_else(|| "application/octet-stream".to_string())
    };
    Ok(CompletedRecording {
        bytes: bytes.into(),
        extension: recording_extension(&mime_type),
        mime_type,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn recording_extension(mime: &str) -> &'static str {
    match mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "audio/webm" | "video/webm" => "webm",
        "audio/ogg" | "application/ogg" => "ogg",
        "audio/mp4" => "m4a",
        "video/mp4" => "mp4",
        "image/png" => "png",
        "audio/mpeg" => "mp3",
        "audio/aac" => "aac",
        "audio/wav" | "audio/wave" | "audio/x-wav" => "wav",
        _ => "bin",
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::{
        process::Stdio,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tempfile::NamedTempFile;
    use tokio::{
        io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _},
        process::{Child, ChildStdin, Command},
        sync::{oneshot, watch},
        task::JoinHandle,
        time::timeout,
    };

    use super::CompletedRecording;

    #[derive(Clone, Debug, PartialEq)]
    pub struct AudioSource {
        pub name: String,
        pub description: String,
        pub monitor: bool,
    }

    #[derive(Default, Clone)]
    pub struct NativeRecorder(Arc<Mutex<RecorderState>>);

    impl PartialEq for NativeRecorder {
        fn eq(&self, other: &Self) -> bool {
            Arc::ptr_eq(&self.0, &other.0)
        }
    }

    #[derive(Default)]
    struct RecorderState {
        closed: bool,
        session: Option<NativeSession>,
        segments: Vec<CompletedRecording>,
        sources: (String, String),
    }

    // Dropping the stop sender cancels the supervisor, which kills and reaps the child
    // before dropping the tempfile. The UI never owns an unsupervised process.
    struct NativeSession {
        stop: Option<oneshot::Sender<()>>,
        result: watch::Receiver<Option<Result<CompletedRecording, String>>>,
        level: Arc<Mutex<Option<(std::time::Instant, f32)>>>,
    }

    const STOP_TIMEOUT: Duration = Duration::from_secs(5);
    const STDERR_LIMIT: usize = 16 * 1024;

    impl NativeSession {
        fn spawn(
            mut command: Command,
            output: NamedTempFile,
            stop_timeout: Duration,
        ) -> Result<Self, String> {
            let child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|error| {
                    format!("Could not start recorder (ffmpeg must be available in PATH): {error}")
                })?;
            let (stop, stop_rx) = oneshot::channel();
            let (result_tx, result) = watch::channel(None);
            let level = Arc::new(Mutex::new(None));
            let capture_level = level.clone();
            tokio::spawn(async move {
                let result = supervise(child, output, stop_rx, stop_timeout, capture_level).await;
                let _ = result_tx.send(Some(result));
            });
            Ok(Self {
                stop: Some(stop),
                result,
                level,
            })
        }
    }

    impl NativeRecorder {
        pub fn is_recording(&self) -> bool {
            self.0
                .lock()
                .unwrap()
                .session
                .as_ref()
                .is_some_and(|session| session.result.borrow().is_none())
        }

        pub fn level(&self) -> Option<f32> {
            let inner = self.0.lock().unwrap();
            let level = *inner.session.as_ref()?.level.lock().unwrap();
            level
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(2))
                .map(|(_, value)| value)
        }

        pub async fn start(&self, mic: &str, system: &str) -> std::result::Result<(), String> {
            {
                let mut inner = self.0.lock().unwrap();
                if inner
                    .session
                    .as_ref()
                    .is_some_and(|session| session.result.borrow().is_none())
                {
                    return Err("A desktop recording is already active.".to_string());
                }
                inner.segments.clear();
                inner.sources = (mic.to_string(), system.to_string());
            }
            self.start_segment(mic, system).await
        }

        async fn start_segment(&self, mic: &str, system: &str) -> Result<(), String> {
            if mic.is_empty() && system.is_empty() {
                return Err("Choose at least one audio source.".to_string());
            }
            let output = tempfile::Builder::new()
                .prefix("semantic-recording-")
                .suffix(".mp3")
                .tempfile()
                .map_err(|error| format!("Could not create a temporary recording: {error}"))?;
            let mut command = Command::new("ffmpeg");
            command.args(["-hide_banner", "-loglevel", "warning", "-y"]);
            let sources: Vec<_> = [mic, system]
                .into_iter()
                .filter(|source| !source.is_empty())
                .collect();
            for source in &sources {
                command.args(["-thread_queue_size", "4096", "-f", "pulse", "-i", source]);
            }
            if sources.len() == 2 {
                command.args(["-filter_complex", "[0:a][1:a]amix=inputs=2:duration=longest:dropout_transition=0,astats=metadata=1:reset=1,ametadata=mode=print:key=lavfi.astats.Overall.RMS_level:file='pipe\\:2'[aout]", "-map", "[aout]"]);
            } else {
                command.args(["-af", "astats=metadata=1:reset=1,ametadata=mode=print:key=lavfi.astats.Overall.RMS_level:file='pipe\\:2'"]);
            }
            command
                .args([
                    "-ac",
                    "2",
                    "-ar",
                    "48000",
                    "-c:a",
                    "libmp3lame",
                    "-b:a",
                    "192k",
                ])
                .arg(output.path());
            let mut result = {
                let mut inner = self.0.lock().unwrap();
                if inner.closed {
                    return Err("The recording page has been closed.".to_string());
                }
                if inner
                    .session
                    .as_ref()
                    .is_some_and(|session| session.result.borrow().is_none())
                {
                    return Err("A desktop recording is already active.".to_string());
                }
                let session = NativeSession::spawn(command, output, STOP_TIMEOUT)?;
                let result = session.result.clone();
                inner.session = Some(session);
                result
            };
            // Surface immediate input/encoder failures before claiming recording started.
            tokio::select! {
                result = wait_for_result(&mut result) => result.map(|_| ()),
                _ = tokio::time::sleep(Duration::from_millis(300)) => Ok(()),
            }
        }

        async fn stop_segment(&self) -> std::result::Result<CompletedRecording, String> {
            let Some(mut session) = self.0.lock().unwrap().session.take() else {
                return Err("No desktop recording is active.".to_string());
            };
            if let Some(stop) = session.stop.take() {
                let _ = stop.send(());
            }
            wait_for_result(&mut session.result).await
        }

        // Pausing closes capture completely. Suspending ffmpeg would leave Pulse
        // buffers and wall-clock timestamps running, leaking paused audio on resume.
        pub async fn pause(&self) -> Result<(), String> {
            let segment = self.stop_segment().await?;
            self.0.lock().unwrap().segments.push(segment);
            Ok(())
        }

        pub async fn resume(&self) -> Result<(), String> {
            let (mic, system) = self.0.lock().unwrap().sources.clone();
            self.start_segment(&mic, &system).await
        }

        pub async fn stop(&self) -> Result<CompletedRecording, String> {
            let active = self.0.lock().unwrap().session.is_some();
            if active {
                self.pause().await?;
            }
            let segments = self.0.lock().unwrap().segments.clone();
            let recording = join_segments(&segments).await?;
            self.0.lock().unwrap().segments.clear();
            Ok(recording)
        }

        pub async fn wait_for_failure(&self) -> Option<String> {
            let mut result = self.0.lock().unwrap().session.as_ref()?.result.clone();
            let error = wait_for_result(&mut result).await.err();
            if error.is_some() {
                let mut inner = self.0.lock().unwrap();
                // Do not clear a newer resumed session if an older watcher wins
                // a race with a user action.
                if inner
                    .session
                    .as_ref()
                    .is_some_and(|session| session.result.same_channel(&result))
                {
                    inner.session = None;
                } else {
                    return None;
                }
            }
            error
        }

        pub fn discard(&self) {
            let mut inner = self.0.lock().unwrap();
            inner.session = None;
            inner.segments.clear();
        }

        pub fn close(&self) {
            let mut inner = self.0.lock().unwrap();
            inner.closed = true;
            inner.session = None;
            inner.segments.clear();
        }
    }

    async fn join_segments(segments: &[CompletedRecording]) -> Result<CompletedRecording, String> {
        if segments.len() == 1 {
            return Ok(segments[0].clone());
        }
        if segments.is_empty() {
            return Err("No audio was captured.".to_string());
        }
        // The concat demuxer removes each segment's container headers and writes
        // a single seekable MP3. All segment files share the same encoder settings.
        let directory = tempfile::Builder::new()
            .prefix("semantic-recording-")
            .tempdir()
            .map_err(|error| format!("Could not prepare recording: {error}"))?;
        let mut manifest = String::new();
        for (index, segment) in segments.iter().enumerate() {
            let name = format!("segment-{index}.mp3");
            tokio::fs::write(directory.path().join(&name), &segment.bytes)
                .await
                .map_err(|error| format!("Could not prepare audio segment: {error}"))?;
            manifest.push_str(&format!("file '{name}'\n"));
        }
        tokio::fs::write(directory.path().join("segments.txt"), manifest)
            .await
            .map_err(|error| format!("Could not prepare recording: {error}"))?;
        let output = directory.path().join("recording.mp3");
        let mut command = Command::new("ffmpeg");
        command
            .current_dir(directory.path())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "concat",
                "-safe",
                "1",
                "-i",
                "segments.txt",
                "-c",
                "copy",
            ])
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not join audio segments: {error}"))?;
        let mut stderr = StderrTask(tokio::spawn(drain_stderr(
            child.stderr.take().expect("piped stderr"),
        )));
        let status = match timeout(Duration::from_secs(60), child.wait()).await {
            Ok(result) => {
                result.map_err(|error| format!("Could not join audio segments: {error}"))?
            }
            Err(_) => {
                let _ = child.kill().await;
                return Err("Joining audio segments timed out.".to_string());
            }
        };
        let diagnostics = (&mut stderr.0)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        if !status.success() {
            return Err(format!(
                "Could not join audio segments: {}",
                String::from_utf8_lossy(&diagnostics)
            ));
        }
        let bytes = tokio::fs::read(output)
            .await
            .map_err(|error| format!("Could not read recording: {error}"))?;
        if bytes.is_empty() {
            return Err("The joined recording is empty.".to_string());
        }
        Ok(CompletedRecording {
            bytes: bytes.into(),
            extension: "mp3",
            mime_type: "audio/mpeg".to_string(),
        })
    }

    async fn wait_for_result(
        result: &mut watch::Receiver<Option<Result<CompletedRecording, String>>>,
    ) -> Result<CompletedRecording, String> {
        loop {
            if let Some(result) = result.borrow().clone() {
                return result;
            }
            result
                .changed()
                .await
                .map_err(|_| "The recording supervisor stopped unexpectedly.".to_string())?;
        }
    }

    struct StderrTask(JoinHandle<Result<Vec<u8>, String>>);

    impl Drop for StderrTask {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    async fn drain_stderr(mut reader: impl AsyncRead + Unpin) -> Result<Vec<u8>, String> {
        drain_stderr_with_level(&mut reader, None).await
    }

    async fn drain_stderr_with_level(
        mut reader: impl AsyncRead + Unpin,
        level: Option<Arc<Mutex<Option<(std::time::Instant, f32)>>>>,
    ) -> Result<Vec<u8>, String> {
        let mut tail = Vec::new();
        let mut partial = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = reader
                .read(&mut buffer)
                .await
                .map_err(|error| format!("Could not read ffmpeg diagnostics: {error}"))?;
            if count == 0 {
                return Ok(tail);
            }
            if let Some(level) = &level {
                partial.extend_from_slice(&buffer[..count]);
                if let Some(last_newline) = partial.iter().rposition(|byte| *byte == b'\n') {
                    for line in partial[..=last_newline].split(|byte| *byte == b'\n') {
                        if let Some(value) = parse_level(line) {
                            *level.lock().unwrap() = Some((std::time::Instant::now(), value));
                        }
                    }
                    partial.drain(..=last_newline);
                }
                if partial.len() > STDERR_LIMIT {
                    partial.clear();
                }
            }
            let overflow = (tail.len() + count).saturating_sub(STDERR_LIMIT);
            tail.drain(..overflow);
            tail.extend_from_slice(&buffer[..count]);
        }
    }

    fn parse_level(line: &[u8]) -> Option<f32> {
        let value = std::str::from_utf8(line)
            .ok()?
            .trim()
            .strip_prefix("lavfi.astats.Overall.RMS_level=")?;
        Some(super::normalized_level(value.parse().ok()?))
    }

    async fn stop_child(
        child: &mut Child,
        stdin: Option<ChildStdin>,
        stop_timeout: Duration,
    ) -> Result<(), String> {
        match timeout(stop_timeout, async {
            let mut stdin =
                stdin.ok_or_else(|| "The recorder control pipe is unavailable.".to_string())?;
            stdin
                .write_all(b"q\n")
                .await
                .map_err(|error| format!("Could not request recording stop: {error}"))?;
            drop(stdin);
            let status = child
                .wait()
                .await
                .map_err(|error| format!("Could not finish ffmpeg: {error}"))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("ffmpeg failed ({status})"))
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Err("ffmpeg did not stop in time; the recording was cancelled.".to_string()),
        }
    }

    async fn supervise(
        mut child: Child,
        output: NamedTempFile,
        stop: oneshot::Receiver<()>,
        stop_timeout: Duration,
        level: Arc<Mutex<Option<(std::time::Instant, f32)>>>,
    ) -> Result<CompletedRecording, String> {
        // Child::wait closes Child::stdin as soon as it is polled. Keep the
        // command pipe separately owned while supervising so Stop/Pause can
        // still send ffmpeg's quit command after an arbitrarily long recording.
        let stdin = child.stdin.take();
        let mut stderr = StderrTask(tokio::spawn(drain_stderr_with_level(
            child.stderr.take().expect("piped stderr"),
            Some(level),
        )));
        let result = tokio::select! {
            status = child.wait() => Err(match status {
                Ok(status) => format!("ffmpeg exited before recording was stopped ({status})"),
                Err(error) => format!("Could not monitor ffmpeg: {error}"),
            }),
            requested = stop => match requested {
                Ok(()) => stop_child(&mut child, stdin, stop_timeout).await,
                Err(_) => Err("Recording cancelled.".to_string()),
            },
        };
        // kill() also waits/reaps. The tempfile remains owned until the child is gone.
        if result.is_err() {
            let _ = child.kill().await;
        }
        let diagnostics = match timeout(Duration::from_secs(1), &mut stderr.0).await {
            Ok(Ok(Ok(bytes))) => String::from_utf8_lossy(&bytes).trim().to_string(),
            Ok(Ok(Err(error))) => error,
            Ok(Err(error)) => format!("Could not collect ffmpeg diagnostics: {error}"),
            Err(_) => "Timed out collecting ffmpeg diagnostics.".to_string(),
        };
        result.map_err(|error| {
            if diagnostics.is_empty() {
                error
            } else {
                format!("{error}: {diagnostics}")
            }
        })?;
        let bytes = tokio::fs::read(output.path())
            .await
            .map_err(|error| format!("Could not read recording: {error}"))?;
        if bytes.is_empty() {
            return Err("ffmpeg produced an empty recording.".to_string());
        }
        Ok(CompletedRecording {
            bytes: bytes.into(),
            extension: "mp3",
            mime_type: "audio/mpeg".to_string(),
        })
    }

    pub async fn list_sources() -> std::result::Result<Vec<AudioSource>, String> {
        require_command("pactl").await?;
        let output = Command::new("pactl")
            .args(["list", "sources"])
            .env("LC_ALL", "C")
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|error| format!("Could not run pactl: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "pactl failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(parse_sources(&String::from_utf8_lossy(&output.stdout)))
    }

    fn parse_sources(output: &str) -> Vec<AudioSource> {
        let mut sources = Vec::new();
        let mut name = None::<String>;
        let mut description = None::<String>;
        let flush = |sources: &mut Vec<AudioSource>,
                     name: &mut Option<String>,
                     description: &mut Option<String>| {
            if let Some(source_name) = name.take() {
                sources.push(AudioSource {
                    monitor: source_name.ends_with(".monitor"),
                    description: description.take().unwrap_or_else(|| source_name.clone()),
                    name: source_name,
                });
            }
        };
        for line in output.lines().map(str::trim) {
            if line.starts_with("Source #") {
                flush(&mut sources, &mut name, &mut description);
            } else if let Some(value) = line.strip_prefix("Name:") {
                name = Some(value.trim().to_string());
            } else if let Some(value) = line.strip_prefix("Description:") {
                description = Some(value.trim().to_string());
            }
        }
        flush(&mut sources, &mut name, &mut description);
        sources
    }

    async fn require_command(name: &str) -> std::result::Result<(), String> {
        Command::new(name)
            .arg("--version")
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|_| ())
            .map_err(|_| format!("Missing dependency: '{name}' is not available in PATH."))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_levels_without_confusing_diagnostics_for_audio() {
            assert_eq!(
                parse_level(b"lavfi.astats.Overall.RMS_level=-30.0"),
                Some(0.5)
            );
            assert_eq!(
                parse_level(b"lavfi.astats.Overall.RMS_level=-inf"),
                Some(0.0)
            );
            assert_eq!(
                parse_level(b"lavfi.astats.Overall.RMS_level=4.0"),
                Some(1.0)
            );
            assert_eq!(parse_level(b"ffmpeg failed"), None);
        }

        #[tokio::test]
        async fn level_parser_handles_partial_pipe_reads() {
            let (mut writer, reader) = tokio::io::duplex(8);
            let level = Arc::new(Mutex::new(None));
            let observed = level.clone();
            let drain = tokio::spawn(drain_stderr_with_level(reader, Some(level)));
            writer
                .write_all(b"lavfi.astats.Overall.RMS_level=-12.0\n")
                .await
                .unwrap();
            drop(writer);
            drain.await.unwrap().unwrap();
            assert_eq!(observed.lock().unwrap().unwrap().1, 0.8);
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn pause_finalizes_and_retains_audio_until_stop() {
            let (session, path) =
                session("printf audio > \"$1\"; read line", Duration::from_secs(1));
            let recorder = NativeRecorder::default();
            recorder.0.lock().unwrap().session = Some(session);
            assert!(recorder.is_recording());
            recorder.pause().await.unwrap();
            assert!(!recorder.is_recording());
            assert!(!path.exists());
            assert_eq!(recorder.0.lock().unwrap().segments.len(), 1);
            let recording = recorder.stop().await.unwrap();
            assert_eq!(recording.bytes.as_ref(), b"audio");
            assert!(recorder.0.lock().unwrap().segments.is_empty());
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn supervision_keeps_control_pipe_open_until_explicit_stop() {
            let (mut session, path) = session(
                "printf 'lavfi.astats.Overall.RMS_level=-30\\n' >&2; read command && [ \"$command\" = q ] && printf 'received quit command' > \"$1\"",
                Duration::from_secs(1),
            );
            // Wait for the child's readiness message through the supervised
            // stderr reader. This guarantees the supervisor has begun polling
            // Child::wait before we request stop; no scheduling sleeps needed.
            timeout(Duration::from_secs(5), async {
                while session.level.lock().unwrap().is_none() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            session.stop.take().unwrap().send(()).unwrap();
            let recording = result(&mut session).await.unwrap();
            assert_eq!(recording.bytes.as_ref(), b"received quit command");
            assert!(!path.exists());
        }

        #[tokio::test]
        async fn joined_segments_form_one_playable_recording() {
            if Command::new("ffmpeg")
                .arg("-version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .is_err()
            {
                eprintln!("Skipping MP3 integration test: ffmpeg is unavailable");
                return;
            }
            let output = Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:duration=0.25",
                    "-ac",
                    "2",
                    "-ar",
                    "48000",
                    "-c:a",
                    "libmp3lame",
                    "-b:a",
                    "192k",
                    "-f",
                    "mp3",
                    "pipe:1",
                ])
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let segment = CompletedRecording {
                bytes: output.stdout.into(),
                extension: "mp3",
                mime_type: "audio/mpeg".to_string(),
            };
            let joined = join_segments(&[segment.clone(), segment]).await.unwrap();
            let mut child = Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-i",
                    "pipe:0",
                    "-f",
                    "s16le",
                    "pipe:1",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&joined.bytes)
                .await
                .unwrap();
            let decoded = child.wait_with_output().await.unwrap();
            assert!(
                decoded.status.success(),
                "{}",
                String::from_utf8_lossy(&decoded.stderr)
            );
            let seconds = decoded.stdout.len() as f64 / (48_000.0 * 2.0 * 2.0);
            assert!(
                (0.45..0.65).contains(&seconds),
                "unexpected decoded duration: {seconds}"
            );
        }
        #[test]
        fn parses_microphone_and_monitor_sources() {
            let sources = parse_sources(
                "Source #1\n Name: mic.one\n Description: USB Mic\nSource #2\n Name: sink.monitor\n Description: System Output\n",
            );
            assert_eq!(sources.len(), 2);
            assert!(!sources[0].monitor);
            assert!(sources[1].monitor);
        }

        #[tokio::test]
        async fn stderr_is_drained_beyond_pipe_capacity_and_keeps_only_the_tail() {
            let (mut writer, reader) = tokio::io::duplex(1024);
            let write = tokio::spawn(async move {
                writer
                    .write_all(&vec![b'x'; STDERR_LIMIT * 8])
                    .await
                    .unwrap();
                writer.write_all(b"last diagnostic").await.unwrap();
            });
            let tail = timeout(Duration::from_secs(5), drain_stderr(reader))
                .await
                .unwrap()
                .unwrap();
            write.await.unwrap();
            assert_eq!(tail.len(), STDERR_LIMIT);
            assert!(tail.ends_with(b"last diagnostic"));
        }

        #[cfg(unix)]
        fn session(script: &str, grace: Duration) -> (NativeSession, std::path::PathBuf) {
            let output = NamedTempFile::new().unwrap();
            let path = output.path().to_owned();
            let mut command = Command::new("sh");
            command.args(["-c", script, "test-recorder"]).arg(&path);
            (NativeSession::spawn(command, output, grace).unwrap(), path)
        }

        #[cfg(unix)]
        async fn result(session: &mut NativeSession) -> Result<CompletedRecording, String> {
            timeout(Duration::from_secs(5), wait_for_result(&mut session.result))
                .await
                .unwrap()
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn early_exit_reports_diagnostics_and_cleans_the_tempfile() {
            let (mut session, path) = session("printf 'invalid input' >&2; exit 7", STOP_TIMEOUT);
            let error = result(&mut session).await.unwrap_err();
            assert!(error.contains("exited before recording was stopped"));
            assert!(error.contains("invalid input"));
            assert!(!path.exists());
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn graceful_stop_drains_noisy_child_and_cleans_the_tempfile() {
            let (mut session, path) = session(
                "i=0; while [ $i -lt 20000 ]; do printf 'recorder diagnostic line\n' >&2; i=$((i+1)); done; read command; printf 'recorded audio' > \"$1\"",
                STOP_TIMEOUT,
            );
            session.stop.take().unwrap().send(()).unwrap();
            let recording = result(&mut session).await.unwrap();
            assert_eq!(recording.bytes.as_ref(), b"recorded audio");
            assert!(!path.exists());
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn stop_timeout_kills_and_cleans_the_tempfile() {
            let (mut session, path) = session("while :; do :; done", Duration::from_millis(30));
            session.stop.take().unwrap().send(()).unwrap();
            assert!(
                result(&mut session)
                    .await
                    .unwrap_err()
                    .contains("did not stop in time")
            );
            assert!(!path.exists());
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn dropping_session_cancels_the_child_and_cleans_the_tempfile() {
            let (session, path) = session("while :; do :; done", STOP_TIMEOUT);
            let mut result = session.result.clone();
            drop(session);
            assert!(
                timeout(Duration::from_secs(5), wait_for_result(&mut result))
                    .await
                    .unwrap()
                    .is_err()
            );
            assert!(!path.exists());
        }

        #[tokio::test]
        async fn spawn_failure_cleans_the_tempfile() {
            let output = NamedTempFile::new().unwrap();
            let path = output.path().to_owned();
            assert!(
                NativeSession::spawn(
                    Command::new("/nonexistent/semantic-test-recorder"),
                    output,
                    STOP_TIMEOUT
                )
                .is_err()
            );
            assert!(!path.exists());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_browser_containers_without_assuming_webm() {
        for (mime, expected) in [
            ("audio/webm;codecs=opus", "webm"),
            ("audio/ogg;codecs=opus", "ogg"),
            ("audio/mp4;codecs=mp4a.40.2", "m4a"),
            ("video/mp4;codecs=avc1", "mp4"),
            ("video/webm;codecs=vp8,opus", "webm"),
            ("image/png", "png"),
            ("audio/mpeg", "mp3"),
            ("audio/unknown", "bin"),
            ("", "bin"),
        ] {
            assert_eq!(recording_extension(mime), expected);
        }
    }

    #[test]
    fn browser_recording_prefers_the_recorders_actual_mime_type() {
        let recording = browser_recording(
            vec![1, 2, 3],
            "audio/mp4;codecs=mp4a.40.2".to_string(),
            Some("audio/webm".to_string()),
        )
        .unwrap();
        assert_eq!(recording.bytes.as_ref(), &[1, 2, 3]);
        assert_eq!(recording.mime_type, "audio/mp4;codecs=mp4a.40.2");
        assert_eq!(recording.extension, "m4a");
    }

    #[test]
    fn browser_recording_uses_chunk_mime_when_the_recorder_omits_it() {
        let recording = browser_recording(
            vec![1],
            String::new(),
            Some("audio/ogg;codecs=opus".to_string()),
        )
        .unwrap();
        assert_eq!(recording.mime_type, "audio/ogg;codecs=opus");
        assert_eq!(recording.extension, "ogg");
    }

    #[test]
    fn browser_recording_does_not_guess_an_unknown_container() {
        for chunk_mime in [None, Some(String::new())] {
            let recording = browser_recording(vec![1], String::new(), chunk_mime).unwrap();
            assert_eq!(recording.mime_type, "application/octet-stream");
            assert_eq!(recording.extension, "bin");
        }
    }

    #[test]
    fn browser_recording_rejects_empty_audio() {
        let error = browser_recording(Vec::new(), "audio/webm".to_string(), None).unwrap_err();
        assert_eq!(error, "The browser returned an empty recording.");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn failed_upload_retains_audio_and_successful_retry_clears_it() {
        use semantic_rpc::file::{FileUploadProgressSender, FileUploadResponse};
        use semantic_rpc::{RpcClient, RpcClientDyn, RpcClientError, client::RpcClientFuture};
        use std::sync::{Arc, Mutex};

        struct RetryClient(Arc<Mutex<Vec<FileUploadRequest>>>);
        impl RpcClientDyn for RetryClient {
            fn invoke_value(
                &self,
                _command: String,
                _payload: Value,
            ) -> RpcClientFuture<Result<Value, RpcClientError>> {
                Box::pin(async { Err(RpcClientError::Transport("unused".to_string())) })
            }

            fn upload_file(
                &self,
                request: FileUploadRequest,
                _progress: Option<FileUploadProgressSender>,
            ) -> RpcClientFuture<Result<FileUploadResponse, RpcClientError>> {
                let mut requests = self.0.lock().unwrap();
                requests.push(request);
                let fail = requests.len() == 1;
                Box::pin(async move {
                    if fail {
                        Err(RpcClientError::Transport("offline".to_string()))
                    } else {
                        Ok(FileUploadResponse {
                            id: "recording-id".to_string(),
                            collection: "files".to_string(),
                            object: Object::new(),
                        })
                    }
                })
            }
        }

        let requests = Arc::new(Mutex::new(Vec::new()));
        let client = RpcClient::new(RetryClient(requests.clone()));
        let mut dom = VirtualDom::new(|| rsx! {});
        dom.rebuild_in_place();
        dom.in_scope(ScopeId::ROOT, || {
            let recording = CompletedRecording {
                bytes: bytes::Bytes::from_static(b"audio"),
                extension: "m4a",
                mime_type: "audio/mp4;codecs=mp4a.40.2".to_string(),
            };
            let completed = Signal::new(Some(recording.clone()));
            let state = Signal::new(RecordingState::Stopping);
            let error = Signal::new(None);
            futures::executor::block_on(save_recording(
                client.clone(),
                None,
                "Title".to_string(),
                recording,
                completed,
                state,
                error,
            ));
            assert_eq!(state(), RecordingState::ReadyToUpload);
            assert!(error().unwrap().contains("offline"));
            let retry = completed
                .peek()
                .clone()
                .expect("failed upload must retain audio");
            assert_eq!(retry.bytes.as_ref(), b"audio");
            futures::executor::block_on(save_recording(
                client,
                None,
                "Title".to_string(),
                retry,
                completed,
                state,
                error,
            ));
            assert_eq!(
                state(),
                RecordingState::Complete(FileUploadResponse {
                    id: "recording-id".to_string(),
                    collection: "files".to_string(),
                    object: Object::new(),
                })
            );
            assert!(completed.peek().is_none());
            assert!(error().is_none());
        });
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests.iter() {
            assert!(request.filename.as_ref().unwrap().ends_with(".m4a"));
            assert_eq!(
                request.mime_type.as_deref(),
                Some("audio/mp4;codecs=mp4a.40.2")
            );
            let FileUploadContent::Bytes(bytes) = &request.content else {
                panic!("expected bytes")
            };
            assert_eq!(bytes.as_ref(), b"audio");
        }
    }
}
