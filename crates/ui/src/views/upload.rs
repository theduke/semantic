use dioxus::html::FileData;
use dioxus::prelude::*;
use futures::StreamExt as _;
use semantic_data::filestore::{DESCRIPTION_ATTRIBUTE_ID, TITLE_ATTRIBUTE_ID};
use semantic_data::value::{Object, Value};
use semantic_rpc::file::{
    FileUploadPhase, FileUploadProgress, FileUploadRequest, FileUploadResponse,
};
use semantic_ui_core::{
    ClassView, ObjectView, RenderMode, use_active_scope_id, use_rpc_client, use_ui_catalog,
};

use crate::views::Route;

type QueueItemId = u64;

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
    progress: Option<FileUploadProgress>,
    error: Option<String>,
    result: Option<FileUploadResponse>,
}

#[derive(Clone, Debug, PartialEq)]
enum UploadItemStatus {
    Queued,
    Reading,
    Uploading,
    Done,
    Error,
    Removed,
}

#[allow(dead_code)]
enum UploadCommand {
    AddFiles(Vec<FileData>),
    Remove(QueueItemId),
    ClearFinished,
    ClearAll,
    UpdateMetadata {
        id: QueueItemId,
        title: String,
        description: String,
    },
    UploadOne(QueueItemId),
    UploadAll,
    Progress {
        id: QueueItemId,
        progress: FileUploadProgress,
    },
    Completed {
        id: QueueItemId,
        response: FileUploadResponse,
    },
    Failed {
        id: QueueItemId,
        error: String,
    },
}

#[component]
pub fn UploadPage() -> Element {
    let client = use_rpc_client();
    let result_client = client.clone();
    let scope_id = use_active_scope_id();
    let catalog = use_ui_catalog();
    let mut queue = use_signal(Vec::<UploadQueueItem>::new);
    let mut notice = use_signal(|| None::<String>);
    let mut next_id = use_signal(|| 1_u64);

    let commands = use_coroutine(move |mut rx: UnboundedReceiver<UploadCommand>| {
        let client = client.clone();
        let scope_id = scope_id.clone();
        async move {
            while let Some(command) = rx.next().await {
                match command {
                    UploadCommand::AddFiles(files) => {
                        let mut queue_write = queue.write();
                        for file in files {
                            let name = file.name();
                            let mime_type = normalize_mime_type(&file);
                            if queue_write.iter().any(|item| same_file(item, &file)) {
                                continue;
                            }
                            let id = *next_id.read();
                            next_id.set(id + 1);
                            let title = title_from_filename(&name);
                            let byte_size = file.size();
                            queue_write.push(UploadQueueItem {
                                id,
                                file,
                                name,
                                mime_type,
                                byte_size,
                                title,
                                description: String::new(),
                                status: UploadItemStatus::Queued,
                                progress: None,
                                error: None,
                                result: None,
                            });
                        }
                        drop(queue_write);
                        notice.set(None);
                    }
                    UploadCommand::Remove(id) => {
                        if !is_busy(&queue.read(), id) {
                            mark_removed_or_drop(&mut queue.write(), id);
                        }
                    }
                    UploadCommand::ClearFinished => {
                        queue.write().retain(|item| {
                            !matches!(
                                item.status,
                                UploadItemStatus::Done | UploadItemStatus::Removed
                            )
                        });
                    }
                    UploadCommand::ClearAll => {
                        if !queue.read().iter().any(|item| {
                            matches!(
                                item.status,
                                UploadItemStatus::Reading | UploadItemStatus::Uploading
                            )
                        }) {
                            queue.write().clear();
                        }
                    }
                    UploadCommand::UpdateMetadata {
                        id,
                        title,
                        description,
                    } => {
                        if let Some(item) = queue.write().iter_mut().find(|item| item.id == id) {
                            item.title = title;
                            item.description = description;
                        }
                    }
                    UploadCommand::UploadOne(id) => {
                        upload_item(id, queue, client.clone(), scope_id.clone()).await;
                    }
                    UploadCommand::UploadAll => {
                        let ids = queue
                            .read()
                            .iter()
                            .filter(|item| {
                                matches!(
                                    item.status,
                                    UploadItemStatus::Queued | UploadItemStatus::Error
                                )
                            })
                            .map(|item| item.id)
                            .collect::<Vec<_>>();
                        for id in ids {
                            upload_item(id, queue, client.clone(), scope_id.clone()).await;
                        }
                    }
                    UploadCommand::Progress { id, progress } => {
                        if let Some(item) = queue.write().iter_mut().find(|item| item.id == id) {
                            item.status = match progress.phase {
                                FileUploadPhase::Preparing => UploadItemStatus::Reading,
                                FileUploadPhase::Uploading | FileUploadPhase::Finalizing => {
                                    UploadItemStatus::Uploading
                                }
                                FileUploadPhase::Done => UploadItemStatus::Done,
                            };
                            item.progress = Some(progress);
                        }
                    }
                    UploadCommand::Completed { id, response } => {
                        if let Some(item) = queue.write().iter_mut().find(|item| item.id == id) {
                            item.status = UploadItemStatus::Done;
                            item.error = None;
                            item.result = Some(response);
                        }
                    }
                    UploadCommand::Failed { id, error } => {
                        if let Some(item) = queue.write().iter_mut().find(|item| item.id == id) {
                            item.status = UploadItemStatus::Error;
                            item.error = Some(error);
                        }
                    }
                }
            }
        }
    });

    let busy = queue.read().iter().any(|item| {
        matches!(
            item.status,
            UploadItemStatus::Reading | UploadItemStatus::Uploading
        )
    });
    let results = queue
        .read()
        .iter()
        .filter_map(|item| item.result.clone())
        .collect::<Vec<_>>();

    rsx! {
        section { class: "semantic-upload",
            div { class: "semantic-upload__header",
                h2 { "Upload Files" }
                div { class: "semantic-upload__actions",
                    dxcomp::Button {
                        disabled: busy,
                        onclick: move |_| commands.send(UploadCommand::UploadAll),
                        "Upload all"
                    }
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        disabled: busy,
                        onclick: move |_| commands.send(UploadCommand::ClearFinished),
                        "Clear finished"
                    }
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        disabled: busy,
                        onclick: move |_| commands.send(UploadCommand::ClearAll),
                        "Clear queue"
                    }
                }
            }
            label { class: "semantic-upload__selector",
                input {
                    r#type: "file",
                    multiple: true,
                    onchange: move |event| commands.send(UploadCommand::AddFiles(event.files()))
                }
            }
            if let Some(message) = notice.read().clone() {
                div { class: "semantic-upload__error", "{message}" }
            }
            div { class: "semantic-upload__queue",
                for item in queue.read().iter().filter(|item| item.status != UploadItemStatus::Removed) {
                    UploadQueueRow { item: item.clone(), commands }
                }
            }
            if !results.is_empty() {
                div { class: "semantic-upload__results",
                    h3 { "Created Files" }
                    for result in results {
                        div { class: "semantic-upload__result",
                            div { class: "semantic-upload__result-actions",
                                Link {
                                    to: Route::EntityPage {
                                        collection: result.collection.clone(),
                                        id: result.id.clone()
                                    },
                                    "Entity"
                                }
                                if let Some(url) = result_client.file_url(&result.id) {
                                    a {
                                        href: "{url}",
                                        target: "_blank",
                                        rel: "noopener noreferrer",
                                        "File"
                                    }
                                }
                            }
                            {
                                let class = catalog.object_class(&result.object).cloned();
                                rsx! {
                                    if let Some(class) = class {
                                        ClassView {
                                            class,
                                            object: with_id(result.object.clone(), &result.id),
                                            collection: Some(result.collection.clone()),
                                            id: Some(result.id.clone()),
                                            mode: RenderMode::Preview
                                        }
                                    } else {
                                        ObjectView {
                                            object: with_id(result.object.clone(), &result.id),
                                            mode: RenderMode::Preview
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn UploadQueueRow(item: UploadQueueItem, commands: Coroutine<UploadCommand>) -> Element {
    let id = item.id;
    let disabled = matches!(
        item.status,
        UploadItemStatus::Reading | UploadItemStatus::Uploading
    );
    let progress = item.progress.clone();
    let title = item.title.clone();
    let description = item.description.clone();
    rsx! {
        div { class: "semantic-upload__item",
            div { class: "semantic-upload__item-main",
                div {
                    strong { "{item.name}" }
                    div { class: "semantic-upload__item-meta",
                        span { "{item.mime_type.clone().unwrap_or_else(|| \"application/octet-stream\".to_string())}" }
                        span { "{format_byte_size(item.byte_size)}" }
                        span { "{status_label(&item.status)}" }
                    }
                }
                div { class: "semantic-upload__metadata",
                    input {
                        value: "{title}",
                        placeholder: "Title",
                        disabled,
                        oninput: {
                            let description = description.clone();
                            move |event: FormEvent| {
                                commands.send(UploadCommand::UpdateMetadata {
                                    id,
                                    title: event.value(),
                                    description: description.clone(),
                                });
                            }
                        }
                    }
                    textarea {
                        value: "{description}",
                        placeholder: "Description",
                        disabled,
                        oninput: {
                            let title = title.clone();
                            move |event: FormEvent| {
                                commands.send(UploadCommand::UpdateMetadata {
                                    id,
                                    title: title.clone(),
                                    description: event.value(),
                                });
                            }
                        }
                    }
                }
                ProgressView { progress }
                if let Some(error) = item.error.clone() {
                    div { class: "semantic-upload__error", "{error}" }
                }
            }
            div { class: "semantic-upload__item-actions",
                dxcomp::Button {
                    size: dxcomp::ButtonSize::Sm,
                    disabled,
                    onclick: move |_| commands.send(UploadCommand::UploadOne(id)),
                    if item.status == UploadItemStatus::Error { "Retry" } else { "Upload" }
                }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    size: dxcomp::ButtonSize::Sm,
                    disabled,
                    onclick: move |_| commands.send(UploadCommand::Remove(id)),
                    "Remove"
                }
            }
        }
    }
}

#[component]
fn ProgressView(progress: Option<FileUploadProgress>) -> Element {
    let (label, percent) = match progress {
        Some(progress) => {
            let label = match progress.phase {
                FileUploadPhase::Preparing => "Preparing".to_string(),
                FileUploadPhase::Uploading => match progress.total_bytes {
                    Some(total) if total > 0 => {
                        format!(
                            "Uploading {}%",
                            progress.uploaded_bytes.saturating_mul(100) / total
                        )
                    }
                    _ => "Uploading".to_string(),
                },
                FileUploadPhase::Finalizing => "Finalizing".to_string(),
                FileUploadPhase::Done => "Done".to_string(),
            };
            let percent = progress
                .total_bytes
                .filter(|total| *total > 0)
                .map(|total| (progress.uploaded_bytes as f64 / total as f64 * 100.0).min(100.0));
            (label, percent)
        }
        None => ("Queued".to_string(), Some(0.0)),
    };
    rsx! {
        div { class: "semantic-upload__progress",
            div {
                class: "semantic-upload__progress-bar",
                style: percent.map(|percent| format!("width: {percent:.0}%")).unwrap_or_default()
            }
            span { "{label}" }
        }
    }
}

async fn upload_item(
    id: QueueItemId,
    mut queue: Signal<Vec<UploadQueueItem>>,
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
) {
    let item = {
        let mut queue = queue.write();
        let Some(item) = queue.iter_mut().find(|item| item.id == id) else {
            return;
        };
        if matches!(
            item.status,
            UploadItemStatus::Reading | UploadItemStatus::Uploading | UploadItemStatus::Done
        ) {
            return;
        }
        item.status = UploadItemStatus::Reading;
        item.error = None;
        item.progress = Some(FileUploadProgress {
            uploaded_bytes: 0,
            total_bytes: Some(item.byte_size),
            phase: FileUploadPhase::Preparing,
        });
        item.clone()
    };
    let bytes = match item.file.read_bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            set_failed(queue, id, err.to_string());
            return;
        }
    };
    let (progress_tx, mut progress_rx) = futures::channel::mpsc::unbounded::<FileUploadProgress>();
    let mut progress_queue = queue;
    spawn(async move {
        while let Some(progress) = progress_rx.next().await {
            if let Some(item) = progress_queue.write().iter_mut().find(|item| item.id == id) {
                item.status = match progress.phase {
                    FileUploadPhase::Preparing => UploadItemStatus::Reading,
                    FileUploadPhase::Uploading | FileUploadPhase::Finalizing => {
                        UploadItemStatus::Uploading
                    }
                    FileUploadPhase::Done => UploadItemStatus::Done,
                };
                item.progress = Some(progress);
            }
        }
    });
    let request = FileUploadRequest {
        scope_id,
        id: None,
        filename: Some(item.name.clone()),
        mime_type: detect_mime_type(&item.name, item.mime_type.as_deref(), &bytes),
        entity: metadata_entity(&item.title, &item.description),
        bytes,
    };
    match client.upload_file(request, Some(progress_tx)).await {
        Ok(response) => {
            if let Some(item) = queue.write().iter_mut().find(|item| item.id == id) {
                item.status = UploadItemStatus::Done;
                item.error = None;
                item.result = Some(response);
            }
        }
        Err(err) => {
            set_failed(queue, id, err.to_string());
        }
    }
}

fn set_failed(mut queue: Signal<Vec<UploadQueueItem>>, id: QueueItemId, error: String) {
    if let Some(item) = queue.write().iter_mut().find(|item| item.id == id) {
        item.status = UploadItemStatus::Error;
        item.error = Some(error);
    }
}

fn is_busy(queue: &[UploadQueueItem], id: QueueItemId) -> bool {
    queue.iter().any(|item| {
        item.id == id
            && matches!(
                item.status,
                UploadItemStatus::Reading | UploadItemStatus::Uploading
            )
    })
}

fn mark_removed_or_drop(queue: &mut Vec<UploadQueueItem>, id: QueueItemId) {
    queue.retain_mut(|item| {
        if item.id != id {
            return true;
        }
        item.status = UploadItemStatus::Removed;
        false
    });
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

fn metadata_entity(title: &str, description: &str) -> Object {
    let mut entity = Object::new();
    let title = title.trim();
    if !title.is_empty() {
        entity.insert(TITLE_ATTRIBUTE_ID, Value::String(title.to_string()));
    }
    let description = description.trim();
    if !description.is_empty() {
        entity.insert(
            DESCRIPTION_ATTRIBUTE_ID,
            Value::String(description.to_string()),
        );
    }
    entity
}

fn with_id(mut object: Object, id: &str) -> Object {
    if !object.contains_key("id") {
        object.insert("id", Value::String(id.to_string()));
    }
    object
}

fn status_label(status: &UploadItemStatus) -> &'static str {
    match status {
        UploadItemStatus::Queued => "Queued",
        UploadItemStatus::Reading => "Reading",
        UploadItemStatus::Uploading => "Uploading",
        UploadItemStatus::Done => "Done",
        UploadItemStatus::Error => "Error",
        UploadItemStatus::Removed => "Removed",
    }
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
        let entity = metadata_entity(" Title ", " Body ");
        assert_eq!(
            entity.get(TITLE_ATTRIBUTE_ID),
            Some(&Value::String("Title".to_string()))
        );
        assert_eq!(
            entity.get(DESCRIPTION_ATTRIBUTE_ID),
            Some(&Value::String("Body".to_string()))
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
}
