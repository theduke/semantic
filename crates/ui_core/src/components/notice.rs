use dioxus::prelude::*;

/// Visual meaning of an inline notice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NoticeVariant {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

impl NoticeVariant {
    fn class(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Screen-reader announcement policy for a notice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NoticeLiveRegion {
    /// Errors are assertive; other variants are polite.
    #[default]
    Automatic,
    Polite,
    Assertive,
    Off,
}

impl NoticeLiveRegion {
    fn attributes(self, variant: NoticeVariant) -> (&'static str, &'static str) {
        match (self, variant) {
            (Self::Assertive, _) | (Self::Automatic, NoticeVariant::Error) => {
                ("alert", "assertive")
            }
            (Self::Off, _) => ("note", "off"),
            (Self::Automatic | Self::Polite, _) => ("status", "polite"),
        }
    }
}

/// Persistent contextual feedback with optional action and dismissal controls.
#[component]
pub fn InlineNotice(
    message: String,
    #[props(default)] title: Option<String>,
    #[props(default)] variant: NoticeVariant,
    #[props(default)] live_region: NoticeLiveRegion,
    #[props(default)] action_label: Option<String>,
    #[props(default)] on_action: Option<EventHandler<()>>,
    #[props(default)] on_dismiss: Option<EventHandler<()>>,
    #[props(default = "Dismiss notification".to_string())] dismiss_label: String,
) -> Element {
    let (role, aria_live) = live_region.attributes(variant);

    rsx! {
        div {
            class: "semantic-inline-notice",
            role,
            aria_live,
            aria_atomic: "true",
            "data-variant": variant.class(),
            div { class: "semantic-inline-notice__content",
                if let Some(title) = title {
                    strong { class: "semantic-inline-notice__title", "{title}" }
                }
                p { class: "semantic-inline-notice__message", "{message}" }
            }
            if (action_label.is_some() && on_action.is_some()) || on_dismiss.is_some() {
                div { class: "semantic-inline-notice__actions",
                    if let (Some(action_label), Some(on_action)) = (action_label, on_action) {
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Secondary,
                            onclick: move |_| on_action.call(()),
                            "{action_label}"
                        }
                    }
                    if let Some(on_dismiss) = on_dismiss {
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Ghost,
                            aria_label: dismiss_label.clone(),
                            title: dismiss_label,
                            onclick: move |_| on_dismiss.call(()),
                            span { aria_hidden: "true", "×" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NoticeLiveRegion, NoticeVariant};

    #[test]
    fn automatic_errors_are_assertive() {
        assert_eq!(
            NoticeLiveRegion::Automatic.attributes(NoticeVariant::Error),
            ("alert", "assertive")
        );
    }

    #[test]
    fn automatic_success_is_polite() {
        assert_eq!(
            NoticeLiveRegion::Automatic.attributes(NoticeVariant::Success),
            ("status", "polite")
        );
    }
}
