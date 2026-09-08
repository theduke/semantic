use dioxus::prelude::*;
use semantic_data::{
    filestore::ATTR_TITLE,
    value::{Object, Value},
};
use semantic_rpc::file::{FileUploadContent, FileUploadRequest};
use semantic_ui_core::{
    components::{InlineNotice, NoticeVariant},
    use_active_scope_id, use_rpc_client,
};

use crate::components::PageHeader;

#[cfg(not(target_arch = "wasm32"))]
use native::NativeRecorder as Recorder;
#[cfg(target_arch = "wasm32")]
use web::BrowserRecorder as Recorder;

#[derive(Clone, Debug, PartialEq)]
enum RecordingState {
    Idle,
    Starting,
    Recording,
    Stopping,
    Uploading,
    ReadyToUpload,
    Complete(String),
}

#[component]
pub fn AudioRecordingPage() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut state = use_signal(|| RecordingState::Idle);
    let mut error = use_signal(|| None::<String>);
    let mut title = use_signal(|| "Audio recording".to_string());
    let mut completed = use_signal(|| None::<CompletedRecording>);
    let recorder = use_hook(Recorder::default);
    let cleanup = recorder.clone();
    use_drop(move || cleanup.close());

    #[cfg(not(target_arch = "wasm32"))]
    let mut sources = use_signal(Vec::<native::AudioSource>::new);
    #[cfg(not(target_arch = "wasm32"))]
    let mut mic_source = use_signal(String::new);
    #[cfg(not(target_arch = "wasm32"))]
    let mut system_source = use_signal(String::new);

    #[cfg(not(target_arch = "wasm32"))]
    use_effect(move || {
        spawn(async move {
            match native::list_sources().await {
                Ok(found) => {
                    if mic_source().is_empty() {
                        if let Some(source) = found.iter().find(|source| !source.monitor) {
                            mic_source.set(source.name.clone());
                        }
                    }
                    if system_source().is_empty() {
                        if let Some(source) = found.iter().find(|source| source.monitor) {
                            system_source.set(source.name.clone());
                        }
                    }
                    sources.set(found);
                }
                Err(message) => error.set(Some(message)),
            }
        });
    });

    let is_recording = state() == RecordingState::Recording;
    let has_recording = completed.read().is_some();
    let is_busy = matches!(
        state(),
        RecordingState::Starting | RecordingState::Stopping | RecordingState::Uploading
    );

    #[cfg(target_arch = "wasm32")]
    let source_controls = rsx! {
        p { class: "semantic-audio-recording__hint",
            "Your browser will ask for microphone permission. Recording stays local until you stop and upload it."
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    let source_controls = rsx! {
        div { class: "semantic-audio-recording__sources",
            label { class: "semantic-audio-recording__field",
                span { "Microphone" }
                select {
                    value: mic_source(),
                    disabled: is_recording || is_busy || has_recording,
                    onchange: move |event| mic_source.set(event.value()),
                    option { value: "", "Select a microphone" }
                    for source in sources().into_iter().filter(|source| !source.monitor) {
                        option { value: source.name.clone(), "{source.description}" }
                    }
                }
            }
            label { class: "semantic-audio-recording__field",
                span { "System audio" }
                select {
                    value: system_source(),
                    disabled: is_recording || is_busy || has_recording,
                    onchange: move |event| system_source.set(event.value()),
                    option { value: "", "Select a monitor source" }
                    for source in sources().into_iter().filter(|source| source.monitor) {
                        option { value: source.name.clone(), "{source.description}" }
                    }
                }
            }
            p { class: "semantic-audio-recording__hint",
                "Desktop recording mixes the selected microphone and system monitor using pactl and ffmpeg."
            }
        }
    };

    rsx! {
        section { class: "semantic-audio-recording",
            PageHeader {
                title: "Record audio",
                description: "Capture audio, then save it using the regular file upload pipeline."
            }
            if let Some(message) = error() {
                InlineNotice {
                    message,
                    variant: NoticeVariant::Error,
                    on_dismiss: move |_| error.set(None),
                }
            }
            div { class: "semantic-audio-recording__panel",
                label { class: "semantic-audio-recording__field",
                    span { "Recording title" }
                    input {
                        value: title(),
                        disabled: is_recording || is_busy,
                        oninput: move |event| title.set(event.value()),
                    }
                }

                {source_controls}

                div { class: "semantic-audio-recording__actions",
                    if has_recording {
                        dxcomp::Button {
                            disabled: is_busy,
                            onclick: {
                                let client = client.clone();
                                let scope_id = scope_id.clone();
                                move |_| {
                                    let Some(recording) = completed.peek().clone() else { return };
                                    spawn(save_recording(client.clone(), scope_id.clone(), title(), recording, completed, state, error));
                                }
                            },
                            "Retry upload"
                        }
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Destructive,
                            disabled: is_busy,
                            onclick: move |_| {
                                completed.set(None);
                                error.set(None);
                                state.set(RecordingState::Idle);
                            },
                            "Discard recording"
                        }
                    } else if !is_recording {
                        dxcomp::Button {
                            disabled: is_busy,
                            onclick: {
                                let recorder = recorder.clone();
                                move |_| {
                                    error.set(None);
                                    state.set(RecordingState::Starting);
                                    #[cfg(target_arch = "wasm32")]
                                    let (mic, system) = (String::new(), String::new());
                                    #[cfg(not(target_arch = "wasm32"))]
                                    let (mic, system) = (mic_source(), system_source());
                                    let recorder = recorder.clone();
                                    spawn(async move {
                                        match recorder.start(&mic, &system).await {
                                            Ok(()) => {
                                                state.set(RecordingState::Recording);
                                                #[cfg(not(target_arch = "wasm32"))]
                                                if let Some(message) = recorder.wait_for_failure().await {
                                                    if state() == RecordingState::Recording {
                                                        state.set(RecordingState::Idle);
                                                        error.set(Some(message));
                                                    }
                                                }
                                            }
                                            Err(message) => {
                                                state.set(RecordingState::Idle);
                                                error.set(Some(message));
                                            }
                                        }
                                    });
                                }
                            },
                            if state() == RecordingState::Starting { "Starting…" } else { "Start recording" }
                        }
                    } else {
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Destructive,
                            onclick: {
                                let recorder = recorder.clone();
                                move |_| {
                                    state.set(RecordingState::Stopping);
                                    let recording_title = title();
                                    let client = client.clone();
                                    let scope_id = scope_id.clone();
                                    let recorder = recorder.clone();
                                    spawn(async move {
                                        match recorder.stop().await {
                                            Ok(recording) => {
                                                completed.set(Some(recording.clone()));
                                                save_recording(client, scope_id, recording_title, recording, completed, state, error).await;
                                            }
                                            Err(message) => { state.set(RecordingState::Idle); error.set(Some(message)); }
                                        }
                                    });
                                }
                            },
                            "Stop and upload"
                        }
                    }
                    span { class: "semantic-audio-recording__status",
                        {match state() {
                            RecordingState::Idle => "Ready".to_string(),
                            RecordingState::Starting => "Starting recorder…".to_string(),
                            RecordingState::Recording => "Recording now".to_string(),
                            RecordingState::Stopping => "Finishing recording…".to_string(),
                            RecordingState::Uploading => "Uploading…".to_string(),
                            RecordingState::ReadyToUpload => "Recording kept on this page. Retry upload or discard it.".to_string(),
                            RecordingState::Complete(ref id) => format!("Uploaded as {id}"),
                        }}
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
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
) -> std::result::Result<String, String> {
    let mut entity = Object::new();
    let title = title.trim();
    if !title.is_empty() {
        entity.insert(ATTR_TITLE, Value::String(title.to_string()));
    }
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let filename = format!("audio-recording-{timestamp}.{}", recording.extension);
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
        .map(|response| response.id)
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
        "audio/mp4" | "video/mp4" => "m4a",
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
        process::{Child, Command},
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

    #[derive(Default)]
    struct RecorderState {
        closed: bool,
        session: Option<NativeSession>,
    }

    // Dropping the stop sender cancels the supervisor, which kills and reaps the child
    // before dropping the tempfile. The UI never owns an unsupervised process.
    struct NativeSession {
        stop: Option<oneshot::Sender<()>>,
        result: watch::Receiver<Option<Result<CompletedRecording, String>>>,
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
            tokio::spawn(async move {
                let result = supervise(child, output, stop_rx, stop_timeout).await;
                let _ = result_tx.send(Some(result));
            });
            Ok(Self {
                stop: Some(stop),
                result,
            })
        }
    }

    impl NativeRecorder {
        pub async fn start(&self, mic: &str, system: &str) -> std::result::Result<(), String> {
            if mic.is_empty() || system.is_empty() {
                return Err("Select both a microphone and a system-audio source.".to_string());
            }
            let output = tempfile::Builder::new()
                .prefix("semantic-recording-")
                .suffix(".mp3")
                .tempfile()
                .map_err(|error| format!("Could not create a temporary recording: {error}"))?;
            let mut command = Command::new("ffmpeg");
            command
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "warning",
                    "-y",
                    "-thread_queue_size",
                    "4096",
                    "-f",
                    "pulse",
                    "-i",
                    mic,
                    "-thread_queue_size",
                    "4096",
                    "-f",
                    "pulse",
                    "-i",
                    system,
                    "-filter_complex",
                    "[0:a][1:a]amix=inputs=2:duration=longest:dropout_transition=0[aout]",
                    "-map",
                    "[aout]",
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

        pub async fn stop(&self) -> std::result::Result<CompletedRecording, String> {
            let Some(mut session) = self.0.lock().unwrap().session.take() else {
                return Err("No desktop recording is active.".to_string());
            };
            if let Some(stop) = session.stop.take() {
                let _ = stop.send(());
            }
            wait_for_result(&mut session.result).await
        }

        pub async fn wait_for_failure(&self) -> Option<String> {
            let mut result = self.0.lock().unwrap().session.as_ref()?.result.clone();
            wait_for_result(&mut result).await.err()
        }

        pub fn close(&self) {
            let mut inner = self.0.lock().unwrap();
            inner.closed = true;
            inner.session = None;
        }
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
        let mut tail = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = reader
                .read(&mut buffer)
                .await
                .map_err(|error| format!("Could not read ffmpeg diagnostics: {error}"))?;
            if count == 0 {
                return Ok(tail);
            }
            let overflow = (tail.len() + count).saturating_sub(STDERR_LIMIT);
            tail.drain(..overflow);
            tail.extend_from_slice(&buffer[..count]);
        }
    }

    async fn stop_child(child: &mut Child, stop_timeout: Duration) -> Result<(), String> {
        match timeout(stop_timeout, async {
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(b"q\n")
                    .await
                    .map_err(|error| format!("Could not request recording stop: {error}"))?;
            }
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
    ) -> Result<CompletedRecording, String> {
        let mut stderr = StderrTask(tokio::spawn(drain_stderr(
            child.stderr.take().expect("piped stderr"),
        )));
        let result = tokio::select! {
            status = child.wait() => Err(match status {
                Ok(status) => format!("ffmpeg exited before recording was stopped ({status})"),
                Err(error) => format!("Could not monitor ffmpeg: {error}"),
            }),
            requested = stop => match requested {
                Ok(()) => stop_child(&mut child, stop_timeout).await,
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
                RecordingState::Complete("recording-id".to_string())
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
