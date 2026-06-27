use std::rc::Rc;

use dioxus::prelude::*;
use futures::FutureExt;

use crate::{
    FieldPath, FormError, FormErrorSource, FormMeta, FormNodeKind, FormNodeState, FormRegistry,
    FormScope, FormValidationContext, FormValidator, ScopeMeta, SubmitContext, SubmitError,
    SubmitHandler, ValidationPhase, ValidationStrategy, Validity,
};

#[derive(Clone)]
pub struct FormOptions<T> {
    pub initial_values: T,
    pub validation: ValidationStrategy,
    pub validators: Vec<FormValidator<T>>,
    pub submit: Option<SubmitHandler<T>>,
    pub clear_submit_errors_on_change: bool,
    pub prevent_concurrent_submit: bool,
}

impl<T> FormOptions<T> {
    pub fn new(initial_values: T) -> Self {
        Self {
            initial_values,
            validation: ValidationStrategy::default(),
            validators: Vec::new(),
            submit: None,
            clear_submit_errors_on_change: true,
            prevent_concurrent_submit: true,
        }
    }

    pub fn validator(mut self, validator: FormValidator<T>) -> Self {
        self.validators.push(validator);
        self
    }

    pub fn on_submit(mut self, submit: SubmitHandler<T>) -> Self {
        self.submit = Some(submit);
        self
    }

    pub fn validation(mut self, validation: ValidationStrategy) -> Self {
        self.validation = validation;
        self
    }
}

pub(crate) struct FormState<Root: Clone + PartialEq + 'static> {
    pub values: Signal<Root>,
    pub initial_values: Signal<Root>,
    pub meta: Signal<FormMeta>,
    pub registry: Signal<FormRegistry<Root>>,
    pub options: Rc<FormOptions<Root>>,
    pub next_key: Signal<u64>,
}

#[derive(Clone)]
pub struct FormRoot<T: Clone + PartialEq + 'static> {
    pub(crate) state: Rc<FormState<T>>,
}

impl<T: Clone + PartialEq + 'static> PartialEq for FormRoot<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.state, &other.state)
    }
}

pub fn use_form<T>(initial: impl FnOnce() -> T) -> FormRoot<T>
where
    T: Clone + PartialEq + 'static,
{
    use_hook(|| FormRoot::new(initial()))
}

pub fn use_form_with_options<T>(options: impl FnOnce() -> FormOptions<T>) -> FormRoot<T>
where
    T: Clone + PartialEq + 'static,
{
    use_hook(|| FormRoot::with_options(options()))
}

impl<T: Clone + PartialEq + 'static> FormRoot<T> {
    pub fn new(initial_values: T) -> Self {
        Self::with_options(FormOptions::new(initial_values))
    }

    pub fn with_options(options: FormOptions<T>) -> Self {
        let initial_values = options.initial_values.clone();
        let mut registry = FormRegistry::default();
        let mut root = FormNodeState::new(FormNodeKind::Scope, FieldPath::root());
        root.empty = false;
        registry.nodes.insert(FieldPath::root(), root);
        let root = Self {
            state: Rc::new(FormState {
                values: Signal::new(initial_values.clone()),
                initial_values: Signal::new(initial_values),
                meta: Signal::new(FormMeta::default()),
                registry: Signal::new(registry),
                options: Rc::new(options),
                next_key: Signal::new(1),
            }),
        };
        root.register_form_validators();
        root.recompute_all_meta();
        root
    }

    pub fn values(&self) -> T {
        self.state.values.read().clone()
    }

    pub fn values_signal(&self) -> Signal<T> {
        self.state.values
    }

    pub fn meta(&self) -> FormMeta {
        self.state.meta.read().clone()
    }

    pub fn meta_signal(&self) -> Signal<FormMeta> {
        self.state.meta
    }

    pub fn scope(&self) -> FormScope<T> {
        FormScope::new(
            self.clone(),
            FieldPath::root(),
            Rc::new(|value: &T| value.clone()),
            Rc::new(|root: &mut T, value: T| *root = value),
            Rc::new(|_value: &T| false),
        )
    }

    pub fn reset(&self) {
        let initial = self.state.initial_values.read().clone();
        self.set_values_signal(initial);
        self.with_registry(|registry| {
            for node in registry.nodes.values_mut() {
                node.touched = false;
                node.focused = false;
                node.dirty = false;
                node.validating = false;
                node.validity = Validity::NotValidated;
                node.errors.clear();
                node.submit_errors.clear();
            }
        });
        self.recompute_all_meta();
    }

    pub fn reset_to(&self, values: T) {
        self.set_initial_values_signal(values.clone());
        self.set_values_signal(values);
        self.reset();
    }

    pub fn commit_current_as_initial(&self) {
        self.set_initial_values_signal(self.values());
        self.with_registry(|registry| {
            for node in registry.nodes.values_mut() {
                node.dirty = false;
            }
        });
        self.recompute_all_meta();
    }

    pub fn touch_all(&self) {
        self.with_registry(|registry| registry.touch_under(&FieldPath::root()));
        self.recompute_all_meta();
    }

    pub fn clear_errors(&self) {
        self.with_registry(|registry| registry.clear_errors_under(&FieldPath::root(), false));
        self.recompute_all_meta();
    }

    pub fn clear_submit_errors(&self) {
        self.with_registry(|registry| registry.clear_errors_under(&FieldPath::root(), true));
        self.recompute_all_meta();
    }

    pub async fn validate(&self) -> std::result::Result<(), Vec<FormError>> {
        self.validate_path(FieldPath::root(), ValidationPhase::Manual)
            .await
    }

    pub async fn submit(&self) -> std::result::Result<T, SubmitError> {
        if self.state.options.prevent_concurrent_submit && self.meta().submitting {
            return Err(SubmitError::message("form is already submitting"));
        }

        self.touch_all();
        self.with_meta(|meta| {
            meta.submit_attempted = true;
            meta.submit_count += 1;
            meta.submit_succeeded = false;
            meta.submit_failed = false;
        });

        if let Err(errors) = self
            .validate_path(FieldPath::root(), ValidationPhase::Submit)
            .await
        {
            self.with_meta(|meta| {
                meta.submit_failed = true;
                meta.errors = errors.clone();
            });
            return Err(SubmitError::errors(errors));
        }

        self.with_meta(|meta| meta.submitting = true);
        let values = self.values();
        let submit_count = self.meta().submit_count;
        let result = if let Some(submit) = &self.state.options.submit {
            submit
                .submit(SubmitContext {
                    values: values.clone(),
                    submit_count,
                })
                .await
        } else {
            Ok(())
        };

        self.with_meta(|meta| {
            meta.submitting = false;
            match &result {
                Ok(()) => {
                    meta.submit_succeeded = true;
                    meta.submit_failed = false;
                    meta.submit_errors.clear();
                    meta.dirty_since_last_submit = false;
                }
                Err(err) => {
                    meta.submit_succeeded = false;
                    meta.submit_failed = true;
                    meta.submit_errors = submit_errors(err);
                }
            }
        });

        match result {
            Ok(()) => Ok(values),
            Err(err) => {
                self.apply_submit_errors(&err);
                Err(err)
            }
        }
    }

    pub fn submit_handler(&self) -> EventHandler<FormEvent> {
        let form = self.clone();
        EventHandler::new(move |event: FormEvent| {
            event.prevent_default();
            let form = form.clone();
            spawn(async move {
                let _ = form.submit().await;
            });
        })
    }

    pub(crate) fn allocate_key(&self) -> u64 {
        let key = *self.state.next_key.read();
        self.set_next_key(key + 1);
        key
    }

    pub(crate) fn mutate_values(&self, f: impl FnOnce(&mut T)) {
        let mut values = self.values();
        f(&mut values);
        self.set_values_signal(values);
        self.with_meta(|meta| {
            meta.submit_succeeded = false;
            meta.dirty_since_last_submit = true;
        });
        if self.state.options.clear_submit_errors_on_change {
            self.clear_submit_errors();
        }
        self.recompute_all_meta();
    }

    pub(crate) fn recompute_all_meta(&self) {
        let current = self.state.values.read().clone();
        let initial = self.state.initial_values.read().clone();
        self.with_registry(|registry| {
            let paths = registry.nodes.keys().cloned().collect::<Vec<_>>();
            for path in paths.iter().rev() {
                let descendants = registry
                    .nodes
                    .values()
                    .filter(|node| node.path != *path && node.path.starts_with(path))
                    .cloned()
                    .collect::<Vec<_>>();
                if let Some(node) = registry.nodes.get_mut(path) {
                    if node.path.is_root() {
                        node.dirty = current != initial;
                    }
                    node.touched |= descendants.iter().any(|child| child.touched);
                    node.dirty |= descendants.iter().any(|child| child.dirty);
                    node.empty = descendants
                        .iter()
                        .filter(|child| {
                            matches!(child.kind, FormNodeKind::Field | FormNodeKind::List)
                        })
                        .all(|child| child.empty);
                    node.validating |= descendants.iter().any(|child| child.validating);
                    let mut errors = node.errors.clone();
                    let mut submit_errors = node.submit_errors.clone();
                    for child in descendants {
                        errors.extend(child.errors);
                        submit_errors.extend(child.submit_errors);
                    }
                    let self_invalid = !node.errors.is_empty() || !node.submit_errors.is_empty();
                    node.validity = if node.validating {
                        Validity::Validating
                    } else if self_invalid {
                        Validity::Invalid
                    } else if node.validity == Validity::NotValidated {
                        Validity::NotValidated
                    } else {
                        Validity::Valid
                    };
                    if !errors.is_empty() {
                        node.validity = Validity::Invalid;
                    }
                    if node.path.is_root() {
                        self.set_meta(FormMeta {
                            touched: node.touched,
                            dirty: current != initial || node.dirty,
                            empty: node.empty,
                            validating: node.validating,
                            validity: node.validity,
                            errors,
                            submit_errors,
                            submitting: self.state.meta.peek().submitting,
                            submit_count: self.state.meta.peek().submit_count,
                            submit_attempted: self.state.meta.peek().submit_attempted,
                            submit_succeeded: self.state.meta.peek().submit_succeeded,
                            submit_failed: self.state.meta.peek().submit_failed,
                            dirty_since_last_submit: self.state.meta.peek().dirty_since_last_submit,
                        });
                    }
                }
            }
        });
    }

    pub(crate) fn scope_meta(&self, path: &FieldPath) -> ScopeMeta {
        self.state
            .registry
            .read()
            .nodes
            .get(path)
            .map(FormNodeState::scope_meta)
            .unwrap_or_else(|| ScopeMeta::new(path.clone()))
    }

    pub(crate) async fn validate_path(
        &self,
        path: FieldPath,
        phase: ValidationPhase,
    ) -> std::result::Result<(), Vec<FormError>> {
        let validators = {
            let registry = self.state.registry.read();
            registry
                .nodes
                .iter()
                .filter(|(node_path, _)| node_path.starts_with(&path))
                .flat_map(|(_, node)| node.validators.clone())
                .collect::<Vec<_>>()
        };

        self.with_registry(|registry| {
            for node in registry
                .nodes
                .values_mut()
                .filter(|node| node.path.starts_with(&path))
            {
                node.validating = true;
                node.validation_epoch += 1;
                node.errors.clear();
            }
        });
        self.recompute_all_meta();

        let values = self.values();
        let mut all_errors = Vec::new();
        for validator in validators {
            all_errors.extend(validator(phase, values.clone()).await);
        }

        self.with_registry(|registry| {
            for node in registry
                .nodes
                .values_mut()
                .filter(|node| node.path.starts_with(&path))
            {
                node.validating = false;
                node.validity = Validity::Valid;
                node.errors.clear();
            }
            for error in &all_errors {
                let error_path = error.path.clone().unwrap_or_else(|| path.clone());
                let node_path = if registry.nodes.contains_key(&error_path) {
                    error_path
                } else {
                    path.clone()
                };
                if let Some(node) = registry.nodes.get_mut(&node_path) {
                    node.errors.push(error.clone());
                    node.validity = Validity::Invalid;
                }
            }
        });
        self.recompute_all_meta();

        if all_errors.is_empty() {
            Ok(())
        } else {
            Err(all_errors)
        }
    }

    pub(crate) fn apply_submit_errors(&self, err: &SubmitError) {
        let errors = submit_errors(err);
        self.with_registry(|registry| {
            for node in registry.nodes.values_mut() {
                node.submit_errors.clear();
            }
            for error in errors {
                let path = error.path.clone().unwrap_or_else(FieldPath::root);
                let node_path = if registry.nodes.contains_key(&path) {
                    path
                } else {
                    FieldPath::root()
                };
                if let Some(node) = registry.nodes.get_mut(&node_path) {
                    node.submit_errors.push(error);
                }
            }
        });
        self.recompute_all_meta();
    }

    pub(crate) fn register_form_validators(&self) {
        let validators = self
            .state
            .options
            .validators
            .iter()
            .cloned()
            .map(|validator| {
                Rc::new(move |phase, values: T| {
                    let validator = validator.clone();
                    async move {
                        validator
                            .validate(FormValidationContext { phase, values })
                            .await
                    }
                    .boxed_local()
                }) as crate::DynValidator<T>
            })
            .collect::<Vec<_>>();
        self.with_registry(|registry| {
            if let Some(node) = registry.nodes.get_mut(&FieldPath::root()) {
                node.validators.extend(validators);
            }
        });
    }

    pub(crate) fn with_registry<R>(&self, f: impl FnOnce(&mut FormRegistry<T>) -> R) -> R {
        let mut registry = self.state.registry;
        registry.with_mut(f)
    }

    pub(crate) fn with_meta<R>(&self, f: impl FnOnce(&mut FormMeta) -> R) -> R {
        let mut meta = self.state.meta;
        meta.with_mut(f)
    }

    pub(crate) fn set_meta(&self, value: FormMeta) {
        let mut meta = self.state.meta;
        meta.set(value);
    }

    pub(crate) fn set_values_signal(&self, value: T) {
        let mut values = self.state.values;
        values.set(value);
    }

    pub(crate) fn set_initial_values_signal(&self, value: T) {
        let mut values = self.state.initial_values;
        values.set(value);
    }

    pub(crate) fn set_next_key(&self, value: u64) {
        let mut next_key = self.state.next_key;
        next_key.set(value);
    }
}

fn submit_errors(err: &SubmitError) -> Vec<FormError> {
    if err.errors.is_empty() {
        err.message
            .as_ref()
            .map(|message| {
                FormError::new(message.clone())
                    .with_source(FormErrorSource::Submission)
                    .at_path(&FieldPath::root())
            })
            .into_iter()
            .collect()
    } else {
        err.errors
            .iter()
            .cloned()
            .map(|error| error.with_source(FormErrorSource::Submission))
            .collect()
    }
}

pub fn provide_form_root<T>(form: FormRoot<T>) -> FormRoot<T>
where
    T: Clone + PartialEq + 'static,
{
    provide_context(form)
}

pub fn use_form_root<T>() -> FormRoot<T>
where
    T: Clone + PartialEq + 'static,
{
    use_context()
}
