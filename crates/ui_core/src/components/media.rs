use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::filestore::{ATTR_FILE_FILENAME, ATTR_TITLE};
use semantic_data::value::{Object, Value};

use crate::ui_catalog::{
    MediaHandleRegistration, MediaKind, MediaPlaybackEvent, MediaPlaybackEventKind,
    MediaPlaybackRenderOptions, MediaPlaybackRendererRegistration, MediaRenderOptions,
    PlaybackMediaHandle, RegisteredPlaybackHandle, UiCatalog, use_ui_catalog,
};

#[component]
pub fn MediaView(object: Object, options: MediaRenderOptions) -> Element {
    let catalog = use_ui_catalog();
    if let Some(renderer) = catalog.media_renderer_for_object(&object) {
        return (renderer.renderer)(object, options);
    }
    let src = object
        .get("url")
        .or_else(|| object.get("path"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let content_type = object
        .get("mime")
        .or_else(|| object.get("content_type"))
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .to_string();
    rsx! {
        div { class: "semantic-media",
            if let Some(src) = src {
                a { href: "{src}", target: "_blank", "{src}" }
                span { " {content_type}" }
            } else {
                span { "media item" }
            }
        }
    }
}

#[component]
pub fn MediaPlaybackView(object: Object, options: MediaPlaybackRenderOptions) -> Element {
    let catalog = use_ui_catalog();
    if let Some(renderer) = catalog.playback_renderer_for_object(&object) {
        return (renderer.renderer)(object, options);
    }
    rsx! {}
}

pub fn register_default_playback_renderers(catalog: &mut UiCatalog) {
    let file_api_prefix = catalog
        .render_settings()
        .file_api_prefix
        .trim_end_matches('/')
        .to_string();
    for (name, media_kind) in [
        ("default-image", MediaKind::Image),
        ("default-audio", MediaKind::Audio),
        ("default-video", MediaKind::Video),
    ] {
        let prefix = file_api_prefix.clone();
        catalog.register_media_playback_renderer(MediaPlaybackRendererRegistration {
            name: name.to_string(),
            class_id: None,
            media_kind,
            renderer: Rc::new(move |object, options| {
                let source = media_source(&object, &prefix);
                let title = media_title(&object);
                match media_kind {
                    MediaKind::Image => rsx! { ImagePlayback { source, title, options } },
                    MediaKind::Audio | MediaKind::Video => rsx! {
                        TimedMediaPlayback { source, title, media_kind, options }
                    },
                    _ => rsx! {},
                }
            }),
        });
    }
}

#[component]
fn ImagePlayback(
    source: Option<String>,
    title: String,
    options: MediaPlaybackRenderOptions,
) -> Element {
    let mounted_options = options.clone();
    use_effect(move || emit_playback(&mounted_options, MediaPlaybackEventKind::Mounted));
    let source_missing = source.is_none();
    let missing_options = options.clone();
    use_effect(move || {
        if source_missing {
            emit_playback(
                &missing_options,
                MediaPlaybackEventKind::Failed {
                    message: "This image has no usable source".to_string(),
                    fatal: true,
                },
            );
        }
    });
    let Some(source) = source else {
        return rsx! {
            div { class: "semantic-media-playback__error", role: "alert",
                "This image has no usable source."
            }
        };
    };
    let loaded_options = options.clone();
    let failed_options = options.clone();
    rsx! {
        img {
            class: "semantic-media-playback__image",
            src: source,
            alt: title,
            onload: move |_| emit_playback(
                &loaded_options,
                MediaPlaybackEventKind::Loaded { duration_seconds: None },
            ),
            onerror: move |_| emit_playback(
                &failed_options,
                MediaPlaybackEventKind::Failed {
                    message: "The browser could not load this image".to_string(),
                    fatal: true,
                },
            ),
        }
    }
}

#[component]
fn TimedMediaPlayback(
    source: Option<String>,
    title: String,
    media_kind: MediaKind,
    options: MediaPlaybackRenderOptions,
) -> Element {
    let element_id = format!("semantic-playback-media-{}", options.session_id);
    let source_missing = source.is_none();
    let missing_options = options.clone();
    use_effect(move || {
        if source_missing {
            emit_playback(
                &missing_options,
                MediaPlaybackEventKind::Failed {
                    message: "This media item has no usable source".to_string(),
                    fatal: true,
                },
            );
        }
    });
    let Some(source) = source else {
        return rsx! {
            div { class: "semantic-media-playback__error", role: "alert",
                "This media item has no usable source."
            }
        };
    };
    let handle = RegisteredPlaybackHandle(Rc::new(DomMediaHandle {
        element_id: element_id.clone(),
        options: options.clone(),
    }));
    let mount_options = options.clone();
    let mount_handle = handle.clone();
    use_effect(move || {
        emit_playback(&mount_options, MediaPlaybackEventKind::Mounted);
        mount_options.on_handle.call(MediaHandleRegistration {
            session_id: mount_options.session_id,
            occurrence_id: mount_options.occurrence_id,
            handle: Some(mount_handle.clone()),
        });
        mount_handle.0.set_muted(mount_options.muted);
    });
    let drop_options = options.clone();
    let drop_handle = handle.clone();
    use_drop(move || {
        drop_handle.0.pause();
        drop_options.on_handle.call(MediaHandleRegistration {
            session_id: drop_options.session_id,
            occurrence_id: drop_options.occurrence_id,
            handle: None,
        });
    });
    let loaded_options = options.clone();
    let progress_options = options.clone();
    let playing_options = options.clone();
    let paused_options = options.clone();
    let finished_options = options.clone();
    let failed_options = options.clone();
    let loaded_id = element_id.clone();
    let progress_id = element_id.clone();
    let controls = options.controls;
    let muted = options.muted;
    match media_kind {
        MediaKind::Audio => rsx! {
            div { class: "semantic-media-playback__audio",
                div { class: "semantic-media-playback__audio-title", "{title}" }
                audio {
                    id: element_id,
                    class: "semantic-media-playback__audio-element",
                    src: source,
                    controls,
                    muted,
                    preload: "metadata",
                    aria_label: "Audio: {title}",
                    onloadedmetadata: move |_| probe_media(loaded_id.clone(), loaded_options.clone(), true),
                    ontimeupdate: move |_| probe_media(progress_id.clone(), progress_options.clone(), false),
                    onplay: move |_| emit_playback(&playing_options, MediaPlaybackEventKind::Playing),
                    onpause: move |_| emit_playback(&paused_options, MediaPlaybackEventKind::Paused),
                    onended: move |_| emit_playback(&finished_options, MediaPlaybackEventKind::Finished),
                    onerror: move |_| emit_playback(&failed_options, MediaPlaybackEventKind::Failed {
                        message: "The browser could not play this audio file".to_string(), fatal: true,
                    }),
                }
            }
        },
        MediaKind::Video => rsx! {
            video {
                id: element_id,
                class: "semantic-media-playback__video",
                src: source,
                controls,
                muted,
                preload: "metadata",
                aria_label: "Video: {title}",
                onloadedmetadata: move |_| probe_media(loaded_id.clone(), loaded_options.clone(), true),
                ontimeupdate: move |_| probe_media(progress_id.clone(), progress_options.clone(), false),
                onplay: move |_| emit_playback(&playing_options, MediaPlaybackEventKind::Playing),
                onpause: move |_| emit_playback(&paused_options, MediaPlaybackEventKind::Paused),
                onended: move |_| emit_playback(&finished_options, MediaPlaybackEventKind::Finished),
                onerror: move |_| emit_playback(&failed_options, MediaPlaybackEventKind::Failed {
                    message: "The browser could not play this video file".to_string(), fatal: true,
                }),
            }
        },
        _ => rsx! {},
    }
}

#[derive(Clone, PartialEq)]
struct DomMediaHandle {
    element_id: String,
    options: MediaPlaybackRenderOptions,
}

impl PlaybackMediaHandle for DomMediaHandle {
    fn play(&self) {
        let element_id = self.element_id.clone();
        let options = self.options.clone();
        spawn(async move {
            let mut eval = document::eval(
                r#"
                const id = await dioxus.recv();
                const media = document.getElementById(id);
                if (!media) { dioxus.send('Media element is no longer mounted'); return; }
                try { await media.play(); dioxus.send(''); }
                catch (error) { dioxus.send(String(error && error.message || error)); }
            "#,
            );
            if eval.send(element_id).is_ok() {
                if let Ok(message) = eval.recv::<String>().await {
                    if !message.is_empty() {
                        emit_playback(&options, MediaPlaybackEventKind::PlayRejected { message });
                    }
                }
            }
        });
    }

    fn pause(&self) {
        media_dom_command(self.element_id.clone(), "pause", None);
    }
    fn set_muted(&self, muted: bool) {
        media_dom_command(
            self.element_id.clone(),
            "mute",
            Some(if muted { 1.0 } else { 0.0 }),
        );
    }
    fn seek(&self, seconds: f64) {
        if seconds.is_finite() && seconds >= 0.0 {
            media_dom_command(self.element_id.clone(), "seek", Some(seconds));
        }
    }
}

fn media_dom_command(element_id: String, command: &'static str, value: Option<f64>) {
    spawn(async move {
        let eval = document::eval(
            r#"
            const id = await dioxus.recv(); const command = await dioxus.recv();
            const value = await dioxus.recv(); const media = document.getElementById(id);
            if (!media) return;
            if (command === 'pause') media.pause();
            else if (command === 'mute') media.muted = value === 1;
            else if (command === 'seek' && Number.isFinite(value)) media.currentTime = value;
        "#,
        );
        let _ = eval.send(element_id);
        let _ = eval.send(command);
        let _ = eval.send(value.unwrap_or(0.0));
    });
}

fn probe_media(element_id: String, options: MediaPlaybackRenderOptions, loaded: bool) {
    spawn(async move {
        let mut eval = document::eval(
            r#"
            const id = await dioxus.recv(); const media = document.getElementById(id);
            if (!media) { dioxus.send([]); return; }
            dioxus.send([Number(media.currentTime) || 0, Number.isFinite(media.duration) ? media.duration : -1]);
        "#,
        );
        if eval.send(element_id).is_err() {
            return;
        }
        let Ok(values) = eval.recv::<Vec<f64>>().await else {
            return;
        };
        if values.len() != 2 {
            return;
        }
        let duration = (values[1] >= 0.0).then_some(values[1]);
        if loaded {
            emit_playback(
                &options,
                MediaPlaybackEventKind::Loaded {
                    duration_seconds: duration,
                },
            );
        }
        emit_playback(
            &options,
            MediaPlaybackEventKind::Progress {
                current_seconds: values[0],
                duration_seconds: duration,
            },
        );
    });
}

fn emit_playback(options: &MediaPlaybackRenderOptions, kind: MediaPlaybackEventKind) {
    options.on_event.call(MediaPlaybackEvent {
        session_id: options.session_id,
        occurrence_id: options.occurrence_id,
        kind,
    });
}

fn media_source(object: &Object, file_api_prefix: &str) -> Option<String> {
    if let Some(url) = object.get("url").and_then(Value::as_str) {
        if url.starts_with("https://") || url.starts_with("http://") {
            return Some(url.to_string());
        }
    }
    object
        .get("id")
        .and_then(Value::as_str)
        .map(|id| format!("{file_api_prefix}/{id}"))
}

fn media_title(object: &Object) -> String {
    for key in ["title", ATTR_TITLE, "filename", ATTR_FILE_FILENAME] {
        if let Some(value) = object
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return value.to_string();
        }
    }
    format!(
        "Media item {}",
        object
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    )
}
