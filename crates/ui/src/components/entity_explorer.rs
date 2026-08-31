use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::value::{Object, Value};
use semantic_ui_core::{
    EntityCard, EntityDisplayMode, EntityDisplayRenderer, EntityRenderOptions, EntityTableRow,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResultDensity {
    Compact,
    #[default]
    Comfortable,
}

impl ResultDensity {
    fn class(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
        }
    }
}

/// Route-agnostic composition boundary for entity discovery pages.
///
/// Applied state stays with the route. Slots let Collection reuse the result
/// surface without inheriting Browse's SQL or routing decisions.
#[component]
pub fn EntityExplorer(
    toolbar: Element,
    children: Element,
    #[props(default)] advanced: Option<Element>,
    #[props(default)] refreshing: bool,
) -> Element {
    rsx! {
        section {
            class: "semantic-entity-explorer",
            aria_busy: refreshing,
            {toolbar}
            if let Some(advanced) = advanced {
                div { class: "semantic-entity-explorer__advanced", {advanced} }
            }
            div { class: "semantic-entity-explorer__results", {children} }
        }
    }
}

/// Shared controls for collection-backed entity result surfaces.
#[component]
pub fn DataToolbar(
    collection: String,
    collections: Rc<[String]>,
    display_mode: EntityDisplayMode,
    renderer: EntityDisplayRenderer,
    grid_columns: usize,
    density: ResultDensity,
    advanced_open: bool,
    custom_query: bool,
    #[props(default)] filters_open: bool,
    #[props(default)] active_filter_count: usize,
    #[props(default)] show_filters: bool,
    #[props(default = true)] show_advanced: bool,
    #[props(default = "Browse controls".to_string())] toolbar_label: String,
    on_collection_change: EventHandler<String>,
    on_display_mode_change: EventHandler<EntityDisplayMode>,
    on_renderer_change: EventHandler<EntityDisplayRenderer>,
    on_grid_columns_change: EventHandler<usize>,
    on_density_change: EventHandler<ResultDensity>,
    on_advanced_open_change: EventHandler<bool>,
    #[props(default)] on_filters_open_change: Option<EventHandler<bool>>,
) -> Element {
    let collection_in_catalog = collections.iter().any(|candidate| candidate == &collection);

    rsx! {
        div { class: "semantic-data-toolbar", role: "toolbar", aria_label: toolbar_label,
            label { class: "semantic-data-toolbar__field semantic-data-toolbar__collection",
                span { "Collection" }
                select {
                    value: "{collection}",
                    onchange: move |event: FormEvent| on_collection_change.call(event.value()),
                    if !collection_in_catalog {
                        option { value: "{collection}", "{collection} (not in catalog)" }
                    }
                    for candidate in collections.iter() {
                        option { key: "{candidate}", value: "{candidate}", "{candidate}" }
                    }
                }
            }

            div { class: "semantic-data-toolbar__group", role: "group", aria_label: "Result view",
                dxcomp::Button {
                    size: dxcomp::ButtonSize::Sm,
                    variant: if display_mode == EntityDisplayMode::Card { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                    aria_pressed: display_mode == EntityDisplayMode::Card,
                    onclick: move |_| on_display_mode_change.call(EntityDisplayMode::Card),
                    "Cards"
                }
                dxcomp::Button {
                    size: dxcomp::ButtonSize::Sm,
                    variant: if display_mode == EntityDisplayMode::Table { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                    aria_pressed: display_mode == EntityDisplayMode::Table,
                    onclick: move |_| on_display_mode_change.call(EntityDisplayMode::Table),
                    "Table"
                }
            }

            if display_mode == EntityDisplayMode::Card {
                label { class: "semantic-data-toolbar__field",
                    span { "Cards per row" }
                    select {
                        value: "{grid_columns}",
                        onchange: move |event: FormEvent| {
                            let columns = event.value().parse::<usize>().unwrap_or(1).clamp(1, 3);
                            on_grid_columns_change.call(columns);
                        },
                        option { value: "1", "1" }
                        option { value: "2", "2" }
                        option { value: "3", "3" }
                    }
                }
                label { class: "semantic-data-toolbar__field",
                    span { "Content" }
                    select {
                        value: if renderer == EntityDisplayRenderer::Custom { "rich" } else { "fields" },
                        onchange: move |event: FormEvent| {
                            let renderer = if event.value() == "fields" {
                                EntityDisplayRenderer::Table
                            } else {
                                EntityDisplayRenderer::Custom
                            };
                            on_renderer_change.call(renderer);
                        },
                        option { value: "rich", "Rich preview" }
                        option { value: "fields", "Fields" }
                    }
                }
            }

            label { class: "semantic-data-toolbar__field",
                span { "Density" }
                select {
                    value: density.class(),
                    onchange: move |event: FormEvent| {
                        let density = if event.value() == "compact" {
                            ResultDensity::Compact
                        } else {
                            ResultDensity::Comfortable
                        };
                        on_density_change.call(density);
                    },
                    option { value: "comfortable", "Comfortable" }
                    option { value: "compact", "Compact" }
                }
            }

            if show_advanced {
                dxcomp::Button {
                    size: dxcomp::ButtonSize::Sm,
                    variant: if advanced_open || custom_query { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                    aria_pressed: advanced_open,
                    onclick: move |_| on_advanced_open_change.call(!advanced_open),
                    if custom_query { "Advanced SQL active" } else { "Advanced SQL" }
                }
            }
            if show_filters {
                dxcomp::Button {
                    size: dxcomp::ButtonSize::Sm,
                    variant: if filters_open || active_filter_count > 0 { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                    aria_pressed: filters_open,
                    onclick: move |_| {
                        if let Some(handler) = on_filters_open_change {
                            handler.call(!filters_open);
                        }
                    },
                    if active_filter_count > 0 { "Filters ({active_filter_count})" } else { "Filters" }
                }
            }
        }
    }
}

/// Keyed entity cards/table rows shared by Browse and Collection.
#[component]
pub fn EntityResults(
    rows: Rc<[Object]>,
    display_mode: EntityDisplayMode,
    renderer: EntityDisplayRenderer,
    collection: Option<String>,
    grid_columns: usize,
    density: ResultDensity,
) -> Element {
    let grid_columns = grid_columns.clamp(1, 3);
    let density_class = density.class();

    match display_mode {
        EntityDisplayMode::Card => rsx! {
            div {
                class: "semantic-entity-results semantic-entity-results--cards semantic-entity-results--grid-{grid_columns} semantic-entity-results--{density_class}",
                for (index, object) in rows.iter().enumerate() {
                    EntityCard {
                        key: "{result_key(collection.as_deref(), object, index)}",
                        object: object.clone(),
                        options: EntityRenderOptions {
                            collection: collection.clone(),
                            id: None,
                            renderer,
                            preview: true,
                            actions: true,
                        }
                    }
                }
            }
        },
        EntityDisplayMode::Table => rsx! {
            div { class: "semantic-entity-results__table-scroll",
                table { class: "semantic-entity-results semantic-entity-results--table semantic-entity-results--{density_class}",
                    caption { class: "semantic-visually-hidden", "Entity query results" }
                    thead {
                        tr {
                            th { scope: "col", "ID" }
                            th { scope: "col", "Title" }
                            th { scope: "col", "Type" }
                            th { scope: "col", "Actions" }
                        }
                    }
                    tbody {
                        for (index, object) in rows.iter().enumerate() {
                            EntityTableRow {
                                key: "{result_key(collection.as_deref(), object, index)}",
                                object: object.clone(),
                                collection: collection.clone()
                            }
                        }
                    }
                }
            }
        },
    }
}

#[component]
pub fn QueryEditor(
    draft: String,
    #[props(default)] error: Option<String>,
    #[props(default)] non_portable: bool,
    #[props(default = "Advanced SQL".to_string())] title: String,
    #[props(default = "Run one read-only SELECT statement. LIMIT and ordering remain under your control.".to_string())]
    description: String,
    #[props(default = "SQL query".to_string())] label: String,
    #[props(default = "semantic-browse-sql".to_string())] editor_id: String,
    #[props(default = "Run query".to_string())] run_label: String,
    #[props(default = "Use collection query".to_string())] clear_label: String,
    #[props(default)] running: bool,
    #[props(default = true)] show_clear: bool,
    #[props(default)] on_cancel: Option<EventHandler<()>>,
    on_change: EventHandler<String>,
    on_run: EventHandler<()>,
    on_clear: EventHandler<()>,
) -> Element {
    let has_error = error.is_some();
    let help_id = format!("{editor_id}-help");
    let cancel_on_key = on_cancel;
    rsx! {
        section { class: "semantic-query-editor", aria_label: "Advanced SQL query",
            div { class: "semantic-query-editor__heading",
                div {
                    h2 { "{title}" }
                    p { "{description}" }
                }
                span { class: "semantic-query-editor__badge", "Read only" }
            }
            label { class: "semantic-query-editor__field",
                span { class: "semantic-query-editor__label", "{label}" }
                textarea {
                    id: editor_id,
                    value: "{draft}",
                    spellcheck: "false",
                    rows: "10",
                    aria_invalid: has_error,
                    aria_describedby: help_id.clone(),
                    oninput: move |event: FormEvent| on_change.call(event.value()),
                    onkeydown: move |event| {
                        let modifiers = event.modifiers();
                        if event.key() == Key::Enter
                            && (modifiers.contains(Modifiers::CONTROL)
                                || modifiers.contains(Modifiers::META))
                        {
                            event.prevent_default();
                            if !running {
                                on_run.call(());
                            }
                        } else if event.key() == Key::Escape && running {
                            if let Some(on_cancel) = cancel_on_key {
                                on_cancel.call(());
                            }
                        }
                    }
                }
            }
            p { id: help_id, class: "semantic-query-editor__help",
                "Ctrl/Command+Enter runs the query. This client check is a safety guard, not an authorization boundary."
            }
            if non_portable {
                p { class: "semantic-query-editor__warning",
                    "This query is too long for a portable URL and is stored only in this browser."
                }
            }
            if let Some(error) = error {
                p { class: "semantic-query-editor__error", role: "alert", "{error}" }
            }
            div { class: "semantic-query-editor__actions",
                dxcomp::Button {
                    disabled: running,
                    onclick: move |_| on_run.call(()),
                    if running { "Running…" } else { "{run_label}" }
                }
                if running {
                    if let Some(on_cancel) = on_cancel {
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| on_cancel.call(()),
                            "Cancel"
                        }
                    }
                }
                if show_clear {
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        disabled: running,
                        onclick: move |_| on_clear.call(()),
                        "{clear_label}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn Pagination(
    page: usize,
    page_size: usize,
    has_more: bool,
    #[props(default)] disabled: bool,
    on_page_change: EventHandler<usize>,
    on_page_size_change: EventHandler<usize>,
) -> Element {
    rsx! {
        nav { class: "semantic-result-pagination", aria_label: "Result pages",
            dxcomp::Button {
                size: dxcomp::ButtonSize::Sm,
                variant: dxcomp::ButtonVariant::Outline,
                disabled: disabled || page == 0,
                onclick: move |_| on_page_change.call(page.saturating_sub(1)),
                "Previous"
            }
            span { class: "semantic-result-pagination__status", aria_live: "polite",
                "Page {page + 1}"
            }
            dxcomp::Button {
                size: dxcomp::ButtonSize::Sm,
                variant: dxcomp::ButtonVariant::Outline,
                disabled: disabled || !has_more,
                onclick: move |_| on_page_change.call(page.saturating_add(1)),
                "Next"
            }
            label { class: "semantic-result-pagination__size",
                span { "Rows per page" }
                select {
                    value: "{page_size}",
                    disabled,
                    onchange: move |event: FormEvent| {
                        on_page_size_change.call(event.value().parse().unwrap_or(page_size));
                    },
                    option { value: "25", "25" }
                    option { value: "50", "50" }
                    option { value: "100", "100" }
                    option { value: "200", "200" }
                }
            }
        }
    }
}

fn result_key(collection: Option<&str>, object: &Object, index: usize) -> String {
    let collection = collection.unwrap_or("entities");
    match object.get("id").and_then(Value::as_str) {
        Some(id) => format!("{collection}:{id}"),
        None => format!("{collection}:row:{index}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_result_keys_prefer_domain_identity() {
        let mut object = Object::new();
        object.insert("id", Value::String("example".to_string()));
        assert_eq!(result_key(Some("items"), &object, 42), "items:example");
    }

    #[test]
    fn entity_result_keys_have_a_deterministic_fallback() {
        assert_eq!(result_key(Some("items"), &Object::new(), 7), "items:row:7");
    }
}
