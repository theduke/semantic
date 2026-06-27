use std::collections::BTreeSet;

use dioxus::prelude::*;
use dxform::{FormScope, use_field};
use semantic_data::{
    schema::{AttributeType, ClassAttribute, ClassType},
    value::{Object, Value},
};
use semantic_db_core::catalog::OBJECT_TYPE_FIELD;

use crate::{
    ValueView,
    form::{
        AttributeFormRenderContext, ClassFormRenderContext, SemanticFormErrors, SemanticFormMode,
        SemanticFormSubmit, build_class_form_options, is_empty_value, render_value_form_scope,
        value_field_spec,
    },
    ui_catalog::{RenderMode, use_ui_catalog},
};

#[derive(Clone, PartialEq)]
pub struct ClassFormField {
    pub field_name: String,
    pub storage_field_name: String,
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
    let known_field_names = fields
        .iter()
        .flat_map(|field| [field.field_name.clone(), field.storage_field_name.clone()])
        .chain(std::iter::once(OBJECT_TYPE_FIELD.to_string()))
        .collect::<BTreeSet<_>>();
    let extra_fields = match ctx.scope.value() {
        Value::Object(object) => object
            .iter()
            .filter(|(key, _)| !known_field_names.contains(*key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
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
                if field.field_name != OBJECT_TYPE_FIELD && field.storage_field_name != OBJECT_TYPE_FIELD {
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
            for (field_name, value) in extra_fields {
                ExtraClassFormFieldRow {
                    key: "{field_name}",
                    scope: ctx.scope.clone(),
                    field_name,
                    value,
                    mode: ctx.mode,
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
                storage_field_name: attribute.id.clone(),
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
    let field_for_spec = field.clone();
    let catalog_for_spec = catalog.clone();
    let field_handle = use_field(scope.clone(), move || {
        crate::form::attribute_field_spec_with_storage_name(
            field_for_spec.field_name,
            field_for_spec.storage_field_name,
            field_for_spec.attribute,
            field_for_spec.class_attribute,
            catalog_for_spec,
        )
    });
    if field.class_attribute.computed.is_some() {
        return rsx! {
            ReadonlyClassFormField {
                field: field_handle,
                label,
                type_hint: field.attribute.ty.clone(),
            }
        };
    }
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
        let field_scope = field_handle.scope();
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

#[component]
fn ExtraClassFormFieldRow(
    scope: FormScope<Value, Value>,
    field_name: String,
    value: Value,
    mode: SemanticFormMode,
) -> Element {
    let field_name_for_spec = field_name.clone();
    let field = use_field(scope, move || {
        value_field_spec(
            field_name_for_spec,
            value,
            std::rc::Rc::new(is_empty_value),
            Vec::new(),
            dxform::ValidationStrategy::submit(),
        )
    });
    let field_scope = field.scope();
    rsx! {
        div { class: "semantic-form__field semantic-form__field--extra",
            label { class: "semantic-form__label", "{field_name}" }
            div { class: "semantic-form__control",
                {render_value_form_scope(crate::form::ValueFormRenderContext {
                    path: field.path(),
                    scope: field_scope,
                    value_type: None,
                    mode,
                })}
            }
            SemanticFormErrors { errors: field.meta().errors }
        }
    }
}

#[component]
fn ReadonlyClassFormField(
    field: dxform::FieldHandle<Value, Value>,
    label: String,
    type_hint: semantic_data::schema::Type,
) -> Element {
    rsx! {
        div { class: "semantic-form__field semantic-form__field--readonly",
            label { class: "semantic-form__label", "{label}" }
            div { class: "semantic-form__control",
                ValueView {
                    value: field.value(),
                    type_hint: Some(type_hint),
                    mode: RenderMode::Detail
                }
            }
        }
    }
}
