use dioxus::prelude::*;
use dioxus_icons::lucide::X;

/// An image preview that opens the original in a compact, dismissible modal.
#[component]
pub(crate) fn ImageLightbox(
    source: String,
    title: String,
    preview_class: String,
    trigger_class: String,
) -> Element {
    let mut open = use_signal(|| false);
    let preview_source = source.clone();
    let preview_title = title.clone();

    rsx! {
        button {
            class: "semantic-image-lightbox__trigger {trigger_class}",
            r#type: "button",
            aria_label: "View {title} at full size",
            onclick: move |_| open.set(true),
            img {
                class: "{preview_class}",
                src: preview_source,
                alt: "{preview_title}",
                loading: "lazy",
            }
        }
        dxcomp::Dialog {
            class: "semantic-image-lightbox",
            open: open(),
            on_open_change: move |next_open: bool| open.set(next_open),
            dxcomp::DialogTitle { class: "semantic-visually-hidden", "{title}" }
            button {
                class: "semantic-image-lightbox__close",
                r#type: "button",
                aria_label: "Close image preview",
                onclick: move |_| open.set(false),
                X { size: "1.1rem" }
            }
            img {
                class: "semantic-image-lightbox__image",
                src: source,
                alt: "{title}",
            }
        }
    }
}
