use brass::vdom::{component, div};

pub mod upload_page;
pub mod uploader;

pub fn upload_page() -> brass::VNode {
    div()
        .and((
            brass_bulma::h2_with("Upload"),
            component::<uploader::FileUploader>(()),
        ))
        .build()
}
