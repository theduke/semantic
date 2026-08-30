use dioxus::prelude::*;
use dxform::{FormError, FormRoot, use_form_scope_from_root, use_form_with_options};
use semantic_data::value::Value;

use crate::{
    form::{
        ClassFormRenderContext, SemanticFormMode, SemanticFormOptions, ValueFormRenderContext,
        build_value_form_options, default_class_form_renderer, provide_semantic_form_mode,
        provide_semantic_form_root, provide_semantic_form_scope,
    },
    ui_catalog::use_ui_catalog,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticFormActionLabels {
    pub submit: String,
    pub reset: String,
}

impl SemanticFormActionLabels {
    pub fn new(submit: impl Into<String>, reset: impl Into<String>) -> Self {
        Self {
            submit: submit.into(),
            reset: reset.into(),
        }
    }

    pub fn create_entity() -> Self {
        Self::new("Create entity", "Discard changes")
    }

    pub fn save_changes() -> Self {
        Self::new("Save changes", "Discard changes")
    }
}

impl Default for SemanticFormActionLabels {
    fn default() -> Self {
        Self::new("Submit", "Reset")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticFormSubmitOutcome {
    pub submit_count: u64,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticFormSubmitFailure {
    pub submit_count: u64,
    pub errors: Vec<FormError>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FormObserverState {
    dirty: Option<bool>,
    submitting: Option<bool>,
    reported_submit_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FormObserverTransition {
    dirty: Option<bool>,
    submitting: Option<bool>,
    submit_succeeded: bool,
    submit_failed: bool,
}

fn semantic_form_is_dirty(meta: &dxform::FormMeta) -> bool {
    meta.dirty && (meta.submit_count == 0 || meta.dirty_since_last_submit)
}

fn observe_form_meta(
    state: &mut FormObserverState,
    meta: &dxform::FormMeta,
) -> FormObserverTransition {
    let dirty = semantic_form_is_dirty(meta);
    let transition = FormObserverTransition {
        dirty: (state.dirty != Some(dirty)).then_some(dirty),
        submitting: (state.submitting != Some(meta.submitting)).then_some(meta.submitting),
        submit_succeeded: meta.submit_count > state.reported_submit_count && meta.submit_succeeded,
        submit_failed: meta.submit_count > state.reported_submit_count && meta.submit_failed,
    };

    state.dirty = Some(dirty);
    state.submitting = Some(meta.submitting);
    if transition.submit_succeeded || transition.submit_failed {
        state.reported_submit_count = meta.submit_count;
    }
    transition
}

#[component]
pub fn DynamicValueForm(
    options: SemanticFormOptions,
    #[props(default)] action_labels: SemanticFormActionLabels,
    #[props(default)] on_dirty_change: Option<EventHandler<bool>>,
    #[props(default)] on_submitting_change: Option<EventHandler<bool>>,
    #[props(default)] on_submit_success: Option<EventHandler<SemanticFormSubmitOutcome>>,
    #[props(default)] on_submit_failure: Option<EventHandler<SemanticFormSubmitFailure>>,
) -> Element {
    let catalog = use_ui_catalog();
    let form_options = build_value_form_options(&options);
    let form = use_form_with_options(move || form_options);
    let scope = use_form_scope_from_root(form.clone());
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
            SemanticFormObserver {
                form: form.clone(),
                on_dirty_change,
                on_submitting_change,
                on_submit_success,
                on_submit_failure,
            }
            {body}
            if options.show_actions {
                DynamicFormActions { form: form.clone(), labels: action_labels }
            }
        }
    }
}

#[component]
fn SemanticFormObserver(
    form: FormRoot<Value>,
    on_dirty_change: Option<EventHandler<bool>>,
    on_submitting_change: Option<EventHandler<bool>>,
    on_submit_success: Option<EventHandler<SemanticFormSubmitOutcome>>,
    on_submit_failure: Option<EventHandler<SemanticFormSubmitFailure>>,
) -> Element {
    let meta = form.meta_signal();
    let mut observer_state = use_signal(FormObserverState::default);

    use_effect(move || {
        let meta = meta();
        let transition = observer_state.with_mut(|state| observe_form_meta(state, &meta));

        if let (Some(callback), Some(dirty)) = (on_dirty_change, transition.dirty) {
            callback.call(dirty);
        }
        if let (Some(callback), Some(submitting)) = (on_submitting_change, transition.submitting) {
            callback.call(submitting);
        }
        if transition.submit_succeeded {
            if let Some(callback) = on_submit_success {
                callback.call(SemanticFormSubmitOutcome {
                    submit_count: meta.submit_count,
                    value: form.values(),
                });
            }
        } else if transition.submit_failed
            && let Some(callback) = on_submit_failure
        {
            let mut errors = meta.errors;
            errors.extend(meta.submit_errors);
            callback.call(SemanticFormSubmitFailure {
                submit_count: meta.submit_count,
                errors,
            });
        }
    });

    rsx! {}
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
pub fn DynamicFormActions(
    form: FormRoot<Value>,
    #[props(default)] labels: SemanticFormActionLabels,
) -> Element {
    let meta = form.meta();
    let status = if meta.submitting {
        "Submitting"
    } else if semantic_form_is_dirty(&meta) {
        "Modified"
    } else {
        "Unchanged"
    };
    rsx! {
        div { class: "semantic-form__actions",
            dxcomp::Button {
                r#type: "submit",
                disabled: meta.submitting,
                aria_busy: meta.submitting,
                "{labels.submit}"
            }
            dxcomp::Button {
                variant: dxcomp::ButtonVariant::Outline,
                r#type: "button",
                disabled: meta.submitting,
                onclick: move |_| form.reset(),
                "{labels.reset}"
            }
            span { class: "semantic-form__status", aria_live: "polite", "{status}" }
            SemanticFormErrors { errors: meta.errors }
            SemanticFormErrors { errors: meta.submit_errors }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_label_presets_express_route_intent() {
        assert_eq!(
            SemanticFormActionLabels::default(),
            SemanticFormActionLabels::new("Submit", "Reset")
        );
        assert_eq!(
            SemanticFormActionLabels::create_entity(),
            SemanticFormActionLabels::new("Create entity", "Discard changes")
        );
        assert_eq!(
            SemanticFormActionLabels::save_changes(),
            SemanticFormActionLabels::new("Save changes", "Discard changes")
        );
    }

    #[test]
    fn observer_reports_each_submit_outcome_once() {
        let mut state = FormObserverState::default();
        let initial = dxform::FormMeta::default();
        assert_eq!(
            observe_form_meta(&mut state, &initial),
            FormObserverTransition {
                dirty: Some(false),
                submitting: Some(false),
                ..FormObserverTransition::default()
            }
        );

        let started = dxform::FormMeta {
            dirty: true,
            dirty_since_last_submit: true,
            submitting: true,
            submit_count: 1,
            ..dxform::FormMeta::default()
        };
        let transition = observe_form_meta(&mut state, &started);
        assert_eq!(transition.dirty, Some(true));
        assert_eq!(transition.submitting, Some(true));
        assert!(!transition.submit_succeeded);

        let succeeded = dxform::FormMeta {
            dirty: true,
            submitting: false,
            submit_count: 1,
            submit_succeeded: true,
            ..dxform::FormMeta::default()
        };
        let transition = observe_form_meta(&mut state, &succeeded);
        assert_eq!(transition.dirty, Some(false));
        assert_eq!(transition.submitting, Some(false));
        assert!(transition.submit_succeeded);
        assert!(!observe_form_meta(&mut state, &succeeded).submit_succeeded);
    }

    #[test]
    fn validation_failure_remains_dirty_and_is_reported() {
        let mut state = FormObserverState::default();
        let failed = dxform::FormMeta {
            dirty: true,
            dirty_since_last_submit: true,
            submit_count: 1,
            submit_failed: true,
            ..dxform::FormMeta::default()
        };

        let transition = observe_form_meta(&mut state, &failed);
        assert_eq!(transition.dirty, Some(true));
        assert!(transition.submit_failed);
        assert!(!observe_form_meta(&mut state, &failed).submit_failed);
    }
}

#[component]
pub fn SemanticFormErrors(errors: Vec<FormError>, #[props(default)] id: Option<String>) -> Element {
    rsx! {
        if !errors.is_empty() {
            ul {
                id,
                class: "semantic-form__errors",
                role: "alert",
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
