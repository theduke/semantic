use dioxus::prelude::*;

/// Compact compatibility view for call sites that only have an error message.
///
/// New data surfaces should prefer [`ErrorState`], which can expose recovery and
/// diagnostic details without making those details the primary announcement.
#[component]
pub fn ErrorView(message: String) -> Element {
    rsx! {
        div { class: "semantic-error", role: "alert", "{message}" }
    }
}

/// A recoverable, page-level error state.
#[component]
pub fn ErrorState(
    message: String,
    #[props(default = "Something went wrong".to_string())] title: String,
    #[props(default)] details: Option<String>,
    #[props(default = "Try again".to_string())] retry_label: String,
    #[props(default)] on_retry: Option<EventHandler<()>>,
) -> Element {
    rsx! {
        section { class: "semantic-error-state", "data-state": "error",
            div { class: "semantic-error-state__summary semantic-error", role: "alert",
                h2 { class: "semantic-error-state__title", "{title}" }
                p { class: "semantic-error-state__message", "{message}" }
            }
            if let Some(details) = details {
                details { class: "semantic-error-state__details",
                    summary { "Technical details" }
                    pre { code { "{details}" } }
                }
            }
            if let Some(on_retry) = on_retry {
                div { class: "semantic-error-state__actions",
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Secondary,
                        onclick: move |_| on_retry.call(()),
                        "{retry_label}"
                    }
                }
            }
        }
    }
}
