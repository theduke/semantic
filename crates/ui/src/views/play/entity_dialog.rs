use dioxus::prelude::*;
use semantic_data::value::Object;
use semantic_ui_core::{EntityCard, EntityDisplayRenderer, EntityRenderOptions, EntityTarget};

#[component]
pub fn PlayerEntityDialog(
    open: bool,
    title: String,
    object: Option<std::result::Result<Object, String>>,
    target: Option<EntityTarget>,
    on_open_change: EventHandler<bool>,
    on_deleted: EventHandler<EntityTarget>,
    on_retry: EventHandler<()>,
) -> Element {
    rsx! {
        dxcomp::Dialog { open, on_open_change,
            dxcomp::DialogTitle { "{title}" }
            if let (Some(Ok(object)), Some(target)) = (object.clone(), target) {
                div { class: "semantic-player__entity-dialog-body",
                    EntityCard {
                        object,
                        options: EntityRenderOptions {
                            collection: target.collection.clone(), id: Some(target.id),
                            renderer: EntityDisplayRenderer::Custom, preview: false, actions: true,
                        },
                        on_delete: on_deleted,
                    }
                }
            } else if let Some(Err(error)) = object {
                div { class: "semantic-player__dialog-error", role: "alert",
                    p { "Could not load this entity." }
                    p { class: "semantic-text-muted", "{error}" }
                    dxcomp::Button { onclick: move |_| on_retry.call(()), "Retry" }
                }
            } else {
                p { role: "status", "Loading entity…" }
            }
            div { class: "semantic-player__dialog-actions",
                dxcomp::Button { variant: dxcomp::ButtonVariant::Outline,
                    onclick: move |_| on_open_change.call(false), "Close" }
            }
        }
    }
}
