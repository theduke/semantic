use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use futures::channel::oneshot;
use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Blob, BlobEvent, Event, MediaRecorder, MediaRecorderErrorEvent, MediaRecorderOptions,
    MediaStream, MediaStreamConstraints, MediaStreamTrack, RecordingState,
};

use super::{CaptureMode, CompletedRecording};

const CLOSED: &str = "The recording page has been closed.";

#[derive(Clone, Default)]
pub(super) struct BrowserRecorder(Rc<RecorderState>);

impl PartialEq for BrowserRecorder {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Default)]
struct RecorderState {
    closed: Cell<bool>,
    starting: Cell<bool>,
    session: RefCell<Option<Rc<BrowserSession>>>,
    preview: RefCell<Option<CaptureStream>>,
}

impl BrowserRecorder {
    pub fn is_recording(&self) -> bool {
        self.0
            .session
            .borrow()
            .as_ref()
            .is_some_and(|session| session.recorder.state() == RecordingState::Recording)
    }

    pub fn level(&self) -> Option<f32> {
        let session = self.0.session.borrow();
        session.as_ref()?.meter.as_ref()?.level()
    }

    pub fn stream(&self) -> Option<MediaStream> {
        self.0
            .preview
            .borrow()
            .as_ref()
            // MediaStream::clone() clones its tracks; clone the JS handle only.
            .map(|stream| Clone::clone(&stream.0))
            .or_else(|| {
                self.0
                    .session
                    .borrow()
                    .as_ref()
                    .map(|session| Clone::clone(&session.stream.0))
            })
    }

    pub fn finished(&self) -> bool {
        self.0
            .session
            .borrow()
            .as_ref()
            .is_some_and(|session| session.capture.borrow().completion.is_none())
    }

    pub fn preview_ended(&self) -> bool {
        self.0.preview.borrow().as_ref().is_some_and(|stream| {
            stream.0.get_video_tracks().iter().all(|track| {
                track.unchecked_into::<MediaStreamTrack>().ready_state()
                    == web_sys::MediaStreamTrackState::Ended
            })
        })
    }

    pub fn has_audio(&self) -> bool {
        self.stream()
            .is_some_and(|stream| stream.get_audio_tracks().length() > 0)
    }

    pub async fn prepare(&self, mode: CaptureMode, audio: bool) -> Result<(), String> {
        self.acquire(mode, audio, true).await
    }

    pub async fn start(&self, mode: CaptureMode, audio: bool) -> Result<(), String> {
        self.acquire(mode, audio, false).await
    }

    async fn acquire(&self, mode: CaptureMode, audio: bool, preview: bool) -> Result<(), String> {
        if self.0.closed.get() {
            return Err(CLOSED.to_string());
        }
        if self.0.starting.get() || self.0.session.borrow().is_some() {
            return Err("A browser recording is already active.".to_string());
        }
        self.0.starting.set(true);
        let state = self.0.clone();
        let prepared = state.preview.borrow_mut().take();
        let (sender, receiver) = oneshot::channel();
        // Component tasks are cancelled on unmount, but getUserMedia cannot be
        // cancelled. Always receive its eventual stream so its tracks get stopped.
        spawn_local(async move {
            let result = match prepared {
                Some(stream) => Ok(stream),
                None => acquire_stream(mode, audio).await,
            }
            .and_then(|stream| {
                if state.closed.get() || sender.is_canceled() {
                    return Err(CLOSED.to_string());
                }
                if preview {
                    *state.preview.borrow_mut() = Some(stream);
                } else {
                    let session = BrowserSession::start(stream, mode)?;
                    *state.session.borrow_mut() = Some(Rc::new(session));
                }
                Ok(())
            });
            state.starting.set(false);
            if sender.send(result).is_err() {
                state.session.borrow_mut().take();
                state.preview.borrow_mut().take();
            }
        });
        receiver
            .await
            .map_err(|_| "Capture initialization was cancelled.".to_string())?
    }

    pub fn photograph(
        &self,
        video: &web_sys::HtmlVideoElement,
    ) -> Result<CompletedRecording, String> {
        use base64::Engine as _;
        if video.ready_state() < web_sys::HtmlMediaElement::HAVE_CURRENT_DATA
            || video.video_width() == 0
            || video.video_height() == 0
            || self.preview_ended()
        {
            return Err(
                "The camera is not ready yet. Wait for the live preview, then try again."
                    .to_string(),
            );
        }
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| "No browser document is available.".to_string())?;
        let canvas = document
            .create_element("canvas")
            .map_err(|error| browser_error("Could not prepare photo", error))?
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .map_err(|error| browser_error("Could not prepare photo canvas", error.into()))?;
        canvas.set_width(video.video_width());
        canvas.set_height(video.video_height());
        let context = canvas
            .get_context("2d")
            .map_err(|error| browser_error("Could not prepare photo", error))?
            .ok_or_else(|| "Photo capture is not supported by this browser.".to_string())?
            .dyn_into::<web_sys::CanvasRenderingContext2d>()
            .map_err(|error| browser_error("Could not prepare photo context", error.into()))?;
        context
            .draw_image_with_html_video_element(video, 0.0, 0.0)
            .map_err(|error| browser_error("Could not capture photo", error))?;
        let data = canvas
            .to_data_url_with_type("image/png")
            .map_err(|error| browser_error("Could not encode photo", error))?;
        let (_, encoded) = data
            .split_once(',')
            .ok_or_else(|| "The browser returned an invalid photo.".to_string())?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "The browser returned an invalid photo.".to_string())?;
        let recording = super::browser_recording(bytes, "image/png".to_string(), None)?;
        self.discard();
        Ok(recording)
    }

    pub async fn pause(&self) -> Result<(), String> {
        let session = self.0.session.borrow();
        let session = session
            .as_ref()
            .ok_or_else(|| "No browser recording is active.".to_string())?;
        session
            .recorder
            .pause()
            .map_err(|error| browser_error("Could not pause recording", error))
    }

    pub async fn resume(&self) -> Result<(), String> {
        let session = self.0.session.borrow();
        let session = session
            .as_ref()
            .ok_or_else(|| "No browser recording is active.".to_string())?;
        session
            .recorder
            .resume()
            .map_err(|error| browser_error("Could not resume recording", error))
    }

    pub async fn stop(&self) -> Result<CompletedRecording, String> {
        let session = self
            .0
            .session
            .borrow()
            .clone()
            .ok_or_else(|| "No browser recording is active.".to_string())?;
        let receiver = session
            .finished
            .borrow_mut()
            .take()
            .ok_or_else(|| "The browser recording is already stopping.".to_string())?;
        // Cancellation while waiting for onstop or array_buffer releases capture too.
        let _cleanup = StopCleanup(self.0.clone());
        if session.recorder.state() != RecordingState::Inactive {
            session
                .recorder
                .stop()
                .map_err(|error| browser_error("Could not stop recording", error))?;
        }
        receiver
            .await
            .map_err(|_| "Browser recording was cancelled.".to_string())??;
        let (blob, chunk_mime) = {
            let capture = session.capture.borrow();
            let parts = js_sys::Array::new();
            for chunk in &capture.chunks {
                parts.push(chunk);
            }
            let chunk_mime = capture
                .chunks
                .iter()
                .map(Blob::type_)
                .find(|mime| !mime.is_empty());
            let blob = Blob::new_with_blob_sequence(&parts)
                .map_err(|error| browser_error("Could not assemble recording", error))?;
            (blob, chunk_mime)
        };
        let buffer = JsFuture::from(blob.array_buffer())
            .await
            .map_err(|error| browser_error("Could not read recording", error))?;
        if self.0.closed.get() {
            return Err(CLOSED.to_string());
        }
        super::browser_recording(
            js_sys::Uint8Array::new(&buffer).to_vec(),
            session.recorder.mime_type(),
            chunk_mime,
        )
    }

    pub fn close(&self) {
        self.0.closed.set(true);
        self.discard();
    }

    pub fn discard(&self) {
        self.0.preview.borrow_mut().take();
        if let Some(session) = self.0.session.borrow_mut().take() {
            session.release();
        }
    }
}

struct StopCleanup(Rc<RecorderState>);

impl Drop for StopCleanup {
    fn drop(&mut self) {
        if let Some(session) = self.0.session.borrow_mut().take() {
            session.release();
        }
    }
}

struct CaptureStream(MediaStream);

impl CaptureStream {
    fn stop(&self) {
        stop_tracks(&self.0);
    }
}

impl Drop for CaptureStream {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn acquire_stream(mode: CaptureMode, audio: bool) -> Result<CaptureStream, String> {
    let window = web_sys::window().ok_or_else(|| "No browser window is available.".to_string())?;
    if mode != CaptureMode::Photo
        && !js_sys::Reflect::has(&window, &JsValue::from_str("MediaRecorder")).unwrap_or(false)
    {
        return Err(
            "This browser does not support media recording. Try a current browser.".to_string(),
        );
    }
    let devices = window.navigator().media_devices().map_err(|error| {
        browser_error(
            "Capture requires a supported browser and HTTPS or localhost",
            error,
        )
    })?;
    if devices.is_undefined() || devices.is_null() {
        return Err("Capture requires a supported browser and HTTPS or localhost.".to_string());
    }
    let request = if mode == CaptureMode::Screen {
        if !js_sys::Reflect::has(&devices, &JsValue::from_str("getDisplayMedia")).unwrap_or(false) {
            return Err("Screen recording is unavailable in this browser. Try a desktop browser that supports screen sharing.".to_string());
        }
        let options = web_sys::DisplayMediaStreamConstraints::new();
        options.set_video(&JsValue::TRUE);
        options.set_audio(&JsValue::from_bool(audio));
        devices.get_display_media_with_constraints(&options)
    } else {
        let constraints = MediaStreamConstraints::new();
        constraints.set_audio(&JsValue::from_bool(mode == CaptureMode::Audio || (mode == CaptureMode::Video && audio)));
        constraints.set_video(&JsValue::from_bool(mode.is_camera()));
        devices.get_user_media_with_constraints(&constraints)
    }.map_err(|error| browser_error("Could not request capture access", error))?;
    let stream = JsFuture::from(request)
        .await
        .map_err(|error| {
            browser_error(
                "Capture did not start. Allow access or select a source and try again",
                error,
            )
        })?
        .dyn_into::<MediaStream>()
        .map_err(|error| browser_error("The browser returned an invalid capture stream", error))?;
    Ok(CaptureStream(stream))
}

struct Capture {
    chunks: Vec<Blob>,
    completion: Option<oneshot::Sender<Result<(), String>>>,
}

impl Capture {
    fn finish(&mut self, result: Result<(), String>) {
        if let Some(sender) = self.completion.take() {
            let _ = sender.send(result);
        }
    }
}

struct BrowserSession {
    recorder: MediaRecorder,
    meter: Option<AudioMeter>,
    stream: CaptureStream,
    capture: Rc<RefCell<Capture>>,
    finished: RefCell<Option<oneshot::Receiver<Result<(), String>>>>,
    released: Cell<bool>,
    // Keep closures alive until all event handlers have been detached by release.
    _on_data: Closure<dyn FnMut(BlobEvent)>,
    _on_stop: Closure<dyn FnMut(Event)>,
    _on_error: Closure<dyn FnMut(Event)>,
    _on_ended: Closure<dyn FnMut(Event)>,
}

impl BrowserSession {
    fn start(stream: CaptureStream, mode: CaptureMode) -> Result<Self, String> {
        let options = MediaRecorderOptions::new();
        let audio_mimes = [
            "audio/mpeg",
            "audio/webm;codecs=opus",
            "audio/webm",
            "audio/ogg;codecs=opus",
            "audio/mp4",
        ];
        let video_mimes = [
            "video/webm;codecs=vp9,opus",
            "video/webm;codecs=vp8,opus",
            "video/webm",
            "video/mp4",
        ];
        let candidates: &[&str] = if mode == CaptureMode::Audio {
            &audio_mimes
        } else {
            &video_mimes
        };
        if let Some(mime) = candidates
            .iter()
            .copied()
            .find(|mime| MediaRecorder::is_type_supported(mime))
        {
            options.set_mime_type(mime);
        }
        let recorder =
            MediaRecorder::new_with_media_stream_and_media_recorder_options(&stream.0, &options)
                .map_err(|error| browser_error("Could not initialize recording", error))?;
        let (sender, receiver) = oneshot::channel();
        let capture = Rc::new(RefCell::new(Capture {
            chunks: Vec::new(),
            completion: Some(sender),
        }));
        let on_data = {
            let capture = capture.clone();
            Closure::new(move |event: BlobEvent| {
                if let Some(blob) = event.data().filter(|blob| blob.size() > 0.0) {
                    capture.borrow_mut().chunks.push(blob);
                }
            })
        };
        let on_stop = {
            let capture = capture.clone();
            let stream = Clone::clone(&stream.0);
            Closure::new(move |_: Event| {
                stop_tracks(&stream);
                capture.borrow_mut().finish(Ok(()));
            })
        };
        let on_error = {
            let capture = capture.clone();
            let stream = Clone::clone(&stream.0);
            Closure::new(move |event: Event| {
                stop_tracks(&stream);
                let message = event
                    .dyn_ref::<MediaRecorderErrorEvent>()
                    .map(|event| event.error().message())
                    .filter(|message| !message.is_empty())
                    .unwrap_or_else(|| "Browser recording failed.".to_string());
                capture.borrow_mut().finish(Err(message));
            })
        };
        recorder.set_ondataavailable(Some(on_data.as_ref().unchecked_ref()));
        recorder.set_onstop(Some(on_stop.as_ref().unchecked_ref()));
        recorder.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        // Ending screen sharing must finalize video even if an audio track remains live.
        let on_ended = {
            let recorder = recorder.clone();
            Closure::new(move |_: Event| {
                if recorder.state() != RecordingState::Inactive {
                    let _ = recorder.stop();
                }
            })
        };
        for track in stream.0.get_tracks().iter() {
            track
                .unchecked_into::<MediaStreamTrack>()
                .set_onended(Some(on_ended.as_ref().unchecked_ref()));
        }
        let session = Self {
            recorder,
            meter: AudioMeter::new(&stream.0).ok(),
            stream,
            capture,
            finished: RefCell::new(Some(receiver)),
            released: Cell::new(false),
            _on_data: on_data,
            _on_stop: on_stop,
            _on_error: on_error,
            _on_ended: on_ended,
        };
        session
            .recorder
            .start_with_time_slice(1000)
            .map_err(|error| browser_error("Could not start recording", error))?;
        Ok(session)
    }

    fn release(&self) {
        if self.released.replace(true) {
            return;
        }
        self.recorder.set_ondataavailable(None);
        self.recorder.set_onstop(None);
        self.recorder.set_onerror(None);
        for track in self.stream.0.get_tracks().iter() {
            track.unchecked_into::<MediaStreamTrack>().set_onended(None);
        }
        if self.recorder.state() != RecordingState::Inactive {
            let _ = self.recorder.stop();
        }
        self.stream.stop();
        if let Some(meter) = &self.meter {
            meter.close();
        }
        let mut capture = self.capture.borrow_mut();
        capture.finish(Err("Browser recording was cancelled.".to_string()));
        capture.chunks.clear();
    }
}

struct AudioMeter {
    context: web_sys::AudioContext,
    closed: Cell<bool>,
    analyser: web_sys::AnalyserNode,
    _source: web_sys::MediaStreamAudioSourceNode,
    samples: RefCell<Vec<f32>>,
}

impl AudioMeter {
    fn new(stream: &MediaStream) -> Result<Self, JsValue> {
        let context = web_sys::AudioContext::new()?;
        let result = (|| {
            let analyser = context.create_analyser()?;
            analyser.set_fft_size(256);
            let source = context.create_media_stream_source(stream)?;
            source.connect_with_audio_node(&analyser)?;
            // No connection to the destination: microphone monitoring must not
            // play back through the speakers or introduce feedback.
            let _ = context.resume();
            Ok(Self {
                context: context.clone(),
                closed: Cell::new(false),
                analyser,
                _source: source,
                samples: RefCell::new(vec![0.0; 256]),
            })
        })();
        if result.is_err() {
            let _ = context.close();
        }
        result
    }

    fn level(&self) -> Option<f32> {
        if self.context.state() != web_sys::AudioContextState::Running {
            return None;
        }
        let mut samples = self.samples.borrow_mut();
        self.analyser.get_float_time_domain_data(&mut samples);
        let rms = (samples.iter().map(|sample| sample * sample).sum::<f32>()
            / samples.len() as f32)
            .sqrt();
        Some(super::normalized_level(20.0 * rms.max(1e-6).log10()))
    }

    fn close(&self) {
        if !self.closed.replace(true) && self.context.state() != web_sys::AudioContextState::Closed
        {
            let _ = self.context.close();
        }
    }
}

impl Drop for AudioMeter {
    fn drop(&mut self) {
        self.close();
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        self.release();
    }
}

fn stop_tracks(stream: &MediaStream) {
    for track in stream.get_tracks().iter() {
        if let Ok(track) = track.dyn_into::<MediaStreamTrack>() {
            track.stop();
        }
    }
}

fn browser_error(context: &str, error: JsValue) -> String {
    let message = error
        .dyn_ref::<web_sys::DomException>()
        .map(|error| error.message())
        .or_else(|| {
            error
                .dyn_ref::<js_sys::Error>()
                .map(|error| error.message().into())
        })
        .or_else(|| error.as_string())
        .unwrap_or_else(|| format!("{error:?}"));
    format!("{context}: {message}")
}
