use dioxus::prelude::*;

#[component]
pub fn ErrorView(message: String) -> Element {
    rsx! {
        div { class: "semantic-error", "{message}" }
    }
}
