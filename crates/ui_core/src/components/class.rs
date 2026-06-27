use dioxus::prelude::*;
use semantic_data::schema::ClassType;
use semantic_data::value::Object;

use crate::components::ValueView;
use crate::form::{DynamicClassForm, mode_from_render_mode};
use crate::ui_catalog::{ClassRenderContext, RenderMode, use_ui_catalog};

#[component]
pub fn ClassView(
    class: ClassType,
    object: Object,
    collection: Option<String>,
    id: Option<String>,
    mode: RenderMode,
) -> Element {
    if let Some(form_mode) = mode_from_render_mode(mode) {
        return rsx! {
            DynamicClassForm {
                class,
                object,
                collection,
                id,
                mode: form_mode,
                scope_id: None,
                submit: None
            }
        };
    }

    let catalog = use_ui_catalog();
    if let Some(renderer) = catalog
        .render_registry()
        .class_renderer(&class.id)
        .or_else(|| {
            class.inherits.as_ref().and_then(|parent| {
                catalog
                    .render_registry()
                    .class_renderer(&parent.id)
                    .filter(|_| catalog.class_inherits(&class.id, &parent.id))
            })
        })
    {
        return renderer(ClassRenderContext {
            collection,
            id,
            class,
            object,
            mode,
        });
    }

    rsx! {
        article { class: "semantic-class",
            header {
                h2 { "{class.name}" }
                if let Some(id) = id {
                    code { "{id}" }
                }
            }
            dl {
                for (field_name, field) in class.attributes.iter() {
                    dt { "{field_name}" }
                    dd {
                        if let Some(attribute) = catalog.attribute_by_id(&field.attribute.id) {
                            ValueView {
                                value: object.get(field_name).cloned().unwrap_or(semantic_data::value::Value::Null),
                                type_hint: Some(attribute.ty.clone()),
                                mode
                            }
                        } else {
                            span { "unknown attribute" }
                        }
                    }
                }
            }
        }
    }
}
