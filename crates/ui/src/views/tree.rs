use dioxus::prelude::*;

use crate::components::PageHeader;

#[component]
pub fn TreePage(root: Option<String>) -> Element {
    let description = root
        .as_deref()
        .map(|root| format!("Organize the current directory ({root}) and its linked entities."))
        .unwrap_or_else(|| "Organize root directories and their linked entities.".to_string());
    rsx! {
        section { class: "semantic-tree-page semantic-route-stack",
            PageHeader {
                title: "Directory tree",
                description,
            }
            semantic_ui_core::DirectoryBrowser { root }
        }
    }
}
