use dioxus::prelude::*;
use semantic_ui_core::use_ui_catalog;

use crate::components::{
    StructuredQueryBuilder, compile_structured_predicate, fields_for_collection,
};

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
    let add_draft = draft.clone();
    let advanced_replace_draft = draft.clone();
    let clear_advanced_draft = draft.clone();
    let query_fields = fields_for_collection(&catalog, &draft.collection);
    let structured_result =
        compile_structured_predicate(&draft.structured, &query_fields, Some("e"), &[]);
    let validation_error = if draft.advanced_sql {
        playlist_query(&draft, 0).err()
    } else {
        structured_result
            .as_ref()
            .err()
            .cloned()
            .or_else(|| playlist_query(&draft, 0).err())
    };
    let displayed_error = error.or(validation_error);
    let structured_draft = draft.clone();
    let prepared_replace = prepare_structured_filter(&draft, &structured_result);
    let prepared_add = prepared_replace.clone();
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
                        on_change.call(change_collection(collection_draft.clone(), event.value()));
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
            if !draft.advanced_sql {
                StructuredQueryBuilder {
                    title: "More conditions".to_string(),
                    description: "Add optional, type-aware conditions. Nested groups can match all, any, or exclude matches.".to_string(),
                    draft: draft.structured.clone(),
                    fields: query_fields.clone(),
                    show_search: false,
                    on_change: move |structured| {
                        let mut next = structured_draft.clone();
                        next.structured = structured;
                        next.structured_predicate = None;
                        on_change.call(next);
                    },
                }
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
                    dxcomp::Button { disabled: loading || prepared_replace.is_none(), onclick: move |_| {
                        if let Some(filter) = prepared_replace.clone() { on_replace.call(filter); }
                    },
                        if loading { "Loading…" } else { "Replace" } }
                }
                dxcomp::Button { disabled: loading || (!draft.advanced_sql && prepared_add.is_none()), variant: dxcomp::ButtonVariant::Outline,
                    onclick: move |_| {
                        if draft.advanced_sql { on_add.call(add_draft.clone()); }
                        else if let Some(filter) = prepared_add.clone() { on_add.call(filter); }
                    }, "Add" }
            }
        }
    }
}

fn prepare_structured_filter(
    draft: &PlaylistFilter,
    predicate: &std::result::Result<Option<String>, String>,
) -> Option<PlaylistFilter> {
    let mut prepared = draft.clone();
    prepared.structured_predicate = predicate.clone().ok()?;
    Some(prepared)
}

fn change_collection(mut draft: PlaylistFilter, collection: String) -> PlaylistFilter {
    draft.collection = collection;
    // Structured fields are collection-specific. Discard both the editable
    // tree and its last compiled predicate so a collection switch cannot leave
    // an invisible, invalid filter that prevents the player from loading.
    draft.structured = Default::default();
    draft.structured_predicate = None;
    draft
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{FilterNode, FilterOperator, FilterRule};

    #[test]
    fn collection_change_clears_collection_specific_structured_state() {
        let mut draft = PlaylistFilter {
            collection: "old".into(),
            search: "keep me".into(),
            images: true,
            ..Default::default()
        };
        draft
            .structured
            .root
            .children
            .push(FilterNode::Rule(FilterRule {
                field: "old_field".into(),
                operator: FilterOperator::Equals,
                values: vec!["value".into()],
            }));
        draft.structured_predicate = Some("\"old_field\" = 'value'".into());

        let changed = change_collection(draft, "new".into());

        assert_eq!(changed.collection, "new");
        assert_eq!(changed.structured, Default::default());
        assert_eq!(changed.structured_predicate, None);
        assert_eq!(changed.search, "keep me");
        assert!(changed.images);
    }
}
