use dioxus::prelude::*;

use super::{ConfirmAction, ConfirmActionRequest, ConfirmActionVariant, PageHeader};

/// Focused route layout shared by create and edit forms.
///
/// Form state deliberately remains with the route and its form engine. This
/// component only composes the route header, immutable context, form content,
/// and optional sticky actions.
#[component]
pub fn FormPage(
    title: String,
    children: Element,
    #[props(default)] description: Option<String>,
    #[props(default)] breadcrumbs: Option<Element>,
    #[props(default)] header_actions: Option<Element>,
    #[props(default)] context: Option<Element>,
    #[props(default = "Form context".to_string())] context_label: String,
    #[props(default)] actions: Option<Element>,
    #[props(default)] busy: bool,
) -> Element {
    rsx! {
        div {
            class: "semantic-page semantic-form-page",
            aria_busy: busy,
            PageHeader {
                title,
                description,
                breadcrumbs,
                actions: header_actions,
            }
            div { class: "semantic-form-page__body",
                if let Some(context) = context {
                    section {
                        class: "semantic-form-page__context",
                        aria_label: context_label,
                        {context}
                    }
                }
                div { class: "semantic-form-page__content", {children} }
            }
            if let Some(actions) = actions {
                {actions}
            }
        }
    }
}

/// Route-controlled state displayed by [`FormActions`].
///
/// The variants contain no writable form state; a route derives one of these
/// values from the form engine and supplies a more specific `status_message`
/// when the default wording does not fit its mutation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FormActionStatus {
    #[default]
    Clean,
    Dirty,
    Pending,
    Succeeded,
    Failed,
}

impl FormActionStatus {
    fn class(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Dirty => "dirty",
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    fn default_message(self) -> &'static str {
        match self {
            Self::Clean => "No unsaved changes",
            Self::Dirty => "Unsaved changes",
            Self::Pending => "Saving changes…",
            Self::Succeeded => "Changes saved",
            Self::Failed => "Changes could not be saved",
        }
    }

    fn is_pending(self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// Sticky form action surface with one visible, accessible status.
///
/// Buttons are slots so routes retain submit, navigation, disabled, and focus
/// behavior. Prefer a single button in `primary`; the optional slots establish
/// consistent ordering for secondary, cancel, and discard actions.
#[component]
pub fn FormActions(
    status: FormActionStatus,
    primary: Element,
    #[props(default)] status_message: Option<String>,
    #[props(default)] secondary: Option<Element>,
    #[props(default)] cancel: Option<Element>,
    #[props(default)] discard: Option<Element>,
) -> Element {
    let message = status_message
        .as_deref()
        .unwrap_or_else(|| status.default_message());

    rsx! {
        section {
            class: "semantic-form-actions",
            "data-status": status.class(),
            aria_label: "Form actions",
            aria_busy: status.is_pending(),
            p {
                class: "semantic-form-actions__status",
                role: "status",
                aria_live: "polite",
                aria_atomic: "true",
                span { class: "semantic-form-actions__status-mark", aria_hidden: "true" }
                span { "{message}" }
            }
            div { class: "semantic-form-actions__controls",
                if let Some(discard) = discard {
                    div { class: "semantic-form-actions__slot semantic-form-actions__slot--discard",
                        {discard}
                    }
                }
                if let Some(secondary) = secondary {
                    div { class: "semantic-form-actions__slot semantic-form-actions__slot--secondary",
                        {secondary}
                    }
                }
                if let Some(cancel) = cancel {
                    div { class: "semantic-form-actions__slot semantic-form-actions__slot--cancel",
                        {cancel}
                    }
                }
                div { class: "semantic-form-actions__slot semantic-form-actions__slot--primary",
                    {primary}
                }
            }
        }
    }
}

/// Controlled UI contract for confirming navigation away from a dirty form.
///
/// The route remains responsible for deciding when navigation should pause,
/// storing the intended destination, and continuing navigation after calling
/// [`ConfirmActionRequest::complete`]. This component does not install router
/// or browser event handlers.
#[component]
pub fn UnsavedChangesPrompt(
    open: bool,
    target: String,
    on_open_change: EventHandler<bool>,
    on_discard: EventHandler<ConfirmActionRequest>,
    #[props(default = "Discard unsaved changes?".to_string())] title: String,
    #[props(default = "Leaving now will permanently discard the changes in this form.".to_string())]
    body: String,
    #[props(default = "Discard changes".to_string())] discard_label: String,
    #[props(default = "Keep editing".to_string())] keep_editing_label: String,
    #[props(default)] on_keep_editing: Option<EventHandler<()>>,
) -> Element {
    rsx! {
        ConfirmAction {
            open,
            title,
            target,
            body,
            confirm_label: discard_label,
            cancel_label: keep_editing_label,
            variant: ConfirmActionVariant::Danger,
            on_open_change,
            on_confirm: on_discard,
            on_cancel: on_keep_editing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FormActionStatus;

    #[test]
    fn form_action_status_has_stable_visual_and_accessible_fallbacks() {
        assert_eq!(FormActionStatus::Clean.class(), "clean");
        assert_eq!(FormActionStatus::Dirty.default_message(), "Unsaved changes");
        assert!(FormActionStatus::Pending.is_pending());
        assert_eq!(FormActionStatus::Succeeded.class(), "succeeded");
        assert_eq!(
            FormActionStatus::Failed.default_message(),
            "Changes could not be saved"
        );
    }
}
