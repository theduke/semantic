use dioxus::prelude::*;
use semantic_data::value::Object;
use semantic_ui_core::{EntityCard, EntityDisplayRenderer, EntityRenderOptions, EntityTarget};

#[component]
pub fn PlayerEntityDialog(
    open: bool,
    title: String,
    object: Option<Object>,
    target: Option<EntityTarget>,
    on_open_change: EventHandler<bool>,
    on_deleted: EventHandler<EntityTarget>,
) -> Element {
    rsx! {
        dxcomp::Dialog { open, on_open_change,
            dxcomp::DialogTitle { "{title}" }
            if let (Some(object), Some(target)) = (object, target) {
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
            } else {
                p { "The entity is still loading." }
            }
            div { class: "semantic-player__dialog-actions",
                dxcomp::Button { variant: dxcomp::ButtonVariant::Outline,
                    onclick: move |_| on_open_change.call(false), "Close" }
            }
        }
    }
}
