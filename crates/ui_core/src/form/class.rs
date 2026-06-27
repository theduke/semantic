use dioxus::prelude::*;
use dxform::{FormScope, use_field, use_subform};
use semantic_data::{
    schema::{AttributeType, ClassAttribute, ClassType},
    value::{Object, Value},
};
use semantic_db_core::catalog::OBJECT_TYPE_FIELD;

use crate::{
    ValueView,
    form::{
        AttributeFormRenderContext, ClassFormRenderContext, SemanticFormErrors, SemanticFormMode,
        SemanticFormSubmit, attribute_field_spec, build_class_form_options,
        render_value_form_scope,
    },
    ui_catalog::{RenderMode, use_ui_catalog},
};

#[derive(Clone, PartialEq)]
pub struct ClassFormField {
    pub field_name: String,
    pub attribute: AttributeType,
    pub class_attribute: ClassAttribute,
    pub declaring_class_id: String,
}

#[component]
pub fn DynamicClassForm(
    class: ClassType,
    object: Object,
    mode: SemanticFormMode,
    collection: Option<String>,
    id: Option<String>,
    scope_id: Option<String>,
    submit: Option<SemanticFormSubmit>,
) -> Element {
    let options = build_class_form_options(class, object, mode, collection, id, scope_id, submit);
    rsx! { crate::form::DynamicValueForm { options } }
}

pub fn default_class_form_renderer(ctx: ClassFormRenderContext) -> Element {
    render_class_form_body(ctx)
}

pub fn render_class_form_body(ctx: ClassFormRenderContext) -> Element {
    let catalog = use_ui_catalog();
    let fields = catalog.class_form_fields(&ctx.class);
    let title = ctx
        .class
        .meta
        .title
        .clone()
        .unwrap_or_else(|| ctx.class.name.clone());
    rsx! {
        div { class: "semantic-form semantic-form--class",
            header { class: "semantic-form__header",
                h2 { "{title}" }
                if let Some(id) = &ctx.id {
                    code { "{id}" }
                }
            }
            for field in fields {
                if field.field_name != OBJECT_TYPE_FIELD {
                    {
                        rsx! {
                            ClassFormFieldRow {
                                key: "{field.field_name}",
                                scope: ctx.scope.clone(),
                                class: ctx.class.clone(),
                                field,
                                mode: ctx.mode,
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn class_form_fields(catalog: &crate::UiCatalog, class: &ClassType) -> Vec<ClassFormField> {
    let mut fields = Vec::<ClassFormField>::new();
    collect_class_fields(catalog, class, &mut fields);
    fields
}

fn collect_class_fields(
    catalog: &crate::UiCatalog,
    class: &ClassType,
    fields: &mut Vec<ClassFormField>,
) {
    if let Some(parent) = &class.inherits {
        if let Some(parent_class) = catalog.class_by_id(&parent.id) {
            collect_class_fields(catalog, parent_class, fields);
        }
    }
    for extended in &class.extends {
        if let Some(extended_class) = catalog.class_by_id(&extended.id) {
            collect_class_fields(catalog, extended_class, fields);
        }
    }
    for (field_name, class_attribute) in &class.attributes {
        if let Some(attribute) = catalog.attribute_by_id(&class_attribute.attribute.id) {
            let field = ClassFormField {
                field_name: field_name.clone(),
                attribute: attribute.clone(),
                class_attribute: class_attribute.clone(),
                declaring_class_id: class.id.clone(),
            };
            if let Some(existing) = fields
                .iter_mut()
                .find(|existing| existing.field_name == field_name.as_str())
            {
                *existing = field;
            } else {
                fields.push(field);
            }
        }
    }
}

#[component]
fn ClassFormFieldRow(
    scope: FormScope<Value, Value>,
    class: ClassType,
    field: ClassFormField,
    mode: SemanticFormMode,
) -> Element {
    let catalog = use_ui_catalog();
    let label = field
        .class_attribute
        .meta
        .title
        .clone()
        .or_else(|| field.attribute.meta.title.clone())
        .unwrap_or_else(|| field.field_name.clone());
    let current = scope.value();
    let readonly_value = match &current {
        Value::Object(object) => object
            .get(&field.field_name)
            .cloned()
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    if field.class_attribute.computed.is_some() {
        return rsx! {
            div { class: "semantic-form__field semantic-form__field--readonly",
                label { class: "semantic-form__label", "{label}" }
                div { class: "semantic-form__control",
                    ValueView {
                        value: readonly_value,
                        type_hint: Some(field.attribute.ty.clone()),
                        mode: RenderMode::Detail
                    }
                }
            }
        };
    }
    let field_for_spec = field.clone();
    let catalog_for_spec = catalog.clone();
    let field_handle = use_field(scope.clone(), move || {
        attribute_field_spec(
            field_for_spec.field_name,
            field_for_spec.attribute,
            field_for_spec.class_attribute,
            catalog_for_spec,
        )
    });
    let renderer = catalog
        .form_registry()
        .class_field_form_renderer(&class.id, &field.field_name)
        .or_else(|| {
            catalog
                .form_registry()
                .attribute_form_renderer(&field.attribute.id)
        });
    let body = if let Some(renderer) = renderer {
        renderer(AttributeFormRenderContext {
            scope: scope.clone(),
            field: field_handle.clone(),
            class: class.clone(),
            field_name: field.field_name.clone(),
            attribute: field.attribute.clone(),
            class_attribute: field.class_attribute.clone(),
            mode,
        })
    } else {
        let field_scope = use_field_handle_scope(scope, field_handle.clone());
        render_value_form_scope(crate::form::ValueFormRenderContext {
            path: field_handle.path(),
            scope: field_scope,
            value_type: Some(field.attribute.ty.clone()),
            mode,
        })
    };
    rsx! {
        div { class: "semantic-form__field",
            label { class: "semantic-form__label", "{label}" }
            div { class: "semantic-form__control", {body} }
            SemanticFormErrors { errors: field_handle.meta().errors }
        }
    }
}

fn use_field_handle_scope(
    parent: FormScope<Value, Value>,
    field: dxform::FieldHandle<Value, Value>,
) -> FormScope<Value, Value> {
    let field_name = field
        .path()
        .as_str()
        .rsplit('.')
        .next()
        .unwrap_or("value")
        .to_string();
    let get_field_name = field_name.clone();
    let set_field_name = field_name.clone();
    use_subform(parent, move || dxform::SubformSpec {
        name: field_name,
        get: std::rc::Rc::new(move |parent: &Value| match parent {
            Value::Object(object) => object.get(&get_field_name).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }),
        set: std::rc::Rc::new(move |parent: &mut Value, value: Value| {
            crate::form::set_object_field_value(parent, &set_field_name, value);
        }),
        is_empty: std::rc::Rc::new(crate::form::is_empty_value),
        validators: Vec::new(),
        validation: dxform::ValidationStrategy::submit(),
    })
}
