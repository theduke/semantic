use dioxus::prelude::*;
use semantic_base::labels::{Label, LabelCatalog, LabelKind, SelectionMode};
use semantic_ui_core::components::labels::{LabelColor, client};
use semantic_ui_core::{use_active_scope_id, use_rpc_client};

#[component]
pub(super) fn LabelDetails(
    label: Label,
    catalog: LabelCatalog,
    on_dirty: EventHandler<bool>,
    on_busy: EventHandler<bool>,
    on_saved: EventHandler<Label>,
    on_deleted: EventHandler<String>,
    on_child: EventHandler<String>,
) -> Element {
    let rpc = use_rpc_client();
    let scope = use_active_scope_id();
    let mut original = use_signal(|| label.clone());
    let mut draft = use_signal(|| label);
    let mut pending = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut saved = use_signal(|| false);
    let mut confirm_delete = use_signal(|| false);
    use_effect(move || on_dirty.call(*draft.read() != *original.read()));
    use_effect(move || on_busy.call(pending()));
    let exists = catalog.labels.contains_key(&draft.read().id);
    let noun = if draft.read().is_group() {
        "group"
    } else {
        "label"
    };
    let has_children = catalog
        .labels
        .values()
        .any(|label| label.parent_id.as_deref() == Some(&draft.read().id));
    let mut parents: Vec<_> = catalog
        .labels
        .values()
        .filter(|label| {
            let mut candidate = draft();
            candidate.name = "validation".into();
            candidate.color = None;
            candidate.parent_id = Some(label.id.clone());
            catalog.validate_label(&candidate).is_ok()
        })
        .cloned()
        .collect();
    parents.sort_by_cached_key(|label| catalog.path(&label.id).to_lowercase());
    let validation = catalog.validate_label(&draft()).err().map(|e| e.message);
    let is_dirty = *draft.read() != *original.read();
    let save_rpc = rpc.clone();
    let save_scope = scope.clone();
    let save = use_callback(move |_| {
        if pending() {
            return;
        }
        pending.set(true);
        error.set(None);
        saved.set(false);
        let rpc = save_rpc.clone();
        let scope = save_scope.clone();
        let label = draft();
        spawn(async move {
            match client::save(&rpc, scope, label).await {
                Ok(label) => {
                    original.set(label.clone());
                    draft.set(label.clone());
                    saved.set(true);
                    on_saved.call(label);
                }
                Err(message) => error.set(Some(message)),
            }
            pending.set(false);
        });
    });
    rsx! {
        form { class: "semantic-label-form", onsubmit: move |event| { event.prevent_default(); save.call(()); },
            div { class: "semantic-label-manager__toolbar",
                h2 { if exists { "Details" } else if draft.read().is_group() { "New group" } else { "New label" } }
                if exists {
                    dxcomp::Button { r#type: "button", variant: dxcomp::ButtonVariant::Outline,
                        disabled: pending(), onclick: move |_| on_child.call(draft.read().id.clone()), "+ Add child" }
                }
            }
            if let Some(message) = error() { p { class: "semantic-error", role: "alert", "{message}" } }
            fieldset { disabled: pending(), class: "semantic-label-form__fields",
                label { class: "semantic-label-field",
                    span { "Kind" }
                    select { value: if draft.read().is_group() { "group" } else { "label" },
                        onchange: move |event| {
                            let mut label = draft.write();
                            label.kind = if event.value() == "group" { LabelKind::Group } else { LabelKind::Label };
                            if !label.is_group() { label.selection_mode = SelectionMode::Multiple; }
                            saved.set(false);
                        },
                        option { value: "label", "Label · Assign to entities" }
                        option { value: "group", "Group · Organize labels" }
                    }
                    small { "Groups cannot be assigned to entities. Remove a label's assignments before converting it to a group." }
                }
                label { class: "semantic-label-field",
                    span { "Name" }
                    input { r#type: "text", required: true, maxlength: 200, autofocus: true,
                        placeholder: "e.g. In progress", value: draft.read().name.clone(),
                        oninput: move |event| { draft.write().name = event.value(); saved.set(false); } }
                }
                label { class: "semantic-label-field",
                    span { "Description (optional)" }
                    textarea { required: false, rows: 3, placeholder: "When should this be used?", value: draft.read().description.clone().unwrap_or_default(),
                        oninput: move |event| { draft.write().description = nonempty(event.value()); saved.set(false); } }
                }
                label { class: "semantic-label-field",
                    span { "Parent label or group" }
                    select { value: draft.read().parent_id.clone().unwrap_or_default(),
                        onchange: move |event| { draft.write().parent_id = nonempty(event.value()); saved.set(false); },
                        option { value: "", "No parent · Top level" }
                        for parent in parents { option { value: parent.id.clone(), "{catalog.path(&parent.id)}" } }
                    }
                    small { "Groups organize labels and cannot be selected. Ordinary parent labels remain assignable." }
                }
                div { class: "semantic-label-field",
                    span { "Color (optional)" }
                    div { class: "semantic-label-color-controls",
                        LabelColor { color: draft.read().color.clone() }
                        if let Some(color) = draft.read().color.clone() {
                            input { r#type: "color", aria_label: "Custom color", value: color,
                                oninput: move |event| { draft.write().color = Some(event.value()); saved.set(false); } }
                        } else {
                            button { r#type: "button", class: "semantic-label-text-button",
                                onclick: move |_| { draft.write().color = Some("#6366f1".into()); saved.set(false); }, "Set custom color" }
                        }
                        for color in ["#6366f1", "#2563eb", "#0891b2", "#16a34a", "#d97706", "#dc2626", "#c026d3"] {
                            button { r#type: "button", class: "semantic-label-swatch", style: "background-color: {color}",
                                aria_label: "Use {color}", aria_pressed: draft.read().color.as_deref() == Some(color),
                                onclick: move |_| { draft.write().color = Some(color.into()); saved.set(false); } }
                        }
                        button { r#type: "button", class: "semantic-label-text-button",
                            aria_pressed: draft.read().color.is_none(),
                            onclick: move |_| { draft.write().color = None; saved.set(false); }, "No color" }
                    }
                }
                if draft.read().is_group() { label { class: "semantic-label-field",
                    span { "Child selection" }
                    select { value: draft.read().selection_mode.as_str(),
                        onchange: move |event| { draft.write().selection_mode = if event.value() == "exclusive" { SelectionMode::Exclusive } else { SelectionMode::Multiple }; saved.set(false); },
                        option { value: "multiple", "Multiple · Allow any combination" }
                        option { value: "exclusive", "Exclusive · Choose one child" }
                    }
                    small { "Applies to direct child labels. Selecting a child in an exclusive group replaces its selected sibling." }
                } }
            }
            if is_dirty && !draft.read().name.trim().is_empty() {
                if let Some(message) = validation.clone() { p { class: "semantic-error", role: "status", "{message}" } }
            }
            div { class: "semantic-label-form__actions",
                span { class: "semantic-label-muted", role: "status",
                    if pending() { "Saving…" } else if saved() { "Saved" } else if is_dirty { "Unsaved changes" }
                }
                dxcomp::Button { r#type: "button", variant: dxcomp::ButtonVariant::Outline, disabled: pending() || !is_dirty,
                    onclick: move |_| { draft.set(original()); error.set(None); saved.set(false); }, "Discard changes" }
                dxcomp::Button { r#type: "submit", disabled: pending() || validation.is_some() || (exists && !is_dirty),
                    if exists { "Save changes" } else if draft.read().is_group() { "Create group" } else { "Create label" }
                }
            }
            if exists {
                div { class: "semantic-label-form__delete",
                    div {
                        strong { "Delete {noun}" }
                        p { if has_children { "Move or delete this {noun}’s children first." } else { "Removes this {noun} and any assignments. Entities are kept." } }
                    }
                    dxcomp::Button { r#type: "button", variant: dxcomp::ButtonVariant::Destructive,
                        disabled: pending() || has_children, onclick: move |_| confirm_delete.set(true), "Delete" }
                }
            }
        }
        dxcomp::AlertDialog { open: confirm_delete(), on_open_change: move |open: bool| if !pending() { confirm_delete.set(open); },
            dxcomp::AlertDialogTitle { "Delete {original.read().name}?" }
            dxcomp::AlertDialogDescription { "This removes the {noun} and any assignments from every entity. This cannot be undone." }
            if let Some(message) = error() { p { class: "semantic-error", role: "alert", "{message}" } }
            dxcomp::AlertDialogActions {
                dxcomp::Button { variant: dxcomp::ButtonVariant::Outline, disabled: pending(), onclick: move |_| confirm_delete.set(false), "Cancel" }
                dxcomp::Button { variant: dxcomp::ButtonVariant::Destructive, disabled: pending(),
                    onclick: move |_| {
                        if pending() { return; }
                        pending.set(true); error.set(None);
                        let rpc = rpc.clone(); let scope = scope.clone(); let id = original.read().id.clone();
                        spawn(async move {
                            match client::delete(&rpc, scope, id.clone()).await {
                                Ok(()) => { pending.set(false); on_busy.call(false); confirm_delete.set(false); on_deleted.call(id); }
                                Err(message) => { error.set(Some(message)); pending.set(false); }
                            }
                        });
                    },
                    if pending() { "Deleting…" } else { "Delete {noun}" }
                }
            }
        }
    }
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}
