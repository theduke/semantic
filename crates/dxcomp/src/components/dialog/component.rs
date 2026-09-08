use dioxus::prelude::*;
use dioxus_primitives::dialog::{self, DialogDescriptionProps, DialogRootProps, DialogTitleProps};
use dioxus_primitives::{dioxus_attributes::attributes, merge_attributes, use_controlled};

#[component]
pub fn Dialog(props: DialogRootProps) -> Element {
    let (open, set_open) = use_controlled(props.open, props.default_open, props.on_open_change);
    // The primitive uses a document-wide outside listener. Keep it controlled here so the
    // backdrop is the single, exact light-dismiss boundary for modal dialogs.
    let backdrop = attributes!(div {
        class: "dx-dialog-backdrop",
        onpointerdown: move |_| set_open.call(false),
    });
    let content = attributes!(div {
        class: "dx-dialog",
        onpointerdown: move |event| event.stop_propagation(),
        onkeydown: move |event: KeyboardEvent| {
            if event.key() == Key::Escape {
                set_open.call(false);
            }
        },
    });
    let content = merge_attributes(vec![props.attributes, content]);

    rsx! {
        dialog::DialogRoot {
            id: props.id,
            is_modal: props.is_modal,
            open: open(),
            on_open_change: move |_| {},
            attributes: backdrop,
            dialog::DialogContent {
                class: None,
                attributes: content,
                {props.children}
            }
        }
    }
}

#[component]
pub fn DialogTitle(props: DialogTitleProps) -> Element {
    let base = attributes!(h2 {
        class: "dx-dialog-title",
    });
    let merged = merge_attributes(vec![base, props.attributes]);

    rsx! {
        dialog::DialogTitle {
            id: props.id,
            attributes: merged,
            {props.children}
        }
    }
}

#[component]
pub fn DialogDescription(props: DialogDescriptionProps) -> Element {
    let base = attributes!(p {
        class: "dx-dialog-description",
    });
    let merged = merge_attributes(vec![base, props.attributes]);

    rsx! {
        dialog::DialogDescription {
            id: props.id,
            attributes: merged,
            {props.children}
        }
    }
}
