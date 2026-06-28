use dioxus::prelude::*;

#[component]
pub fn TreePage(root: Option<String>) -> Element {
    rsx! {
        semantic_ui_core::DirectoryBrowser { root }
    }
}
