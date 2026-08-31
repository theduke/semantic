use dioxus::{
    html::{FileData, HasFileData as _},
    prelude::*,
};

/// Keyboard-accessible file picker with drag-and-drop enhancement.
///
/// The native input remains keyboard operable; dropping files is an additional
/// interaction. Clipboard files are intentionally not advertised because
/// Dioxus 0.7 does not expose them through its clipboard event.
#[component]
pub fn DropZone(
    id: String,
    label: String,
    hint: String,
    on_files: EventHandler<Vec<FileData>>,
    #[props(default)] accept: Option<String>,
    #[props(default = true)] multiple: bool,
    #[props(default)] disabled: bool,
) -> Element {
    let mut dragging = use_signal(|| false);
    let hint_id = format!("{id}-hint");

    rsx! {
        div {
            class: "semantic-drop-zone",
            "data-dragging": dragging(),
            "data-disabled": disabled,
            role: "group",
            aria_label: label.clone(),
            ondragenter: move |event: DragEvent| {
                event.prevent_default();
                if !disabled {
                    dragging.set(true);
                }
            },
            ondragover: move |event: DragEvent| event.prevent_default(),
            ondragleave: move |_| dragging.set(false),
            ondrop: move |event: DragEvent| {
                event.prevent_default();
                dragging.set(false);
                if !disabled {
                    let files = event.data().files();
                    if !files.is_empty() {
                        on_files.call(files);
                    }
                }
            },
            div { class: "semantic-drop-zone__copy",
                strong { "{label}" }
                span { id: hint_id.clone(), "{hint}" }
            }
            label {
                class: "dx-button semantic-drop-zone__button",
                "data-style": "primary",
                "data-size": "default",
                r#for: id.clone(),
                "Choose files"
            }
            input {
                id,
                class: "semantic-drop-zone__input",
                r#type: "file",
                multiple,
                accept,
                disabled,
                aria_describedby: hint_id,
                onchange: move |event| {
                    let files = event.files();
                    if !files.is_empty() {
                        on_files.call(files);
                    }
                },
            }
        }
    }
}

/// Small, reusable native progress presentation for upload and background jobs.
#[component]
pub fn JobProgress(
    id: String,
    label: String,
    current: u64,
    #[props(default)] total: Option<u64>,
    #[props(default = "running".to_string())] state: String,
) -> Element {
    let label_id = format!("{id}-label");
    let current = total.map(|total| current.min(total)).unwrap_or(current);
    let is_active = matches!(
        state.as_str(),
        "running" | "queued" | "reading" | "uploading" | "finalizing" | "cancelling"
    );

    rsx! {
        div { class: "semantic-job-progress", "data-state": state,
            if let Some(total) = total.filter(|total| *total > 0) {
                progress {
                    id,
                    max: "{total}",
                    value: "{current}",
                    aria_labelledby: label_id.clone(),
                }
            } else if is_active {
                progress { id, aria_labelledby: label_id.clone() }
            } else {
                progress { id, max: "1", value: "0", aria_labelledby: label_id.clone() }
            }
            span { id: label_id, class: "semantic-job-progress__label", "{label}" }
        }
    }
}
