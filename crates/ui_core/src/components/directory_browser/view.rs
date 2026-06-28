use std::collections::BTreeSet;

use dioxus::prelude::*;
use dioxus_icons::lucide::{
    ChevronDown, ChevronRight, FileText, Folder, Grid2x2, List, PanelLeft, RefreshCw,
};

use crate::{
    components::{ClassView, ObjectView},
    context::{use_active_scope_id, use_rpc_client},
    ui_catalog::{RenderMode, use_ui_catalog},
};

use super::{
    data::{load_directory_page, load_tree_rows},
    types::{
        BrowseViewMode, DirectoryBrowseItem, DirectoryBrowserProps, DirectorySort, DirectoryTreeRow,
    },
};

#[allow(non_snake_case)]
pub fn DirectoryBrowser(props: DirectoryBrowserProps) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();

    let root = props.root.clone();
    let config = props.config.clone();
    let mut page = use_signal(|| 0usize);
    let mut page_size = {
        let default_page_size = config.default_page_size;
        use_signal(move || default_page_size)
    };
    let mut sort = use_signal(|| DirectorySort::Order);
    let mut view_mode = use_signal(|| BrowseViewMode::List);
    let mut tree_open = {
        let default_tree_open = config.default_tree_open;
        use_signal(move || default_tree_open)
    };
    let expanded_tree = use_signal(BTreeSet::<String>::new);
    let mut selected_item = use_signal(|| None::<DirectoryBrowseItem>);
    let mut refresh = use_signal(|| 0usize);

    let content_resource = use_resource({
        let client = client.clone();
        let scope_id = scope_id.clone();
        let root = root.clone();
        move || {
            let client = client.clone();
            let scope_id = scope_id.clone();
            let root = root.clone();
            let page = page();
            let page_size = page_size();
            let sort = sort();
            let _refresh = refresh();
            async move { load_directory_page(client, scope_id, root, page, page_size, sort).await }
        }
    });

    let tree_resource = use_resource({
        let client = client.clone();
        let scope_id = scope_id.clone();
        move || {
            let client = client.clone();
            let scope_id = scope_id.clone();
            let expanded = expanded_tree();
            let _refresh = refresh();
            async move { load_tree_rows(client, scope_id, expanded).await }
        }
    });

    rsx! {
        section { class: "semantic-directory-browser",
            div { class: "semantic-directory-browser__toolbar",
                div { class: "semantic-directory-browser__breadcrumbs",
                    match &*content_resource.read_unchecked() {
                        Some(Ok(page_data)) => rsx! {
                            button {
                                class: "semantic-directory-browser__breadcrumb",
                                onclick: move |_| navigate_to_tree_root(None),
                                "Tree"
                            }
                            for crumb in page_data.breadcrumbs.iter().cloned() {
                                span { class: "semantic-directory-browser__breadcrumb-separator", "/" }
                                button {
                                    class: "semantic-directory-browser__breadcrumb",
                                    onclick: move |_| navigate_to_tree_root(Some(crumb.id.clone())),
                                    "{crumb.title}"
                                }
                            }
                            if page_data.breadcrumb_cycle {
                                span { class: "semantic-directory-browser__warning", "Cycle stopped" }
                            }
                        },
                        _ => rsx! {
                            button {
                                class: "semantic-directory-browser__breadcrumb",
                                onclick: move |_| navigate_to_tree_root(None),
                                "Tree"
                            }
                        },
                    }
                }
                div { class: "semantic-directory-browser__toolbar-controls",
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::IconSm,
                        title: "Toggle tree",
                        onclick: move |_| tree_open.set(!tree_open()),
                        PanelLeft { size: "1rem" }
                    }
                    label { class: "semantic-directory-browser__control",
                        span { "Sort" }
                        select {
                            value: "{sort().as_value()}",
                            onchange: move |event| {
                                sort.set(DirectorySort::from_value(&event.value()));
                                page.set(0);
                            },
                            for option in DirectorySort::all() {
                                option { value: "{option.as_value()}", "{option.label()}" }
                            }
                        }
                    }
                    label { class: "semantic-directory-browser__control",
                        span { "Page" }
                        select {
                            value: "{page_size()}",
                            onchange: move |event| {
                                if let Ok(next) = event.value().parse::<usize>() {
                                    page_size.set(next.max(1));
                                    page.set(0);
                                }
                            },
                            for option in config.page_size_options.iter().copied() {
                                option { value: "{option}", "{option}" }
                            }
                        }
                    }
                    div { class: "semantic-directory-browser__view-toggle",
                        dxcomp::Button {
                            variant: if view_mode() == BrowseViewMode::List { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                            size: dxcomp::ButtonSize::IconSm,
                            title: "List view",
                            onclick: move |_| view_mode.set(BrowseViewMode::List),
                            List { size: "1rem" }
                        }
                        dxcomp::Button {
                            variant: if view_mode() == BrowseViewMode::Icons { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                            size: dxcomp::ButtonSize::IconSm,
                            title: "Icon view",
                            onclick: move |_| view_mode.set(BrowseViewMode::Icons),
                            Grid2x2 { size: "1rem" }
                        }
                    }
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::IconSm,
                        title: "Refresh",
                        onclick: move |_| refresh.set(refresh().saturating_add(1)),
                        RefreshCw { size: "1rem" }
                    }
                }
            }
            div { class: "semantic-directory-browser__body",
                if tree_open() {
                    aside { class: "semantic-directory-browser__tree",
                        match &*tree_resource.read_unchecked() {
                            Some(Ok(rows)) => rsx! {
                                if rows.is_empty() {
                                    div { class: "semantic-empty", "No directories" }
                                } else {
                                    for row in rows.iter().cloned() {
                                        DirectoryTreeRowView {
                                            row,
                                            active_root: root.clone(),
                                            expanded: expanded_tree,
                                        }
                                    }
                                }
                            },
                            Some(Err(err)) => rsx! { div { class: "semantic-error", "{err}" } },
                            None => rsx! { div { class: "semantic-loading", "Loading tree..." } },
                        }
                    }
                }
                div { class: "semantic-directory-browser__content",
                    match &*content_resource.read_unchecked() {
                        Some(Ok(page_data)) => rsx! {
                            if page_data.items.is_empty() {
                                div { class: "semantic-empty", "No items" }
                            } else if view_mode() == BrowseViewMode::List {
                                DirectoryList {
                                    items: page_data.items.clone(),
                                    selected_item,
                                }
                            } else {
                                DirectoryGrid {
                                    items: page_data.items.clone(),
                                    selected_item,
                                }
                            }
                            DirectoryPagination {
                                page: page(),
                                has_next: page_data.has_next,
                                on_previous: move |_| page.set(page().saturating_sub(1)),
                                on_next: move |_| page.set(page().saturating_add(1)),
                            }
                        },
                        Some(Err(err)) => rsx! { div { class: "semantic-error", "{err}" } },
                        None => rsx! { div { class: "semantic-loading", "Loading directory..." } },
                    }
                }
            }
            EntityDetailDialog {
                open: selected_item().is_some(),
                item: selected_item(),
                on_open_change: move |open: bool| {
                    if !open {
                        selected_item.set(None);
                    }
                },
            }
        }
    }
}

#[component]
fn DirectoryList(
    items: Vec<DirectoryBrowseItem>,
    selected_item: Signal<Option<DirectoryBrowseItem>>,
) -> Element {
    let count = items.len();
    let item_count = use_signal(move || count);
    rsx! {
        dxcomp::VirtualList {
            count: item_count,
            estimate_size: move |_| 56,
            render_item: move |index: usize| {
                let Some(item) = items.get(index).cloned() else {
                    return rsx! {};
                };
                rsx! {
                    DirectoryListRow {
                        key: "{item.id}",
                        item,
                        selected_item,
                    }
                }
            }
        }
    }
}

#[component]
fn DirectoryGrid(
    items: Vec<DirectoryBrowseItem>,
    selected_item: Signal<Option<DirectoryBrowseItem>>,
) -> Element {
    let columns = 4usize;
    let rows = items.len().div_ceil(columns);
    let row_count = use_signal(move || rows);
    rsx! {
        dxcomp::VirtualList {
            count: row_count,
            estimate_size: move |_| 132,
            render_item: move |row_index: usize| {
                let start = row_index * columns;
                let end = (start + columns).min(items.len());
                rsx! {
                    div { class: "semantic-directory-browser__grid-row",
                        for item in items[start..end].iter().cloned() {
                            DirectoryTile {
                                key: "{item.id}",
                                item,
                                selected_item,
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn DirectoryListRow(
    item: DirectoryBrowseItem,
    selected_item: Signal<Option<DirectoryBrowseItem>>,
) -> Element {
    let type_label = item.type_id.clone().unwrap_or_else(|| "entity".to_string());
    let order = item
        .order
        .map(|value| value.to_string())
        .unwrap_or_default();
    let updated = item
        .updated_at
        .clone()
        .or(item.created_at.clone())
        .unwrap_or_default();
    rsx! {
        button {
            class: "semantic-directory-browser__row",
            onclick: move |_| open_item(item.clone(), selected_item),
            if item.is_directory {
                Folder { size: "1.2rem" }
            } else {
                FileText { size: "1.2rem" }
            }
            span { class: "semantic-directory-browser__item-title", "{item.title}" }
            span { class: "semantic-directory-browser__item-type", "{type_label}" }
            code { class: "semantic-directory-browser__item-id", "{item.id}" }
            span { class: "semantic-directory-browser__item-order", "{order}" }
            span { class: "semantic-directory-browser__item-date", "{updated}" }
        }
    }
}

#[component]
fn DirectoryTile(
    item: DirectoryBrowseItem,
    selected_item: Signal<Option<DirectoryBrowseItem>>,
) -> Element {
    let type_label = item.type_id.clone().unwrap_or_else(|| "entity".to_string());
    rsx! {
        button {
            class: "semantic-directory-browser__tile",
            onclick: move |_| open_item(item.clone(), selected_item),
            if item.is_directory {
                Folder { size: "2rem" }
            } else {
                FileText { size: "2rem" }
            }
            span { class: "semantic-directory-browser__item-title", "{item.title}" }
            span { class: "semantic-directory-browser__item-type", "{type_label}" }
            code { class: "semantic-directory-browser__item-id", "{item.id}" }
        }
    }
}

#[component]
fn DirectoryTreeRowView(
    row: DirectoryTreeRow,
    active_root: Option<String>,
    expanded: Signal<BTreeSet<String>>,
) -> Element {
    let item = row.item.clone();
    let is_active = active_root.as_deref() == Some(item.id.as_str());
    let is_expanded = expanded.read().contains(&item.id);
    let padding = row.depth * 16;
    rsx! {
        div {
            class: "semantic-directory-browser__tree-row",
            "data-active": is_active,
            style: "--semantic-directory-depth: {padding}px",
            button {
                class: "semantic-directory-browser__tree-expander",
                disabled: row.cycle,
                onclick: {
                    let item_id = item.id.clone();
                    move |_| {
                        let mut next = expanded();
                        if next.contains(&item_id) {
                            next.remove(&item_id);
                        } else {
                            next.insert(item_id.clone());
                        }
                        expanded.set(next);
                    }
                },
                if is_expanded {
                    ChevronDown { size: "1rem" }
                } else {
                    ChevronRight { size: "1rem" }
                }
            }
            button {
                class: "semantic-directory-browser__tree-link",
                onclick: move |_| navigate_to_tree_root(Some(item.id.clone())),
                Folder { size: "1rem" }
                span { "{item.title}" }
            }
        }
    }
}

#[component]
fn DirectoryPagination(
    page: usize,
    has_next: bool,
    on_previous: EventHandler<MouseEvent>,
    on_next: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        dxcomp::Pagination { class: "semantic-directory-browser__pagination",
            dxcomp::PaginationContent {
                dxcomp::PaginationItem {
                    dxcomp::PaginationPrevious {
                        aria_disabled: page == 0,
                        onclick: move |event| {
                            if page > 0 {
                                on_previous.call(event);
                            }
                        },
                    }
                }
                dxcomp::PaginationItem {
                    span { class: "semantic-directory-browser__page-label", "Page {page + 1}" }
                }
                dxcomp::PaginationItem {
                    dxcomp::PaginationNext {
                        aria_disabled: !has_next,
                        onclick: move |event| {
                            if has_next {
                                on_next.call(event);
                            }
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn EntityDetailDialog(
    open: bool,
    item: Option<DirectoryBrowseItem>,
    on_open_change: EventHandler<bool>,
) -> Element {
    let catalog = use_ui_catalog();
    let title = item
        .as_ref()
        .map(|item| item.title.clone())
        .unwrap_or_else(|| "Entity".to_string());
    rsx! {
        dxcomp::Dialog {
            open,
            on_open_change,
            dxcomp::DialogTitle { "{title}" }
            if let Some(item) = item {
                if let Some(class) = catalog.object_class(&item.object).cloned() {
                    ClassView {
                        class,
                        object: item.object.clone(),
                        collection: Some(item.collection.clone()),
                        id: Some(item.id.clone()),
                        mode: RenderMode::Detail,
                    }
                } else {
                    ObjectView {
                        object: item.object.clone(),
                        mode: RenderMode::Detail,
                    }
                }
            }
            div { class: "semantic-directory-browser__dialog-actions",
                dxcomp::Button {
                    onclick: move |_| on_open_change.call(false),
                    "Close"
                }
            }
        }
    }
}

fn open_item(item: DirectoryBrowseItem, mut selected_item: Signal<Option<DirectoryBrowseItem>>) {
    if item.is_directory {
        navigate_to_tree_root(Some(item.id));
    } else {
        selected_item.set(Some(item));
    }
}

fn navigate_to_tree_root(root: Option<String>) {
    let target = match root {
        Some(root) => format!("/tree?root={}", url_query_escape(&root)),
        None => "/tree".to_string(),
    };
    let _ = navigator().push(target);
}

fn url_query_escape(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => {
                let encoded = format!("%{byte:02X}");
                encoded.chars().collect()
            }
        })
        .collect()
}
