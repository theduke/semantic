use dioxus::prelude::*;
use semantic_base::labels::{LabelCatalog, SelectionMode};
use semantic_ui_core::components::labels::LabelColor;
use std::{collections::BTreeSet, rc::Rc};

#[component]
pub(super) fn LabelTree(
    catalog: Rc<LabelCatalog>,
    selected: Option<String>,
    query: String,
    on_select: EventHandler<String>,
    #[props(default)] parent: Option<String>,
    #[props(default)] ancestors: BTreeSet<String>,
) -> Element {
    let mut collapsed = use_signal(BTreeSet::<String>::new);
    let mut children: Vec<_> = catalog
        .labels
        .values()
        .filter(|label| label.parent_id == parent && !ancestors.contains(&label.id))
        .cloned()
        .collect();
    children.sort_by_cached_key(|label| label.name.to_lowercase());
    rsx! {
        ul { class: "semantic-label-tree", aria_label: if parent.is_none() { "Label hierarchy" } else { "Child labels" },
            for label in children {
                if branch_matches(&catalog, &label.id, &query) {
                    li { key: "{label.id}",
                        div { class: "semantic-label-tree__row", "data-selected": (selected.as_ref() == Some(&label.id)).to_string(),
                            if catalog.labels.values().any(|child| child.parent_id.as_deref() == Some(&label.id)) {
                                button { r#type: "button", class: "semantic-label-tree__toggle",
                                    aria_label: format!("{} {}", if collapsed.read().contains(&label.id) { "Expand" } else { "Collapse" }, label.name),
                                    aria_expanded: !collapsed.read().contains(&label.id) || !query.is_empty(),
                                    onclick: { let id = label.id.clone(); move |_| { if !collapsed.write().remove(&id) { collapsed.write().insert(id.clone()); } } },
                                    if collapsed.read().contains(&label.id) && query.is_empty() { "›" } else { "⌄" }
                                }
                            } else { span { class: "semantic-label-tree__spacer", aria_hidden: "true" } }
                            button { r#type: "button", class: "semantic-label-tree__select",
                                aria_pressed: selected.as_ref() == Some(&label.id),
                                onclick: { let id = label.id.clone(); move |_| on_select.call(id.clone()) },
                                LabelColor { color: label.color.clone() }
                                span { "{label.name}" }
                                if label.is_group() { small { class: "semantic-label-mode", "Group" } }
                                if label.selection_mode == SelectionMode::Exclusive {
                                    small { class: "semantic-label-mode", title: "Only one direct child can be selected per entity", "Exclusive" }
                                }
                            }
                        }
                        if !collapsed.read().contains(&label.id) || !query.is_empty() {
                            LabelTree { catalog: catalog.clone(), selected: selected.clone(), query: query.clone(), on_select,
                                parent: Some(label.id.clone()), ancestors: { let mut path = ancestors.clone(); path.insert(label.id.clone()); path } }
                        }
                    }
                }
            }
        }
    }
}

fn branch_matches(catalog: &LabelCatalog, id: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    catalog.labels.values().any(|label| {
        if !catalog.path(&label.id).to_lowercase().contains(query) {
            return false;
        }
        let mut current = Some(label.id.as_str());
        let mut seen = BTreeSet::new();
        while let Some(next) = current {
            if next == id {
                return true;
            }
            if !seen.insert(next) {
                break;
            }
            current = catalog
                .labels
                .get(next)
                .and_then(|label| label.parent_id.as_deref());
        }
        false
    })
}
