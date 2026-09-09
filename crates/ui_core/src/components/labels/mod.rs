pub mod client;
mod editor;

pub use editor::{EntityLabelsButton, LabelEditor, LabelEditorProps};

use dioxus::prelude::*;
use semantic_base::labels::valid_label_color;

#[component]
pub fn LabelColor(#[props(default)] color: Option<String>) -> Element {
    let color = color
        .filter(|color| valid_label_color(color))
        .unwrap_or_else(|| "#818a9d".into());
    rsx! { span { class: "semantic-label-color", style: "background-color: {color}", aria_hidden: "true" } }
}
