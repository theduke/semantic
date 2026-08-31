use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::schema::{Meta, TypeKind};
use semantic_ui_core::{components::EmptyState, use_ui_catalog, use_ui_catalog_context};

use crate::{
    components::{CopyRequest, CopyableCode, PageHeader},
    views::Route,
};

const INITIAL_VISIBLE_ROWS: usize = 100;
const DETAIL_LIST_LIMIT: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatalogKind {
    Collections,
    Classes,
    Attributes,
}

impl CatalogKind {
    fn label(self) -> &'static str {
        match self {
            Self::Collections => "Collections",
            Self::Classes => "Classes",
            Self::Attributes => "Attributes",
        }
    }

    fn singular(self) -> &'static str {
        match self {
            Self::Collections => "collection",
            Self::Classes => "class",
            Self::Attributes => "attribute",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CatalogRow {
    kind: CatalogKind,
    canonical_id: String,
    title: String,
    description: Option<String>,
    summary: String,
    search_text: String,
}

impl CatalogRow {
    fn new(
        kind: CatalogKind,
        canonical_id: String,
        name: String,
        title: Option<&str>,
        description: Option<&str>,
        summary: String,
    ) -> Self {
        let title = title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(&name);
        let search_text = format!("{title}\n{name}\n{canonical_id}").to_lowercase();
        Self {
            kind,
            canonical_id,
            title: title.to_string(),
            description: description.map(str::to_string),
            summary,
            search_text,
        }
    }

    fn matches(&self, normalized_query: &str) -> bool {
        normalized_query.is_empty() || self.search_text.contains(normalized_query)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CatalogIndex {
    collections: Rc<[CatalogRow]>,
    classes: Rc<[CatalogRow]>,
    attributes: Rc<[CatalogRow]>,
}

impl CatalogIndex {
    fn rows(&self, kind: CatalogKind) -> &[CatalogRow] {
        match kind {
            CatalogKind::Collections => &self.collections,
            CatalogKind::Classes => &self.classes,
            CatalogKind::Attributes => &self.attributes,
        }
    }

    fn count(&self, kind: CatalogKind) -> usize {
        self.rows(kind).len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CatalogSelection {
    kind: CatalogKind,
    canonical_id: String,
}

#[component]
pub fn CatalogPage() -> Element {
    let catalog = use_ui_catalog();
    let catalog_signal = use_ui_catalog_context().catalog_signal();
    let catalog_index = use_memo(move || {
        catalog_signal
            .read()
            .as_ref()
            .map(build_catalog_index)
            .unwrap_or_default()
    });
    let mut active_kind = use_signal(|| CatalogKind::Collections);
    let mut search_query = use_signal(String::new);
    let mut selected = use_signal(|| None::<CatalogSelection>);
    let mut visible_limit = use_signal(|| INITIAL_VISIBLE_ROWS);
    let index_for_filter = catalog_index;
    let filtered_indices = use_memo(move || -> Rc<[usize]> {
        let index = index_for_filter.read();
        let kind = active_kind();
        let normalized_query = normalize_query(&search_query());
        index
            .rows(kind)
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.matches(&normalized_query).then_some(index))
            .collect::<Vec<_>>()
            .into()
    });

    let index = catalog_index();
    let filtered = filtered_indices();
    let kind = active_kind();
    let rows = index.rows(kind);
    let visible_count = visible_row_count(filtered.len(), visible_limit());
    let hidden_count = filtered.len().saturating_sub(visible_count);
    let query_value = search_query();
    let has_query = !query_value.trim().is_empty();
    let result_status = if has_query {
        format!(
            "{} matching {}",
            filtered.len(),
            pluralize(filtered.len(), kind.singular())
        )
    } else {
        format!("{} {}", rows.len(), pluralize(rows.len(), kind.singular()))
    };

    rsx! {
        div { class: "semantic-page semantic-catalog",
            PageHeader {
                title: "Catalog",
                description: Some(
                    "Find collections, classes, and attributes in the loaded schema, then inspect their definitions."
                        .to_string(),
                ),
                actions: Some(rsx! {
                    Link {
                        to: Route::BrowsePage {
                            collection: None,
                            view: None,
                            renderer: None,
                            page: None,
                            page_size: None,
                            filters: None,
                            sql: None,
                        },
                        class: "dx-button",
                        "data-style": "outline",
                        "data-size": "default",
                        "Browse data"
                    }
                    if index.count(CatalogKind::Collections) > 0
                        && index.count(CatalogKind::Classes) > 0
                    {
                        Link {
                            to: Route::CreateEntityPage,
                            class: "dx-button",
                            "data-style": "primary",
                            "data-size": "default",
                            "Create entity"
                        }
                    }
                }),
            }

            section {
                class: "semantic-catalog__explorer semantic-surface",
                aria_labelledby: "catalog-explorer-heading",
                h2 { id: "catalog-explorer-heading", class: "semantic-visually-hidden", "Schema explorer" }
                div { class: "semantic-catalog__toolbar",
                    div {
                        class: "semantic-catalog__tabs",
                        role: "group",
                        aria_label: "Catalog section",
                        for tab_kind in [
                            CatalogKind::Collections,
                            CatalogKind::Classes,
                            CatalogKind::Attributes,
                        ] {
                            button {
                                class: "semantic-catalog__tab",
                                r#type: "button",
                                aria_pressed: tab_kind == kind,
                                onclick: move |_| {
                                    active_kind.set(tab_kind);
                                    selected.set(None);
                                    visible_limit.set(INITIAL_VISIBLE_ROWS);
                                },
                                span { "{tab_kind.label()}" }
                                span {
                                    class: "semantic-catalog__tab-count",
                                    aria_label: "{index.count(tab_kind)} {tab_kind.label().to_lowercase()}",
                                    "{index.count(tab_kind)}"
                                }
                            }
                        }
                    }
                    label { class: "semantic-catalog__search", r#for: "catalog-search",
                        span { class: "semantic-visually-hidden", "Search {kind.label()}" }
                        input {
                            id: "catalog-search",
                            class: "semantic-catalog__search-input",
                            r#type: "search",
                            value: query_value,
                            placeholder: "Search {kind.label().to_lowercase()} by title or ID",
                            autocomplete: "off",
                            aria_describedby: "catalog-results-status",
                            oninput: move |event: FormEvent| {
                                search_query.set(event.value());
                                visible_limit.set(INITIAL_VISIBLE_ROWS);
                            },
                        }
                    }
                }

                p {
                    id: "catalog-results-status",
                    class: "semantic-catalog__results-status",
                    role: "status",
                    aria_live: "polite",
                    "{result_status}"
                }

                div { class: "semantic-catalog__layout",
                    div { class: "semantic-catalog__results",
                        if filtered.is_empty() {
                            EmptyState {
                                title: if has_query {
                                    format!("No matching {}", kind.label().to_lowercase())
                                } else {
                                    format!("No {} available", kind.label().to_lowercase())
                                },
                                description: Some(if has_query {
                                    "Try another title or canonical ID, or clear the search.".to_string()
                                } else {
                                    "The loaded catalog does not define entries in this section.".to_string()
                                }),
                                action_label: has_query.then(|| "Clear search".to_string()),
                                on_action: has_query.then(|| EventHandler::new(move |_| {
                                    search_query.set(String::new());
                                    visible_limit.set(INITIAL_VISIBLE_ROWS);
                                })),
                            }
                        } else {
                            ul { class: "semantic-catalog__list", aria_label: "{kind.label()} results",
                                for row_index in filtered.iter().take(visible_count).copied() {
                                    {
                                        let row = &rows[row_index];
                                        let row_id = row.canonical_id.clone();
                                        let row_kind = row.kind;
                                        let is_selected = selected
                                            .read()
                                            .as_ref()
                                            .is_some_and(|item| {
                                                item.kind == row_kind && item.canonical_id == row.canonical_id
                                            });
                                        rsx! {
                                            li { key: "{row_kind:?}:{row.canonical_id}",
                                                button {
                                                    class: "semantic-catalog__row",
                                                    r#type: "button",
                                                    aria_pressed: is_selected,
                                                    aria_controls: "catalog-detail",
                                                    onclick: move |_| {
                                                        selected.set(Some(CatalogSelection {
                                                            kind: row_kind,
                                                            canonical_id: row_id.clone(),
                                                        }));
                                                    },
                                                    span { class: "semantic-catalog__row-copy",
                                                        strong { class: "semantic-catalog__row-title", "{row.title}" }
                                                        span { class: "semantic-catalog__row-id", title: row.canonical_id.clone(),
                                                            "{row.canonical_id}"
                                                        }
                                                        if let Some(description) = &row.description {
                                                            span { class: "semantic-catalog__row-description", "{description}" }
                                                        }
                                                    }
                                                    span { class: "semantic-catalog__row-summary", "{row.summary}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if hidden_count > 0 {
                                div { class: "semantic-catalog__disclosure",
                                    dxcomp::Button {
                                        variant: dxcomp::ButtonVariant::Outline,
                                        aria_controls: "catalog-results-status",
                                        onclick: move |_| {
                                            visible_limit.set(
                                                visible_limit().saturating_add(INITIAL_VISIBLE_ROWS),
                                            );
                                        },
                                        "Show {hidden_count.min(INITIAL_VISIBLE_ROWS)} more"
                                    }
                                    span { "Results are shown in batches to keep the explorer responsive." }
                                }
                            }
                        }
                    }

                    aside {
                        id: "catalog-detail",
                        class: "semantic-catalog__detail",
                        aria_label: "Selected catalog entry",
                        if let Some(selection) = selected.read().as_ref() {
                            {render_catalog_detail(&catalog, selection)}
                        } else {
                            div { class: "semantic-catalog__detail-placeholder",
                                h2 { "Select an entry" }
                                p { "Choose a row to inspect its real schema metadata and available actions." }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn build_catalog_index(catalog: &semantic_ui_core::UiCatalog) -> CatalogIndex {
    let collections = catalog
        .collections()
        .map(|collection| {
            CatalogRow::new(
                CatalogKind::Collections,
                collection.name.clone(),
                collection.name.clone(),
                None,
                None,
                format!(
                    "{} {}",
                    collection.field_ids.len(),
                    pluralize(collection.field_ids.len(), "field")
                ),
            )
        })
        .collect::<Vec<_>>()
        .into();
    let classes = catalog
        .classes()
        .map(|class| {
            CatalogRow::new(
                CatalogKind::Classes,
                class.id.clone(),
                class.name.clone(),
                class.meta.title.as_deref(),
                class.meta.description.as_deref(),
                format!(
                    "{} {}",
                    class.attributes.len(),
                    pluralize(class.attributes.len(), "attribute")
                ),
            )
        })
        .collect::<Vec<_>>()
        .into();
    let attributes = catalog
        .attributes()
        .map(|attribute| {
            CatalogRow::new(
                CatalogKind::Attributes,
                attribute.id.clone(),
                attribute.name.clone(),
                attribute.meta.title.as_deref(),
                attribute.meta.description.as_deref(),
                type_kind_label(&attribute.ty.kind).to_string(),
            )
        })
        .collect::<Vec<_>>()
        .into();
    CatalogIndex {
        collections,
        classes,
        attributes,
    }
}

fn render_catalog_detail(
    catalog: &semantic_ui_core::UiCatalog,
    selection: &CatalogSelection,
) -> Element {
    match selection.kind {
        CatalogKind::Collections => {
            let Some(collection) = catalog.collection_by_name(&selection.canonical_id) else {
                return unavailable_detail();
            };
            let field_count = collection.field_ids.len();
            rsx! {
                div { class: "semantic-catalog__detail-content",
                    div { class: "semantic-catalog__detail-heading",
                        span { class: "semantic-catalog__eyebrow", "Collection" }
                        h2 { "{collection.name}" }
                        CopyableCode {
                            value: collection.name.clone(),
                            label: "Collection ID".to_string(),
                            on_copy: copy_handler(),
                        }
                    }
                    dl { class: "semantic-catalog__facts",
                        div { dt { "Fields" } dd { "{field_count}" } }
                        div { dt { "Integrity mode" } dd { "{collection.integrity_mode:?}" } }
                    }
                    if field_count > 0 {
                        section { class: "semantic-catalog__definition",
                            h3 { "Canonical fields" }
                            ul { class: "semantic-catalog__definition-list",
                                for field in collection.field_ids.iter().take(DETAIL_LIST_LIMIT) {
                                    li { key: "{field.canonical_field}",
                                        code { title: field.canonical_field.clone(), "{field.canonical_field}" }
                                    }
                                }
                            }
                            {truncation_note(field_count, "fields")}
                        }
                    }
                    div { class: "semantic-catalog__detail-actions",
                        Link {
                            to: Route::BrowsePage {
                                collection: Some(collection.name.clone()),
                                view: None,
                                renderer: None,
                                page: None,
                                page_size: None,
                                filters: None,
                                sql: None,
                            },
                            class: "dx-button",
                            "data-style": "primary",
                            "data-size": "default",
                            "Browse collection"
                        }
                    }
                }
            }
        }
        CatalogKind::Classes => {
            let Some(class) = catalog.class_by_id(&selection.canonical_id) else {
                return unavailable_detail();
            };
            let title = class.meta.title.as_deref().unwrap_or(&class.name);
            let attribute_count = class.attributes.len();
            rsx! {
                div { class: "semantic-catalog__detail-content",
                    div { class: "semantic-catalog__detail-heading",
                        span { class: "semantic-catalog__eyebrow", "Class" }
                        h2 { "{title}" }
                        if title != class.name { p { class: "semantic-catalog__schema-name", "Schema name: {class.name}" } }
                        CopyableCode {
                            value: class.id.clone(),
                            label: "Class ID".to_string(),
                            on_copy: copy_handler(),
                        }
                    }
                    {render_meta(&class.meta)}
                    dl { class: "semantic-catalog__facts",
                        div { dt { "Attributes" } dd { "{attribute_count}" } }
                        div { dt { "Class constraints" } dd { "{class.constraints.len()}" } }
                        div { dt { "Extensions" } dd { "{class.extends.len()}" } }
                    }
                    if let Some(parent) = &class.inherits {
                        section { class: "semantic-catalog__definition",
                            h3 { "Inherits" }
                            CopyableCode {
                                value: parent.id.clone(),
                                label: "Parent class ID".to_string(),
                                on_copy: copy_handler(),
                            }
                        }
                    }
                    if !class.extends.is_empty() {
                        section { class: "semantic-catalog__definition",
                            h3 { "Extends" }
                            ul { class: "semantic-catalog__definition-list",
                                for extension in class.extends.iter().take(DETAIL_LIST_LIMIT) {
                                    li { key: "{extension.id}",
                                        code { title: extension.id.clone(), "{extension.id}" }
                                    }
                                }
                            }
                            {truncation_note(class.extends.len(), "extensions")}
                        }
                    }
                    if attribute_count > 0 {
                        section { class: "semantic-catalog__definition",
                            h3 { "Declared attributes" }
                            ul { class: "semantic-catalog__attribute-list",
                                for (field_name, class_attribute) in class.attributes.iter().take(DETAIL_LIST_LIMIT) {
                                    li { key: "{field_name}",
                                        div {
                                            strong { "{field_name}" }
                                            code { title: class_attribute.attribute.id.clone(), "{class_attribute.attribute.id}" }
                                        }
                                        span { class: "semantic-catalog__badges",
                                            if class_attribute.required { span { class: "semantic-catalog__badge", "Required" } }
                                            if class_attribute.computed.is_some() { span { class: "semantic-catalog__badge", "Computed" } }
                                            if !class_attribute.constraints.is_empty() {
                                                span { class: "semantic-catalog__badge",
                                                    "{class_attribute.constraints.len()} local constraints"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            {truncation_note(attribute_count, "attributes")}
                        }
                    }
                    div { class: "semantic-catalog__detail-actions",
                        Link {
                            to: Route::CreateEntityPage,
                            class: "dx-button",
                            "data-style": "primary",
                            "data-size": "default",
                            "Open entity creator"
                        }
                    }
                }
            }
        }
        CatalogKind::Attributes => {
            let Some(attribute) = catalog.attribute_by_id(&selection.canonical_id) else {
                return unavailable_detail();
            };
            let title = attribute.meta.title.as_deref().unwrap_or(&attribute.name);
            rsx! {
                div { class: "semantic-catalog__detail-content",
                    div { class: "semantic-catalog__detail-heading",
                        span { class: "semantic-catalog__eyebrow", "Attribute" }
                        h2 { "{title}" }
                        if title != attribute.name {
                            p { class: "semantic-catalog__schema-name", "Schema name: {attribute.name}" }
                        }
                        CopyableCode {
                            value: attribute.id.clone(),
                            label: "Attribute ID".to_string(),
                            on_copy: copy_handler(),
                        }
                    }
                    {render_meta(&attribute.meta)}
                    dl { class: "semantic-catalog__facts",
                        div { dt { "Value type" } dd { "{type_kind_label(&attribute.ty.kind)}" } }
                        div { dt { "Attribute constraints" } dd { "{attribute.constraints.len()}" } }
                        div { dt { "Type constraints" } dd { "{attribute.ty.constraints.len()}" } }
                        div { dt { "Type annotations" } dd { "{attribute.ty.annotations.len()}" } }
                    }
                }
            }
        }
    }
}

fn render_meta(meta: &Meta) -> Element {
    let has_metadata = meta.description.is_some()
        || meta.deprecated.is_some()
        || !meta.aliases.is_empty()
        || !meta.tags.is_empty()
        || meta.docs_url.is_some();
    if !has_metadata {
        return rsx! {};
    }
    let aliases = meta.aliases.join(", ");
    let tags = meta.tags.join(", ");
    rsx! {
        section { class: "semantic-catalog__metadata",
            if let Some(description) = &meta.description {
                p { class: "semantic-catalog__description", "{description}" }
            }
            if let Some(deprecation) = &meta.deprecated {
                p { class: "semantic-catalog__deprecated",
                    strong { "Deprecated." }
                    if let Some(note) = &deprecation.note { " {note}" }
                }
            }
            if !meta.aliases.is_empty() {
                div { class: "semantic-catalog__metadata-row",
                    strong { "Aliases" }
                    span { "{aliases}" }
                }
            }
            if !meta.tags.is_empty() {
                div { class: "semantic-catalog__metadata-row",
                    strong { "Tags" }
                    span { "{tags}" }
                }
            }
            if let Some(docs_url) = &meta.docs_url {
                a { href: docs_url.clone(), target: "_blank", rel: "noreferrer", "Open schema documentation" }
            }
        }
    }
}

fn unavailable_detail() -> Element {
    rsx! {
        div { class: "semantic-catalog__detail-placeholder",
            h2 { "Entry unavailable" }
            p { "This schema entry is no longer present in the loaded catalog." }
        }
    }
}

fn truncation_note(total: usize, noun: &str) -> Element {
    if total <= DETAIL_LIST_LIMIT {
        return rsx! {};
    }
    rsx! {
        p { class: "semantic-catalog__truncation",
            "Showing the first {DETAIL_LIST_LIMIT} of {total} {noun}. Use search to locate another schema entry."
        }
    }
}

fn copy_handler() -> EventHandler<CopyRequest> {
    EventHandler::new(copy_catalog_value)
}

fn copy_catalog_value(request: CopyRequest) {
    let value = request.value().to_string();
    spawn(async move {
        let eval = document::eval(
            r#"
                const value = await dioxus.recv();
                await navigator.clipboard.writeText(value);
                return true;
            "#,
        );
        let result = match eval.send(value) {
            Ok(()) => eval.await.map(|_| ()).map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        request.complete(result);
    });
}

fn normalize_query(query: &str) -> String {
    query.trim().to_lowercase()
}

fn visible_row_count(filtered_count: usize, requested_limit: usize) -> usize {
    filtered_count.min(requested_limit.max(INITIAL_VISIBLE_ROWS))
}

fn pluralize(count: usize, singular: &str) -> String {
    if count == 1 {
        singular.to_string()
    } else {
        format!("{singular}s")
    }
}

fn type_kind_label(kind: &TypeKind) -> &'static str {
    match kind {
        TypeKind::Any(_) => "Any",
        TypeKind::Never(_) => "Never",
        TypeKind::Unknown(_) => "Unknown",
        TypeKind::Null(_) => "Null",
        TypeKind::Bool(_) => "Boolean",
        TypeKind::Char(_) => "Character",
        TypeKind::Number(_) => "Number",
        TypeKind::String(_) => "String",
        TypeKind::Bytes(_) => "Bytes",
        TypeKind::Temporal(_) => "Temporal",
        TypeKind::Uuid => "UUID",
        TypeKind::IpAddr(_) => "IP address",
        TypeKind::Json => "JSON",
        TypeKind::Optional(_) => "Optional",
        TypeKind::Array(_) => "Array",
        TypeKind::List(_) => "List",
        TypeKind::Tuple(_) => "Tuple",
        TypeKind::Map(_) => "Map",
        TypeKind::Set(_) => "Set",
        TypeKind::Record(_) => "Record",
        TypeKind::Attribute(_) => "Attribute",
        TypeKind::Class(_) => "Class",
        TypeKind::Union(_) => "Union",
        TypeKind::Intersection(_) => "Intersection",
        TypeKind::Variant(_) => "Variant",
        TypeKind::Enum(_) => "Enum",
        TypeKind::Result(_) => "Result",
        TypeKind::Function(_) => "Function",
        TypeKind::Interface(_) => "Interface",
        TypeKind::Handle(_) => "Handle",
        TypeKind::Stream(_) => "Stream",
        TypeKind::Opaque(_) => "Opaque",
        TypeKind::Extension(_) => "Extension",
        TypeKind::Ref(_) => "Reference",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_search_matches_title_name_and_canonical_id_case_insensitively() {
        let row = CatalogRow::new(
            CatalogKind::Classes,
            "semantic:MediaFile".to_string(),
            "media_file".to_string(),
            Some("Media file"),
            None,
            "3 attributes".to_string(),
        );

        assert!(row.matches(&normalize_query("MEDIA FILE")));
        assert!(row.matches(&normalize_query("media_file")));
        assert!(row.matches(&normalize_query("SEMANTIC:MEDIAFILE")));
        assert!(!row.matches(&normalize_query("directory")));
    }

    #[test]
    fn catalog_rows_are_disclosed_in_bounded_batches() {
        assert_eq!(visible_row_count(10, INITIAL_VISIBLE_ROWS), 10);
        assert_eq!(visible_row_count(250, INITIAL_VISIBLE_ROWS), 100);
        assert_eq!(visible_row_count(250, INITIAL_VISIBLE_ROWS * 2), 200);
        assert_eq!(visible_row_count(250, INITIAL_VISIBLE_ROWS * 3), 250);
    }

    #[test]
    fn type_labels_are_compact_and_human_readable() {
        assert_eq!(type_kind_label(&TypeKind::Uuid), "UUID");
        assert_eq!(type_kind_label(&TypeKind::Json), "JSON");
    }
}
