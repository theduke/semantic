use brass::vdom::div;

pub mod upload_page;
pub mod uploader;

pub fn upload_page() -> brass::VNode {
    div()
        .and((brass_bulma::h2_with("Upload"), uploader::FileUploader))
        .build()
}
