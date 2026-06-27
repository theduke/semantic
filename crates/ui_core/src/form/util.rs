use dioxus::prelude::*;
use dxform::{FormError, FormRoot, use_form_with_options};
use semantic_data::value::Value;

use crate::{
    form::{
        ClassFormRenderContext, SemanticFormMode, SemanticFormOptions, ValueFormRenderContext,
        build_value_form_options, default_class_form_renderer, provide_semantic_form_mode,
        provide_semantic_form_root, provide_semantic_form_scope,
    },
    ui_catalog::use_ui_catalog,
};

#[component]
pub fn DynamicValueForm(options: SemanticFormOptions) -> Element {
    let catalog = use_ui_catalog();
    let form_options = build_value_form_options(&options);
    let form = use_form_with_options(move || form_options);
    let scope = form.scope();
    provide_semantic_form_root(form.clone());
    provide_semantic_form_scope(scope.clone());
    provide_semantic_form_mode(options.mode);

    let body = if let Some(class) = options.class.clone() {
        let renderer = catalog
            .form_registry()
            .class_form_renderer(&class.id)
            .or_else(|| {
                class.inherits.as_ref().and_then(|parent| {
                    catalog
                        .form_registry()
                        .class_form_renderer(&parent.id)
                        .filter(|_| catalog.class_inherits(&class.id, &parent.id))
                })
            });
        let ctx = ClassFormRenderContext {
            form: form.clone(),
            scope: scope.clone(),
            class,
            collection: options.collection.clone(),
            id: options.id.clone(),
            mode: options.mode,
        };
        if let Some(renderer) = renderer {
            renderer(ctx)
        } else {
            default_class_form_renderer(ctx)
        }
    } else {
        let path = scope.path();
        render_value_form_scope(ValueFormRenderContext {
            scope: scope.clone(),
            value_type: options.type_hint.clone(),
            mode: options.mode,
            path,
        })
    };

    rsx! {
        form {
            class: "semantic-form",
            onsubmit: form.submit_handler(),
            {body}
            if options.show_actions {
                DynamicFormActions { form: form.clone() }
            }
        }
    }
}

pub fn render_value_form_scope(ctx: ValueFormRenderContext) -> Element {
    let catalog = use_ui_catalog();
    if let Some(ty) = &ctx.value_type {
        if let Some(renderer) = catalog.form_registry().type_form_renderer(&ty.kind) {
            return renderer(ctx);
        }
    }
    if let Some(renderer) = catalog.form_registry().fallback_form_renderer() {
        return renderer(ctx);
    }
    rsx! { div { class: "semantic-form__unsupported", "No form renderer registered" } }
}

#[component]
pub fn DynamicFormActions(form: FormRoot<Value>) -> Element {
    let meta = form.meta();
    let status = if meta.submitting {
        "Submitting"
    } else if meta.dirty {
        "Modified"
    } else {
        "Unchanged"
    };
    rsx! {
        div { class: "semantic-form__actions",
            button {
                r#type: "submit",
                disabled: meta.submitting,
                "Submit"
            }
            button {
                r#type: "button",
                disabled: meta.submitting,
                onclick: move |_| form.reset(),
                "Reset"
            }
            span { class: "semantic-form__status", "{status}" }
            SemanticFormErrors { errors: meta.errors }
            SemanticFormErrors { errors: meta.submit_errors }
        }
    }
}

#[component]
pub fn SemanticFormErrors(errors: Vec<FormError>) -> Element {
    rsx! {
        if !errors.is_empty() {
            ul { class: "semantic-form__errors",
                for error in errors {
                    li { class: "semantic-form__error",
                        if let Some(path) = &error.path {
                            code { "{path}" }
                            " "
                        }
                        "{error.message}"
                    }
                }
            }
        }
    }
}

pub(crate) fn mode_from_render_mode(
    mode: crate::ui_catalog::RenderMode,
) -> Option<SemanticFormMode> {
    match mode {
        crate::ui_catalog::RenderMode::Create => Some(SemanticFormMode::Create),
        crate::ui_catalog::RenderMode::Edit => Some(SemanticFormMode::Edit),
        _ => None,
    }
}
