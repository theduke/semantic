use std::collections::BTreeSet;

use dioxus::prelude::*;
use dioxus_icons::lucide::{ChevronDown, ChevronRight, FileText, Folder};

use crate::context::{use_active_scope_id, use_rpc_client};

use super::{data::load_file_tree_rows, types::DirectoryTreeRow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTreeSelection {
    pub id: String,
    pub title: String,
    pub is_directory: bool,
}

#[derive(Clone, PartialEq, Props)]
pub struct FileTreePickerProps {
    #[props(default)]
    pub selected: Option<String>,

    pub on_select: EventHandler<FileTreeSelection>,

    #[props(default = true)]
    pub show_files: bool,

    #[props(default = true)]
    pub select_directories: bool,

    #[props(default = true)]
    pub select_files: bool,

    #[props(default)]
    pub disabled: bool,

    #[props(default = "Filter files and directories".to_string())]
    pub filter_placeholder: String,
}

#[component]
pub fn FileTreePicker(props: FileTreePickerProps) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut filter = use_signal(String::new);
    let expanded = use_signal(BTreeSet::<String>::new);
    let show_files = props.show_files;
    let rows = use_resource(use_reactive(
        (&client, &scope_id, &show_files),
        move |(client, scope_id, show_files)| async move {
            load_file_tree_rows(client, scope_id, show_files).await
        },
    ));
    let rows = rows.read().clone();

    rsx! {
        div { class: "semantic-directory-picker",
            div { class: "semantic-directory-picker__filter",
                input {
                    r#type: "search",
                    value: "{filter}",
                    placeholder: "{props.filter_placeholder}",
                    aria_label: "Filter files and directories",
                    disabled: props.disabled,
                    oninput: move |event| filter.set(event.value()),
                }
            }
            div { id: "semantic-directory-picker-tree", class: "semantic-directory-picker__tree", role: "tree", aria_label: "Files and directories",
                match rows {
                    None => rsx! { div { class: "semantic-loading", "Loading tree..." } },
                    Some(Err(error)) => rsx! { div { class: "semantic-error", "{error}" } },
                    Some(Ok(rows)) if rows.is_empty() => rsx! {
                        div { class: "semantic-empty", "No items" }
                    },
                    Some(Ok(rows)) => {
                        let visible_rows = visible_picker_rows(&rows, &filter(), &expanded());
                        if visible_rows.is_empty() {
                            rsx! { div { class: "semantic-empty", "No matching items" } }
                        } else {
                            rsx! {
                                for (row, has_children) in visible_rows {
                                    DirectoryPickerRow {
                                        key: "{row.item.id}-{row.depth}",
                                        selected: props.selected.as_deref() == Some(row.item.id.as_str()),
                                        has_children,
                                        expanded: expanded,
                                        filtering: !filter().trim().is_empty(),
                                        disabled: props.disabled,
                                        selectable: if row.item.is_directory {
                                            props.select_directories
                                        } else {
                                            props.select_files
                                        },
                                        on_select: props.on_select,
                                        row,
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn DirectoryPickerRow(
    row: DirectoryTreeRow,
    has_children: bool,
    selected: bool,
    expanded: Signal<BTreeSet<String>>,
    filtering: bool,
    disabled: bool,
    selectable: bool,
    on_select: EventHandler<FileTreeSelection>,
) -> Element {
    let is_expanded = filtering || expanded.read().contains(&row.item.id);
    let selection = FileTreeSelection {
        id: row.item.id.clone(),
        title: row.item.title.clone(),
        is_directory: row.item.is_directory,
    };
    let directory_selection = selection.clone();
    let button_selection = selection;
    let padding = row.depth * 16;
    rsx! {
        div {
            class: "semantic-directory-picker__row",
            "data-selected": selected,
            role: "treeitem",
            aria_selected: selected,
            aria_expanded: if has_children { Some(is_expanded) } else { None },
            style: "--semantic-directory-depth: {padding}px",
            button {
                class: "semantic-directory-picker__expander",
                r#type: "button",
                title: if is_expanded { "Collapse directory" } else { "Expand directory" },
                aria_label: if is_expanded { "Collapse directory" } else { "Expand directory" },
                aria_expanded: is_expanded,
                aria_controls: "semantic-directory-picker-tree",
                disabled: disabled || filtering || !has_children || row.cycle,
                onclick: {
                    let id = row.item.id.clone();
                    move |_| {
                        let mut values = expanded.write();
                        if !values.remove(&id) {
                            values.insert(id.clone());
                        }
                    }
                },
                if has_children {
                    if is_expanded {
                        ChevronDown { size: "1rem" }
                    } else {
                        ChevronRight { size: "1rem" }
                    }
                }
            }
            button {
                class: "semantic-directory-picker__directory",
                r#type: "button",
                "data-selected": selected,
                disabled: disabled || !selectable,
                onclick: move |_| on_select.call(directory_selection.clone()),
                if row.item.is_directory {
                    Folder { size: "1rem" }
                } else {
                    FileText { size: "1rem" }
                }
                span { "{row.item.title}" }
                if row.cycle {
                    small { "Cycle" }
                }
            }
            if selectable {
                dxcomp::Button {
                    variant: if selected { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                    size: dxcomp::ButtonSize::Sm,
                    disabled,
                    onclick: move |_| on_select.call(button_selection.clone()),
                    if selected { "Selected" } else { "Select" }
                }
            }
        }
    }
}

fn visible_picker_rows(
    rows: &[DirectoryTreeRow],
    filter: &str,
    expanded: &BTreeSet<String>,
) -> Vec<(DirectoryTreeRow, bool)> {
    let filter = filter.trim().to_ascii_lowercase();
    let included = if filter.is_empty() {
        None
    } else {
        let mut included = BTreeSet::new();
        for (index, row) in rows.iter().enumerate() {
            if row.item.title.to_ascii_lowercase().contains(&filter)
                || row.item.id.to_ascii_lowercase().contains(&filter)
            {
                included.insert(index);
                let mut ancestor_depth = row.depth;
                for ancestor_index in (0..index).rev() {
                    if ancestor_depth == 0 {
                        break;
                    }
                    if rows[ancestor_index].depth + 1 == ancestor_depth {
                        included.insert(ancestor_index);
                        ancestor_depth -= 1;
                    }
                }
            }
        }
        Some(included)
    };

    let mut collapsed_depth = None;
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let has_children = rows
                .get(index + 1)
                .is_some_and(|next| next.depth > row.depth);
            if let Some(included) = &included {
                return included
                    .contains(&index)
                    .then(|| (row.clone(), has_children));
            }
            if collapsed_depth.is_some_and(|depth| row.depth > depth) {
                return None;
            }
            collapsed_depth = None;
            if has_children && !expanded.contains(&row.item.id) {
                collapsed_depth = Some(row.depth);
            }
            Some((row.clone(), has_children))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use semantic_data::value::Object;

    use super::*;
    use crate::components::directory_browser::types::DirectoryBrowseItem;
    use crate::components::directory_browser::types::DirectoryLocationKind;

    fn row(id: &str, title: &str, depth: usize) -> DirectoryTreeRow {
        DirectoryTreeRow {
            item: DirectoryBrowseItem {
                id: id.to_string(),
                collection: "entities".to_string(),
                object: Object::new(),
                title: title.to_string(),
                type_id: None,
                is_directory: true,
                has_semantic_children: false,
                order: None,
                created_at: None,
                updated_at: None,
            },
            depth,
            cycle: false,
            location_kind: DirectoryLocationKind::Directory,
        }
    }

    #[test]
    fn filter_keeps_matching_directory_and_ancestors() {
        let rows = vec![
            row("root", "Root", 0),
            row("child", "Child", 1),
            row("match", "Needle", 2),
            row("other", "Other", 0),
        ];
        let visible = visible_picker_rows(&rows, "needle", &BTreeSet::new());
        let ids = visible
            .iter()
            .map(|(row, _)| row.item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["root", "child", "match"]);
    }

    #[test]
    fn collapsed_directories_hide_descendants() {
        let rows = vec![
            row("root", "Root", 0),
            row("child", "Child", 1),
            row("other", "Other", 0),
        ];
        let visible = visible_picker_rows(&rows, "", &BTreeSet::new());
        let ids = visible
            .iter()
            .map(|(row, _)| row.item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["root", "other"]);
    }
}
