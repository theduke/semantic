use dioxus::prelude::*;
use semantic_data::value::Object;
use semantic_ui_core::{
    EntityCard, EntityDisplayRenderer, EntityRenderOptions, MediaHandleRegistration, MediaKind,
    MediaPlaybackEvent, MediaPlaybackRenderOptions, MediaPlaybackView,
};

use super::state::{PlaybackIntent, QueueEntry};

#[component]
pub fn PlayerStage(
    entry: Option<QueueEntry>,
    object: Option<std::result::Result<Object, String>>,
    session_id: u64,
    playing: PlaybackIntent,
    muted: bool,
    on_handle: EventHandler<MediaHandleRegistration>,
    on_event: EventHandler<MediaPlaybackEvent>,
) -> Element {
    rsx! {
        div { class: "semantic-player__stage", role: "region", aria_label: "Media stage",
            match (entry, object) {
                (None, _) => rsx! {
                    div { class: "semantic-player__empty",
                        h2 { "No playable items" }
                        p { "Open Filter to load images, audio, or video." }
                    }
                },
                (Some(entry), None) => rsx! {
                    div { class: "semantic-player__stage-status", role: "status", "Loading {entry.title}…" }
                },
                (Some(entry), Some(Err(error))) => rsx! {
                    div { class: "semantic-player__stage-error", role: "alert",
                        h2 { "Could not load {entry.title}" }
                        p { "{error}" }
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
