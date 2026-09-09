use dioxus::prelude::*;
use dioxus_primitives::context_menu::{
    self, ContextMenuContentProps, ContextMenuItemProps, ContextMenuProps, ContextMenuTriggerProps,
};

#[derive(Clone, Copy)]
struct MenuState {
    open: Memo<bool>,
    set_open: Callback<bool>,
}

#[component]
pub fn ContextMenu(mut props: ContextMenuProps) -> Element {
    let (open, set_open) =
        dioxus_primitives::use_controlled(props.open, props.default_open, props.on_open_change);
    use_context_provider(|| MenuState { open, set_open });

    // Let the trigger handle the event before stopping it at this menu's
    // boundary. Otherwise nested menus also open their ancestor's menu.
    props.attributes.extend([
        oncontextmenu(|event: MouseEvent| event.stop_propagation()),
        onpointerdown(|event: PointerEvent| event.stop_propagation()),
    ]);

    rsx! {
        context_menu::ContextMenu {
            disabled: props.disabled,
            open: Some(open()),
            on_open_change: set_open,
            roving_loop: props.roving_loop,
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn ContextMenuTrigger(props: ContextMenuTriggerProps) -> Element {
    rsx! {
        context_menu::ContextMenuTrigger {
            cursor: "context-menu",
            user_select: "none",
            attributes: props.attributes,
            {props.children}
        }
    }
}

#[component]
pub fn ContextMenuContent(mut props: ContextMenuContentProps) -> Element {
    let state: MenuState = use_context();
    let fallback_id = use_hook(|| {
        format!(
            "dx-context-menu-content-{}",
            dioxus::core::current_scope_id().0
        )
    });
    let id = use_memo(move || (props.id)().unwrap_or_else(|| fallback_id.clone()));
    props
        .attributes
        .push(onmouseleave(move |_| state.set_open.call(false)));

    rsx! {
        context_menu::ContextMenuContent {
            class: "dx-context-menu-content",
            id: Some(id()),
            attributes: props.attributes,
            {props.children}
        }
        if (state.open)() {
            ContextMenuDismiss { id: id(), on_dismiss: move |_| state.set_open.call(false) }
        }
    }
}

/// Listen only while open, and use the popup rather than the trigger as the
/// inside boundary. Capture listeners also see clicks whose handlers stop bubbling.
#[component]
fn ContextMenuDismiss(id: String, on_dismiss: Callback<()>) -> Element {
    let mut listener = use_hook(|| CopyValue::new(None::<document::Eval>));
    use_effect(move || {
        let mut eval = document::eval(include_str!("dismiss.js"));
        let _ = eval.send(&id);
        listener.set(Some(eval));
        spawn(async move {
            while let Ok(true) = eval.recv::<bool>().await {
                on_dismiss.call(());
            }
        });
    });
    use_drop(move || {
        if let Some(eval) = listener.take() {
            let _ = eval.send(());
        }
    });
    rsx! {}
}

#[component]
pub fn ContextMenuItem(props: ContextMenuItemProps) -> Element {
    rsx! {
        context_menu::ContextMenuItem {
            class: "dx-context-menu-item",
            disabled: props.disabled,
            value: props.value,
            index: props.index,
            on_select: props.on_select,
            attributes: props.attributes,
            {props.children}
        }
    }
}
