use super::{LabelColor, client};
use crate::{EntityTarget, use_active_scope_id, use_rpc_client, use_ui_catalog};
use dioxus::prelude::*;
use dioxus_icons::lucide::Tags;
use semantic_base::labels::{Label, LabelCatalog};
use std::collections::BTreeSet;

#[derive(Clone, Default, PartialEq)]
struct EditorState {
    catalog: LabelCatalog,
    original: BTreeSet<String>,
    selected: BTreeSet<String>,
    loaded: bool,
    pending: bool,
    error: Option<String>,
    saved: bool,
}

impl EditorState {
    fn loaded(labels: Vec<Label>, assigned: Vec<Label>) -> Self {
        let selected: BTreeSet<_> = assigned.iter().map(|label| label.id.clone()).collect();
        let mut catalog = LabelCatalog::new(labels);
        // The reads can observe different revisions. Always display assigned
        // labels, including ones created after the catalog read completed.
        for label in assigned {
            catalog.labels.insert(label.id.clone(), label);
        }
        Self {
            catalog,
            original: selected.clone(),
            selected,
            loaded: true,
            ..Default::default()
        }
    }

    fn dirty(&self) -> bool {
        self.original != self.selected
    }
}

/// A modal label editor. Keep mounted until `on_close` fires: Escape, outside click,
/// and Close all save pending changes first. Failed saves keep the draft open.
/// `on_changed` receives the persisted selection after a successful change.
/// Changing the target or active scope resets the draft and cancels local tasks.
/// A save already submitted to the server may still finish for its original target.
#[component]
pub fn LabelEditor(
    target: EntityTarget,
    on_close: EventHandler<()>,
    #[props(default)] on_changed: Option<EventHandler<Vec<Label>>>,
) -> Element {
    let scope = use_active_scope_id();
    let key = format!("{:?}:{:?}", scope, target);
    rsx! {
        // Dioxus applies keys when diffing a dynamic child list. A key on a
        // directly rendered singleton component does not establish this boundary.
        for (key, target, scope) in [(key, target, scope)] {
            LabelEditorSession { key: "{key}", target, scope, on_close, on_changed }
        }
    }
}

#[component]
fn LabelEditorSession(
    target: EntityTarget,
    scope: Option<String>,
    on_close: EventHandler<()>,
    on_changed: Option<EventHandler<Vec<Label>>>,
) -> Element {
    let rpc = use_rpc_client();
    let mut state = use_signal(EditorState::default);
    let mut filter = use_signal(String::new);
    let mut reload = use_signal(|| 0u64);
    let load_rpc = rpc.clone();
    let load_target = target.clone();
    let load_scope = scope.clone();
    use_resource(move || {
        let rpc = load_rpc.clone();
        let target = load_target.clone();
        let scope = load_scope.clone();
        let _ = reload();
        async move {
            let result = futures::try_join!(
                client::list(&rpc, scope.clone()),
                client::load(&rpc, scope, &target)
            );
            match result {
                Ok((labels, assigned)) => {
                    state.set(EditorState::loaded(labels, assigned));
                }
                Err(error) => state.write().error = Some(error),
            }
        }
    });

    let save = use_callback(move |close: bool| {
        if state.peek().pending {
            return;
        }
        if !state.peek().loaded || !state.peek().dirty() {
            if close {
                on_close.call(());
            }
            return;
        }
        let ids = state.peek().selected.iter().cloned().collect();
        // Removal-only cleanup must not reassign remaining legacy groups.
        let remove = {
            let current = state.peek();
            (current.selected.is_subset(&current.original)
                && current
                    .selected
                    .iter()
                    .any(|id| current.catalog.labels.get(id).is_some_and(Label::is_group)))
            .then(|| {
                current
                    .original
                    .difference(&current.selected)
                    .cloned()
                    .collect()
            })
        };
        state.write().pending = true;
        state.write().error = None;
        let rpc = rpc.clone();
        let scope = scope.clone();
        let target = target.clone();
        spawn(async move {
            let result = match remove {
                Some(removed) => client::remove(&rpc, scope, &target, removed).await,
                None => client::replace(&rpc, scope, &target, ids).await,
            };
            match result {
                Ok(labels) => {
                    let ids = labels.iter().map(|label| label.id.clone()).collect();
                    let mut current = state.write();
                    current.original = ids;
                    current.selected = current.original.clone();
                    current.pending = false;
                    current.saved = true;
                    drop(current);
                    if let Some(on_changed) = on_changed {
                        on_changed.call(labels);
                    }
                    if close {
                        on_close.call(());
                    }
                }
                Err(error) => {
                    state.write().pending = false;
                    state.write().error = Some(error);
                }
            }
        });
    });
    let current = state.read();
    let query = filter().trim().to_lowercase();
    let mut available: Vec<_> = current
        .catalog
        .labels
        .values()
        .filter(|label| {
            current
                .catalog
                .path(&label.id)
                .to_lowercase()
                .contains(&query)
                || label
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&query)
        })
        .cloned()
        .collect();
    available.sort_by_cached_key(|label| current.catalog.path(&label.id).to_lowercase());

    rsx! {
        dxcomp::Dialog {
            class: "semantic-label-dialog",
            open: true,
            on_open_change: move |open: bool| if !open { save.call(true); },
            div { class: "semantic-label-editor",
                div { class: "semantic-label-editor__heading",
                    div {
                        dxcomp::DialogTitle { "Edit labels" }
                        dxcomp::DialogDescription { "Organize this entity with labels. Changes save when you close." }
                    }
                    dxcomp::Button { variant: dxcomp::ButtonVariant::Ghost, disabled: current.pending,
                        aria_label: "Close and save labels", onclick: move |_| save.call(true), "Close" }
                }
                if let Some(error) = &current.error {
                    p { class: "semantic-error", role: "alert", "{error}" }
                    if !current.loaded {
                        dxcomp::Button { variant: dxcomp::ButtonVariant::Outline,
                            onclick: move |_| { state.write().error = None; reload += 1; }, "Try again" }
                    }
                }
                if !current.loaded && current.error.is_none() {
                    p { role: "status", "Loading labels…" }
                }
                if current.loaded {
                    section { class: "semantic-label-editor__selection", aria_label: "Selected labels",
                        div { class: "semantic-label-section-title", "Selected · {current.selected.len()}" }
                        div { class: "semantic-label-chips",
                            if current.selected.is_empty() { span { class: "semantic-label-muted", "No labels selected" } }
                            for id in current.selected.iter() {
                                if let Some(label) = current.catalog.labels.get(id) {
                                    button { key: "{id}", r#type: "button", class: "semantic-label-chip",
                                        title: current.catalog.path(id), disabled: current.pending,
                                        aria_label: format!("Remove {}", current.catalog.path(id)),
                                        onclick: { let id = id.clone(); move |_| { state.write().selected.remove(&id); state.write().saved = false; } },
                                        LabelColor { color: label.color.clone() }
                                        "{label.name}"
                                        if label.is_group() { span { " (legacy group assignment)" } }
                                        span { aria_hidden: "true", "×" }
                                    }
                                }
                            }
                        }
                    }
                    label { class: "semantic-label-field",
                        span { "Find labels" }
                        input { r#type: "search", placeholder: "Filter by name, group, or description…",
                            value: filter(), oninput: move |event| filter.set(event.value()), autofocus: true }
                    }
                    div { class: "semantic-label-options", role: "group", aria_label: "Available labels",
                        if available.is_empty() {
                            p { class: "semantic-label-empty",
                                if current.catalog.labels.is_empty() { "No labels yet. Create labels in the label manager." }
                                else { "No matching labels. Try a different search." }
                            }
                        }
                            for label in available {
                                if label.is_group() {
                                    h3 { key: "group-{label.id}", class: "semantic-label-section-title", "{current.catalog.path(&label.id)} · Group" }
                                } else {
                                label { key: "{label.id}", class: "semantic-label-option",
                                "data-selected": current.selected.contains(&label.id).to_string(),
                                input { r#type: "checkbox", checked: current.selected.contains(&label.id), disabled: current.pending,
                                    onchange: { let id = label.id.clone(); move |_| {
                                        let mut current = state.write();
                                        if !current.selected.remove(&id) {
                                            let EditorState { catalog, selected, .. } = &mut *current;
                                            let _ = catalog.select(selected, &id);
                                        }
                                        current.saved = false;
                                    } },
                                }
                                LabelColor { color: label.color.clone() }
                                span { class: "semantic-label-option__text",
                                    strong { "{label.name}" }
                                    if label.parent_id.is_some() { small { "{current.catalog.path(&label.id)}" } }
                                    if let Some(description) = label.description { small { "{description}" } }
                                }
                                if current.catalog.exclusive_group(&label.id).is_some() {
                                    span { class: "semantic-label-mode", title: "Selecting this replaces a selected sibling", "Choose one" }
                                }
                                }
                            }
                        }
                    }
                    div { class: "semantic-label-editor__footer",
                        span { class: "semantic-label-muted", role: "status",
                            if current.pending { "Saving…" } else if current.dirty() { "Unsaved changes" }
                            else if current.saved { "Labels saved" } else { "Up to date" }
                        }
                        dxcomp::Button { variant: dxcomp::ButtonVariant::Outline, disabled: current.pending,
                            onclick: move |_| on_close.call(()), "Discard" }
                        dxcomp::Button { aria_label: "Save labels", disabled: current.pending || !current.dirty(),
                            onclick: move |_| save.call(false), "Save changes" }
                    }
                }
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "editor_tests.rs"]
mod tests;

#[component]
pub fn EntityLabelsButton(target: EntityTarget) -> Element {
    let catalog = use_ui_catalog();
    let mut open = use_signal(|| false);
    if !catalog.render_settings().enable_label_editor {
        return rsx! {};
    }
    rsx! {
        dxcomp::Button {
            variant: dxcomp::ButtonVariant::Outline, size: dxcomp::ButtonSize::IconSm,
            title: "Edit labels", aria_label: "Edit labels", onclick: move |_| open.set(true),
            Tags { size: "1rem" }
        }
        if open() { LabelEditor { target, on_close: move |_| open.set(false) } }
    }
}
