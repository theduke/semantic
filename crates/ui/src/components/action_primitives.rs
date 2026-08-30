use dioxus::prelude::*;

/// Visual treatment for an icon-only action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum IconButtonVariant {
    #[default]
    Outline,
    Primary,
    Destructive,
    Ghost,
}

impl IconButtonVariant {
    fn button_variant(self) -> dxcomp::ButtonVariant {
        match self {
            Self::Outline => dxcomp::ButtonVariant::Outline,
            Self::Primary => dxcomp::ButtonVariant::Primary,
            Self::Destructive => dxcomp::ButtonVariant::Destructive,
            Self::Ghost => dxcomp::ButtonVariant::Ghost,
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Outline => "outline",
            Self::Primary => "primary",
            Self::Destructive => "destructive",
            Self::Ghost => "ghost",
        }
    }
}

/// Visual size for an icon-only action. Every size retains a 44 px hit target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum IconButtonSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl IconButtonSize {
    fn button_size(self) -> dxcomp::ButtonSize {
        match self {
            Self::Small => dxcomp::ButtonSize::IconSm,
            Self::Medium => dxcomp::ButtonSize::Icon,
            Self::Large => dxcomp::ButtonSize::IconLg,
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
}

/// Accessible icon-only button with a consistent minimum hit target.
///
/// `label` is intentionally required. `tooltip` is optional because an
/// accessible name and a visual tooltip serve different purposes.
#[component]
pub fn IconButton(
    label: String,
    children: Element,
    #[props(default)] tooltip: Option<String>,
    #[props(default)] variant: IconButtonVariant,
    #[props(default)] size: IconButtonSize,
    #[props(default)] pressed: Option<bool>,
    #[props(default)] disabled: bool,
    #[props(default)] on_click: Option<EventHandler<MouseEvent>>,
) -> Element {
    rsx! {
        dxcomp::Button {
            class: "semantic-icon-button",
            "data-icon-variant": variant.class(),
            "data-icon-size": size.class(),
            variant: variant.button_variant(),
            size: size.button_size(),
            aria_label: label,
            aria_pressed: pressed,
            title: tooltip,
            disabled,
            onclick: move |event| {
                if let Some(on_click) = on_click {
                    on_click.call(event);
                }
            },
            {children}
        }
    }
}

/// Completion handle passed to a route's confirmation mutation.
///
/// The route may finish synchronously or retain the handle across an async
/// task. Success closes the dialog; failure keeps it open and presents the
/// returned message.
#[derive(Clone, Copy, Debug)]
pub struct ConfirmActionRequest {
    completion: Callback<std::result::Result<(), String>>,
}

impl ConfirmActionRequest {
    fn new(completion: Callback<std::result::Result<(), String>>) -> Self {
        Self { completion }
    }

    pub fn complete(self, result: std::result::Result<(), String>) {
        self.completion.call(result);
    }
}

/// Visual meaning of a confirmation's primary action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfirmActionVariant {
    #[default]
    Default,
    Danger,
}

impl ConfirmActionVariant {
    fn button_variant(self) -> dxcomp::ButtonVariant {
        match self {
            Self::Default => dxcomp::ButtonVariant::Primary,
            Self::Danger => dxcomp::ButtonVariant::Destructive,
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Danger => "danger",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum ConfirmActionStatus {
    #[default]
    Idle,
    Pending,
    Failed(String),
}

impl ConfirmActionStatus {
    fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    fn error(&self) -> Option<&str> {
        match self {
            Self::Failed(error) => Some(error),
            Self::Idle | Self::Pending => None,
        }
    }
}

/// Controlled confirmation dialog with locally owned mutation feedback.
///
/// The caller owns `open` because its trigger is domain-specific. Mutation
/// pending/error state stays here and is reported through
/// [`ConfirmActionRequest`], avoiding duplicated dialog state machines in
/// routes. `target` is always rendered so a destructive action cannot become
/// an unnamed generic prompt.
#[component]
pub fn ConfirmAction(
    open: bool,
    title: String,
    target: String,
    body: String,
    on_open_change: EventHandler<bool>,
    on_confirm: EventHandler<ConfirmActionRequest>,
    #[props(default = "Confirm".to_string())] confirm_label: String,
    #[props(default = "Cancel".to_string())] cancel_label: String,
    #[props(default)] variant: ConfirmActionVariant,
    #[props(default)] on_cancel: Option<EventHandler<()>>,
) -> Element {
    let mut status = use_signal(ConfirmActionStatus::default);
    let pending = status.read().is_pending();

    use_effect(move || {
        if !open && !matches!(&*status.read(), ConfirmActionStatus::Idle) {
            status.set(ConfirmActionStatus::Idle);
        }
    });

    let completion = use_callback(
        move |result: std::result::Result<(), String>| match result {
            Ok(()) => {
                status.set(ConfirmActionStatus::Idle);
                on_open_change.call(false);
            }
            Err(error) => status.set(ConfirmActionStatus::Failed(error)),
        },
    );

    rsx! {
        dxcomp::AlertDialog {
            class: "semantic-confirm-action",
            "data-variant": variant.class(),
            open,
            on_open_change: move |next_open: bool| {
                if !status.read().is_pending() {
                    if !next_open {
                        status.set(ConfirmActionStatus::Idle);
                    }
                    on_open_change.call(next_open);
                }
            },
            dxcomp::AlertDialogTitle { "{title}" }
            dxcomp::AlertDialogDescription {
                span { class: "semantic-confirm-action__body", "{body}" }
                span { class: "semantic-confirm-action__target-label", "Target" }
                code { class: "semantic-confirm-action__target", title: target.clone(), "{target}" }
            }
            if let Some(error) = status.read().error() {
                p {
                    class: "semantic-confirm-action__error",
                    role: "alert",
                    "{error}"
                }
            }
            dxcomp::AlertDialogActions {
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    disabled: pending,
                    onclick: move |_| {
                        status.set(ConfirmActionStatus::Idle);
                        if let Some(on_cancel) = on_cancel {
                            on_cancel.call(());
                        }
                        on_open_change.call(false);
                    },
                    "{cancel_label}"
                }
                dxcomp::Button {
                    variant: variant.button_variant(),
                    disabled: pending,
                    aria_busy: pending,
                    onclick: move |_| {
                        status.set(ConfirmActionStatus::Pending);
                        on_confirm.call(ConfirmActionRequest::new(completion));
                    },
                    if pending {
                        "{confirm_label}…"
                    } else {
                        "{confirm_label}"
                    }
                }
            }
        }
    }
}

/// Destructive convenience wrapper for [`ConfirmAction`].
#[component]
pub fn ConfirmDangerDialog(
    open: bool,
    title: String,
    target: String,
    body: String,
    on_open_change: EventHandler<bool>,
    on_confirm: EventHandler<ConfirmActionRequest>,
    #[props(default = "Delete".to_string())] confirm_label: String,
    #[props(default = "Cancel".to_string())] cancel_label: String,
    #[props(default)] on_cancel: Option<EventHandler<()>>,
) -> Element {
    rsx! {
        ConfirmAction {
            open,
            title,
            target,
            body,
            confirm_label,
            cancel_label,
            variant: ConfirmActionVariant::Danger,
            on_open_change,
            on_confirm,
            on_cancel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfirmActionStatus, IconButtonSize, IconButtonVariant};

    #[test]
    fn icon_button_classes_are_stable_for_css_and_audits() {
        assert_eq!(IconButtonVariant::Destructive.class(), "destructive");
        assert_eq!(IconButtonSize::Small.class(), "small");
        assert_eq!(IconButtonSize::Large.class(), "large");
    }

    #[test]
    fn confirmation_status_exposes_only_failures_as_errors() {
        assert!(ConfirmActionStatus::Pending.is_pending());
        assert_eq!(ConfirmActionStatus::Idle.error(), None);
        assert_eq!(
            ConfirmActionStatus::Failed("permission denied".to_string()).error(),
            Some("permission denied")
        );
    }
}
