use dioxus::prelude::*;

use super::query::PlaylistFilter;

#[component]
pub fn PlayerFilters(
    draft: PlaylistFilter,
    loading: bool,
    error: Option<String>,
    on_change: EventHandler<PlaylistFilter>,
    on_replace: EventHandler<PlaylistFilter>,
    on_add: EventHandler<PlaylistFilter>,
    on_cancel: EventHandler<MouseEvent>,
) -> Element {
    let search_draft = draft.clone();
    let collection_draft = draft.clone();
    let images_draft = draft.clone();
    let audio_draft = draft.clone();
    let video_draft = draft.clone();
    let advanced_draft = draft.clone();
    let sql_draft = draft.clone();
    let expand_draft = draft.clone();
    let replace_draft = draft.clone();
    let add_draft = draft.clone();
    rsx! {
        section { id: "semantic-player-filter", class: "semantic-player__filter-panel", aria_label: "Playlist filter",
            header {
                div { h2 { "Load playlist" } p { "Changes apply only when you choose Replace or Add." } }
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: dxcomp::ButtonVariant::Ghost,
                    onclick: on_cancel, "Close" }
            }
            label {
                span { "Collection" }
                input {
                    value: "{draft.collection}", disabled: draft.advanced_sql,
                    oninput: move |event: FormEvent| {
                        let mut next = collection_draft.clone(); next.collection = event.value(); on_change.call(next);
                    }
                }
            }
            label {
                span { "Search title or ID" }
                input {
                    value: "{draft.search}", disabled: draft.advanced_sql,
                    oninput: move |event: FormEvent| {
                        let mut next = search_draft.clone(); next.search = event.value(); on_change.call(next);
                    }
                }
            }
            fieldset { disabled: draft.advanced_sql,
                legend { "Media kinds" }
                label { input { r#type: "checkbox", checked: draft.images,
                    onchange: move |event| { let mut next = images_draft.clone(); next.images = event.checked(); on_change.call(next); }
                } "Images" }
                label { input { r#type: "checkbox", checked: draft.audio,
                    onchange: move |event| { let mut next = audio_draft.clone(); next.audio = event.checked(); on_change.call(next); }
                } "Audio" }
                label { input { r#type: "checkbox", checked: draft.video,
                    onchange: move |event| { let mut next = video_draft.clone(); next.video = event.checked(); on_change.call(next); }
                } "Video" }
            }
            label { class: "semantic-player__filter-check",
                input { r#type: "checkbox", checked: draft.expand_to_media,
                    onchange: move |event| { let mut next = expand_draft.clone(); next.expand_to_media = event.checked(); on_change.call(next); }
                }
                "Expand directories recursively"
            }
            label { class: "semantic-player__filter-check",
                input { r#type: "checkbox", checked: draft.advanced_sql,
                    onchange: move |event| { let mut next = advanced_draft.clone(); next.advanced_sql = event.checked(); on_change.call(next); }
                }
                "Advanced SQL"
            }
            if draft.advanced_sql {
                label { class: "semantic-player__filter-sql",
                    span { "Read-only SELECT" }
                    textarea {
                        rows: 8, value: "{draft.sql}", spellcheck: false,
                        oninput: move |event: FormEvent| { let mut next = sql_draft.clone(); next.sql = event.value(); on_change.call(next); }
                    }
                }
                p { class: "semantic-player__filter-note",
                    "Raw SQL results are capped at 100,000 entries. Add a deterministic ORDER BY and LIMIT."
                }
            }
            if let Some(error) = error {
                div { class: "semantic-player__filter-error", role: "alert", "{error}" }
            }
            div { class: "semantic-player__filter-actions",
                dxcomp::Button { disabled: loading, onclick: move |_| on_replace.call(replace_draft.clone()),
                    if loading { "Loading…" } else { "Replace" } }
                dxcomp::Button { disabled: loading, variant: dxcomp::ButtonVariant::Outline,
                    onclick: move |_| on_add.call(add_draft.clone()), "Add" }
            }
        }
    }
}
