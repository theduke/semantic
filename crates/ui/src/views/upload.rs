use std::{cell::RefCell, collections::HashMap, rc::Rc};

use dioxus::{html::FileData, prelude::*};
use futures::{
    StreamExt as _,
    channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded},
    future::{AbortHandle, Abortable},
    lock::Mutex,
};
use semantic_data::filestore::{ATTR_DESCRIPTION, ATTR_PARENT, ATTR_TITLE};
use semantic_data::value::{Object, Value};
use semantic_rpc::RpcClientError;
use semantic_rpc::file::{
    FileUploadContent, FileUploadPhase, FileUploadProgress, FileUploadRequest, FileUploadResponse,
};
use semantic_ui_core::{
    EntityAutocomplete, EntityCard, EntityDisplayRenderer, EntityRenderOptions, FileTreePicker,
    FileTreeSelection, add_items_to_directory,
    components::{InlineNotice, NoticeVariant},
    use_active_scope_id, use_rpc_client,
};

use crate::components::{
    ConfirmAction, ConfirmActionRequest, ConfirmActionVariant, DropZone, JobProgress, PageHeader,
};

type QueueItemId = u64;
type UploadEntry = (QueueItemId, Signal<UploadQueueItem>);
type WorkReceiver = Rc<Mutex<UnboundedReceiver<UploadWork>>>;
type AbortRegistry = Rc<RefCell<HashMap<QueueItemId, (u64, AbortHandle)>>>;

#[derive(Clone)]
struct UploadWorkerHandle {
    sender: UnboundedSender<UploadWork>,
    aborts: AbortRegistry,
}

impl PartialEq for UploadWorkerHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.aborts, &other.aborts)
    }
}

const MAX_FILE_UPLOAD_SIZE: u64 = 100 * 1024 * 1024 * 1024;
const MAX_QUEUE_ITEMS: usize = 1_000;
const MAX_COMPLETED_HISTORY: usize = 50;
const INITIAL_VISIBLE_PER_GROUP: usize = 50;

#[derive(Clone, Debug, PartialEq)]
struct UploadQueueItem {
    id: QueueItemId,
    file: FileData,
    name: String,
    mime_type: Option<String>,
    byte_size: u64,
    title: String,
    description: String,
    status: UploadItemStatus,
    progress: Signal<Option<FileUploadProgress>>,
    error: Option<String>,
    result: Option<FileUploadResponse>,
    generation: u64,
    started_options: Option<UploadRequestOptions>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UploadItemStatus {
    Ready,
    Queued,
    Reading,
    Uploading,
    Finalizing,
    Cancelling,
    Success,
    Partial,
    Error,
    Canceled,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct UploadRequestOptions {
    destination_directory: Option<String>,
    parent_entity: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UploadWorkKind {
    Upload,
    DirectoryLink,
}

#[derive(Clone, Debug)]
struct UploadWork {
    id: QueueItemId,
    generation: u64,
    kind: UploadWorkKind,
    options: UploadRequestOptions,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct UploadSummary {
    total: usize,
    queued: usize,
    active: usize,
    attention: usize,
    completed: usize,
    uploaded_bytes: u64,
    total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct UploadNotice {
    message: String,
    variant: NoticeVariant,
}

#[component]
pub fn UploadPage() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut queue = use_signal(Vec::<UploadEntry>::new);
    let summary = use_signal(UploadSummary::default);
    let mut notice = use_signal(|| None::<UploadNotice>);
    let next_id = use_signal(|| 1_u64);
    let mut destination_directory = use_signal(|| None::<FileTreeSelection>);
    let mut parent_entity = use_signal(|| None::<String>);
    let mut settings_open = use_signal(|| false);
    let mut clear_confirm_open = use_signal(|| false);
    let mut visible_per_group = use_signal(|| INITIAL_VISIBLE_PER_GROUP);
    let aborts = use_hook(|| Rc::new(RefCell::new(HashMap::new())));
    let (work_tx, work_rx) = use_hook(|| {
        let (tx, rx) = unbounded::<UploadWork>();
        (tx, Rc::new(Mutex::new(rx)))
    });

    use_upload_worker(
        queue,
        summary,
        client.clone(),
        scope_id.clone(),
        work_rx.clone(),
        aborts.clone(),
    );
    use_upload_worker(
        queue,
        summary,
        client.clone(),
        scope_id.clone(),
        work_rx.clone(),
        aborts.clone(),
    );
    use_upload_worker(queue, summary, client, scope_id, work_rx, aborts.clone());
    let worker = UploadWorkerHandle {
        sender: work_tx.clone(),
        aborts,
    };

    let defaults = UploadRequestOptions {
        destination_directory: destination_directory
            .read()
            .as_ref()
            .map(|item| item.id.clone()),
        parent_entity: parent_entity.read().clone(),
    };
    let mut ready = Vec::new();
    let mut active = Vec::new();
    let mut attention = Vec::new();
    let mut complete = Vec::new();
    for (id, item) in queue.read().iter().copied() {
        match item.read().status {
            UploadItemStatus::Ready => ready.push((id, item)),
            UploadItemStatus::Queued
            | UploadItemStatus::Reading
            | UploadItemStatus::Uploading
            | UploadItemStatus::Finalizing
            | UploadItemStatus::Cancelling => active.push((id, item)),
            UploadItemStatus::Error | UploadItemStatus::Canceled | UploadItemStatus::Partial => {
                attention.push((id, item));
            }
            UploadItemStatus::Success => complete.push((id, item)),
        }
    }
    let busy = !active.is_empty();
    let has_more = [ready.len(), active.len(), attention.len(), complete.len()]
        .into_iter()
        .any(|count| count > visible_per_group());
    let queue_len = queue.read().len();
    let ready_to_start = ready.clone();
    let attention_to_retry = attention.clone();
    let retryable_count = attention
        .iter()
        .filter(|(_, item)| {
            matches!(
                item.read().status,
                UploadItemStatus::Error | UploadItemStatus::Canceled
            )
        })
        .count();
    let has_retryable = retryable_count > 0;
    let upload_label = if ready.len() == 1 {
        "Upload 1 file".to_string()
    } else {
        format!("Upload {} files", ready.len())
    };
    let retry_label = if retryable_count == 1 {
        "Retry 1 file".to_string()
    } else {
        format!("Retry {retryable_count} files")
    };
    rsx! {
        section { class: "semantic-upload",
            PageHeader { title: "Upload" }
            DropZone {
                id: "semantic-upload-input",
                label: "Drop files here",
                hint: "Any file type · Up to 100 GiB each",
                on_files: move |files| add_files(files, queue, summary, next_id, notice),
            }
            if queue_len > 0 {
                div { class: "semantic-upload__selection-actions",
                    if !ready.is_empty() {
                        dxcomp::Button {
                            onclick: {
                                let work_tx = work_tx.clone();
                                let defaults = defaults.clone();
                                move |_| {
                                    for (_, item) in ready_to_start.iter().copied() {
                                        start_work(item, UploadWorkKind::Upload, defaults.clone(), &work_tx);
                                    }
                                    refresh_summary(queue, summary);
                                }
                            },
                            "{upload_label}"
                        }
                    }
                    if has_retryable {
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: {
                                let work_tx = work_tx.clone();
                                let defaults = defaults.clone();
                                move |_| {
                                    for (_, item) in attention_to_retry.iter().copied() {
                                        if matches!(item.read().status, UploadItemStatus::Error | UploadItemStatus::Canceled) {
                                            start_work(item, UploadWorkKind::Upload, defaults.clone(), &work_tx);
                                        }
                                    }
                                    refresh_summary(queue, summary);
                                }
                            },
                            "{retry_label}"
                        }
                    }
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        disabled: busy,
                        onclick: move |_| clear_confirm_open.set(true),
                        "Clear all"
                    }
                }
            }
            div { class: "semantic-upload__settings-toggle",
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    aria_expanded: settings_open(),
                    aria_controls: "semantic-upload-settings",
                    onclick: move |_| settings_open.toggle(),
                    if settings_open() { "Hide metadata" } else { "Add metadata" }
                }
            }
            if settings_open() {
                section { id: "semantic-upload-settings", class: "semantic-upload__defaults",
                    div { class: "semantic-upload__defaults-header",
                        h2 { "Metadata" }
                        if destination_directory.read().is_some() || parent_entity.read().is_some() {
                            dxcomp::Button {
                                variant: dxcomp::ButtonVariant::Ghost,
                                size: dxcomp::ButtonSize::Sm,
                                onclick: move |_| {
                                    destination_directory.set(None);
                                    parent_entity.set(None);
                                },
                                "Clear"
                            }
                        }
                    }
                    div { class: "semantic-upload__defaults-grid",
                        section { class: "semantic-upload__default-section",
                            div { class: "semantic-upload__default-label",
                                strong { "Destination directory" }
                                if let Some(directory) = destination_directory.read().as_ref() {
                                    span { "Selected: {directory.title}" }
                                } else {
                                    span { "Choose a directory for uploaded files" }
                                }
                            }
                            FileTreePicker {
                                selected: destination_directory.read().as_ref().map(|item| item.id.clone()),
                                show_files: false,
                                select_directories: true,
                                select_files: false,
                                filter_placeholder: "Filter directories",
                                disabled: false,
                                on_select: move |selection| destination_directory.set(Some(selection)),
                            }
                        }
                        section { class: "semantic-upload__default-section",
                            label { class: "semantic-upload__default-field",
                                div { class: "semantic-upload__default-label",
                                    strong { "Parent entity" }
                                    span { "Link uploaded files to an entity" }
                                }
                                EntityAutocomplete {
                                    value: parent_entity.read().clone(),
                                    disabled: false,
                                    placeholder: "Search entities",
                                    aria_label: "Parent entity",
                                    on_value_change: move |value| parent_entity.set(value),
                                }
                            }
                        }
                    }
                }
            }
            if let Some(current_notice) = notice.read().clone() {
                InlineNotice {
                    message: current_notice.message,
                    variant: current_notice.variant,
                    on_dismiss: move |_| notice.set(None),
                }
            }
            if queue_len > 0 {
                UploadAggregate { summary }
                UploadGroup { title: "Ready", entries: ready, visible: visible_per_group(), defaults: defaults.clone(), worker: worker.clone(), queue, summary }
                UploadGroup { title: "In progress", entries: active, visible: visible_per_group(), defaults: defaults.clone(), worker: worker.clone(), queue, summary }
                UploadGroup { title: "Needs attention", entries: attention, visible: visible_per_group(), defaults: defaults.clone(), worker: worker.clone(), queue, summary }
                UploadGroup { title: "Complete", entries: complete, visible: visible_per_group(), defaults, worker, queue, summary }
                if has_more {
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        onclick: move |_| visible_per_group.with_mut(|limit| *limit += INITIAL_VISIBLE_PER_GROUP),
                        "Show more queue items"
                    }
                }
            }
            ConfirmAction {
                open: clear_confirm_open(),
                title: "Clear all files",
                target: format!("{queue_len} queued files"),
                body: "Uploaded files won't be deleted.",
                confirm_label: "Clear all",
                variant: ConfirmActionVariant::Danger,
                on_open_change: move |open| clear_confirm_open.set(open),
                on_confirm: move |request: ConfirmActionRequest| {
                    queue.write().clear();
                    refresh_summary(queue, summary);
                    request.complete(Ok(()));
                },
            }
        }
    }
}

#[component]
fn UploadAggregate(summary: Signal<UploadSummary>) -> Element {
    let summary = summary.read().clone();
    let mut states = Vec::new();
    if summary.active > 0 {
        states.push(format!("{} active", summary.active));
    }
    if summary.queued > 0 {
        states.push(format!("{} ready", summary.queued));
    }
    if summary.attention > 0 {
        states.push(format!("{} need attention", summary.attention));
    }
    if summary.completed > 0 {
        states.push(format!("{} complete", summary.completed));
    }
    let label = states.join(" · ");
    let file_count = if summary.total == 1 {
        "1 file".to_string()
    } else {
        format!("{} files", summary.total)
    };
    rsx! {
        section { class: "semantic-upload__aggregate", aria_label: "Upload queue summary",
            div {
                strong { "{file_count}" }
                span { "{label}" }
            }
            if summary.active > 0 {
                JobProgress {
                    id: "semantic-upload-aggregate-progress",
                    label: format!("{} of {} transferred", format_byte_size(summary.uploaded_bytes), format_byte_size(summary.total_bytes)),
                    current: summary.uploaded_bytes,
                    total: (summary.total_bytes > 0).then_some(summary.total_bytes),
                    state: "running",
                }
            }
        }
    }
}

#[component]
fn UploadGroup(
    title: String,
    entries: Vec<UploadEntry>,
    visible: usize,
    defaults: UploadRequestOptions,
    worker: UploadWorkerHandle,
    queue: Signal<Vec<UploadEntry>>,
    summary: Signal<UploadSummary>,
) -> Element {
    if entries.is_empty() {
        return rsx! {};
    }
    let count = entries.len();
    rsx! {
        section { class: "semantic-upload__group",
            h2 { "{title}" span { "{count}" } }
            div { class: "semantic-upload__queue",
                for (id, item) in entries.into_iter().take(visible) {
                    UploadQueueRow {
                        key: "{id}",
                        item,
                        defaults: defaults.clone(),
                        worker: worker.clone(),
                        queue,
                        summary,
                    }
                }
            }
        }
    }
}

#[component]
fn UploadQueueRow(
    item: Signal<UploadQueueItem>,
    defaults: UploadRequestOptions,
    worker: UploadWorkerHandle,
    queue: Signal<Vec<UploadEntry>>,
    summary: Signal<UploadSummary>,
) -> Element {
    let mut metadata_open = use_signal(|| false);
    let snapshot = item.read().clone();
    let id = snapshot.id;
    let status = snapshot.status;
    let busy = status.is_busy();
    let has_result = snapshot.result.is_some();
    let can_cancel = upload_can_be_canceled(status, has_result);
    let title_id = format!("upload-{id}-title");
    let description_id = format!("upload-{id}-description");
    let progress = snapshot.progress;
    let progress_snapshot = progress.read().clone();
    let (progress_label, progress_current, progress_total) =
        progress_details(status, progress_snapshot.as_ref());
    let mime_label = snapshot
        .mime_type
        .clone()
        .unwrap_or_else(|| "Unknown type".to_string());

    rsx! {
        article { class: "semantic-upload__item", "data-status": status.slug(),
            div { class: "semantic-upload__item-main",
                div { class: "semantic-upload__item-heading",
                    strong { title: snapshot.name.clone(), "{snapshot.name}" }
                    if !matches!(status, UploadItemStatus::Ready | UploadItemStatus::Success) {
                        span { class: "semantic-upload__status", "data-status": status.slug(), "{status.label()}" }
                    }
                }
                div { class: "semantic-upload__item-meta",
                    span { "{mime_label}" }
                    span { "{format_byte_size(snapshot.byte_size)}" }
                    if let Some(options) = snapshot.started_options.as_ref() {
                        if options.destination_directory.is_some() || options.parent_entity.is_some() {
                            span { "Organization applied" }
                        }
                    }
                }
                if metadata_open() {
                    div { id: "upload-{id}-metadata", class: "semantic-upload__metadata",
                        label { r#for: title_id.clone(),
                            span { "Title" }
                            input {
                                id: title_id,
                                value: snapshot.title,
                                disabled: busy || matches!(status, UploadItemStatus::Success | UploadItemStatus::Partial),
                                oninput: move |event| item.write().title = event.value(),
                            }
                        }
                        label { r#for: description_id.clone(),
                            span { "Description" }
                            textarea {
                                id: description_id,
                                value: snapshot.description,
                                disabled: busy || matches!(status, UploadItemStatus::Success | UploadItemStatus::Partial),
                                placeholder: "Optional",
                                oninput: move |event| item.write().description = event.value(),
                            }
                        }
                    }
                }
                if busy {
                    JobProgress {
                        id: format!("upload-{id}-progress"),
                        label: progress_label,
                        current: progress_current,
                        total: progress_total,
                        state: status.slug(),
                    }
                }
                if let Some(error) = snapshot.error {
                    InlineNotice { message: error, variant: if status == UploadItemStatus::Partial { NoticeVariant::Warning } else { NoticeVariant::Error } }
                }
                if has_result && matches!(status, UploadItemStatus::Success | UploadItemStatus::Partial) {
                    UploadResultPreview { item }
                }
            }
            div { class: "semantic-upload__item-actions",
                if can_cancel {
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::Sm,
                        disabled: status == UploadItemStatus::Cancelling,
                        onclick: move |_| cancel_item(item, &worker.aborts, queue, summary),
                        if status == UploadItemStatus::Cancelling { "Cancelling…" } else { "Cancel" }
                    }
                } else if status == UploadItemStatus::Partial {
                    dxcomp::Button {
                        size: dxcomp::ButtonSize::Sm,
                        onclick: {
                            let sender = worker.sender.clone();
                            move |_| start_work(item, UploadWorkKind::DirectoryLink, snapshot.started_options.clone().unwrap_or_default(), &sender)
                        },
                        "Retry organization"
                    }
                } else if matches!(status, UploadItemStatus::Ready | UploadItemStatus::Error | UploadItemStatus::Canceled) {
                    dxcomp::Button {
                        size: dxcomp::ButtonSize::Sm,
                        onclick: {
                            let sender = worker.sender.clone();
                            move |_| start_work(item, UploadWorkKind::Upload, defaults.clone(), &sender)
                        },
                        if status == UploadItemStatus::Ready { "Upload" } else { "Retry" }
                    }
                }
                if metadata_open() || (!busy && !matches!(status, UploadItemStatus::Success | UploadItemStatus::Partial)) {
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        size: dxcomp::ButtonSize::Sm,
                        aria_expanded: metadata_open(),
                        aria_controls: "upload-{id}-metadata",
                        onclick: move |_| metadata_open.toggle(),
                        if metadata_open() { "Hide details" } else { "Edit details" }
                    }
                }
                if !busy {
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Ghost,
                        size: dxcomp::ButtonSize::Sm,
                        onclick: move |_| {
                            queue.write().retain(|(entry_id, _)| *entry_id != id);
                            refresh_summary(queue, summary);
                        },
                        "Remove"
                    }
                }
            }
        }
    }
}

#[component]
fn UploadResultPreview(item: Signal<UploadQueueItem>) -> Element {
    let Some(result) = item.read().result.clone() else {
        return rsx! {};
    };
    rsx! {
        EntityCard {
            object: result.object,
            options: EntityRenderOptions {
                collection: Some(result.collection),
                id: Some(result.id),
                renderer: EntityDisplayRenderer::Custom,
                preview: true,
                actions: true,
            }
        }
    }
}

fn use_upload_worker(
    queue: Signal<Vec<UploadEntry>>,
    summary: Signal<UploadSummary>,
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    receiver: WorkReceiver,
    aborts: AbortRegistry,
) {
    use_future(move || {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let receiver = receiver.clone();
        let aborts = aborts.clone();
        async move {
            loop {
                let work = {
                    let mut receiver = receiver.lock().await;
                    receiver.next().await
                };
                let Some(work) = work else { break };
                let Some(mut item) = find_item(queue, work.id) else {
                    continue;
                };
                if item.read().generation != work.generation {
                    continue;
                }
                let (abort_handle, registration) = AbortHandle::new_pair();
                aborts
                    .borrow_mut()
                    .insert(work.id, (work.generation, abort_handle));
                let operation = match work.kind {
                    UploadWorkKind::Upload => {
                        Abortable::new(
                            upload_item(
                                item,
                                queue,
                                summary,
                                client.clone(),
                                scope_id.clone(),
                                work.options,
                            ),
                            registration,
                        )
                        .await
                    }
                    UploadWorkKind::DirectoryLink => {
                        Abortable::new(
                            retry_directory_link(
                                item,
                                queue,
                                summary,
                                client.clone(),
                                scope_id.clone(),
                                work.options,
                            ),
                            registration,
                        )
                        .await
                    }
                };
                if aborts
                    .borrow()
                    .get(&work.id)
                    .is_some_and(|(generation, _)| *generation == work.generation)
                {
                    aborts.borrow_mut().remove(&work.id);
                }
                if operation.is_err() && item.read().generation == work.generation {
                    let mut item = item.write();
                    item.status = UploadItemStatus::Canceled;
                    item.error = None;
                }
                prune_completed_history(queue);
                refresh_summary(queue, summary);
            }
        }
    });
}

async fn upload_item(
    mut item: Signal<UploadQueueItem>,
    queue: Signal<Vec<UploadEntry>>,
    summary: Signal<UploadSummary>,
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    options: UploadRequestOptions,
) {
    let snapshot = {
        let mut item = item.write();
        item.status = UploadItemStatus::Reading;
        item.error = None;
        item.result = None;
        item.started_options = Some(options.clone());
        let byte_size = item.byte_size;
        let mut progress = item.progress;
        progress.set(Some(FileUploadProgress {
            uploaded_bytes: 0,
            total_bytes: Some(byte_size),
            phase: FileUploadPhase::Preparing,
        }));
        item.clone()
    };
    refresh_summary(queue, summary);
    let content = match file_upload_content(&snapshot.file) {
        Ok(content) => content,
        Err(error) => {
            set_failed(item, error.to_string());
            return;
        }
    };
    let (progress_tx, mut progress_rx) = unbounded::<FileUploadProgress>();
    let mut progress_signal = snapshot.progress;
    let mut progress_item = item;
    spawn(async move {
        while let Some(progress) = progress_rx.next().await {
            if progress_item.read().status == UploadItemStatus::Cancelling {
                continue;
            }
            match progress.phase {
                FileUploadPhase::Preparing => {
                    progress_item.write().status = UploadItemStatus::Reading
                }
                FileUploadPhase::Uploading => {
                    progress_item.write().status = UploadItemStatus::Uploading
                }
                FileUploadPhase::Finalizing => {
                    progress_item.write().status = UploadItemStatus::Finalizing
                }
                FileUploadPhase::Done => {}
            }
            progress_signal.set(Some(progress));
            refresh_summary(queue, summary);
        }
    });
    let request = FileUploadRequest {
        scope_id: scope_id.clone(),
        id: None,
        filename: Some(snapshot.name.clone()),
        mime_type: detect_mime_type(&snapshot.name, snapshot.mime_type.as_deref(), &[]),
        entity: metadata_entity(
            &snapshot.title,
            &snapshot.description,
            options.parent_entity.as_deref(),
        ),
        content,
    };
    match client.upload_file(request, Some(progress_tx)).await {
        Ok(response) => {
            item.write().result = Some(response.clone());
            if let Some(directory_id) = options.destination_directory {
                item.write().status = UploadItemStatus::Finalizing;
                refresh_summary(queue, summary);
                match add_items_to_directory(client, scope_id, directory_id, vec![response.id])
                    .await
                {
                    Ok(()) => set_succeeded(item),
                    Err(error) => {
                        let mut item = item.write();
                        item.status = UploadItemStatus::Partial;
                        item.error = Some(format!(
                            "File uploaded, but couldn't be added to the directory: {error}"
                        ));
                    }
                }
            } else {
                set_succeeded(item);
            }
        }
        Err(error) => set_failed(item, error.to_string()),
    }
}

async fn retry_directory_link(
    mut item: Signal<UploadQueueItem>,
    queue: Signal<Vec<UploadEntry>>,
    summary: Signal<UploadSummary>,
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    options: UploadRequestOptions,
) {
    let (Some(directory_id), Some(response)) =
        (options.destination_directory, item.read().result.clone())
    else {
        set_failed(
            item,
            "The directory or uploaded file is no longer available.".to_string(),
        );
        return;
    };
    item.write().status = UploadItemStatus::Finalizing;
    refresh_summary(queue, summary);
    match add_items_to_directory(client, scope_id, directory_id, vec![response.id]).await {
        Ok(()) => set_succeeded(item),
        Err(error) => {
            let mut item = item.write();
            item.status = UploadItemStatus::Partial;
            item.error = Some(format!(
                "File uploaded, but still couldn't be added to the directory: {error}"
            ));
        }
    }
}

fn set_succeeded(mut item: Signal<UploadQueueItem>) {
    item.write().status = UploadItemStatus::Success;
    item.write().error = None;
    let byte_size = item.read().byte_size;
    let mut progress = item.read().progress;
    progress.set(Some(FileUploadProgress {
        uploaded_bytes: byte_size,
        total_bytes: Some(byte_size),
        phase: FileUploadPhase::Done,
    }));
}

fn set_failed(mut item: Signal<UploadQueueItem>, error: String) {
    item.write().status = UploadItemStatus::Error;
    item.write().error = Some(error);
}

fn same_file(item: &UploadQueueItem, file: &FileData) -> bool {
    item.name == file.name()
        && item.byte_size == file.size()
        && item.file.last_modified() == file.last_modified()
        && item.file.path() == file.path()
}

fn normalize_mime_type(file: &FileData) -> Option<String> {
    detect_mime_type(&file.name(), file.content_type().as_deref(), &[])
}

impl UploadItemStatus {
    fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Reading | Self::Uploading | Self::Finalizing | Self::Cancelling
        )
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Queued => "Queued",
            Self::Reading => "Reading",
            Self::Uploading => "Uploading",
            Self::Finalizing => "Finalizing",
            Self::Cancelling => "Cancelling",
            Self::Success => "Complete",
            Self::Partial => "Not in directory",
            Self::Error => "Failed",
            Self::Canceled => "Canceled",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Queued => "queued",
            Self::Reading => "reading",
            Self::Uploading => "uploading",
            Self::Finalizing => "finalizing",
            Self::Cancelling => "cancelling",
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Error => "error",
            Self::Canceled => "canceled",
        }
    }
}

fn upload_can_be_canceled(status: UploadItemStatus, has_result: bool) -> bool {
    status.is_busy() && !has_result
}

fn add_files(
    files: Vec<FileData>,
    mut queue: Signal<Vec<UploadEntry>>,
    summary: Signal<UploadSummary>,
    mut next_id: Signal<QueueItemId>,
    mut notice: Signal<Option<UploadNotice>>,
) {
    let mut added = 0;
    let mut duplicates = 0;
    let mut oversized = 0;
    let mut invalid = 0;
    let mut full = 0;

    for file in files {
        if queue.read().len() >= MAX_QUEUE_ITEMS {
            full += 1;
            continue;
        }
        if file.name().trim().is_empty() {
            invalid += 1;
            continue;
        }
        if file.size() > MAX_FILE_UPLOAD_SIZE {
            oversized += 1;
            continue;
        }
        if queue
            .read()
            .iter()
            .any(|(_, item)| same_file(&item.read(), &file))
        {
            duplicates += 1;
            continue;
        }

        let id = *next_id.read();
        next_id.set(id.saturating_add(1));
        let name = file.name();
        let byte_size = file.size();
        let item = Signal::new(UploadQueueItem {
            id,
            mime_type: normalize_mime_type(&file),
            title: title_from_filename(&name),
            file,
            name,
            byte_size,
            description: String::new(),
            status: UploadItemStatus::Ready,
            progress: Signal::new(None),
            error: None,
            result: None,
            generation: 0,
            started_options: None,
        });
        queue.write().push((id, item));
        added += 1;
    }

    refresh_summary(queue, summary);
    notice.set(if duplicates + oversized + invalid + full == 0 {
        None
    } else {
        let mut reasons = Vec::new();
        if duplicates > 0 {
            reasons.push(format!(
                "{duplicates} duplicate{}",
                if duplicates == 1 { "" } else { "s" }
            ));
        }
        if oversized > 0 {
            reasons.push(format!("{oversized} over the 100 GiB limit"));
        }
        if invalid > 0 {
            reasons.push(format!("{invalid} without a valid filename"));
        }
        if full > 0 {
            reasons.push(format!(
                "{full} beyond the {MAX_QUEUE_ITEMS}-item queue limit"
            ));
        }
        let added = (added > 0)
            .then(|| format!("{added} added. "))
            .unwrap_or_default();
        Some(UploadNotice {
            message: format!("{added}Skipped {}.", reasons.join(", ")),
            variant: NoticeVariant::Warning,
        })
    });
}

fn start_work(
    mut item: Signal<UploadQueueItem>,
    kind: UploadWorkKind,
    options: UploadRequestOptions,
    work_tx: &UnboundedSender<UploadWork>,
) {
    let work = {
        let mut item = item.write();
        let can_start = match kind {
            UploadWorkKind::Upload => matches!(
                item.status,
                UploadItemStatus::Ready | UploadItemStatus::Error | UploadItemStatus::Canceled
            ),
            UploadWorkKind::DirectoryLink => item.status == UploadItemStatus::Partial,
        };
        if !can_start {
            return;
        }
        item.generation = item.generation.saturating_add(1);
        item.status = match kind {
            UploadWorkKind::Upload => UploadItemStatus::Queued,
            UploadWorkKind::DirectoryLink => UploadItemStatus::Finalizing,
        };
        item.error = None;
        item.started_options = Some(options.clone());
        UploadWork {
            id: item.id,
            generation: item.generation,
            kind,
            options,
        }
    };
    if work_tx.unbounded_send(work).is_err() {
        set_failed(
            item,
            "Uploads are unavailable. Reload and try again.".to_string(),
        );
    }
}

fn cancel_item(
    mut item: Signal<UploadQueueItem>,
    aborts: &AbortRegistry,
    queue: Signal<Vec<UploadEntry>>,
    summary: Signal<UploadSummary>,
) {
    let generation = item.read().generation;
    let aborting = aborts
        .borrow()
        .get(&item.read().id)
        .filter(|(active_generation, _)| *active_generation == generation)
        .map(|(_, handle)| {
            handle.abort();
        })
        .is_some();
    if aborting {
        item.write().status = UploadItemStatus::Cancelling;
    } else {
        let mut item = item.write();
        item.generation = item.generation.saturating_add(1);
        item.status = UploadItemStatus::Canceled;
        item.error = None;
    }
    refresh_summary(queue, summary);
}

fn find_item(queue: Signal<Vec<UploadEntry>>, id: QueueItemId) -> Option<Signal<UploadQueueItem>> {
    queue
        .read()
        .iter()
        .find_map(|(entry_id, item)| (*entry_id == id).then_some(*item))
}

fn refresh_summary(queue: Signal<Vec<UploadEntry>>, mut summary: Signal<UploadSummary>) {
    let mut next = UploadSummary::default();
    for (_, item) in queue.read().iter() {
        let item = item.read();
        next.total += 1;
        next.total_bytes = next.total_bytes.saturating_add(item.byte_size);
        match item.status {
            UploadItemStatus::Ready | UploadItemStatus::Queued => next.queued += 1,
            UploadItemStatus::Reading
            | UploadItemStatus::Uploading
            | UploadItemStatus::Finalizing
            | UploadItemStatus::Cancelling => next.active += 1,
            UploadItemStatus::Error | UploadItemStatus::Canceled | UploadItemStatus::Partial => {
                next.attention += 1;
            }
            UploadItemStatus::Success => next.completed += 1,
        }
        let uploaded = item
            .progress
            .read()
            .as_ref()
            .map(|progress| progress.uploaded_bytes)
            .unwrap_or(0)
            .min(item.byte_size);
        next.uploaded_bytes = next.uploaded_bytes.saturating_add(uploaded);
    }
    if *summary.peek() != next {
        summary.set(next);
    }
}

fn prune_completed_history(mut queue: Signal<Vec<UploadEntry>>) {
    let completed = queue
        .read()
        .iter()
        .filter(|(_, item)| item.read().status == UploadItemStatus::Success)
        .count();
    let mut remove = completed.saturating_sub(MAX_COMPLETED_HISTORY);
    if remove == 0 {
        return;
    }
    queue.write().retain(|(_, item)| {
        if remove > 0 && item.read().status == UploadItemStatus::Success {
            remove -= 1;
            false
        } else {
            true
        }
    });
}

fn progress_details(
    status: UploadItemStatus,
    progress: Option<&FileUploadProgress>,
) -> (String, u64, Option<u64>) {
    let current = progress
        .map(|progress| progress.uploaded_bytes)
        .unwrap_or(0);
    let total = progress.and_then(|progress| progress.total_bytes);
    let label = match (status, total) {
        (UploadItemStatus::Uploading, Some(total)) if total > 0 => {
            format!("Uploading {}%", current.saturating_mul(100) / total)
        }
        _ => status.label().to_string(),
    };
    (label, current, total)
}

#[cfg(not(target_arch = "wasm32"))]
fn file_upload_content(file: &FileData) -> std::result::Result<FileUploadContent, RpcClientError> {
    let path = file.path();
    let size = Some(file.size());
    let stream = futures::stream::try_unfold(
        None::<tokio_util::io::ReaderStream<tokio::fs::File>>,
        move |reader| {
            let path = path.clone();
            async move {
                let mut reader = match reader {
                    Some(reader) => reader,
                    None => tokio_util::io::ReaderStream::new(
                        tokio::fs::File::open(&path)
                            .await
                            .map_err(|err| RpcClientError::Transport(err.to_string()))?,
                    ),
                };
                match reader.next().await {
                    Some(Ok(chunk)) => Ok(Some((chunk, Some(reader)))),
                    Some(Err(err)) => Err(RpcClientError::Transport(err.to_string())),
                    None => Ok(None),
                }
            }
        },
    )
    .boxed();
    Ok(FileUploadContent::Stream { stream, size })
}

#[cfg(target_arch = "wasm32")]
fn file_upload_content(file: &FileData) -> std::result::Result<FileUploadContent, RpcClientError> {
    use wasm_bindgen::JsCast as _;

    let file = file
        .inner()
        .downcast_ref::<web_sys::File>()
        .ok_or_else(|| RpcClientError::Transport("browser file handle is unavailable".to_string()))?
        .clone();
    Ok(FileUploadContent::Blob(file.unchecked_into()))
}

fn detect_mime_type(name: &str, declared_mime_type: Option<&str>, bytes: &[u8]) -> Option<String> {
    infer::get(bytes)
        .map(|kind| kind.mime_type().to_string())
        .or_else(|| mime_from_name(name).map(str::to_string))
        .or_else(|| normalize_declared_mime_type(declared_mime_type))
}

fn normalize_declared_mime_type(mime_type: Option<&str>) -> Option<String> {
    mime_type
        .map(str::trim)
        .filter(|mime| !mime.is_empty())
        .filter(|mime| !is_unhelpful_file_mime(mime))
        .map(ToOwned::to_owned)
}

fn is_unhelpful_file_mime(mime: &str) -> bool {
    let mime = mime.trim().to_ascii_lowercase();
    mime == "application/octet-stream" || mime.starts_with("text/html")
}

fn mime_from_name(name: &str) -> Option<&'static str> {
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("avif") => Some("image/avif"),
        Some("svg") => Some("image/svg+xml"),
        _ => None,
    }
}

fn title_from_filename(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(name)
        .to_string()
}

fn metadata_entity(title: &str, description: &str, parent_entity: Option<&str>) -> Object {
    let mut entity = Object::new();
    let title = title.trim();
    if !title.is_empty() {
        entity.insert(ATTR_TITLE, Value::String(title.to_string()));
    }
    let description = description.trim();
    if !description.is_empty() {
        entity.insert(ATTR_DESCRIPTION, Value::String(description.to_string()));
    }
    if let Some(parent_entity) = parent_entity.map(str::trim).filter(|id| !id.is_empty()) {
        entity.insert(ATTR_PARENT, Value::String(parent_entity.to_string()));
    }
    entity
}

fn format_byte_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_from_name_infers_common_image_extensions() {
        assert_eq!(mime_from_name("photo.png"), Some("image/png"));
        assert_eq!(mime_from_name("photo.WEBP"), Some("image/webp"));
        assert_eq!(mime_from_name("notes.txt"), None);
    }

    #[test]
    fn detect_mime_type_prefers_binary_signature() {
        let bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert_eq!(
            detect_mime_type("download.bin", Some("text/html; charset=utf-8"), bytes),
            Some("image/png".to_string())
        );
    }

    #[test]
    fn detect_mime_type_falls_back_to_extension_before_declared_type() {
        assert_eq!(
            detect_mime_type("photo.png", Some("application/octet-stream"), &[]),
            Some("image/png".to_string())
        );
    }

    #[test]
    fn detect_mime_type_uses_helpful_declared_type_last() {
        assert_eq!(
            detect_mime_type("archive.custom", Some("application/zip"), &[]),
            Some("application/zip".to_string())
        );
    }

    #[test]
    fn metadata_entity_sets_namespaced_user_fields() {
        let entity = metadata_entity(" Title ", " Body ", Some("parent-id"));
        assert_eq!(
            entity.get(ATTR_TITLE),
            Some(&Value::String("Title".to_string()))
        );
        assert_eq!(
            entity.get(ATTR_DESCRIPTION),
            Some(&Value::String("Body".to_string()))
        );
        assert_eq!(
            entity.get(ATTR_PARENT),
            Some(&Value::String("parent-id".to_string()))
        );
        assert!(!entity.contains_key("title"));
        assert!(!entity.contains_key("description"));
        assert!(!entity.contains_key("filestore_locator"));
        assert!(!entity.contains_key("filename"));
    }

    #[test]
    fn unhelpful_file_mime_is_detected() {
        assert!(is_unhelpful_file_mime("application/octet-stream"));
        assert!(is_unhelpful_file_mime("text/html; charset=utf-8"));
        assert!(!is_unhelpful_file_mime("image/png"));
        assert!(!is_unhelpful_file_mime("text/plain"));
    }

    #[test]
    fn byte_size_formats_compactly() {
        assert_eq!(format_byte_size(12), "12 B");
        assert_eq!(format_byte_size(1536), "1.5 KB");
    }

    #[test]
    fn completed_or_materialized_upload_cannot_be_canceled() {
        assert!(upload_can_be_canceled(UploadItemStatus::Uploading, false));
        assert!(!upload_can_be_canceled(UploadItemStatus::Finalizing, true));
        assert!(!upload_can_be_canceled(UploadItemStatus::Success, true));
    }
}
