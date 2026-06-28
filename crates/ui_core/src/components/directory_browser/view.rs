use std::{collections::BTreeSet, time::Duration};

use dioxus::logger::tracing::{error, info, warn};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    ChevronDown, ChevronRight, ClipboardPaste, Copy, FileText, Folder, FolderPlus, Grid2x2, Link,
    List, PanelLeft, Pencil, Plus, RefreshCw, Scissors, Trash2, X,
};
use futures::StreamExt;

use crate::{
    components::{ClassView, ObjectView},
    context::{use_active_scope_id, use_rpc_client},
    ui_catalog::{RenderMode, use_ui_catalog},
};

use super::{
    data::{
        add_items_to_directory, copy_items_to_directory, create_directory, cut_items_to_directory,
        hard_delete_items, load_addable_entity_options, load_directory_page, load_tree_rows,
        move_directory_item, rename_directory_item, unlink_items_from_directory,
        unlink_items_from_directory_confirmed,
    },
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum DirectoryClipboardMode {
    Cut,
    Copy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryClipboard {
    mode: DirectoryClipboardMode,
    item_ids: BTreeSet<String>,
    source_directory_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum DirectoryDialog {
    NewDirectory {
        parent: Option<String>,
    },
    AddExisting {
        parent: String,
    },
    Rename {
        item_id: String,
        title: String,
    },
    ConfirmRemove {
        parent: String,
        item_ids: Vec<String>,
        orphaned_directory_ids: Vec<String>,
    },
    ConfirmDelete {
        item_ids: Vec<String>,
    },
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
    ToggleSelection(DirectoryBrowseItem),
    SelectVisible(Vec<String>),
    ClearSelection,
    CutSelected,
    CopySelected,
    PasteInto(String),
    RemoveSelected,
    ConfirmRemove {
        parent: String,
        item_ids: Vec<String>,
    },
    ConfirmDelete(Vec<String>),
    OpenDialog(DirectoryDialog),
    CloseDialog,
    CreateDirectory {
        parent: Option<String>,
        title: String,
    },
    AddExisting {
        parent: String,
        item_id: String,
    },
    RenameItem {
        item_id: String,
        title: String,
    },
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
    selected_items: Signal<BTreeSet<String>>,
    focused_item: Signal<Option<String>>,
    clipboard: Signal<Option<DirectoryClipboard>>,
    pending_dialog: Signal<Option<DirectoryDialog>>,
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
                            state.selected_items.set(BTreeSet::new());
                            state.focused_item.set(None);
                            state.clipboard.set(None);
                            state.pending_dialog.set(None);
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
                            state.selected_items.set(BTreeSet::new());
                            state.focused_item.set(None);
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
                    DirectoryBrowserCommand::ToggleSelection(item) => {
                        let mut next = (state.selected_items)();
                        if !next.insert(item.id.clone()) {
                            next.remove(&item.id);
                        }
                        state.focused_item.set(Some(item.id));
                        state.selected_items.set(next);
                    }
                    DirectoryBrowserCommand::SelectVisible(item_ids) => {
                        state.selected_items.set(item_ids.into_iter().collect());
                    }
                    DirectoryBrowserCommand::ClearSelection => {
                        state.selected_items.set(BTreeSet::new());
                        state.focused_item.set(None);
                    }
                    DirectoryBrowserCommand::CutSelected => {
                        let item_ids = (state.selected_items)();
                        if !item_ids.is_empty() {
                            state.clipboard.set(Some(DirectoryClipboard {
                                mode: DirectoryClipboardMode::Cut,
                                item_ids,
                                source_directory_id: (state.current_root)(),
                            }));
                        }
                    }
                    DirectoryBrowserCommand::CopySelected => {
                        let item_ids = (state.selected_items)();
                        if !item_ids.is_empty() {
                            state.clipboard.set(Some(DirectoryClipboard {
                                mode: DirectoryClipboardMode::Copy,
                                item_ids,
                                source_directory_id: (state.current_root)(),
                            }));
                        }
                    }
                    DirectoryBrowserCommand::PasteInto(target_directory_id) => {
                        let Some(clipboard) = (state.clipboard)() else {
                            continue;
                        };
                        let Some(client) = active_client.clone() else {
                            state
                                .move_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        let item_ids = clipboard.item_ids.iter().cloned().collect::<Vec<_>>();
                        let result = match clipboard.mode {
                            DirectoryClipboardMode::Cut => {
                                cut_items_to_directory(
                                    client,
                                    active_scope_id.clone(),
                                    clipboard.source_directory_id.clone(),
                                    target_directory_id,
                                    item_ids,
                                )
                                .await
                            }
                            DirectoryClipboardMode::Copy => {
                                copy_items_to_directory(
                                    client,
                                    active_scope_id.clone(),
                                    target_directory_id,
                                    item_ids,
                                )
                                .await
                            }
                        };
                        match result {
                            Ok(()) => {
                                state.move_error.set(None);
                                state.selected_items.set(BTreeSet::new());
                                if clipboard.mode == DirectoryClipboardMode::Cut {
                                    state.clipboard.set(None);
                                }
                                reload_after_mutation(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                            }
                            Err(err) => state.move_error.set(Some(err)),
                        }
                    }
                    DirectoryBrowserCommand::RemoveSelected => {
                        let Some(parent) = (state.current_root)() else {
                            state.move_error.set(Some(
                                "Remove is only available inside a directory".to_string(),
                            ));
                            continue;
                        };
                        let item_ids = (state.selected_items)().into_iter().collect::<Vec<_>>();
                        if item_ids.is_empty() {
                            continue;
                        }
                        let Some(client) = active_client.clone() else {
                            state
                                .move_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        match unlink_items_from_directory(
                            client,
                            active_scope_id.clone(),
                            parent.clone(),
                            item_ids.clone(),
                        )
                        .await
                        {
                            Ok(outcome) if outcome.requires_confirmation => {
                                state
                                    .pending_dialog
                                    .set(Some(DirectoryDialog::ConfirmRemove {
                                        parent,
                                        item_ids,
                                        orphaned_directory_ids: outcome.orphaned_directory_ids,
                                    }));
                            }
                            Ok(_) => {
                                state.move_error.set(None);
                                state.selected_items.set(BTreeSet::new());
                                reload_after_mutation(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                            }
                            Err(err) => state.move_error.set(Some(err)),
                        }
                    }
                    DirectoryBrowserCommand::ConfirmRemove { parent, item_ids } => {
                        let Some(client) = active_client.clone() else {
                            state
                                .move_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        match unlink_items_from_directory_confirmed(
                            client,
                            active_scope_id.clone(),
                            parent,
                            item_ids,
                        )
                        .await
                        {
                            Ok(_) => {
                                state.move_error.set(None);
                                state.selected_items.set(BTreeSet::new());
                                state.pending_dialog.set(None);
                                reload_after_mutation(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                            }
                            Err(err) => state.move_error.set(Some(err)),
                        }
                    }
                    DirectoryBrowserCommand::ConfirmDelete(item_ids) => {
                        let Some(client) = active_client.clone() else {
                            state
                                .move_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        match hard_delete_items(client, active_scope_id.clone(), item_ids.clone())
                            .await
                        {
                            Ok(()) => {
                                if let Some(root) = (state.current_root)()
                                    && item_ids.iter().any(|item_id| item_id == &root)
                                {
                                    navigate_to_tree_root(None);
                                }
                                state.move_error.set(None);
                                state.selected_items.set(BTreeSet::new());
                                state.pending_dialog.set(None);
                                reload_after_mutation(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                            }
                            Err(err) => state.move_error.set(Some(err)),
                        }
                    }
                    DirectoryBrowserCommand::OpenDialog(dialog) => {
                        state.pending_dialog.set(Some(dialog));
                    }
                    DirectoryBrowserCommand::CloseDialog => {
                        state.pending_dialog.set(None);
                    }
                    DirectoryBrowserCommand::CreateDirectory { parent, title } => {
                        let Some(client) = active_client.clone() else {
                            state
                                .move_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        match create_directory(client, active_scope_id.clone(), parent, title).await
                        {
                            Ok(_) => {
                                state.move_error.set(None);
                                state.pending_dialog.set(None);
                                reload_after_mutation(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                            }
                            Err(err) => state.move_error.set(Some(err)),
                        }
                    }
                    DirectoryBrowserCommand::AddExisting { parent, item_id } => {
                        let Some(client) = active_client.clone() else {
                            state
                                .move_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        match add_items_to_directory(
                            client,
                            active_scope_id.clone(),
                            parent,
                            vec![item_id],
                        )
                        .await
                        {
                            Ok(()) => {
                                state.move_error.set(None);
                                state.pending_dialog.set(None);
                                reload_after_mutation(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                            }
                            Err(err) => state.move_error.set(Some(err)),
                        }
                    }
                    DirectoryBrowserCommand::RenameItem { item_id, title } => {
                        let Some(client) = active_client.clone() else {
                            state
                                .move_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        match rename_directory_item(client, active_scope_id.clone(), item_id, title)
                            .await
                        {
                            Ok(()) => {
                                state.move_error.set(None);
                                state.pending_dialog.set(None);
                                reload_after_mutation(
                                    active_client.clone(),
                                    active_scope_id.clone(),
                                    state,
                                )
                                .await;
                            }
                            Err(err) => state.move_error.set(Some(err)),
                        }
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
    let selected_items = use_signal(BTreeSet::<String>::new);
    let focused_item = use_signal(|| None::<String>);
    let clipboard = use_signal(|| None::<DirectoryClipboard>);
    let pending_dialog = use_signal(|| None::<DirectoryDialog>);
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
        selected_items,
        focused_item,
        clipboard,
        pending_dialog,
        dragged_item,
        suppress_click,
        move_error,
        content_state,
        tree_state,
    };

    let commands = use_directory_browser_coroutine(state);
    let selected_count = selected_items().len();
    let current_items = match &*content_state.read() {
        LoadState::Ready(page_data) => page_data.items.clone(),
        LoadState::Loading | LoadState::Error(_) => Vec::new(),
    };
    let visible_ids = current_items
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let current_root_title = match &*content_state.read() {
        LoadState::Ready(page_data) => page_data
            .breadcrumbs
            .last()
            .map(|crumb| crumb.title.clone())
            .unwrap_or_else(|| "Tree".to_string()),
        LoadState::Loading | LoadState::Error(_) => "Tree".to_string(),
    };

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
                        title: "New Directory",
                        onclick: {
                            let current_root = current_root();
                            move |_| {
                                commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::NewDirectory {
                                    parent: current_root.clone(),
                                }));
                            }
                        },
                        FolderPlus { size: "1rem" }
                    }
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::IconSm,
                        title: "Add Existing Item",
                        disabled: current_root().is_none(),
                        onclick: {
                            let current_root = current_root();
                            move |_| {
                                if let Some(parent) = current_root.clone() {
                                    commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::AddExisting {
                                        parent,
                                    }));
                                }
                            }
                        },
                        Link { size: "1rem" }
                    }
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::IconSm,
                        title: "Rename current directory",
                        disabled: current_root().is_none(),
                        onclick: {
                            let current_root = current_root();
                            let current_root_title = current_root_title.clone();
                            move |_| {
                                if let Some(item_id) = current_root.clone() {
                                    commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::Rename {
                                        item_id,
                                        title: current_root_title.clone(),
                                    }));
                                }
                            }
                        },
                        Pencil { size: "1rem" }
                    }
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
            div {
                class: "semantic-directory-browser__selection-toolbar",
                "data-active": selected_count > 0 || move_error().is_some(),
                span { class: "semantic-directory-browser__selection-count", "{selected_count} selected" }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    size: dxcomp::ButtonSize::Sm,
                    disabled: selected_count == 0 || current_root().is_none(),
                    onclick: move |_| commands.send(DirectoryBrowserCommand::RemoveSelected),
                    X { size: "1rem" }
                    "Remove"
                }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    size: dxcomp::ButtonSize::Sm,
                    disabled: selected_count == 0,
                    onclick: move |_| commands.send(DirectoryBrowserCommand::CutSelected),
                    Scissors { size: "1rem" }
                    "Cut"
                }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    size: dxcomp::ButtonSize::Sm,
                    disabled: selected_count == 0,
                    onclick: move |_| commands.send(DirectoryBrowserCommand::CopySelected),
                    Copy { size: "1rem" }
                    "Copy"
                }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    size: dxcomp::ButtonSize::Sm,
                    disabled: clipboard().is_none() || current_root().is_none(),
                    onclick: {
                        let current_root = current_root();
                        move |_| {
                            if let Some(parent) = current_root.clone() {
                                commands.send(DirectoryBrowserCommand::PasteInto(parent));
                            }
                        }
                    },
                    ClipboardPaste { size: "1rem" }
                    "Paste"
                }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Destructive,
                    size: dxcomp::ButtonSize::Sm,
                    disabled: selected_count == 0,
                    onclick: {
                        let selected_items = selected_items();
                        move |_| {
                            commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::ConfirmDelete {
                                item_ids: selected_items.iter().cloned().collect(),
                            }));
                        }
                    },
                    Trash2 { size: "1rem" }
                    "Delete"
                }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Ghost,
                    size: dxcomp::ButtonSize::Sm,
                    disabled: selected_count == 0,
                    onclick: move |_| commands.send(DirectoryBrowserCommand::ClearSelection),
                    "Clear"
                }
                if let Some(err) = move_error() {
                    div { class: "semantic-error", "{err}" }
                }
            }
            div {
                class: "semantic-directory-browser__body",
                "data-tree-open": tree_open(),
                tabindex: "0",
                onmouseup: move |_| end_drag(commands),
                onkeydown: {
                    let visible_ids = visible_ids.clone();
                    move |event: KeyboardEvent| {
                        let key = event.key().to_string();
                        let modifiers = event.modifiers();
                        let command = match key.as_str() {
                            "Escape" => Some(DirectoryBrowserCommand::ClearSelection),
                            "a" | "A" if modifiers.ctrl() || modifiers.meta() => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::SelectVisible(visible_ids.clone()))
                            }
                            "Delete" if modifiers.shift() => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::ConfirmDelete {
                                    item_ids: selected_items().iter().cloned().collect(),
                                }))
                            }
                            "Delete" => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::RemoveSelected)
                            }
                            "x" | "X" if modifiers.ctrl() || modifiers.meta() => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::CutSelected)
                            }
                            "c" | "C" if modifiers.ctrl() || modifiers.meta() => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::CopySelected)
                            }
                            "v" | "V" if modifiers.ctrl() || modifiers.meta() => {
                                event.prevent_default();
                                current_root().map(DirectoryBrowserCommand::PasteInto)
                            }
                            "F2" => current_root().map(|item_id| {
                                DirectoryBrowserCommand::OpenDialog(DirectoryDialog::Rename {
                                    item_id,
                                    title: current_root_title.clone(),
                                })
                            }),
                            _ => None,
                        };
                        if let Some(command) = command {
                            commands.send(command);
                        }
                    }
                },
                if tree_open() {
                    aside { class: "semantic-directory-browser__tree",
                        match &*tree_state.read() {
                            LoadState::Ready(rows) => rsx! {
                                if rows.is_empty() {
                                    div { class: "semantic-empty", "No directories" }
                                } else {
                                    for row in rows.iter().cloned() {
                                        {
                                            let row_id = row.item.id.clone();
                                            let row_selected = selected_items().contains(&row_id);
                                            let row_cut = clipboard()
                                                .as_ref()
                                                .is_some_and(|clipboard| clipboard.mode == DirectoryClipboardMode::Cut && clipboard.item_ids.contains(&row_id));
                                            rsx! {
                                        DirectoryTreeRowView {
                                            row,
                                            active_root: current_root(),
                                            selected: row_selected,
                                            cut: row_cut,
                                            expanded: expanded_tree,
                                            commands,
                                        }
                                            }
                                        }
                                    }
                                }
                            },
                            LoadState::Error(err) => rsx! { div { class: "semantic-error", "{err}" } },
                            LoadState::Loading => rsx! { div { class: "semantic-loading", "Loading tree..." } },
                        }
                    }
                }
                dxcomp::ContextMenu {
                    dxcomp::ContextMenuTrigger {
                        div { class: "semantic-directory-browser__content",
                            match &*content_state.read() {
                                LoadState::Ready(page_data) => rsx! {
                                    div { class: "semantic-directory-browser__items",
                                        if page_data.items.is_empty() {
                                            DirectoryEmptyState {
                                                root: current_root(),
                                                clipboard_available: clipboard().is_some(),
                                                commands,
                                            }
                                        } else if view_mode() == BrowseViewMode::List {
                                            DirectoryList {
                                                items: page_data.items.clone(),
                                                selected_items: selected_items(),
                                                cut_items: cut_item_ids(clipboard()),
                                                commands,
                                            }
                                        } else {
                                            DirectoryGrid {
                                                items: page_data.items.clone(),
                                                selected_items: selected_items(),
                                                cut_items: cut_item_ids(clipboard()),
                                                commands,
                                            }
                                        }
                                    },
                                    DirectoryPagination {
                                        page: page(),
                                        has_next: page_data.has_next,
                                        on_previous: move |_| commands.send(DirectoryBrowserCommand::PreviousPage),
                                        on_next: move |_| commands.send(DirectoryBrowserCommand::NextPage),
                                    }
                                },
                                LoadState::Error(err) => rsx! {
                                    div { class: "semantic-directory-browser__items",
                                        div { class: "semantic-error", "{err}" }
                                    }
                                },
                                LoadState::Loading => rsx! {
                                    div { class: "semantic-directory-browser__items",
                                        div { class: "semantic-loading", "Loading directory..." }
                                    }
                                },
                            }
                        }
                    }
                    DirectoryBackgroundContextMenuContent {
                        root: current_root(),
                        clipboard_available: clipboard().is_some(),
                        commands,
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
            DirectoryOperationDialog {
                dialog: pending_dialog(),
                scope_id: scope_id.clone(),
                commands,
            }
        }
    }
}

#[component]
fn DirectoryList(
    items: Vec<DirectoryBrowseItem>,
    selected_items: BTreeSet<String>,
    cut_items: BTreeSet<String>,
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
                        selected: selected_items.contains(&item.id),
                        cut: cut_items.contains(&item.id),
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
    selected_items: BTreeSet<String>,
    cut_items: BTreeSet<String>,
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
                                selected: selected_items.contains(&item.id),
                                cut: cut_items.contains(&item.id),
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
    selected: bool,
    cut: bool,
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
    let click_item = item.clone();
    rsx! {
        dxcomp::ContextMenu {
            dxcomp::ContextMenuTrigger {
        button {
            class: "semantic-directory-browser__row",
            "data-selected": selected,
            "data-cut": cut,
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
            onclick: move |event: MouseEvent| {
                event.stop_propagation();
                if event.modifiers().shift() {
                    event.prevent_default();
                    commands.send(DirectoryBrowserCommand::ToggleSelection(click_item.clone()));
                } else {
                    commands.send(DirectoryBrowserCommand::OpenItem(click_item.clone()));
                }
            },
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
            DirectoryItemContextMenuContent { item, commands }
        }
    }
}

#[component]
fn DirectoryTile(
    item: DirectoryBrowseItem,
    selected: bool,
    cut: bool,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let type_label = item.type_id.clone().unwrap_or_else(|| "entity".to_string());
    let click_item = item.clone();
    rsx! {
        dxcomp::ContextMenu {
            dxcomp::ContextMenuTrigger {
        button {
            class: "semantic-directory-browser__tile",
            "data-selected": selected,
            "data-cut": cut,
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
            onclick: move |event: MouseEvent| {
                event.stop_propagation();
                if event.modifiers().shift() {
                    event.prevent_default();
                    commands.send(DirectoryBrowserCommand::ToggleSelection(click_item.clone()));
                } else {
                    commands.send(DirectoryBrowserCommand::OpenItem(click_item.clone()));
                }
            },
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
            DirectoryItemContextMenuContent { item, commands }
        }
    }
}

#[component]
fn DirectoryTreeRowView(
    row: DirectoryTreeRow,
    active_root: Option<String>,
    selected: bool,
    cut: bool,
    expanded: Signal<BTreeSet<String>>,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let item = row.item.clone();
    let click_item = item.clone();
    let expander_item_id = item.id.clone();
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
                    let item_id = expander_item_id.clone();
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
            dxcomp::ContextMenu {
                dxcomp::ContextMenuTrigger {
                    button {
                        class: "semantic-directory-browser__tree-link",
                        "data-selected": selected,
                        "data-cut": cut,
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
                        onclick: move |event: MouseEvent| {
                            event.stop_propagation();
                            if event.modifiers().shift() {
                                event.prevent_default();
                                commands.send(DirectoryBrowserCommand::ToggleSelection(click_item.clone()));
                            } else {
                                commands.send(DirectoryBrowserCommand::OpenTreeRoot(click_item.id.clone()));
                            }
                        },
                        Folder { size: "1rem" }
                        span { "{item.title}" }
                    }
                }
                DirectoryItemContextMenuContent { item, commands }
            }
        }
    }
}

#[component]
fn DirectoryItemContextMenuContent(
    item: DirectoryBrowseItem,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let item_id = item.id.clone();
    let item_title = item.title.clone();
    let open_item = item.clone();
    let is_directory = item.is_directory;
    rsx! {
        dxcomp::ContextMenuContent {
            dxcomp::ContextMenuItem {
                value: "open".to_string(),
                index: 0usize,
                on_select: move |_| commands.send(DirectoryBrowserCommand::OpenItem(open_item.clone())),
                "Open"
            }
            dxcomp::ContextMenuItem {
                value: "rename".to_string(),
                index: 1usize,
                on_select: {
                    let item_id = item_id.clone();
                    let item_title = item_title.clone();
                    move |_| commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::Rename {
                        item_id: item_id.clone(),
                        title: item_title.clone(),
                    }))
                },
                "Rename"
            }
            dxcomp::ContextMenuItem {
                value: "cut".to_string(),
                index: 2usize,
                on_select: {
                    let item_id = item_id.clone();
                    move |_| {
                        commands.send(DirectoryBrowserCommand::SelectVisible(vec![item_id.clone()]));
                        commands.send(DirectoryBrowserCommand::CutSelected);
                    }
                },
                "Cut"
            }
            dxcomp::ContextMenuItem {
                value: "copy".to_string(),
                index: 3usize,
                on_select: {
                    let item_id = item_id.clone();
                    move |_| {
                        commands.send(DirectoryBrowserCommand::SelectVisible(vec![item_id.clone()]));
                        commands.send(DirectoryBrowserCommand::CopySelected);
                    }
                },
                "Copy"
            }
            if is_directory {
                dxcomp::ContextMenuItem {
                    value: "new-child".to_string(),
                    index: 4usize,
                    on_select: {
                        let item_id = item_id.clone();
                        move |_| commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::NewDirectory {
                            parent: Some(item_id.clone()),
                        }))
                    },
                    "New Child Directory"
                }
                dxcomp::ContextMenuItem {
                    value: "paste-into".to_string(),
                    index: 5usize,
                    on_select: {
                        let item_id = item_id.clone();
                        move |_| commands.send(DirectoryBrowserCommand::PasteInto(item_id.clone()))
                    },
                    "Paste Into Directory"
                }
                dxcomp::ContextMenuItem {
                    value: "add-existing".to_string(),
                    index: 6usize,
                    on_select: {
                        let item_id = item_id.clone();
                        move |_| commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::AddExisting {
                            parent: item_id.clone(),
                        }))
                    },
                    "Add Existing Item"
                }
            }
            dxcomp::ContextMenuItem {
                value: "remove".to_string(),
                index: 7usize,
                on_select: {
                    let item_id = item_id.clone();
                    move |_| {
                        commands.send(DirectoryBrowserCommand::SelectVisible(vec![item_id.clone()]));
                        commands.send(DirectoryBrowserCommand::RemoveSelected);
                    }
                },
                "Remove"
            }
            dxcomp::ContextMenuItem {
                value: "delete".to_string(),
                index: 8usize,
                on_select: {
                    let item_id = item_id.clone();
                    move |_| commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::ConfirmDelete {
                        item_ids: vec![item_id.clone()],
                    }))
                },
                "Delete"
            }
            dxcomp::ContextMenuItem {
                value: "refresh".to_string(),
                index: 9usize,
                on_select: move |_| commands.send(DirectoryBrowserCommand::Refresh),
                "Refresh"
            }
        }
    }
}

#[component]
fn DirectoryBackgroundContextMenuContent(
    root: Option<String>,
    clipboard_available: bool,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    rsx! {
        dxcomp::ContextMenuContent {
            dxcomp::ContextMenuItem {
                value: "new-directory".to_string(),
                index: 0usize,
                on_select: {
                    let root = root.clone();
                    move |_| commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::NewDirectory {
                        parent: root.clone(),
                    }))
                },
                "New Directory"
            }
            if let Some(parent) = root.clone() {
                dxcomp::ContextMenuItem {
                    value: "paste".to_string(),
                    index: 1usize,
                    disabled: !clipboard_available,
                    on_select: {
                        let parent = parent.clone();
                        move |_| commands.send(DirectoryBrowserCommand::PasteInto(parent.clone()))
                    },
                    "Paste"
                }
                dxcomp::ContextMenuItem {
                    value: "add-existing".to_string(),
                    index: 2usize,
                    on_select: {
                        let parent = parent.clone();
                        move |_| commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::AddExisting {
                            parent: parent.clone(),
                        }))
                    },
                    "Add Existing Item"
                }
            }
            dxcomp::ContextMenuItem {
                value: "refresh".to_string(),
                index: 3usize,
                on_select: move |_| commands.send(DirectoryBrowserCommand::Refresh),
                "Refresh"
            }
        }
    }
}

#[component]
fn DirectoryEmptyState(
    root: Option<String>,
    clipboard_available: bool,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let title = if root.is_some() {
        "No items"
    } else {
        "No directories"
    };
    rsx! {
        div { class: "semantic-empty semantic-directory-browser__empty-actions",
            span { "{title}" }
            dxcomp::Button {
                variant: dxcomp::ButtonVariant::Outline,
                size: dxcomp::ButtonSize::Sm,
                onclick: {
                    let root = root.clone();
                    move |_| commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::NewDirectory {
                        parent: root.clone(),
                    }))
                },
                Plus { size: "1rem" }
                "New Directory"
            }
            if let Some(parent) = root.clone() {
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    size: dxcomp::ButtonSize::Sm,
                    onclick: {
                        let parent = parent.clone();
                        move |_| commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::AddExisting {
                            parent: parent.clone(),
                        }))
                    },
                    Link { size: "1rem" }
                    "Add Existing"
                }
                if clipboard_available {
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::Sm,
                        onclick: {
                            let parent = parent.clone();
                            move |_| commands.send(DirectoryBrowserCommand::PasteInto(parent.clone()))
                        },
                        ClipboardPaste { size: "1rem" }
                        "Paste"
                    }
                }
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

#[component]
fn DirectoryOperationDialog(
    dialog: Option<DirectoryDialog>,
    scope_id: Option<String>,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let client = use_rpc_client();
    let mut title = use_signal(String::new);
    let mut add_query = use_signal(String::new);
    let mut add_selected = use_signal(|| None::<String>);
    let add_parent = match dialog.as_ref() {
        Some(DirectoryDialog::AddExisting { parent }) => Some(parent.clone()),
        _ => None,
    };
    let options = use_resource(move || {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let parent = add_parent.clone();
        let search = add_query();
        async move {
            dioxus_sdk_time::sleep(Duration::from_millis(250)).await;
            let Some(parent) = parent else {
                return Vec::new();
            };
            load_addable_entity_options(client, scope_id, parent, search, 50)
                .await
                .unwrap_or_default()
        }
    });
    let options = options.read().clone().unwrap_or_default();
    let open = dialog.is_some();
    rsx! {
        dxcomp::Dialog {
            open,
            on_open_change: move |open: bool| {
                if !open {
                    commands.send(DirectoryBrowserCommand::CloseDialog);
                }
            },
            match dialog {
                Some(DirectoryDialog::NewDirectory { parent }) => rsx! {
                    dxcomp::DialogTitle { "New Directory" }
                    label { class: "semantic-directory-browser__dialog-field",
                        span { "Title" }
                        input {
                            value: "{title()}",
                            oninput: move |event| title.set(event.value()),
                        }
                    }
                    div { class: "semantic-directory-browser__dialog-actions",
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::CloseDialog),
                            "Cancel"
                        }
                        dxcomp::Button {
                            disabled: title().trim().is_empty(),
                            onclick: move |_| commands.send(DirectoryBrowserCommand::CreateDirectory {
                                parent: parent.clone(),
                                title: title(),
                            }),
                            "Create"
                        }
                    }
                },
                Some(DirectoryDialog::AddExisting { parent }) => rsx! {
                    dxcomp::DialogTitle { "Add Existing Item" }
                    dxcomp::Combobox::<String> {
                        default_value: add_selected(),
                        on_value_change: move |value| add_selected.set(value),
                        on_query_change: move |value| add_query.set(value),
                        placeholder: "Search entities",
                        aria_label: "Existing entity",
                        list_aria_label: "Existing entities",
                        dxcomp::ComboboxEmpty { "No entity found." }
                        for (index, option) in options.iter().enumerate() {
                            dxcomp::ComboboxOption::<String> {
                                index,
                                value: option.id.clone(),
                                text_value: format!("{} {}", option.title, option.id),
                                span { "{option.title}" }
                                code { " {option.id}" }
                            }
                        }
                    }
                    div { class: "semantic-directory-browser__dialog-actions",
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::CloseDialog),
                            "Cancel"
                        }
                        dxcomp::Button {
                            disabled: add_selected().is_none(),
                            onclick: move |_| {
                                if let Some(item_id) = add_selected() {
                                    commands.send(DirectoryBrowserCommand::AddExisting {
                                        parent: parent.clone(),
                                        item_id,
                                    });
                                }
                            },
                            "Add"
                        }
                    }
                },
                Some(DirectoryDialog::Rename { item_id, title: initial_title }) => {
                    if title().is_empty() {
                        title.set(initial_title.clone());
                    }
                    rsx! {
                        dxcomp::DialogTitle { "Rename" }
                        label { class: "semantic-directory-browser__dialog-field",
                            span { "Title" }
                            input {
                                value: "{title()}",
                                oninput: move |event| title.set(event.value()),
                            }
                        }
                        div { class: "semantic-directory-browser__dialog-actions",
                            dxcomp::Button {
                                variant: dxcomp::ButtonVariant::Outline,
                                onclick: move |_| commands.send(DirectoryBrowserCommand::CloseDialog),
                                "Cancel"
                            }
                            dxcomp::Button {
                                disabled: title().trim().is_empty(),
                                onclick: move |_| commands.send(DirectoryBrowserCommand::RenameItem {
                                    item_id: item_id.clone(),
                                    title: title(),
                                }),
                                "Rename"
                            }
                        }
                    }
                },
                Some(DirectoryDialog::ConfirmRemove { parent, item_ids, orphaned_directory_ids }) => rsx! {
                    dxcomp::DialogTitle { "Remove Items" }
                    dxcomp::DialogDescription {
                        "Removing this selection will unlink it from the current directory. {orphaned_directory_ids.len()} directory item(s) have no other parent and will be deleted, but descendant entities are not recursively deleted."
                    }
                    div { class: "semantic-directory-browser__dialog-actions",
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::CloseDialog),
                            "Cancel"
                        }
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Destructive,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::ConfirmRemove {
                                parent: parent.clone(),
                                item_ids: item_ids.clone(),
                            }),
                            "Remove"
                        }
                    }
                },
                Some(DirectoryDialog::ConfirmDelete { item_ids }) => rsx! {
                    dxcomp::DialogTitle { "Delete Items" }
                    dxcomp::DialogDescription {
                        "This deletes the selected entities. Directory child entities are not recursively deleted, but directory links owned by deleted directories are removed."
                    }
                    div { class: "semantic-directory-browser__dialog-actions",
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::CloseDialog),
                            "Cancel"
                        }
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Destructive,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::ConfirmDelete(item_ids.clone())),
                            "Delete"
                        }
                    }
                },
                None => rsx! {},
            }
        }
    }
}

fn cut_item_ids(clipboard: Option<DirectoryClipboard>) -> BTreeSet<String> {
    clipboard
        .filter(|clipboard| clipboard.mode == DirectoryClipboardMode::Cut)
        .map(|clipboard| clipboard.item_ids)
        .unwrap_or_default()
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

async fn reload_after_mutation(
    client: Option<semantic_rpc::RpcClient>,
    scope_id: Option<String>,
    state: DirectoryBrowserSignals,
) {
    reload_current_content(client.clone(), scope_id.clone(), state).await;
    if (state.tree_open)() {
        reload_current_tree(client, scope_id, state).await;
    }
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
