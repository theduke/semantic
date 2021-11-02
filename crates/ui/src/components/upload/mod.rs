use brass::dom::{TagBuilder, builder::div};
use semantic_ui_core::components::util::title_2;

mod clipboard_reader;
pub mod uploader;

pub fn upload_page() -> TagBuilder {
    div()
        .and((title_2().and("Upload"), uploader::FileUploader))
}
