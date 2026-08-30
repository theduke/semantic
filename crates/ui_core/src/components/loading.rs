use dioxus::prelude::*;

/// Compact compatibility view for call sites that only need a loading label.
#[component]
pub fn LoadingView(label: String) -> Element {
    rsx! {
        div {
            class: "semantic-loading",
            role: "status",
            aria_live: "polite",
            aria_busy: "true",
            "{label}"
        }
    }
}

/// A stable placeholder for initial loading states.
#[component]
pub fn LoadingSkeleton(
    #[props(default = "Loading".to_string())] label: String,
    #[props(default = 3)] line_count: usize,
) -> Element {
    let line_count = line_count.clamp(1, 12);

    rsx! {
        div {
            class: "semantic-loading-skeleton",
            role: "status",
            aria_live: "polite",
            aria_busy: "true",
            span { class: "semantic-visually-hidden", "{label}" }
            div { class: "semantic-loading-skeleton__body", aria_hidden: "true",
                for index in 0..line_count {
                    div {
                        key: "{index}",
                        class: "semantic-loading-skeleton__line",
                        "data-line": "{index}"
                    }
                }
            }
        }
    }
}

/// Non-blocking busy feedback for content retained during a refresh.
#[component]
pub fn RefreshingIndicator(#[props(default = "Refreshing".to_string())] label: String) -> Element {
    rsx! {
        div {
            class: "semantic-refreshing-indicator",
            role: "status",
            aria_live: "polite",
            aria_busy: "true",
            span { class: "semantic-refreshing-indicator__mark", aria_hidden: "true" }
            span { class: "semantic-refreshing-indicator__label", "{label}" }
        }
    }
}
