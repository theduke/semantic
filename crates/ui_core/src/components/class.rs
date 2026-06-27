use std::collections::BTreeSet;

use dioxus::prelude::*;
use semantic_data::schema::ClassType;
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::OBJECT_TYPE_FIELD;

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

    let fields = catalog.class_form_fields(&class);
    let known_field_names = fields
        .iter()
        .flat_map(|field| [field.field_name.clone(), field.storage_field_name.clone()])
        .chain(std::iter::once(OBJECT_TYPE_FIELD.to_string()))
        .collect::<BTreeSet<_>>();
    let extra_fields = object
        .iter()
        .filter(|(key, _)| !known_field_names.contains(*key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Vec<_>>();

    rsx! {
        article { class: "semantic-class",
            header {
                h2 { "{class.name}" }
                if let Some(id) = id {
                    code { "{id}" }
                }
            }
            dl {
                for field in fields {
                    if field.field_name != OBJECT_TYPE_FIELD && field.storage_field_name != OBJECT_TYPE_FIELD {
                    dt { "{field.field_name}" }
                    dd {
                            ValueView {
                                value: object
                                    .get(&field.storage_field_name)
                                    .or_else(|| object.get(&field.field_name))
                                    .cloned()
                                    .unwrap_or(Value::Null),
                                type_hint: Some(field.attribute.ty.clone()),
                                mode
                            }
                    }
                    }
                }
                for (key, value) in extra_fields {
                    dt { class: "semantic-class__extra-field", "{key}" }
                    dd {
                        ValueView {
                            value,
                            type_hint: None,
                            mode
                        }
                    }
                }
            }
        }
    }
}
