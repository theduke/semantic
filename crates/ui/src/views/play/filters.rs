use dioxus::prelude::*;
use semantic_ui_core::use_ui_catalog;

use super::query::{PlaylistFilter, playlist_query};

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
    let catalog = use_ui_catalog();
    let mut collections = catalog
        .collections()
        .map(|collection| collection.name.clone())
        .collect::<Vec<_>>();
    collections.sort();
    let collection_in_catalog = collections
        .iter()
        .any(|collection| collection == &draft.collection);
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
    let advanced_replace_draft = draft.clone();
    let clear_advanced_draft = draft.clone();
    let validation_error = draft
        .advanced_sql
        .then(|| playlist_query(&draft, 0).err())
        .flatten();
    let displayed_error = error.or(validation_error);
    rsx! {
        section { id: "semantic-player-filter", class: "semantic-player__filter-panel", aria_label: "Playlist filter",
            header {
                div { h2 { "Load playlist" } p { "Changes apply only when you choose Replace or Add." } }
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: dxcomp::ButtonVariant::Ghost,
                    onclick: on_cancel, "Close" }
            }
            label {
                span { "Collection" }
                select {
                    value: "{draft.collection}", disabled: draft.advanced_sql,
                    onchange: move |event: FormEvent| {
                        let mut next = collection_draft.clone(); next.collection = event.value(); on_change.call(next);
                    },
                    if !collection_in_catalog {
                        option { value: "{draft.collection}", "{draft.collection} (not in catalog)" }
                    }
                    for collection in collections {
                        option { key: "{collection}", value: "{collection}", "{collection}" }
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
                crate::components::QueryEditor {
                    draft: draft.sql.clone(),
                    error: displayed_error.clone(),
                    title: "Advanced playlist SQL".to_string(),
                    description: "Load media from one deterministic, bounded, read-only SELECT statement.".to_string(),
                    editor_id: "semantic-player-sql".to_string(),
                    run_label: "Replace queue".to_string(),
                    clear_label: "Use structured filter".to_string(),
                    running: loading,
                    on_change: move |sql| {
                        let mut next = sql_draft.clone(); next.sql = sql; on_change.call(next);
                    },
                    on_run: move |_| on_replace.call(advanced_replace_draft.clone()),
                    on_clear: move |_| {
                        let mut next = clear_advanced_draft.clone();
                        next.advanced_sql = false;
                        on_change.call(next);
                    },
                }
                p { class: "semantic-player__filter-note",
                    "Raw SQL results are capped at 100,000 entries. Add a deterministic ORDER BY and LIMIT."
                }
            }
            if !draft.advanced_sql && let Some(error) = displayed_error {
                div { class: "semantic-player__filter-error", role: "alert", "{error}" }
            }
            div { class: "semantic-player__filter-actions",
                if !draft.advanced_sql {
                    dxcomp::Button { disabled: loading, onclick: move |_| on_replace.call(replace_draft.clone()),
                        if loading { "Loading…" } else { "Replace" } }
                }
                dxcomp::Button { disabled: loading, variant: dxcomp::ButtonVariant::Outline,
                    onclick: move |_| on_add.call(add_draft.clone()), "Add" }
            }
        }
    }
}
