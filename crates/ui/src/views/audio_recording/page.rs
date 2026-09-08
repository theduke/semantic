use std::time::Duration;

use dioxus::prelude::*;
use semantic_ui_core::{
    EntityCard, EntityDisplayRenderer, EntityRenderOptions,
    components::{InlineNotice, NoticeVariant},
    use_active_scope_id, use_rpc_client,
};
use web_time::Instant;

use crate::components::{ConfirmAction, ConfirmActionVariant, PageHeader};

use super::{CompletedRecording, Recorder, RecordingState, save_recording};

#[component]
pub fn AudioRecordingPage() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut state = use_signal(|| RecordingState::Idle);
    let mut error = use_signal(|| None::<String>);
    let mut title = use_signal(|| "Audio recording".to_string());
    let mut completed = use_signal(|| None::<CompletedRecording>);
    let mut clock = use_signal(CaptureClock::default);
    let mut discard_open = use_signal(|| false);
    let recorder = use_hook(Recorder::default);
    let cleanup = recorder.clone();
    use_drop(move || cleanup.close());

    #[cfg(not(target_arch = "wasm32"))]
    let mut sources = use_signal(Vec::<super::native::AudioSource>::new);
    #[cfg(not(target_arch = "wasm32"))]
    let mut source_loading = use_signal(|| true);
    #[cfg(not(target_arch = "wasm32"))]
    let mut sources_initialized = use_signal(|| false);
    #[cfg(not(target_arch = "wasm32"))]
    let mut mic_source = use_signal(String::new);
    #[cfg(not(target_arch = "wasm32"))]
    let mut system_source = use_signal(String::new);
    #[cfg(not(target_arch = "wasm32"))]
    let load_sources = use_callback(move |_: ()| {
        source_loading.set(true);
        error.set(None);
        spawn(async move {
            match super::native::list_sources().await {
                Ok(found) => {
                    if !*sources_initialized.peek() {
                        mic_source.set(
                            found
                                .iter()
                                .find(|source| !source.monitor)
                                .map(|source| source.name.clone())
                                .unwrap_or_default(),
                        );
                        system_source.set(
                            found
                                .iter()
                                .find(|source| source.monitor)
                                .map(|source| source.name.clone())
                                .unwrap_or_default(),
                        );
                    } else {
                        if !found.iter().any(|source| source.name == *mic_source.peek()) {
                            mic_source.set(String::new());
                        }
                        if !found
                            .iter()
                            .any(|source| source.name == *system_source.peek())
                        {
                            system_source.set(String::new());
                        }
                    }
                    sources_initialized.set(true);
                    sources.set(found);
                }
                Err(message) => error.set(Some(message)),
            }
            source_loading.set(false);
        });
    });
    #[cfg(not(target_arch = "wasm32"))]
    use_effect(move || load_sources.call(()));

    let current = state();
    let capturing = matches!(
        current,
        RecordingState::Recording
            | RecordingState::Paused
            | RecordingState::Pausing
            | RecordingState::Resuming
    );
    let busy = matches!(
        current,
        RecordingState::Starting
            | RecordingState::Pausing
            | RecordingState::Resuming
            | RecordingState::Stopping
            | RecordingState::Uploading
    );
    let has_recording = completed.read().is_some();
    let saved = matches!(current, RecordingState::Complete(_));
    #[cfg(target_arch = "wasm32")]
    let can_start = true;
    #[cfg(not(target_arch = "wasm32"))]
    let can_start = !source_loading() && (!mic_source().is_empty() || !system_source().is_empty());

    #[cfg(target_arch = "wasm32")]
    let source_controls = rsx! {};
    #[cfg(not(target_arch = "wasm32"))]
    let source_controls = rsx! {
        div { class: "semantic-audio-recording__sources", "aria-busy": source_loading(),
            label { class: "semantic-audio-recording__field",
                span { "Microphone" }
                select { value: mic_source(), disabled: capturing || busy || source_loading(), onchange: move |event| mic_source.set(event.value()),
                    option { value: "", "Off" }
                    for source in sources().into_iter().filter(|source| !source.monitor) {
                        option { key: "{source.name}", value: source.name.clone(), "{source.description}" }
                    }
                }
            }
            label { class: "semantic-audio-recording__field",
                span { "System audio" }
                select { value: system_source(), disabled: capturing || busy || source_loading(), onchange: move |event| system_source.set(event.value()),
                    option { value: "", "Off" }
                    for source in sources().into_iter().filter(|source| source.monitor) {
                        option { key: "{source.name}", value: source.name.clone(), "{source.description}" }
                    }
                }
            }
        }
        p { class: "semantic-audio-recording__hint",
            if source_loading() { "Finding audio sources…" }
            else if !can_start { "Choose at least one audio source to start recording." }
            else { "Record either source, or combine both into one recording." }
        }
        dxcomp::Button { variant: dxcomp::ButtonVariant::Ghost, disabled: capturing || busy || source_loading(), onclick: move |_| load_sources.call(()), "Refresh sources" }
    };

    rsx! {
        section { class: "semantic-audio-recording",
            PageHeader {
                title: "Record audio",
                description: "Record, listen back, and save when you’re ready."
            }
            ol { class: "semantic-audio-recording__steps", "aria-label": "Recording steps",
                for (index, label, active) in [(1, "Record", !has_recording && !saved), (2, "Review", has_recording), (3, "Saved", saved)] {
                    li { key: "{index}", "data-active": active, "aria-current": if active { "step" } else { "false" },
                        span { "aria-hidden": "true", "{index}" }
                        "{label}"
                    }
                }
            }
            if let Some(message) = error() {
                InlineNotice { message, variant: NoticeVariant::Error, on_dismiss: move |_| error.set(None) }
            }
            if let RecordingState::Complete(result) = current.clone() {
                section { class: "semantic-audio-recording__result", "aria-labelledby": "recording-saved-title",
                    div { class: "semantic-audio-recording__result-heading",
                        div {
                            h2 { id: "recording-saved-title", "Recording saved" }
                            p { role: "status", "Your audio is ready in your library." }
                        }
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| { state.set(RecordingState::Idle); clock.set(CaptureClock::default()); title.set("Audio recording".to_string()); },
                            "Record another"
                        }
                    }
                    EntityCard {
                        object: result.object,
                        options: EntityRenderOptions {
                            collection: Some(result.collection), id: Some(result.id),
                            renderer: EntityDisplayRenderer::Custom, preview: true, actions: true,
                        }
                    }
                }
            } else {
                div { class: "semantic-audio-recording__panel", "aria-busy": busy,
                    if !has_recording {
                        div { class: "semantic-audio-recording__capture", "data-state": current.slug(),
                            p { class: "semantic-audio-recording__status", role: "status",
                                span { class: "semantic-audio-recording__indicator", "aria-hidden": "true" }
                                "{current.label()}"
                            }
                            RecordingTimer { clock }
                            if capturing {
                                RecordingLevels { recorder: recorder.clone(), active: current == RecordingState::Recording }
                            }
                            p { class: "semantic-audio-recording__hint",
                                if capturing { "Paused time is left out of your recording." }
                                else if current == RecordingState::Stopping { "Preparing your audio for playback…" }
                                else { "Your audio stays on this page until you upload it." }
                            }
                        }
                    } else {
                        div { class: "semantic-audio-recording__review-heading",
                            h2 { "Listen before saving" }
                            p { class: "semantic-audio-recording__hint", "Review your recording, give it a title, then upload it to your library." }
                        }
                        if let Some(recording) = completed() {
                            RecordingPlayback { recording }
                        }
                        p { class: "semantic-audio-recording__hint", "Captured time: {format_duration(clock.read().elapsed(Instant::now()))}" }
                    }

                    label { class: "semantic-audio-recording__field",
                        span { "Recording title" }
                        input {
                            value: title(), disabled: current == RecordingState::Uploading,
                            placeholder: "Give this recording a name",
                            oninput: move |event| title.set(event.value()),
                        }
                    }

                    if !has_recording {
                        div { class: "semantic-audio-recording__source-section",
                            h2 { "Audio sources" }
                            if cfg!(target_arch = "wasm32") {
                                p { "Microphone" }
                                p { class: "semantic-audio-recording__hint", "Your browser will ask for microphone access when you start." }
                            }
                            {source_controls}
                        }
                    }

                    div { class: "semantic-audio-recording__actions",
                        if has_recording {
                            dxcomp::Button {
                                disabled: busy,
                                onclick: {
                                    let client = client.clone();
                                    let scope_id = scope_id.clone();
                                    move |_| {
                                        let Some(recording) = completed.peek().clone() else { return };
                                        spawn(save_recording(client.clone(), scope_id.clone(), title(), recording, completed, state, error));
                                    }
                                },
                                if busy { "Uploading recording…" } else if error.read().is_some() { "Retry upload" } else { "Upload recording" }
                            }
                            dxcomp::Button {
                                variant: dxcomp::ButtonVariant::Outline, disabled: busy,
                                onclick: move |_| discard_open.set(true), "Discard & start over"
                            }
                        } else if capturing {
                            dxcomp::Button {
                                disabled: busy,
                                onclick: {
                                    let recorder = recorder.clone();
                                    move |_| {
                                        clock.write().pause(Instant::now());
                                        state.set(RecordingState::Stopping);
                                        error.set(None);
                                        let recorder = recorder.clone();
                                        spawn(async move {
                                            match recorder.stop().await {
                                                Ok(recording) => { completed.set(Some(recording)); state.set(RecordingState::ReadyToUpload); }
                                                Err(message) => {
                                                    #[cfg(not(target_arch = "wasm32"))]
                                                    state.set(RecordingState::Paused);
                                                    #[cfg(target_arch = "wasm32")]
                                                    state.set(RecordingState::Idle);
                                                    error.set(Some(message));
                                                }
                                            }
                                        });
                                    }
                                },
                                "Stop & review"
                            }
                            dxcomp::Button {
                                variant: dxcomp::ButtonVariant::Outline, disabled: busy,
                                onclick: {
                                    let recorder = recorder.clone();
                                    move |_| {
                                        let pause = *state.peek() == RecordingState::Recording;
                                        if pause { clock.write().pause(Instant::now()); }
                                        state.set(if pause { RecordingState::Pausing } else { RecordingState::Resuming });
                                        error.set(None);
                                        let recorder = recorder.clone();
                                        spawn(async move {
                                            let result = if pause { recorder.pause().await } else { recorder.resume().await };
                                            match result {
                                                Ok(()) => {
                                                    if !pause { clock.write().resume(Instant::now()); }
                                                    state.set(if pause { RecordingState::Paused } else { RecordingState::Recording });
                                                    #[cfg(not(target_arch = "wasm32"))]
                                                    if !pause {
                                                        if let Some(message) = recorder.wait_for_failure().await {
                                                            if *state.peek() == RecordingState::Recording {
                                                                clock.write().pause(Instant::now()); state.set(RecordingState::Paused); error.set(Some(message));
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(message) => {
                                                    if recorder.is_recording() {
                                                        clock.write().resume(Instant::now());
                                                        state.set(RecordingState::Recording);
                                                    } else { state.set(RecordingState::Paused); }
                                                    error.set(Some(message));
                                                }
                                            }
                                        });
                                    }
                                },
                                if current == RecordingState::Paused { "Resume recording" } else if current == RecordingState::Pausing { "Pausing…" } else if current == RecordingState::Resuming { "Resuming…" } else { "Pause recording" }
                            }
                            dxcomp::Button {
                                variant: dxcomp::ButtonVariant::Ghost, disabled: busy,
                                onclick: move |_| discard_open.set(true), "Discard recording"
                            }
                        } else {
                            dxcomp::Button {
                                disabled: busy || !can_start,
                                onclick: {
                                    let recorder = recorder.clone();
                                    move |_| {
                                        error.set(None); clock.set(CaptureClock::default()); state.set(RecordingState::Starting);
                                        #[cfg(target_arch = "wasm32")]
                                        let (mic, system) = (String::new(), String::new());
                                        #[cfg(not(target_arch = "wasm32"))]
                                        let (mic, system) = (mic_source(), system_source());
                                        let recorder = recorder.clone();
                                        spawn(async move {
                                            match recorder.start(&mic, &system).await {
                                                Ok(()) => {
                                                    clock.write().resume(Instant::now()); state.set(RecordingState::Recording);
                                                    #[cfg(not(target_arch = "wasm32"))]
                                                    if let Some(message) = recorder.wait_for_failure().await {
                                                        if *state.peek() == RecordingState::Recording {
                                                            clock.write().pause(Instant::now()); state.set(RecordingState::Idle); error.set(Some(message));
                                                        }
                                                    }
                                                }
                                                Err(message) => { state.set(RecordingState::Idle); error.set(Some(message)); }
                                            }
                                        });
                                    }
                                },
                                if current == RecordingState::Starting { "Starting recording…" } else if current == RecordingState::Stopping { "Preparing recording…" } else { "Start recording" }
                            }
                        }
                    }
                    if has_recording {
                        p { class: "semantic-audio-recording__hint", role: "status",
                            if current == RecordingState::Uploading { "Uploading… Keep this page open until your recording is saved." }
                            else { "Not uploaded yet. Leaving this page will discard this recording." }
                        }
                    }
                }
            }
            ConfirmAction {
                open: discard_open(), title: "Discard this recording?", target: title(),
                body: "This recording has not been uploaded. Discarding it cannot be undone.",
                confirm_label: "Discard recording", variant: ConfirmActionVariant::Danger,
                on_open_change: move |open| discard_open.set(open),
                on_confirm: {
                    let recorder = recorder.clone();
                    move |request: crate::components::ConfirmActionRequest| {
                        recorder.discard();
                        completed.set(None); error.set(None); clock.set(CaptureClock::default()); state.set(RecordingState::Idle);
                        request.complete(Ok(()));
                    }
                },
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CaptureClock {
    captured: Duration,
    started: Option<Instant>,
}

impl CaptureClock {
    fn elapsed(self, now: Instant) -> Duration {
        self.captured
            + self
                .started
                .map(|start| now.saturating_duration_since(start))
                .unwrap_or_default()
    }

    fn pause(&mut self, now: Instant) {
        self.captured = self.elapsed(now);
        self.started = None;
    }

    fn resume(&mut self, now: Instant) {
        if self.started.is_none() {
            self.started = Some(now);
        }
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

#[component]
fn RecordingTimer(clock: Signal<CaptureClock>) -> Element {
    let mut tick = use_signal(Instant::now);
    use_resource(move || async move {
        let running = clock.read().started.is_some();
        if running {
            loop {
                tick.set(Instant::now());
                dioxus_sdk_time::sleep(Duration::from_millis(200)).await;
            }
        }
    });
    let elapsed = clock.read().elapsed(if clock.read().started.is_some() {
        tick()
    } else {
        Instant::now()
    });
    rsx! { div { class: "semantic-audio-recording__timer", role: "timer", "aria-label": "Captured recording duration", "aria-live": "off", "{format_duration(elapsed)}" } }
}

#[component]
fn RecordingPlayback(recording: CompletedRecording) -> Element {
    let preview = use_hook(|| super::preview::PlaybackUrl::new(&recording));
    match preview {
        Ok(url) => {
            rsx! { audio { class: "semantic-audio-recording__playback", controls: true, preload: "metadata", src: url.as_str(), "aria-label": "Preview your recording" } }
        }
        Err(message) => {
            rsx! { InlineNotice { title: "Playback preview unavailable", message, variant: NoticeVariant::Warning } }
        }
    }
}

#[component]
fn RecordingLevels(recorder: Recorder, active: bool) -> Element {
    let mut history =
        use_signal(|| std::collections::VecDeque::<Option<f32>>::from(vec![None; 48]));
    use_resource(use_reactive!(|(active)| {
        let recorder = recorder.clone();
        async move {
            if active {
                loop {
                    let level = recorder.level();
                    {
                        let mut samples = history.write();
                        samples.pop_front();
                        samples.push_back(level);
                    }
                    dioxus_sdk_time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }));
    let latest = history.read().back().copied().flatten();
    rsx! {
        div { class: "semantic-audio-recording__levels",
            svg { view_box: "0 0 288 48", preserve_aspect_ratio: "none", role: "img", "aria-label": "Recent captured audio level, from left to right",
                for (index, level) in history.read().iter().enumerate() {
                    rect {
                        key: "{index}", x: "{index * 6}", y: "{24.0 - level.unwrap_or(0.0) * 22.0}",
                        width: "3", height: "{(level.unwrap_or(0.0) * 44.0).max(2.0)}", rx: "1.5",
                        opacity: if level.is_some() { "1" } else { "0.2" },
                    }
                }
            }
            p { class: "semantic-audio-recording__hint",
                if !active { "Input activity paused" }
                else if latest.is_none() { "Audio level unavailable · recording continues" }
                else if latest.is_some_and(|level| level < 0.08) { "Quiet input · check your audio source" }
                else { "Input activity · last 12 seconds" }
            }
        }
    }
}

impl RecordingState {
    fn slug(&self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Paused | Self::Pausing => "paused",
            _ => "idle",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Ready to record",
            Self::Starting => "Starting recorder…",
            Self::Recording => "Recording",
            Self::Pausing => "Pausing recording…",
            Self::Paused => "Recording paused",
            Self::Resuming => "Resuming recording…",
            Self::Stopping => "Preparing recording…",
            Self::Uploading => "Uploading…",
            Self::ReadyToUpload => "Ready to review",
            Self::Complete(_) => "Recording saved",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_excludes_pauses_and_handles_repeated_transitions() {
        let start = Instant::now();
        let mut clock = CaptureClock::default();
        clock.resume(start);
        clock.resume(start + Duration::from_secs(2));
        clock.pause(start + Duration::from_secs(10));
        clock.pause(start + Duration::from_secs(20));
        assert_eq!(
            clock.elapsed(start + Duration::from_secs(30)),
            Duration::from_secs(10)
        );
        clock.resume(start + Duration::from_secs(40));
        assert_eq!(
            clock.elapsed(start + Duration::from_secs(45)),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn timer_formats_long_recordings_without_wrapping() {
        assert_eq!(format_duration(Duration::ZERO), "00:00");
        assert_eq!(format_duration(Duration::from_secs(65)), "01:05");
        assert_eq!(format_duration(Duration::from_secs(3601)), "1:00:01");
    }
}
