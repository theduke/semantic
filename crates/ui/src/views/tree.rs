use dioxus::prelude::*;
use futures::FutureExt;
use semantic_data::{
    builtin::{ATTR_ID, DEFAULT_COLLECTION},
    value::Value,
};
use semantic_ui_core::{
    DirectoryActionTarget, DirectoryLocationKind, SemanticFormSubmit, SubmitError,
    context::{Toast, use_toast_dispatcher},
    create_entity_in_directory, use_active_scope_id, use_rpc_client,
};

use crate::components::{
    ConfirmActionRequest, EntityCreateFailure, EntityCreateForm, EntityCreateOutcome,
    UnsavedChangesPrompt,
};

use super::upload::UploadWorkspace;

#[derive(Clone, Debug, PartialEq, Eq)]
enum TreeDialog {
    Create(DirectoryActionTarget),
    Upload(DirectoryActionTarget),
}

#[component]
pub fn TreePage(
    root: ReadSignal<Option<String>>,
    hierarchy: ReadSignal<Option<bool>>,
    kind: ReadSignal<Option<String>>,
) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let toast = use_toast_dispatcher();
    let mut dialog = use_signal(|| None::<TreeDialog>);
    let mut create_dirty = use_signal(|| false);
    let mut dialog_busy = use_signal(|| false);
    let mut discard_confirm_open = use_signal(|| false);
    let mut create_error = use_signal(|| None::<String>);
    let mut refresh_revision = use_signal(|| 0_u64);

    let request_close = Callback::new(move |_| {
        if dialog_busy() {
            return;
        }
        if matches!(dialog(), Some(TreeDialog::Create(_))) && create_dirty() {
            discard_confirm_open.set(true);
        } else {
            dialog.set(None);
            create_error.set(None);
        }
    });

    let create_target = match dialog() {
        Some(TreeDialog::Create(target)) => Some(target),
        _ => None,
    };
    let upload_target = match dialog() {
        Some(TreeDialog::Upload(target)) => Some(target),
        _ => None,
    };
    let create_submit = create_target.as_ref().map(|target| {
        let target_directory_id = target.id.clone();
        SemanticFormSubmit::async_(move |ctx| {
            let client = client.clone();
            let scope_id = scope_id.clone();
            let target_directory_id = target_directory_id.clone();
            async move {
                let Value::Object(object) = ctx.value else {
                    return Err(SubmitError::message("submitted value must be an object"));
                };
                let id = object
                    .get(ATTR_ID)
                    .or_else(|| object.get("semantic:catalog:id"))
                    .and_then(Value::as_str)
                    .filter(|id| !id.trim().is_empty())
                    .ok_or_else(|| SubmitError::message("id is required"))?
                    .to_string();
                create_entity_in_directory(client, scope_id, target_directory_id, id, object)
                    .await
                    .map_err(SubmitError::message)
            }
            .boxed_local()
        })
    });

    rsx! {
        section { class: "semantic-tree-page semantic-route-stack",
            h1 { class: "semantic-visually-hidden", "Directory tree" }
            semantic_ui_core::DirectoryBrowser {
                root: root(),
                hierarchy: hierarchy().unwrap_or(false),
                location_kind: DirectoryLocationKind::from_query_value(kind.read().as_deref()),
                refresh_revision: refresh_revision(),
                on_create_entity: move |target| {
                    create_dirty.set(false);
                    create_error.set(None);
                    dialog.set(Some(TreeDialog::Create(target)));
                },
                on_upload_files: move |target| {
                    dialog_busy.set(false);
                    dialog.set(Some(TreeDialog::Upload(target)));
                },
            }
        }
        dxcomp::Dialog {
            class: "semantic-tree-action-dialog semantic-tree-action-dialog--create",
            open: create_target.is_some(),
            on_open_change: move |open: bool| if !open { request_close.call(()) },
            if let Some(target) = create_target {
                dxcomp::DialogTitle { "Create in {target.title}" }
                dxcomp::DialogDescription {
                    "Create an entity and link it to this directory in one operation."
                }
                if let Some(error) = create_error() {
                    div { class: "semantic-error", role: "alert", "{error}" }
                }
                if let Some(submit) = create_submit {
                    div { class: "semantic-tree-action-dialog__body",
                        EntityCreateForm {
                            fixed_collection: Some(DEFAULT_COLLECTION.to_string()),
                            submit,
                            on_dirty_change: move |next| create_dirty.set(next),
                            on_submitting_change: move |next| dialog_busy.set(next),
                            on_failure: move |failure: EntityCreateFailure| {
                                create_error.set(Some(failure.message));
                            },
                            on_created: move |_outcome: EntityCreateOutcome| {
                                create_dirty.set(false);
                                dialog_busy.set(false);
                                dialog.set(None);
                                refresh_revision.with_mut(|revision| *revision += 1);
                                toast.show(
                                    Toast::success("The entity was created and linked to the directory.")
                                        .title("Entity created"),
                                );
                            },
                        }
                    }
                }
                div { class: "semantic-directory-browser__dialog-actions",
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        disabled: dialog_busy(),
                        onclick: move |_| request_close.call(()),
                        "Cancel"
                    }
                }
            }
        }
        dxcomp::Dialog {
            class: "semantic-tree-action-dialog semantic-tree-action-dialog--upload",
            open: upload_target.is_some(),
            on_open_change: move |open: bool| if !open { request_close.call(()) },
            if let Some(target) = upload_target {
                dxcomp::DialogTitle { "Upload to {target.title}" }
                dxcomp::DialogDescription {
                    "Choose one or more files. Each successful upload is linked to this directory."
                }
                div { class: "semantic-tree-action-dialog__body",
                    UploadWorkspace {
                        destination: Some(target),
                        compact: true,
                        on_busy_change: move |busy| dialog_busy.set(busy),
                        on_directory_changed: move |_| {
                            refresh_revision.with_mut(|revision| *revision += 1);
                        },
                    }
                }
                div { class: "semantic-directory-browser__dialog-actions",
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        disabled: dialog_busy(),
                        onclick: move |_| request_close.call(()),
                        "Close"
                    }
                }
            }
        }
        UnsavedChangesPrompt {
            open: discard_confirm_open(),
            target: "directory contents",
            body: "Closing this dialog will discard the current entity draft.",
            on_open_change: move |open| discard_confirm_open.set(open),
            on_discard: move |request: ConfirmActionRequest| {
                create_dirty.set(false);
                discard_confirm_open.set(false);
                dialog.set(None);
                create_error.set(None);
                request.complete(Ok(()));
            },
        }
    }
}
