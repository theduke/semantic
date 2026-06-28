use std::collections::BTreeSet;

use dioxus::logger::tracing::{error, info, warn};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    ChevronDown, ChevronRight, FileText, Folder, Grid2x2, List, PanelLeft, RefreshCw,
};
use futures::StreamExt;

use crate::{
    components::{ClassView, ObjectView},
    context::{use_active_scope_id, use_rpc_client},
    ui_catalog::{RenderMode, use_ui_catalog},
};

use super::{
    data::{load_directory_page, load_tree_rows, move_directory_item},
    types::{
        BrowseViewMode, DirectoryBrowseItem, DirectoryBrowserProps, DirectoryPage, DirectorySort,
        DirectoryTreeRow,
    },
};

#[derive(Clone, Debug, PartialEq)]
enum LoadState<T> {
    Loading,
    Ready(T),
    Error(String),
}

enum DirectoryBrowserCommand {
    SyncInputs {
        client: semantic_rpc::RpcClient,
        scope_id: Option<String>,
        root: Option<String>,
        default_page_size: usize,
        default_tree_open: bool,
    },
    SetSort(DirectorySort),
    SetPageSize(usize),
    SetViewMode(BrowseViewMode),
    ToggleTree,
    Refresh,
    PreviousPage,
    NextPage,
    ToggleTreeExpansion(String),
    BeginDrag(String),
    EndDrag,
    DropOnDirectory {
        item_id: Option<String>,
        target_directory_id: String,
        is_directory: bool,
        suppress_click: bool,
    },
    OpenItem(DirectoryBrowseItem),
    OpenTreeRoot(String),
    CloseDetail,
}

#[derive(Clone, Copy)]
struct DirectoryBrowserSignals {
    current_root: Signal<Option<String>>,
    page: Signal<usize>,
    page_size: Signal<usize>,
    sort: Signal<DirectorySort>,
    view_mode: Signal<BrowseViewMode>,
    tree_open: Signal<bool>,
    expanded_tree: Signal<BTreeSet<String>>,
    selected_item: Signal<Option<DirectoryBrowseItem>>,
    dragged_item: Signal<Option<String>>,
    suppress_click: Signal<bool>,
    move_error: Signal<Option<String>>,
    content_state: Signal<LoadState<DirectoryPage>>,
    tree_state: Signal<LoadState<Vec<DirectoryTreeRow>>>,
}

fn use_directory_browser_coroutine(
    mut state: DirectoryBrowserSignals,
) -> Coroutine<DirectoryBrowserCommand> {
    use_coroutine(
        move |mut rx: UnboundedReceiver<DirectoryBrowserCommand>| async move {
            info!("directory browser coroutine started");
            let mut active_client = None::<semantic_rpc::RpcClient>;
            let mut active_scope_id = None::<String>;
            let mut initialized = false;

            while let Some(command) = rx.next().await {
                match command {
                    DirectoryBrowserCommand::SyncInputs {
                        client,
                        scope_id,
                        root,
                        default_page_size,
                        default_tree_open,
                    } => {
                        info!(
                            root = root.as_deref(),
                            scope_id = scope_id.as_deref(),
                            default_page_size,
                            default_tree_open,
                            initialized,
                            "directory browser sync inputs"
                        );
                        active_client = Some(client);
                        let scope_changed = initialized && active_scope_id != scope_id;
                        active_scope_id = scope_id;

                        if !initialized {
                            initialized = true;
                            state.current_root.set(root);
                            state.page.set(0);
                            state.page_size.set(default_page_size.max(1));
                            state.tree_open.set(default_tree_open);
                            state.selected_item.set(None);
                            state.move_error.set(None);
                            reload_current_content(
                                active_client.clone(),
                                active_scope_id.clone(),
                                state,
                            )
                            .await;
                            if (state.tree_open)() {
                                reload_current_tree(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                            }
                            continue;
                        }

                        let root_changed = (state.current_root)() != root;
                        info!(
                            root_changed,
                            scope_changed,
                            current_root = (state.current_root)().as_deref(),
                            next_root = root.as_deref(),
                            "directory browser input change evaluated"
                        );
                        if root_changed {
                            state.current_root.set(root);
                            state.page.set(0);
                            state.selected_item.set(None);
                        }
                        if root_changed || scope_changed {
                            state.move_error.set(None);
                            reload_current_content(
                                active_client.clone(),
                                active_scope_id.clone(),
                                state,
                            )
                            .await;
                            if (state.tree_open)() {
                                reload_current_tree(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                            }
                        }
                    }
                    DirectoryBrowserCommand::SetSort(next) => {
                        info!(sort = next.as_value(), "directory browser set sort");
                        state.sort.set(next);
                        state.page.set(0);
                        reload_current_content(
                            active_client.clone(),
                            active_scope_id.clone(),
                            state,
                        )
                        .await;
                    }
                    DirectoryBrowserCommand::SetPageSize(next) => {
                        info!(page_size = next, "directory browser set page size");
                        state.page_size.set(next.max(1));
                        state.page.set(0);
                        reload_current_content(
                            active_client.clone(),
                            active_scope_id.clone(),
                            state,
                        )
                        .await;
                    }
                    DirectoryBrowserCommand::SetViewMode(next) => {
                        info!(view_mode = ?next, "directory browser set view mode");
                        state.view_mode.set(next);
                    }
                    DirectoryBrowserCommand::ToggleTree => {
                        let next_open = !(state.tree_open)();
                        info!(tree_open = next_open, "directory browser toggle tree");
                        state.tree_open.set(next_open);
                        if next_open {
                            reload_current_tree(
                                active_client.clone(),
                                active_scope_id.clone(),
                                state,
                            )
                            .await;
                        }
                    }
                    DirectoryBrowserCommand::Refresh => {
                        info!("directory browser refresh");
                        reload_current_content(
                            active_client.clone(),
                            active_scope_id.clone(),
                            state,
                        )
                        .await;
                        if (state.tree_open)() {
                            reload_current_tree(
                                active_client.clone(),
                                active_scope_id.clone(),
                                state,
                            )
                            .await;
                        }
                    }
                    DirectoryBrowserCommand::PreviousPage => {
                        state.page.set((state.page)().saturating_sub(1));
                        info!(page = (state.page)(), "directory browser previous page");
                        reload_current_content(
                            active_client.clone(),
                            active_scope_id.clone(),
                            state,
                        )
                        .await;
                    }
                    DirectoryBrowserCommand::NextPage => {
                        state.page.set((state.page)().saturating_add(1));
                        info!(page = (state.page)(), "directory browser next page");
                        reload_current_content(
                            active_client.clone(),
                            active_scope_id.clone(),
                            state,
                        )
                        .await;
                    }
                    DirectoryBrowserCommand::ToggleTreeExpansion(item_id) => {
                        let logged_item_id = item_id.clone();
                        let mut next = (state.expanded_tree)();
                        if next.contains(&item_id) {
                            next.remove(&item_id);
                        } else {
                            next.insert(item_id);
                        }
                        state.expanded_tree.set(next);
                        info!(
                            item_id = logged_item_id.as_str(),
                            expanded_count = (state.expanded_tree)().len(),
                            "directory browser toggle tree expansion"
                        );
                        reload_current_tree(active_client.clone(), active_scope_id.clone(), state)
                            .await;
                    }
                    DirectoryBrowserCommand::BeginDrag(item_id) => {
                        info!(item_id = item_id.as_str(), "directory browser begin drag");
                        state.suppress_click.set(false);
                        state.dragged_item.set(Some(item_id));
                    }
                    DirectoryBrowserCommand::EndDrag => {
                        info!("directory browser end drag");
                        state.dragged_item.set(None);
                    }
                    DirectoryBrowserCommand::DropOnDirectory {
                        item_id,
                        target_directory_id,
                        is_directory,
                        suppress_click: should_suppress_click,
                    } => {
                        let item_id = item_id.or_else(|| (state.dragged_item)());
                        state.dragged_item.set(None);
                        if !is_directory {
                            warn!(
                                target_directory_id = target_directory_id.as_str(),
                                "directory browser drop ignored because target is not a directory"
                            );
                            continue;
                        }
                        let Some(item_id) = item_id else {
                            warn!(
                                target_directory_id = target_directory_id.as_str(),
                                "directory browser drop ignored because no dragged item id was available"
                            );
                            continue;
                        };
                        if item_id == target_directory_id {
                            info!(
                                item_id = item_id.as_str(),
                                "directory browser drop ignored because source equals target"
                            );
                            continue;
                        }
                        if should_suppress_click {
                            state.suppress_click.set(true);
                        }
                        let Some(client) = active_client.clone() else {
                            error!(
                                item_id = item_id.as_str(),
                                target_directory_id = target_directory_id.as_str(),
                                "directory browser drop ignored because no active RPC client is available"
                            );
                            continue;
                        };
                        info!(
                            item_id = item_id.as_str(),
                            target_directory_id = target_directory_id.as_str(),
                            "directory browser moving item"
                        );
                        match move_directory_item(
                            client,
                            active_scope_id.clone(),
                            item_id,
                            target_directory_id,
                        )
                        .await
                        {
                            Ok(()) => {
                                info!("directory browser move completed");
                                state.move_error.set(None);
                                reload_current_content(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                                if (state.tree_open)() {
                                    reload_current_tree(
                                        active_client.clone(),
                                        active_scope_id.clone(),
                                        state,
                                    )
                                    .await;
                                }
                            }
                            Err(err) => {
                                error!(error = err.as_str(), "directory browser move failed");
                                state.move_error.set(Some(err));
                            }
                        }
                    }
                    DirectoryBrowserCommand::OpenItem(item) => {
                        if (state.suppress_click)() {
                            info!(
                                item_id = item.id.as_str(),
                                "directory browser open item suppressed after drag"
                            );
                            state.suppress_click.set(false);
                            continue;
                        }
                        if item.is_directory {
                            info!(
                                item_id = item.id.as_str(),
                                "directory browser navigate to directory item"
                            );
                            navigate_to_tree_root(Some(item.id));
                        } else {
                            info!(
                                item_id = item.id.as_str(),
                                "directory browser open detail item"
                            );
                            state.selected_item.set(Some(item));
                        }
                    }
                    DirectoryBrowserCommand::OpenTreeRoot(item_id) => {
                        if (state.suppress_click)() {
                            info!(
                                item_id = item_id.as_str(),
                                "directory browser open tree root suppressed after drag"
                            );
                            state.suppress_click.set(false);
                            continue;
                        }
                        info!(
                            item_id = item_id.as_str(),
                            "directory browser navigate to tree root"
                        );
                        navigate_to_tree_root(Some(item_id));
                    }
                    DirectoryBrowserCommand::CloseDetail => {
                        info!("directory browser close detail");
                        state.selected_item.set(None);
                    }
                }
            }
            warn!("directory browser coroutine stopped");
        },
    )
}

#[allow(non_snake_case)]
pub fn DirectoryBrowser(props: DirectoryBrowserProps) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();

    let root = props.root.clone();
    let config = props.config.clone();
    let current_root = {
        let root = root.clone();
        use_signal(move || root)
    };
    let page = use_signal(|| 0usize);
    let page_size = {
        let default_page_size = config.default_page_size;
        use_signal(move || default_page_size)
    };
    let sort = use_signal(|| DirectorySort::Order);
    let view_mode = use_signal(|| BrowseViewMode::List);
    let tree_open = {
        let default_tree_open = config.default_tree_open;
        use_signal(move || default_tree_open)
    };
    let expanded_tree = use_signal(BTreeSet::<String>::new);
    let selected_item = use_signal(|| None::<DirectoryBrowseItem>);
    let dragged_item = use_signal(|| None::<String>);
    let suppress_click = use_signal(|| false);
    let move_error = use_signal(|| None::<String>);
    let content_state = use_signal(|| LoadState::<DirectoryPage>::Loading);
    let tree_state = use_signal(|| LoadState::<Vec<DirectoryTreeRow>>::Loading);

    let state = DirectoryBrowserSignals {
        current_root,
        page,
        page_size,
        sort,
        view_mode,
        tree_open,
        expanded_tree,
        selected_item,
        dragged_item,
        suppress_click,
        move_error,
        content_state,
        tree_state,
    };

    let commands = use_directory_browser_coroutine(state);

    use_effect(use_reactive(
        (
            &root,
            &config.default_page_size,
            &config.default_tree_open,
            &scope_id,
        ),
        {
            let client = client.clone();
            move |(root, default_page_size, default_tree_open, scope_id)| {
                commands.send(DirectoryBrowserCommand::SyncInputs {
                    client: client.clone(),
                    scope_id,
                    root,
                    default_page_size,
                    default_tree_open,
                });
            }
        },
    ));

    rsx! {
        section { class: "semantic-directory-browser",
            div { class: "semantic-directory-browser__toolbar",
                div { class: "semantic-directory-browser__breadcrumbs",
                    match &*content_state.read() {
                        LoadState::Ready(page_data) => rsx! {
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
                        LoadState::Loading | LoadState::Error(_) => rsx! {
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
                        onclick: move |_| commands.send(DirectoryBrowserCommand::ToggleTree),
                        PanelLeft { size: "1rem" }
                    }
                    label { class: "semantic-directory-browser__control",
                        span { "Sort" }
                        select {
                            value: "{sort().as_value()}",
                            onchange: move |event| {
                                commands.send(DirectoryBrowserCommand::SetSort(DirectorySort::from_value(&event.value())));
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
                                    commands.send(DirectoryBrowserCommand::SetPageSize(next));
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
                            onclick: move |_| commands.send(DirectoryBrowserCommand::SetViewMode(BrowseViewMode::List)),
                            List { size: "1rem" }
                        }
                        dxcomp::Button {
                            variant: if view_mode() == BrowseViewMode::Icons { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                            size: dxcomp::ButtonSize::IconSm,
                            title: "Icon view",
                            onclick: move |_| commands.send(DirectoryBrowserCommand::SetViewMode(BrowseViewMode::Icons)),
                            Grid2x2 { size: "1rem" }
                        }
                    }
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::IconSm,
                        title: "Refresh",
                        onclick: move |_| commands.send(DirectoryBrowserCommand::Refresh),
                        RefreshCw { size: "1rem" }
                    }
                }
            }
            if let Some(err) = move_error() {
                div { class: "semantic-error", "{err}" }
            }
            div {
                class: "semantic-directory-browser__body",
                onmouseup: move |_| end_drag(commands),
                if tree_open() {
                    aside { class: "semantic-directory-browser__tree",
                        match &*tree_state.read() {
                            LoadState::Ready(rows) => rsx! {
                                if rows.is_empty() {
                                    div { class: "semantic-empty", "No directories" }
                                } else {
                                    for row in rows.iter().cloned() {
                                        DirectoryTreeRowView {
                                            row,
                                            active_root: current_root(),
                                            expanded: expanded_tree,
                                            commands,
                                        }
                                    }
                                }
                            },
                            LoadState::Error(err) => rsx! { div { class: "semantic-error", "{err}" } },
                            LoadState::Loading => rsx! { div { class: "semantic-loading", "Loading tree..." } },
                        }
                    }
                }
                div { class: "semantic-directory-browser__content",
                    match &*content_state.read() {
                        LoadState::Ready(page_data) => rsx! {
                            if page_data.items.is_empty() {
                                div { class: "semantic-empty", "No items" }
                            } else if view_mode() == BrowseViewMode::List {
                                DirectoryList {
                                    items: page_data.items.clone(),
                                    commands,
                                }
                            } else {
                                DirectoryGrid {
                                    items: page_data.items.clone(),
                                    commands,
                                }
                            }
                            DirectoryPagination {
                                page: page(),
                                has_next: page_data.has_next,
                                on_previous: move |_| commands.send(DirectoryBrowserCommand::PreviousPage),
                                on_next: move |_| commands.send(DirectoryBrowserCommand::NextPage),
                            }
                        },
                        LoadState::Error(err) => rsx! { div { class: "semantic-error", "{err}" } },
                        LoadState::Loading => rsx! { div { class: "semantic-loading", "Loading directory..." } },
                    }
                }
            }
            EntityDetailDialog {
                open: selected_item().is_some(),
                item: selected_item(),
                on_open_change: move |open: bool| {
                    if !open {
                        commands.send(DirectoryBrowserCommand::CloseDetail);
                    }
                },
            }
        }
    }
}

#[component]
fn DirectoryList(
    items: Vec<DirectoryBrowseItem>,
    commands: Coroutine<DirectoryBrowserCommand>,
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
                        commands,
                    }
                }
            }
        }
    }
}

#[component]
fn DirectoryGrid(
    items: Vec<DirectoryBrowseItem>,
    commands: Coroutine<DirectoryBrowserCommand>,
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
                                commands,
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
    commands: Coroutine<DirectoryBrowserCommand>,
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
            "data-drop-target": item.is_directory,
            "data-draggable": true,
            onmousedown: {
                let item_id = item.id.clone();
                move |_| begin_drag(item_id.clone(), commands)
            },
            onmouseup: {
                let target_id = item.id.clone();
                let is_directory = item.is_directory;
                move |event: MouseEvent| {
                    event.stop_propagation();
                    move_dragged_item_to_directory(None, target_id.clone(), is_directory, true, commands);
                }
            },
            onclick: move |_| commands.send(DirectoryBrowserCommand::OpenItem(item.clone())),
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
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let type_label = item.type_id.clone().unwrap_or_else(|| "entity".to_string());
    rsx! {
        button {
            class: "semantic-directory-browser__tile",
            "data-drop-target": item.is_directory,
            "data-draggable": true,
            onmousedown: {
                let item_id = item.id.clone();
                move |_| begin_drag(item_id.clone(), commands)
            },
            onmouseup: {
                let target_id = item.id.clone();
                let is_directory = item.is_directory;
                move |event: MouseEvent| {
                    event.stop_propagation();
                    move_dragged_item_to_directory(None, target_id.clone(), is_directory, true, commands);
                }
            },
            onclick: move |_| commands.send(DirectoryBrowserCommand::OpenItem(item.clone())),
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
    commands: Coroutine<DirectoryBrowserCommand>,
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
                        commands.send(DirectoryBrowserCommand::ToggleTreeExpansion(item_id.clone()));
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
                "data-drop-target": true,
                "data-draggable": true,
                onmousedown: {
                    let item_id = item.id.clone();
                    move |_| begin_drag(item_id.clone(), commands)
                },
                onmouseup: {
                    let target_id = item.id.clone();
                    move |event: MouseEvent| {
                        event.stop_propagation();
                        move_dragged_item_to_directory(None, target_id.clone(), true, true, commands);
                    }
                },
                onclick: move |_| commands.send(DirectoryBrowserCommand::OpenTreeRoot(item.id.clone())),
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

async fn reload_current_content(
    client: Option<semantic_rpc::RpcClient>,
    scope_id: Option<String>,
    state: DirectoryBrowserSignals,
) {
    let Some(client) = client else {
        error!(
            "directory browser content reload skipped because no active RPC client is available"
        );
        return;
    };
    info!(
        root = (state.current_root)().as_deref(),
        page = (state.page)(),
        page_size = (state.page_size)(),
        sort = (state.sort)().as_value(),
        scope_id = scope_id.as_deref(),
        "directory browser content reload requested"
    );
    reload_content(
        client,
        scope_id,
        (state.current_root)(),
        (state.page)(),
        (state.page_size)(),
        (state.sort)(),
        state.content_state,
    )
    .await;
}

async fn reload_current_tree(
    client: Option<semantic_rpc::RpcClient>,
    scope_id: Option<String>,
    state: DirectoryBrowserSignals,
) {
    let Some(client) = client else {
        error!("directory browser tree reload skipped because no active RPC client is available");
        return;
    };
    info!(
        expanded_count = (state.expanded_tree)().len(),
        scope_id = scope_id.as_deref(),
        "directory browser tree reload requested"
    );
    reload_tree(client, scope_id, (state.expanded_tree)(), state.tree_state).await;
}

async fn reload_content(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    root: Option<String>,
    page: usize,
    page_size: usize,
    sort: DirectorySort,
    mut content_state: Signal<LoadState<DirectoryPage>>,
) {
    info!(
        root = root.as_deref(),
        page,
        page_size,
        sort = sort.as_value(),
        scope_id = scope_id.as_deref(),
        "directory browser loading content"
    );
    content_state.set(LoadState::Loading);
    let next = match load_directory_page(client, scope_id, root, page, page_size, sort).await {
        Ok(page_data) => {
            info!(
                item_count = page_data.items.len(),
                breadcrumb_count = page_data.breadcrumbs.len(),
                has_next = page_data.has_next,
                "directory browser content loaded"
            );
            LoadState::Ready(page_data)
        }
        Err(err) => {
            error!(
                error = err.as_str(),
                "directory browser content load failed"
            );
            LoadState::Error(err)
        }
    };
    content_state.set(next);
}

async fn reload_tree(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    expanded: BTreeSet<String>,
    mut tree_state: Signal<LoadState<Vec<DirectoryTreeRow>>>,
) {
    info!(
        expanded_count = expanded.len(),
        scope_id = scope_id.as_deref(),
        "directory browser loading tree"
    );
    tree_state.set(LoadState::Loading);
    let next = match load_tree_rows(client, scope_id, expanded).await {
        Ok(rows) => {
            info!(row_count = rows.len(), "directory browser tree loaded");
            LoadState::Ready(rows)
        }
        Err(err) => {
            error!(error = err.as_str(), "directory browser tree load failed");
            LoadState::Error(err)
        }
    };
    tree_state.set(next);
}

fn begin_drag(item_id: String, commands: Coroutine<DirectoryBrowserCommand>) {
    commands.send(DirectoryBrowserCommand::BeginDrag(item_id));
}

fn end_drag(commands: Coroutine<DirectoryBrowserCommand>) {
    commands.send(DirectoryBrowserCommand::EndDrag);
}

fn move_dragged_item_to_directory(
    item_id: Option<String>,
    target_directory_id: String,
    is_directory: bool,
    suppress_click: bool,
    commands: Coroutine<DirectoryBrowserCommand>,
) {
    commands.send(DirectoryBrowserCommand::DropOnDirectory {
        item_id,
        target_directory_id,
        is_directory,
        suppress_click,
    });
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
