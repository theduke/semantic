use dioxus::prelude::*;

use crate::context::{
    DEFAULT_MAX_TOASTS, ToastDispatcher, ToastEntry, provide_toast_dispatcher, use_toast_dispatcher,
};

use super::{InlineNotice, NoticeLiveRegion};

/// Provides a route-facing toast dispatcher and renders its viewport.
#[component]
pub fn ToastProvider(
    #[props(default = DEFAULT_MAX_TOASTS)] max_toasts: usize,
    children: Element,
) -> Element {
    provide_toast_dispatcher(max_toasts);

    rsx! {
        {children}
        ToastViewport {}
    }
}

/// Keyed global notification viewport.
///
/// The viewport itself is not a live region. Each newly inserted notice owns
/// its status/alert semantics, so unrelated queue updates are not re-announced.
#[component]
pub fn ToastViewport() -> Element {
    let dispatcher = use_toast_dispatcher();
    let entries = dispatcher.entries();

    rsx! {
        div {
            class: "semantic-toast-viewport",
            aria_label: "Notifications",
            for entry in entries {
                ToastItem {
                    key: "{entry.id.get()}",
                    entry,
                    dispatcher,
                }
            }
        }
    }
}

#[component]
fn ToastItem(entry: ToastEntry, dispatcher: ToastDispatcher) -> Element {
    let id = entry.id;
    let timeout = entry.toast.resolved_timeout();
    let action_label = entry
        .toast
        .action
        .as_ref()
        .map(|action| action.label.clone());
    let action_handler = entry.toast.action.as_ref().map(|action| action.on_trigger);

    use_future(move || async move {
        let Some(timeout) = timeout else {
            return;
        };
        dioxus_sdk_time::sleep(timeout).await;
        dispatcher.dismiss(id);
    });

    let on_action = use_callback(move |()| {
        if let Some(action_handler) = action_handler {
            action_handler.call(());
            dispatcher.dismiss(id);
        }
    });
    let on_dismiss = use_callback(move |()| {
        dispatcher.dismiss(id);
    });

    rsx! {
        div {
            class: "semantic-toast",
            "data-toast-id": entry.id.get(),
            InlineNotice {
                title: entry.toast.title,
                message: entry.toast.message,
                variant: entry.toast.variant,
                live_region: NoticeLiveRegion::Automatic,
                action_label,
                on_action: action_handler.map(|_| on_action),
                on_dismiss: Some(on_dismiss),
                dismiss_label: "Dismiss notification".to_string(),
            }
        }
    }
}
