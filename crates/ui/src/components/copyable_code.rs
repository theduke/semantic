use dioxus::prelude::*;
use dioxus_icons::lucide::{Check, Copy};

use super::{IconButton, IconButtonSize, IconButtonVariant};

/// Clipboard request emitted by [`CopyableCode`].
///
/// This keeps platform clipboard code at the application boundary. A web or
/// desktop integration can retain the request while it performs asynchronous
/// work, then call [`CopyRequest::complete`] with the outcome.
#[derive(Clone, Debug)]
pub struct CopyRequest {
    value: String,
    completion: Callback<std::result::Result<(), String>>,
}

impl CopyRequest {
    fn new(value: String, completion: Callback<std::result::Result<(), String>>) -> Self {
        Self { value, completion }
    }

    /// Exact, untruncated value that should be written to the clipboard.
    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn complete(self, result: std::result::Result<(), String>) {
        self.completion.call(result);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum CopyStatus {
    #[default]
    Idle,
    Pending,
    Succeeded,
    Failed(String),
}

impl CopyStatus {
    fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    fn feedback(&self) -> Option<String> {
        match self {
            Self::Idle => None,
            Self::Pending => Some("Copying…".to_string()),
            Self::Succeeded => Some("Copied to clipboard.".to_string()),
            Self::Failed(error) => Some(format!(
                "Could not copy: {error}. Select the value and copy it manually."
            )),
        }
    }
}

/// Selectable mono value with an optional platform-provided copy action.
///
/// When `on_copy` is omitted the full value remains selectable; this is the
/// dependency-free fallback for renderers without clipboard integration.
#[component]
pub fn CopyableCode(
    value: String,
    #[props(default = "Value".to_string())] label: String,
    #[props(default)] on_copy: Option<EventHandler<CopyRequest>>,
) -> Element {
    let mut status = use_signal(CopyStatus::default);
    let pending = status.read().is_pending();
    let copied = matches!(&*status.read(), CopyStatus::Succeeded);
    let feedback = status.read().feedback();
    let copy_value = value.clone();

    let completion = use_callback(
        move |result: std::result::Result<(), String>| match result {
            Ok(()) => status.set(CopyStatus::Succeeded),
            Err(error) => status.set(CopyStatus::Failed(error)),
        },
    );

    rsx! {
        span {
            class: "semantic-copyable-code",
            "data-copy-available": on_copy.is_some(),
            span { class: "semantic-visually-hidden", "{label}: " }
            code {
                class: "semantic-copyable-code__value",
                title: value.clone(),
                tabindex: "0",
                "{value}"
            }
            if let Some(on_copy) = on_copy {
                IconButton {
                    label: format!("Copy {label}"),
                    tooltip: format!("Copy {label}"),
                    variant: IconButtonVariant::Ghost,
                    size: IconButtonSize::Small,
                    disabled: pending,
                    on_click: move |_| {
                        status.set(CopyStatus::Pending);
                        on_copy.call(CopyRequest::new(copy_value.clone(), completion));
                    },
                    span { aria_hidden: "true",
                        if copied {
                            Check { size: "1rem" }
                        } else {
                            Copy { size: "1rem" }
                        }
                    }
                }
            } else {
                span {
                    class: "semantic-copyable-code__fallback",
                    title: "Select this value to copy it manually",
                    "Select to copy"
                }
            }
            span {
                class: "semantic-copyable-code__feedback semantic-visually-hidden",
                role: "status",
                aria_live: "polite",
                aria_atomic: "true",
                if let Some(feedback) = feedback {
                    "{feedback}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CopyStatus;

    #[test]
    fn copy_feedback_is_polite_and_actionable() {
        assert_eq!(CopyStatus::Idle.feedback(), None);
        assert_eq!(
            CopyStatus::Succeeded.feedback().as_deref(),
            Some("Copied to clipboard.")
        );
        assert_eq!(
            CopyStatus::Failed("clipboard denied".to_string())
                .feedback()
                .as_deref(),
            Some("Could not copy: clipboard denied. Select the value and copy it manually.")
        );
    }
}
