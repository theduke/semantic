use dioxus::{html::FileData, prelude::*, web::WebFileData};
use wasm_bindgen::{JsCast as _, closure::Closure};
use web_sys::{ClipboardEvent, Document, Element, FileReader};

/// Dioxus clipboard events do not expose files, so retain the native browser
/// files directly. This also avoids reading or base64-encoding image contents.
pub(super) fn use_image_paste(enabled: bool, on_files: EventHandler<Vec<FileData>>) {
    let on_paste = use_callback(move |event: ClipboardEvent| {
        if !enabled || event.default_prevented() {
            return;
        }
        // Leave editors and metadata fields in charge of their own clipboard.
        if let Some(target) = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
        {
            if target
                .closest("input, textarea, [contenteditable]:not([contenteditable='false']), [role='textbox']")
                .ok()
                .flatten()
                .is_some()
            {
                return;
            }
        }
        let Some(data) = event.clipboard_data() else {
            return;
        };
        let items = data.items();
        let mut files = Vec::new();
        for index in 0..items.length() {
            let Some(item) = items.get(index) else {
                continue;
            };
            if item.kind() != "file" || !item.type_().starts_with("image/") {
                continue;
            }
            let Ok(Some(file)) = item.get_as_file() else {
                continue;
            };
            let Ok(reader) = FileReader::new() else {
                continue;
            };
            files.push(FileData::new(WebFileData::new(file, reader)));
        }
        if !files.is_empty() {
            event.prevent_default();
            on_files.call(files);
        }
    });

    // Keep the closure alive until unmount, then remove the document listener.
    // A document listener allows pasting without first focusing the drop zone.
    let listener = use_hook(move || {
        let document = web_sys::window()?.document()?;
        let callback = Closure::wrap(
            Box::new(move |event| on_paste.call(event)) as Box<dyn FnMut(ClipboardEvent)>
        );
        if let Err(error) =
            document.add_event_listener_with_callback("paste", callback.as_ref().unchecked_ref())
        {
            tracing::warn!(?error, "Could not listen for pasted images");
            return None;
        }
        Some(std::rc::Rc::new(PasteListener { document, callback }))
    });
    use_drop(move || drop(listener));
}

struct PasteListener {
    document: Document,
    callback: Closure<dyn FnMut(ClipboardEvent)>,
}

impl Drop for PasteListener {
    fn drop(&mut self) {
        let _ = self
            .document
            .remove_event_listener_with_callback("paste", self.callback.as_ref().unchecked_ref());
    }
}
