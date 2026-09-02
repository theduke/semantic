use std::{collections::BTreeSet, time::Duration};

use dioxus::logger::tracing::{error, info, warn};
use dioxus::prelude::*;
use dioxus_icons::lucide::{
    ChevronDown, ChevronRight, ClipboardPaste, Copy, FileText, Folder, FolderPlus, Grid2x2, Link,
    List, PanelLeft, Pencil, Plus, RefreshCw, Scissors, Trash2, Upload, X,
};
use futures::StreamExt;

use crate::{
    components::{EntityCard, EntityDisplayRenderer, EntityRenderOptions},
    context::{use_active_scope_id, use_rpc_client},
};

use super::{
    data::{
        add_items_to_directory, copy_items_to_directory, create_directory, cut_items_to_directory,
        hard_delete_items, load_addable_entity_options, load_directory_page, load_tree_rows,
        move_directory_item, rename_directory_item, unlink_items_from_directory,
        unlink_items_from_directory_confirmed,
    },
    picker::{FileTreePicker, FileTreeSelection},
    types::{
        BrowseViewMode, DirectoryActionTarget, DirectoryBrowseItem, DirectoryBrowserProps,
        DirectoryPage, DirectorySort, DirectoryTreeRow,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum DirectoryOperation {
    Move,
    Paste,
    Remove,
    Delete,
    Create,
    AddExisting,
    Rename,
    MoveTo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusMove {
    Previous,
    Next,
    First,
    Last,
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
    MoveTo {
        source: Option<String>,
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
    SelectRange(DirectoryBrowseItem),
    MoveFocus {
        direction: FocusMove,
        extend_selection: bool,
    },
    ToggleFocused,
    OpenFocused,
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
    MoveItems {
        source: Option<String>,
        target: String,
        item_ids: Vec<String>,
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
    selection_anchor: Signal<Option<String>>,
    clipboard: Signal<Option<DirectoryClipboard>>,
    pending_dialog: Signal<Option<DirectoryDialog>>,
    dragged_item: Signal<Option<String>>,
    suppress_click: Signal<bool>,
    move_error: Signal<Option<String>>,
    dialog_error: Signal<Option<String>>,
    busy_operations: Signal<BTreeSet<DirectoryOperation>>,
    content_generation: Signal<u64>,
    tree_generation: Signal<u64>,
    dialog_generation: Signal<u64>,
    content_loading: Signal<bool>,
    tree_loading: Signal<bool>,
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
                            state.selection_anchor.set(None);
                            state.clipboard.set(None);
                            state.pending_dialog.set(None);
                            state.move_error.set(None);
                            state.dialog_error.set(None);
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
                            state.selection_anchor.set(None);
                        }
                        if root_changed || scope_changed {
                            state
                                .dialog_generation
                                .set((state.dialog_generation)().saturating_add(1));
                            state.pending_dialog.set(None);
                            state.dialog_error.set(None);
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
                        if operation_busy(state.busy_operations, DirectoryOperation::Move) {
                            state
                                .move_error
                                .set(Some("Another move is still in progress".to_string()));
                            continue;
                        }
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
                        set_operation_busy(state.busy_operations, DirectoryOperation::Move, true);
                        let scope_id = active_scope_id.clone();
                        let reload_client = client.clone();
                        spawn(async move {
                            match move_directory_item(
                                client,
                                scope_id.clone(),
                                item_id,
                                target_directory_id,
                            )
                            .await
                            {
                                Ok(()) => {
                                    info!("directory browser move completed");
                                    state.move_error.set(None);
                                    reload_after_mutation(Some(reload_client), scope_id, state)
                                        .await;
                                }
                                Err(err) => {
                                    error!(error = err.as_str(), "directory browser move failed");
                                    state.move_error.set(Some(err));
                                }
                            }
                            set_operation_busy(
                                state.busy_operations,
                                DirectoryOperation::Move,
                                false,
                            );
                        });
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
                        let next = expanded_for_open((state.expanded_tree)(), &item_id);
                        if next != (state.expanded_tree)() {
                            state.expanded_tree.set(next);
                        }
                        navigate_to_tree_root(Some(item_id));
                    }
                    DirectoryBrowserCommand::ToggleSelection(item) => {
                        let mut next = (state.selected_items)();
                        if !next.insert(item.id.clone()) {
                            next.remove(&item.id);
                        }
                        state.selection_anchor.set(Some(item.id.clone()));
                        state.focused_item.set(Some(item.id));
                        state.selected_items.set(next);
                    }
                    DirectoryBrowserCommand::SelectRange(item) => {
                        let ids = visible_item_ids(state.content_state);
                        let anchor = (state.selection_anchor)()
                            .or_else(|| (state.focused_item)())
                            .unwrap_or_else(|| item.id.clone());
                        state
                            .selected_items
                            .set(selection_range(&ids, &anchor, &item.id));
                        state.focused_item.set(Some(item.id));
                    }
                    DirectoryBrowserCommand::MoveFocus {
                        direction,
                        extend_selection,
                    } => {
                        let ids = visible_item_ids(state.content_state);
                        if let Some(next_id) =
                            moved_focus(&ids, (state.focused_item)().as_deref(), direction)
                        {
                            if (state.selection_anchor)().is_none() {
                                state.selection_anchor.set(Some(next_id.clone()));
                            }
                            if extend_selection {
                                let anchor =
                                    (state.selection_anchor)().unwrap_or_else(|| next_id.clone());
                                state
                                    .selected_items
                                    .set(selection_range(&ids, &anchor, &next_id));
                            }
                            state.focused_item.set(Some(next_id));
                        }
                    }
                    DirectoryBrowserCommand::ToggleFocused => {
                        if let Some(id) = (state.focused_item)()
                            && let Some(item) = visible_item(state.content_state, &id)
                        {
                            let mut next = (state.selected_items)();
                            if !next.insert(item.id.clone()) {
                                next.remove(&item.id);
                            }
                            state.selection_anchor.set(Some(item.id));
                            state.selected_items.set(next);
                        }
                    }
                    DirectoryBrowserCommand::OpenFocused => {
                        if let Some(id) = (state.focused_item)()
                            && let Some(item) = visible_item(state.content_state, &id)
                        {
                            if item.is_directory {
                                navigate_to_tree_root(Some(item.id));
                            } else {
                                state.selected_item.set(Some(item));
                            }
                        }
                    }
                    DirectoryBrowserCommand::SelectVisible(item_ids) => {
                        state.selection_anchor.set(item_ids.first().cloned());
                        state.focused_item.set(item_ids.first().cloned());
                        state.selected_items.set(item_ids.into_iter().collect());
                    }
                    DirectoryBrowserCommand::ClearSelection => {
                        state.selected_items.set(BTreeSet::new());
                        state.selection_anchor.set(None);
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
                        if operation_busy(state.busy_operations, DirectoryOperation::Paste) {
                            continue;
                        }
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
                        set_operation_busy(state.busy_operations, DirectoryOperation::Paste, true);
                        let scope_id = active_scope_id.clone();
                        let reload_client = client.clone();
                        spawn(async move {
                            let result = match clipboard.mode {
                                DirectoryClipboardMode::Cut => {
                                    cut_items_to_directory(
                                        client,
                                        scope_id.clone(),
                                        clipboard.source_directory_id.clone(),
                                        target_directory_id,
                                        item_ids,
                                    )
                                    .await
                                }
                                DirectoryClipboardMode::Copy => {
                                    copy_items_to_directory(
                                        client,
                                        scope_id.clone(),
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
                                    reload_after_mutation(Some(reload_client), scope_id, state)
                                        .await;
                                }
                                Err(err) => state.move_error.set(Some(err)),
                            }
                            set_operation_busy(
                                state.busy_operations,
                                DirectoryOperation::Paste,
                                false,
                            );
                        });
                    }
                    DirectoryBrowserCommand::RemoveSelected => {
                        if operation_busy(state.busy_operations, DirectoryOperation::Remove) {
                            continue;
                        }
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
                        set_operation_busy(state.busy_operations, DirectoryOperation::Remove, true);
                        let scope_id = active_scope_id.clone();
                        let reload_client = client.clone();
                        spawn(async move {
                            match unlink_items_from_directory(
                                client,
                                scope_id.clone(),
                                parent.clone(),
                                item_ids.clone(),
                            )
                            .await
                            {
                                Ok(outcome) if outcome.requires_confirmation => {
                                    state.pending_dialog.set(Some(
                                        DirectoryDialog::ConfirmRemove {
                                            parent,
                                            item_ids,
                                            orphaned_directory_ids: outcome.orphaned_directory_ids,
                                        },
                                    ));
                                }
                                Ok(_) => {
                                    state.move_error.set(None);
                                    state.selected_items.set(BTreeSet::new());
                                    reload_after_mutation(Some(reload_client), scope_id, state)
                                        .await;
                                }
                                Err(err) => state.move_error.set(Some(err)),
                            }
                            set_operation_busy(
                                state.busy_operations,
                                DirectoryOperation::Remove,
                                false,
                            );
                        });
                    }
                    DirectoryBrowserCommand::ConfirmRemove { parent, item_ids } => {
                        if operation_busy(state.busy_operations, DirectoryOperation::Remove) {
                            continue;
                        }
                        let Some(client) = active_client.clone() else {
                            state
                                .dialog_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        state.dialog_error.set(None);
                        let dialog_generation = (state.dialog_generation)();
                        set_operation_busy(state.busy_operations, DirectoryOperation::Remove, true);
                        let scope_id = active_scope_id.clone();
                        let reload_client = client.clone();
                        spawn(async move {
                            match unlink_items_from_directory_confirmed(
                                client,
                                scope_id.clone(),
                                parent,
                                item_ids,
                            )
                            .await
                            {
                                Ok(_) => {
                                    state.selected_items.set(BTreeSet::new());
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.pending_dialog.set(None);
                                    }
                                    reload_after_mutation(Some(reload_client), scope_id, state)
                                        .await;
                                }
                                Err(err) => {
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.dialog_error.set(Some(err));
                                    }
                                }
                            }
                            set_operation_busy(
                                state.busy_operations,
                                DirectoryOperation::Remove,
                                false,
                            );
                        });
                    }
                    DirectoryBrowserCommand::ConfirmDelete(item_ids) => {
                        if operation_busy(state.busy_operations, DirectoryOperation::Delete) {
                            continue;
                        }
                        let Some(client) = active_client.clone() else {
                            state
                                .dialog_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        state.dialog_error.set(None);
                        let dialog_generation = (state.dialog_generation)();
                        set_operation_busy(state.busy_operations, DirectoryOperation::Delete, true);
                        let scope_id = active_scope_id.clone();
                        let reload_client = client.clone();
                        spawn(async move {
                            match hard_delete_items(client, scope_id.clone(), item_ids.clone())
                                .await
                            {
                                Ok(()) => {
                                    if let Some(root) = (state.current_root)()
                                        && item_ids.iter().any(|item_id| item_id == &root)
                                    {
                                        navigate_to_tree_root(None);
                                    }
                                    state.selected_items.set(BTreeSet::new());
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.pending_dialog.set(None);
                                    }
                                    reload_after_mutation(Some(reload_client), scope_id, state)
                                        .await;
                                }
                                Err(err) => {
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.dialog_error.set(Some(err));
                                    }
                                }
                            }
                            set_operation_busy(
                                state.busy_operations,
                                DirectoryOperation::Delete,
                                false,
                            );
                        });
                    }
                    DirectoryBrowserCommand::OpenDialog(dialog) => {
                        state
                            .dialog_generation
                            .set((state.dialog_generation)().saturating_add(1));
                        state.dialog_error.set(None);
                        state.pending_dialog.set(Some(dialog));
                    }
                    DirectoryBrowserCommand::CloseDialog => {
                        state
                            .dialog_generation
                            .set((state.dialog_generation)().saturating_add(1));
                        state.dialog_error.set(None);
                        state.pending_dialog.set(None);
                    }
                    DirectoryBrowserCommand::CreateDirectory { parent, title } => {
                        if operation_busy(state.busy_operations, DirectoryOperation::Create) {
                            continue;
                        }
                        let Some(client) = active_client.clone() else {
                            state
                                .dialog_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        state.dialog_error.set(None);
                        let dialog_generation = (state.dialog_generation)();
                        set_operation_busy(state.busy_operations, DirectoryOperation::Create, true);
                        let scope_id = active_scope_id.clone();
                        let reload_client = client.clone();
                        spawn(async move {
                            match create_directory(client, scope_id.clone(), parent, title).await {
                                Ok(_) => {
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.pending_dialog.set(None);
                                    }
                                    reload_after_mutation(Some(reload_client), scope_id, state)
                                        .await;
                                }
                                Err(err) => {
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.dialog_error.set(Some(err));
                                    }
                                }
                            }
                            set_operation_busy(
                                state.busy_operations,
                                DirectoryOperation::Create,
                                false,
                            );
                        });
                    }
                    DirectoryBrowserCommand::AddExisting { parent, item_id } => {
                        if operation_busy(state.busy_operations, DirectoryOperation::AddExisting) {
                            continue;
                        }
                        let Some(client) = active_client.clone() else {
                            state
                                .dialog_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        state.dialog_error.set(None);
                        let dialog_generation = (state.dialog_generation)();
                        set_operation_busy(
                            state.busy_operations,
                            DirectoryOperation::AddExisting,
                            true,
                        );
                        let scope_id = active_scope_id.clone();
                        let reload_client = client.clone();
                        spawn(async move {
                            match add_items_to_directory(
                                client,
                                scope_id.clone(),
                                parent,
                                vec![item_id],
                            )
                            .await
                            {
                                Ok(()) => {
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.pending_dialog.set(None);
                                    }
                                    reload_after_mutation(Some(reload_client), scope_id, state)
                                        .await;
                                }
                                Err(err) => {
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.dialog_error.set(Some(err));
                                    }
                                }
                            }
                            set_operation_busy(
                                state.busy_operations,
                                DirectoryOperation::AddExisting,
                                false,
                            );
                        });
                    }
                    DirectoryBrowserCommand::RenameItem { item_id, title } => {
                        if operation_busy(state.busy_operations, DirectoryOperation::Rename) {
                            continue;
                        }
                        let Some(client) = active_client.clone() else {
                            state
                                .dialog_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        state.dialog_error.set(None);
                        let dialog_generation = (state.dialog_generation)();
                        set_operation_busy(state.busy_operations, DirectoryOperation::Rename, true);
                        let scope_id = active_scope_id.clone();
                        let reload_client = client.clone();
                        spawn(async move {
                            match rename_directory_item(client, scope_id.clone(), item_id, title)
                                .await
                            {
                                Ok(()) => {
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.pending_dialog.set(None);
                                    }
                                    reload_after_mutation(Some(reload_client), scope_id, state)
                                        .await;
                                }
                                Err(err) => {
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.dialog_error.set(Some(err));
                                    }
                                }
                            }
                            set_operation_busy(
                                state.busy_operations,
                                DirectoryOperation::Rename,
                                false,
                            );
                        });
                    }
                    DirectoryBrowserCommand::MoveItems {
                        source,
                        target,
                        item_ids,
                    } => {
                        if operation_busy(state.busy_operations, DirectoryOperation::MoveTo) {
                            continue;
                        }
                        let Some(client) = active_client.clone() else {
                            state
                                .dialog_error
                                .set(Some("No active RPC client".to_string()));
                            continue;
                        };
                        state.dialog_error.set(None);
                        let dialog_generation = (state.dialog_generation)();
                        set_operation_busy(state.busy_operations, DirectoryOperation::MoveTo, true);
                        let scope_id = active_scope_id.clone();
                        let reload_client = client.clone();
                        spawn(async move {
                            match cut_items_to_directory(
                                client,
                                scope_id.clone(),
                                source,
                                target,
                                item_ids,
                            )
                            .await
                            {
                                Ok(()) => {
                                    state.selected_items.set(BTreeSet::new());
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.pending_dialog.set(None);
                                    }
                                    reload_after_mutation(Some(reload_client), scope_id, state)
                                        .await;
                                }
                                Err(err) => {
                                    if (state.dialog_generation)() == dialog_generation {
                                        state.dialog_error.set(Some(err));
                                    }
                                }
                            }
                            set_operation_busy(
                                state.busy_operations,
                                DirectoryOperation::MoveTo,
                                false,
                            );
                        });
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
    let selection_anchor = use_signal(|| None::<String>);
    let clipboard = use_signal(|| None::<DirectoryClipboard>);
    let pending_dialog = use_signal(|| None::<DirectoryDialog>);
    let dragged_item = use_signal(|| None::<String>);
    let suppress_click = use_signal(|| false);
    let move_error = use_signal(|| None::<String>);
    let dialog_error = use_signal(|| None::<String>);
    let busy_operations = use_signal(BTreeSet::<DirectoryOperation>::new);
    let content_generation = use_signal(|| 0_u64);
    let tree_generation = use_signal(|| 0_u64);
    let dialog_generation = use_signal(|| 0_u64);
    let content_loading = use_signal(|| true);
    let tree_loading = use_signal(|| true);
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
        selection_anchor,
        clipboard,
        pending_dialog,
        dragged_item,
        suppress_click,
        move_error,
        dialog_error,
        busy_operations,
        content_generation,
        tree_generation,
        dialog_generation,
        content_loading,
        tree_loading,
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
    let busy = busy_operations();
    let content_busy = content_loading();
    let current_target = match (&*content_state.read(), current_root(), content_busy) {
        (LoadState::Ready(_), Some(id), false) => Some(DirectoryActionTarget {
            id,
            title: current_root_title.clone(),
        }),
        (LoadState::Loading | LoadState::Error(_), _, _)
        | (LoadState::Ready(_), None, _)
        | (LoadState::Ready(_), Some(_), true) => None,
    };
    let focused_dom_id = focused_item()
        .filter(|id| visible_ids.iter().any(|visible_id| visible_id == id))
        .map(|id| directory_item_dom_id(&id));

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

    let refresh_revision = props.refresh_revision;
    let mut observed_refresh_revision = use_signal(|| refresh_revision);
    use_effect(use_reactive((&refresh_revision,), move |(revision,)| {
        if observed_refresh_revision() != revision {
            observed_refresh_revision.set(revision);
            commands.send(DirectoryBrowserCommand::Refresh);
        }
    }));

    rsx! {
        section { class: "semantic-directory-browser", aria_label: "Directory browser",
            div { class: "semantic-directory-browser__toolbar",
                nav { class: "semantic-directory-browser__breadcrumbs", aria_label: "Current directory",
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
                div { class: "semantic-directory-browser__command-row",
                div { class: "semantic-directory-browser__toolbar-controls", role: "group", aria_label: "View controls",
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::IconSm,
                        title: "Toggle tree",
                        aria_label: if tree_open() { "Hide directory tree" } else { "Show directory tree" },
                        aria_pressed: tree_open(),
                        aria_expanded: tree_open(),
                        aria_controls: "semantic-directory-tree",
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
                    div { class: "semantic-directory-browser__view-toggle", role: "group", aria_label: "Directory view",
                        dxcomp::Button {
                            variant: if view_mode() == BrowseViewMode::List { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                            size: dxcomp::ButtonSize::IconSm,
                            title: "List view",
                            aria_label: "List view",
                            aria_pressed: view_mode() == BrowseViewMode::List,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::SetViewMode(BrowseViewMode::List)),
                            List { size: "1rem" }
                        }
                        dxcomp::Button {
                            variant: if view_mode() == BrowseViewMode::Icons { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                            size: dxcomp::ButtonSize::IconSm,
                            title: "Icon view",
                            aria_label: "Icon view",
                            aria_pressed: view_mode() == BrowseViewMode::Icons,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::SetViewMode(BrowseViewMode::Icons)),
                            Grid2x2 { size: "1rem" }
                        }
                    }
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::IconSm,
                        title: "Refresh",
                        aria_label: "Refresh directory",
                        disabled: content_busy,
                        onclick: move |_| commands.send(DirectoryBrowserCommand::Refresh),
                        RefreshCw { size: "1rem" }
                    }
                }
                div { class: "semantic-directory-browser__current-actions", role: "group", aria_label: "Current directory actions",
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::IconSm,
                        title: "New Directory",
                        aria_label: "New directory",
                        disabled: busy.contains(&DirectoryOperation::Create),
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
                        aria_label: "Add existing item",
                        disabled: current_target.is_none() || busy.contains(&DirectoryOperation::AddExisting),
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
                        aria_label: "Rename current directory",
                        disabled: current_target.is_none() || busy.contains(&DirectoryOperation::Rename),
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
                    if let Some(on_create_entity) = props.on_create_entity {
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            size: dxcomp::ButtonSize::Sm,
                            disabled: current_target.is_none() || content_busy,
                            onclick: {
                                let target = current_target.clone();
                                move |_| if let Some(target) = target.clone() { on_create_entity.call(target) }
                            },
                            Plus { size: "1rem" }
                            "Create"
                        }
                    }
                    if let Some(on_upload_files) = props.on_upload_files {
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            size: dxcomp::ButtonSize::Sm,
                            disabled: current_target.is_none() || content_busy,
                            onclick: {
                                let target = current_target.clone();
                                move |_| if let Some(target) = target.clone() { on_upload_files.call(target) }
                            },
                            Upload { size: "1rem" }
                            "Upload"
                        }
                    }
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
                disabled: selected_count == 0 || current_root().is_none() || busy.contains(&DirectoryOperation::Remove),
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
                    disabled: clipboard().is_none() || current_root().is_none() || busy.contains(&DirectoryOperation::Paste),
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
                    variant: dxcomp::ButtonVariant::Outline,
                    size: dxcomp::ButtonSize::Sm,
                    disabled: selected_count == 0 || busy.contains(&DirectoryOperation::MoveTo),
                    onclick: {
                        let source = current_root();
                        let item_ids = selected_items().into_iter().collect::<Vec<_>>();
                        move |_| commands.send(DirectoryBrowserCommand::OpenDialog(DirectoryDialog::MoveTo {
                            source: source.clone(),
                            item_ids: item_ids.clone(),
                        }))
                    },
                    Folder { size: "1rem" }
                    "Move to…"
                }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Destructive,
                    size: dxcomp::ButtonSize::Sm,
                    disabled: selected_count == 0 || busy.contains(&DirectoryOperation::Delete),
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
                role: "region",
                aria_label: "Directory workspace",
                aria_activedescendant: focused_dom_id,
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
                            "ArrowUp" => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::MoveFocus {
                                    direction: FocusMove::Previous,
                                    extend_selection: modifiers.shift(),
                                })
                            }
                            "ArrowDown" => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::MoveFocus {
                                    direction: FocusMove::Next,
                                    extend_selection: modifiers.shift(),
                                })
                            }
                            "Home" => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::MoveFocus {
                                    direction: FocusMove::First,
                                    extend_selection: modifiers.shift(),
                                })
                            }
                            "End" => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::MoveFocus {
                                    direction: FocusMove::Last,
                                    extend_selection: modifiers.shift(),
                                })
                            }
                            " " => {
                                event.prevent_default();
                                Some(DirectoryBrowserCommand::ToggleFocused)
                            }
                            "Enter" => Some(DirectoryBrowserCommand::OpenFocused),
                            _ => None,
                        };
                        if let Some(command) = command {
                            commands.send(command);
                        }
                    }
                },
                if tree_open() {
                    aside { id: "semantic-directory-tree", class: "semantic-directory-browser__tree", role: "tree", aria_label: "Directory hierarchy", aria_busy: tree_loading(),
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
                                            key: "{row.item.id}-{row.depth}",
                                            row,
                                            active_root: current_root(),
                                            selected: row_selected,
                                            cut: row_cut,
                                            dragged_item: dragged_item(),
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
                    class: "semantic-directory-browser__content-menu",
                    dxcomp::ContextMenuTrigger {
                        class: "semantic-directory-browser__content-trigger",
                        div { class: "semantic-directory-browser__content", role: "region", aria_label: "Directory contents", aria_busy: content_busy,
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
                                                focused_item: focused_item(),
                                                dragged_item: dragged_item(),
                                                commands,
                                            }
                                        } else {
                                            DirectoryGrid {
                                                items: page_data.items.clone(),
                                                selected_items: selected_items(),
                                                cut_items: cut_item_ids(clipboard()),
                                                focused_item: focused_item(),
                                                dragged_item: dragged_item(),
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
                error: dialog_error(),
                busy_operations: busy,
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
    focused_item: Option<String>,
    dragged_item: Option<String>,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let count = items.len();
    let mut item_count = use_signal(move || count);
    use_effect(use_reactive((&count,), move |(count,)| {
        item_count.set(count);
    }));
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
                        focused: focused_item.as_deref() == Some(item.id.as_str()),
                        dragged_item: dragged_item.clone(),
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
    focused_item: Option<String>,
    dragged_item: Option<String>,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let columns = 4usize;
    let rows = items.len().div_ceil(columns);
    let mut row_count = use_signal(move || rows);
    use_effect(use_reactive((&rows,), move |(rows,)| {
        row_count.set(rows);
    }));
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
                                focused: focused_item.as_deref() == Some(item.id.as_str()),
                                dragged_item: dragged_item.clone(),
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
    focused: bool,
    dragged_item: Option<String>,
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
            id: directory_item_dom_id(&item.id),
            class: "semantic-directory-browser__row",
            "data-selected": selected,
            "data-cut": cut,
            "data-focused": focused,
            "data-dragging": dragged_item.as_deref() == Some(item.id.as_str()),
            "data-drop-target": dragged_item.is_some() && item.is_directory && dragged_item.as_deref() != Some(item.id.as_str()),
            "data-draggable": true,
            aria_pressed: selected,
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
                    commands.send(DirectoryBrowserCommand::SelectRange(click_item.clone()));
                } else if event.modifiers().ctrl() || event.modifiers().meta() {
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
    focused: bool,
    dragged_item: Option<String>,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let type_label = item.type_id.clone().unwrap_or_else(|| "entity".to_string());
    let click_item = item.clone();
    rsx! {
        dxcomp::ContextMenu {
            dxcomp::ContextMenuTrigger {
        button {
            id: directory_item_dom_id(&item.id),
            class: "semantic-directory-browser__tile",
            "data-selected": selected,
            "data-cut": cut,
            "data-focused": focused,
            "data-dragging": dragged_item.as_deref() == Some(item.id.as_str()),
            "data-drop-target": dragged_item.is_some() && item.is_directory && dragged_item.as_deref() != Some(item.id.as_str()),
            "data-draggable": true,
            aria_pressed: selected,
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
                    commands.send(DirectoryBrowserCommand::SelectRange(click_item.clone()));
                } else if event.modifiers().ctrl() || event.modifiers().meta() {
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
    dragged_item: Option<String>,
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
            role: "treeitem",
            aria_selected: selected,
            aria_expanded: is_expanded,
            style: "--semantic-directory-depth: {padding}px",
            button {
                class: "semantic-directory-browser__tree-expander",
                aria_label: if is_expanded { "Collapse directory" } else { "Expand directory" },
                aria_expanded: is_expanded,
                aria_controls: "semantic-directory-tree",
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
                        "data-dragging": dragged_item.as_deref() == Some(item.id.as_str()),
                        "data-drop-target": dragged_item.is_some() && dragged_item.as_deref() != Some(item.id.as_str()),
                        "data-draggable": true,
                        aria_current: is_active.then_some("page"),
                        aria_pressed: selected,
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
                                commands.send(DirectoryBrowserCommand::SelectRange(click_item.clone()));
                            } else if event.modifiers().ctrl() || event.modifiers().meta() {
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
                div { class: "semantic-directory-browser__entity-dialog-body",
                    EntityCard {
                        object: item.object,
                        options: EntityRenderOptions {
                            collection: Some(item.collection),
                            id: Some(item.id),
                            renderer: EntityDisplayRenderer::Custom,
                            preview: false,
                            actions: true,
                        },
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
    error: Option<String>,
    busy_operations: BTreeSet<DirectoryOperation>,
    commands: Coroutine<DirectoryBrowserCommand>,
) -> Element {
    let client = use_rpc_client();
    let mut title = use_signal(String::new);
    let mut add_query = use_signal(String::new);
    let mut add_selected = use_signal(|| None::<String>);
    let mut move_target = use_signal(|| None::<String>);
    let reset_dialog = dialog.clone();
    use_effect(use_reactive((&reset_dialog,), move |(dialog,)| {
        title.set(match dialog {
            Some(DirectoryDialog::Rename { title, .. }) => title,
            _ => String::new(),
        });
        add_query.set(String::new());
        add_selected.set(None);
        move_target.set(None);
    }));
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
                return Ok(Vec::new());
            };
            load_addable_entity_options(client, scope_id, parent, search, 50).await
        }
    });
    let options_state = options.read().clone();
    let options = options_state
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let open = dialog.is_some();
    if let Some(DirectoryDialog::NewDirectory { parent }) = dialog.as_ref() {
        let parent = parent.clone();
        let create_busy = busy_operations.contains(&DirectoryOperation::Create);
        return rsx! {
            div {
                class: "dx-dialog-backdrop semantic-directory-browser__new-directory-backdrop",
                "data-state": "open",
                onclick: move |_| commands.send(DirectoryBrowserCommand::CloseDialog),
                form {
                    class: "dx-dialog semantic-directory-browser__new-directory-dialog",
                    role: "dialog",
                    aria_modal: "true",
                    aria_labelledby: "semantic-new-directory-title",
                    onclick: move |event| event.stop_propagation(),
                    onkeydown: move |event: KeyboardEvent| {
                        if event.key().to_string() == "Escape" {
                            event.prevent_default();
                            commands.send(DirectoryBrowserCommand::CloseDialog);
                        }
                    },
                    onsubmit: move |event| {
                        event.prevent_default();
                        if !create_busy && !title().trim().is_empty() {
                            commands.send(DirectoryBrowserCommand::CreateDirectory {
                                parent: parent.clone(),
                                title: title(),
                            });
                        }
                    },
                    h2 { id: "semantic-new-directory-title", class: "dx-dialog-title", "New Directory" }
                    if let Some(error) = error {
                        div { class: "semantic-error semantic-directory-browser__dialog-error", role: "alert", "{error}" }
                    }
                    label { class: "semantic-directory-browser__dialog-field",
                        span { "Title" }
                        input {
                            autofocus: true,
                            value: "{title()}",
                            oninput: move |event| title.set(event.value()),
                        }
                    }
                    div { class: "semantic-directory-browser__dialog-actions",
                        dxcomp::Button {
                            r#type: "button",
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::CloseDialog),
                            "Cancel"
                        }
                        dxcomp::Button {
                            r#type: "submit",
                            disabled: title().trim().is_empty() || create_busy,
                            if create_busy { "Creating…" } else { "Create" }
                        }
                    }
                }
            }
        };
    }
    rsx! {
        dxcomp::Dialog {
            open,
            on_open_change: move |open: bool| {
                if !open {
                    commands.send(DirectoryBrowserCommand::CloseDialog);
                }
            },
            if let Some(error) = error {
                div { class: "semantic-error semantic-directory-browser__dialog-error", role: "alert", "{error}" }
            }
            match dialog {
                Some(DirectoryDialog::NewDirectory { .. }) => rsx! {},
                Some(DirectoryDialog::AddExisting { parent }) => rsx! {
                    dxcomp::DialogTitle { "Add Existing Item" }
                    dxcomp::Combobox::<String> {
                        default_value: add_selected(),
                        on_value_change: move |value| add_selected.set(value),
                        on_query_change: move |value| add_query.set(value),
                        placeholder: "Search entities",
                        aria_label: "Existing entity",
                        list_aria_label: "Existing entities",
                        dxcomp::ComboboxEmpty {
                            if options_state.is_none() {
                                "Searching…"
                            } else if options_state.as_ref().is_some_and(|result| result.is_err()) {
                                "Unable to load entities."
                            } else {
                                "No entity found."
                            }
                        }
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
                    if let Some(Err(error)) = options_state.as_ref() {
                        div { class: "semantic-error semantic-directory-browser__dialog-error", role: "alert", "{error}" }
                    }
                    div { class: "semantic-directory-browser__dialog-actions",
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::CloseDialog),
                            "Cancel"
                        }
                        dxcomp::Button {
                            disabled: add_selected().is_none() || busy_operations.contains(&DirectoryOperation::AddExisting),
                            onclick: move |_| {
                                if let Some(item_id) = add_selected() {
                                    commands.send(DirectoryBrowserCommand::AddExisting {
                                        parent: parent.clone(),
                                        item_id,
                                    });
                                }
                            },
                            if busy_operations.contains(&DirectoryOperation::AddExisting) { "Adding…" } else { "Add" }
                        }
                    }
                },
                Some(DirectoryDialog::Rename { item_id, title: _ }) => {
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
                                disabled: title().trim().is_empty() || busy_operations.contains(&DirectoryOperation::Rename),
                                onclick: move |_| commands.send(DirectoryBrowserCommand::RenameItem {
                                    item_id: item_id.clone(),
                                    title: title(),
                                }),
                                if busy_operations.contains(&DirectoryOperation::Rename) { "Renaming…" } else { "Rename" }
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
                            disabled: busy_operations.contains(&DirectoryOperation::Remove),
                            onclick: move |_| commands.send(DirectoryBrowserCommand::ConfirmRemove {
                                parent: parent.clone(),
                                item_ids: item_ids.clone(),
                            }),
                            if busy_operations.contains(&DirectoryOperation::Remove) { "Removing…" } else { "Remove" }
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
                            disabled: busy_operations.contains(&DirectoryOperation::Delete),
                            onclick: move |_| commands.send(DirectoryBrowserCommand::ConfirmDelete(item_ids.clone())),
                            if busy_operations.contains(&DirectoryOperation::Delete) { "Deleting…" } else { "Delete" }
                        }
                    }
                },
                Some(DirectoryDialog::MoveTo { source, item_ids }) => rsx! {
                    dxcomp::DialogTitle { "Move Items" }
                    dxcomp::DialogDescription {
                        "Choose a destination directory. This is the keyboard and touch alternative to drag and drop."
                    }
                    FileTreePicker {
                        selected: move_target(),
                        show_files: false,
                        select_directories: true,
                        select_files: false,
                        disabled: busy_operations.contains(&DirectoryOperation::MoveTo),
                        on_select: move |selection: FileTreeSelection| {
                            if selection.is_directory {
                                move_target.set(Some(selection.id));
                            }
                        },
                    }
                    div { class: "semantic-directory-browser__dialog-actions",
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| commands.send(DirectoryBrowserCommand::CloseDialog),
                            "Cancel"
                        }
                        dxcomp::Button {
                            disabled: move_target().is_none() || busy_operations.contains(&DirectoryOperation::MoveTo),
                            onclick: move |_| {
                                if let Some(target) = move_target() {
                                    commands.send(DirectoryBrowserCommand::MoveItems {
                                        source: source.clone(),
                                        target,
                                        item_ids: item_ids.clone(),
                                    });
                                }
                            },
                            if busy_operations.contains(&DirectoryOperation::MoveTo) { "Moving…" } else { "Move" }
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

fn operation_busy(
    busy_operations: Signal<BTreeSet<DirectoryOperation>>,
    operation: DirectoryOperation,
) -> bool {
    busy_operations.read().contains(&operation)
}

fn set_operation_busy(
    mut busy_operations: Signal<BTreeSet<DirectoryOperation>>,
    operation: DirectoryOperation,
    busy: bool,
) {
    let mut next = busy_operations();
    if busy {
        next.insert(operation);
    } else {
        next.remove(&operation);
    }
    busy_operations.set(next);
}

fn visible_item_ids(content_state: Signal<LoadState<DirectoryPage>>) -> Vec<String> {
    match &*content_state.read() {
        LoadState::Ready(page) => page.items.iter().map(|item| item.id.clone()).collect(),
        LoadState::Loading | LoadState::Error(_) => Vec::new(),
    }
}

fn visible_item(
    content_state: Signal<LoadState<DirectoryPage>>,
    id: &str,
) -> Option<DirectoryBrowseItem> {
    match &*content_state.read() {
        LoadState::Ready(page) => page.items.iter().find(|item| item.id == id).cloned(),
        LoadState::Loading | LoadState::Error(_) => None,
    }
}

fn moved_focus(ids: &[String], focused: Option<&str>, direction: FocusMove) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    if focused.is_none() {
        return match direction {
            FocusMove::Last => ids.last().cloned(),
            FocusMove::Previous | FocusMove::Next | FocusMove::First => ids.first().cloned(),
        };
    }
    let current = focused
        .and_then(|focused| ids.iter().position(|id| id == focused))
        .unwrap_or(0);
    let next = match direction {
        FocusMove::Previous => current.saturating_sub(1),
        FocusMove::Next => (current + 1).min(ids.len() - 1),
        FocusMove::First => 0,
        FocusMove::Last => ids.len() - 1,
    };
    ids.get(next).cloned()
}

fn selection_range(ids: &[String], anchor: &str, target: &str) -> BTreeSet<String> {
    let Some(anchor_index) = ids.iter().position(|id| id == anchor) else {
        return [target.to_string()].into_iter().collect();
    };
    let Some(target_index) = ids.iter().position(|id| id == target) else {
        return [target.to_string()].into_iter().collect();
    };
    let (start, end) = if anchor_index <= target_index {
        (anchor_index, target_index)
    } else {
        (target_index, anchor_index)
    };
    ids[start..=end].iter().cloned().collect()
}

fn directory_item_dom_id(id: &str) -> String {
    format!("semantic-directory-item-{}", super::data::hex_id_part(id))
}

async fn reload_current_content(
    client: Option<semantic_rpc::RpcClient>,
    scope_id: Option<String>,
    mut state: DirectoryBrowserSignals,
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
    let generation = (state.content_generation)().saturating_add(1);
    state.content_generation.set(generation);
    let root = (state.current_root)();
    let page = (state.page)();
    let page_size = (state.page_size)();
    let sort = (state.sort)();
    spawn(reload_content(
        client,
        scope_id,
        root,
        page,
        page_size,
        sort,
        state.content_state,
        state.content_loading,
        state.content_generation,
        generation,
    ));
}

async fn reload_current_tree(
    client: Option<semantic_rpc::RpcClient>,
    scope_id: Option<String>,
    mut state: DirectoryBrowserSignals,
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
    let generation = (state.tree_generation)().saturating_add(1);
    state.tree_generation.set(generation);
    let expanded = (state.expanded_tree)();
    spawn(reload_tree(
        client,
        scope_id,
        expanded,
        state.tree_state,
        state.tree_loading,
        state.tree_generation,
        generation,
    ));
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
    mut content_loading: Signal<bool>,
    content_generation: Signal<u64>,
    generation: u64,
) {
    info!(
        root = root.as_deref(),
        page,
        page_size,
        sort = sort.as_value(),
        scope_id = scope_id.as_deref(),
        "directory browser loading content"
    );
    content_loading.set(true);
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
    if *content_generation.peek() == generation {
        content_state.set(next);
        content_loading.set(false);
    } else {
        info!(
            generation,
            "directory browser ignored stale content response"
        );
    }
}

async fn reload_tree(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    expanded: BTreeSet<String>,
    mut tree_state: Signal<LoadState<Vec<DirectoryTreeRow>>>,
    mut tree_loading: Signal<bool>,
    tree_generation: Signal<u64>,
    generation: u64,
) {
    info!(
        expanded_count = expanded.len(),
        scope_id = scope_id.as_deref(),
        "directory browser loading tree"
    );
    tree_loading.set(true);
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
    if *tree_generation.peek() == generation {
        tree_state.set(next);
        tree_loading.set(false);
    } else {
        info!(generation, "directory browser ignored stale tree response");
    }
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

fn expanded_for_open(mut expanded: BTreeSet<String>, item_id: &str) -> BTreeSet<String> {
    expanded.insert(item_id.to_string());
    expanded
}

#[cfg(test)]
mod tests {
    use super::expanded_for_open;
    use std::collections::BTreeSet;

    #[test]
    fn opening_a_tree_directory_expands_it_idempotently() {
        let expanded = expanded_for_open(BTreeSet::new(), "directory-1");
        assert!(expanded.contains("directory-1"));
        assert_eq!(expanded_for_open(expanded.clone(), "directory-1"), expanded);
    }
}
