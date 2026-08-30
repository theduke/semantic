use dioxus::prelude::*;
use semantic_data::value::Object;
use semantic_ui_core::{
    EntityCard, EntityDisplayRenderer, EntityRenderOptions, MediaHandleRegistration, MediaKind,
    MediaPlaybackEvent, MediaPlaybackRenderOptions, MediaPlaybackView,
    components::{EmptyState, ErrorState},
};

use super::state::{PlaybackIntent, QueueEntry};

#[component]
pub fn PlayerStage(
    entry: Option<QueueEntry>,
    object: Option<std::result::Result<Object, String>>,
    session_id: u64,
    playing: PlaybackIntent,
    muted: bool,
    playlist_loading: bool,
    playlist_loaded: bool,
    playlist_error: Option<String>,
    on_handle: EventHandler<MediaHandleRegistration>,
    on_event: EventHandler<MediaPlaybackEvent>,
    on_retry_playlist: EventHandler<()>,
    on_edit_filter: EventHandler<()>,
    on_retry_object: EventHandler<()>,
    on_skip: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "semantic-player__stage", role: "region", aria_label: "Media stage",
            match (entry, object) {
                (None, _) if playlist_loading && !playlist_loaded => rsx! {
                    div { class: "semantic-player__stage-status", role: "status", aria_live: "polite",
                        span { class: "semantic-refreshing-indicator__mark", aria_hidden: "true" }
                        h2 { "Loading playlist" }
                        p { "Finding playable images, audio, and video…" }
                    }
                },
                (None, _) if !playlist_loaded && playlist_error.is_some() => {
                    let error = playlist_error.unwrap_or_default();
                    rsx! {
                        div { class: "semantic-player__stage-state",
                            ErrorState {
                                title: "Could not load playlist".to_string(),
                                message: error,
                                on_retry: move |_| on_retry_playlist.call(()),
                            }
                            dxcomp::Button { variant: dxcomp::ButtonVariant::Outline,
                                onclick: move |_| on_edit_filter.call(()), "Edit filter" }
                        }
                    }
                },
                (None, _) => rsx! {
                    div { class: "semantic-player__stage-state",
                        EmptyState {
                            title: "The playlist is empty".to_string(),
                            description: "Adjust the media filter or add items to start playing.".to_string(),
                            action_label: "Edit filter".to_string(),
                            on_action: move |_| on_edit_filter.call(()),
                        }
                    }
                },
                (Some(entry), None) => rsx! {
                    div { class: "semantic-player__stage-status", role: "status", "Loading {entry.title}…" }
                },
                (Some(entry), Some(Err(error))) => rsx! {
                    div { class: "semantic-player__stage-error", role: "alert",
                        h2 { "Could not load {entry.title}" }
                        p { "{error}" }
                        div { class: "semantic-player__stage-actions",
                            dxcomp::Button { onclick: move |_| on_retry_object.call(()), "Retry" }
                            dxcomp::Button { variant: dxcomp::ButtonVariant::Outline,
                                onclick: move |_| on_skip.call(()), "Skip" }
                        }
                    }
                },
                (Some(entry), Some(Ok(object))) => {
                    if matches!(entry.media_kind, MediaKind::Image | MediaKind::Audio | MediaKind::Video) {
                        let options = MediaPlaybackRenderOptions {
                            session_id,
                            occurrence_id: entry.occurrence_id,
                            playing: playing == PlaybackIntent::Playing,
                            muted,
                            controls: true,
                            on_handle,
                            on_event,
                        };
                        rsx! {
                            div { class: "semantic-player__media", key: "{session_id}",
                                MediaPlaybackView { object, options }
                            }
                        }
                    } else {
                        rsx! {
                            div { class: "semantic-player__fallback", key: "{session_id}",
                                EntityCard {
                                    object,
                                    options: EntityRenderOptions {
                                        collection: entry.target.collection.clone(),
                                        id: Some(entry.target.id.clone()),
                                        renderer: EntityDisplayRenderer::Custom,
                                        preview: true,
                                        actions: true,
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}
