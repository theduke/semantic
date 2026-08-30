use dioxus::prelude::*;

/// An actionable empty state for a data region.
#[component]
pub fn EmptyState(
    title: String,
    #[props(default)] description: Option<String>,
    #[props(default)] action_label: Option<String>,
    #[props(default)] on_action: Option<EventHandler<()>>,
) -> Element {
    rsx! {
        section { class: "semantic-empty-state semantic-empty", "data-state": "empty",
            h2 { class: "semantic-empty-state__title", "{title}" }
            if let Some(description) = description {
                p { class: "semantic-empty-state__description", "{description}" }
            }
            if let (Some(action_label), Some(on_action)) = (action_label, on_action) {
                div { class: "semantic-empty-state__actions",
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Secondary,
                        onclick: move |_| on_action.call(()),
                        "{action_label}"
                    }
                }
            }
        }
    }
}
