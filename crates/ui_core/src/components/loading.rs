use dioxus::prelude::*;

#[component]
pub fn LoadingView(label: String) -> Element {
    rsx! {
        div { class: "semantic-loading", "{label}" }
    }
}
