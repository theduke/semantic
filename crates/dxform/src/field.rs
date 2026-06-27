use std::rc::Rc;

use dioxus::prelude::*;
use futures::FutureExt;

use crate::{
    DynValidator, FieldMeta, FieldPath, FieldValidationContext, FieldValidator, FormError,
    FormNodeKind, FormNodeState, FormRoot, FormScope, ValidationPhase, ValidationStrategy,
    set_signal_if_changed,
};

pub struct FieldSpec<Parent, Value> {
    pub name: String,
    pub get: Rc<dyn Fn(&Parent) -> Value>,
    pub set: Rc<dyn Fn(&mut Parent, Value)>,
    pub is_empty: Rc<dyn Fn(&Value) -> bool>,
    pub validators: Vec<FieldValidator<Parent, Value>>,
    pub validation: ValidationStrategy,
}

impl<Parent, Value> FieldSpec<Parent, Value>
where
    Value: Default + PartialEq + 'static,
{
    pub fn new(
        name: impl Into<String>,
        get: impl Fn(&Parent) -> Value + 'static,
        set: impl Fn(&mut Parent, Value) + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            get: Rc::new(get),
            set: Rc::new(set),
            is_empty: Rc::new(|value: &Value| value == &Value::default()),
            validators: Vec::new(),
            validation: ValidationStrategy::default(),
        }
    }
}

impl<Parent, Value> FieldSpec<Parent, Value> {
    pub fn validator(mut self, validator: FieldValidator<Parent, Value>) -> Self {
        self.validators.push(validator);
        self
    }

    pub fn is_empty(mut self, f: impl Fn(&Value) -> bool + 'static) -> Self {
        self.is_empty = Rc::new(f);
        self
    }

    pub fn validation(mut self, strategy: ValidationStrategy) -> Self {
        self.validation = strategy;
        self
    }
}

pub struct FieldHandle<Value, Root = Value>
where
    Root: Clone + PartialEq + 'static,
    Value: Clone + PartialEq + 'static,
{
    pub(crate) root: FormRoot<Root>,
    pub(crate) path: FieldPath,
    get: Rc<dyn Fn(&Root) -> Value>,
    set: Rc<dyn Fn(&mut Root, Value)>,
    is_empty: Rc<dyn Fn(&Value) -> bool>,
    value_signal: Signal<Value>,
    initial_value_signal: Signal<Value>,
    meta_signal: Signal<FieldMeta>,
}

impl<Value, Root> Clone for FieldHandle<Value, Root>
where
    Root: Clone + PartialEq + 'static,
    Value: Clone + PartialEq + 'static,
{
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            path: self.path.clone(),
            get: self.get.clone(),
            set: self.set.clone(),
            is_empty: self.is_empty.clone(),
            value_signal: self.value_signal,
            initial_value_signal: self.initial_value_signal,
            meta_signal: self.meta_signal,
        }
    }
}

impl<Value, Root> PartialEq for FieldHandle<Value, Root>
where
    Root: Clone + PartialEq + 'static,
    Value: Clone + PartialEq + 'static,
{
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.path == other.path
    }
}

impl<Value, Root> FieldHandle<Value, Root>
where
    Root: Clone + PartialEq + 'static,
    Value: Clone + PartialEq + 'static,
{
    pub(crate) fn new<Parent>(
        scope: &FormScope<Parent, Root>,
        spec: FieldSpec<Parent, Value>,
    ) -> Self
    where
        Parent: Clone + PartialEq + 'static,
    {
        let path = scope.path.child(&spec.name);
        let parent_get_for_child = scope.get.clone();
        let parent_get_for_set = scope.get.clone();
        let parent_set = scope.set.clone();
        let field_get = spec.get.clone();
        let field_set = spec.set.clone();
        let get: Rc<dyn Fn(&Root) -> Value> = Rc::new(move |root| {
            let parent = parent_get_for_child(root);
            field_get(&parent)
        });
        let set: Rc<dyn Fn(&mut Root, Value)> = Rc::new(move |root, value| {
            let mut parent = parent_get_for_set(root);
            field_set(&mut parent, value);
            parent_set(root, parent);
        });
        let validators = spec
            .validators
            .into_iter()
            .map(|validator| {
                let get = get.clone();
                let parent_get = scope.get.clone();
                let path = path.clone();
                Rc::new(move |phase, root: Root| {
                    let validator = validator.clone();
                    let path = path.clone();
                    let parent = parent_get(&root);
                    let value = get(&root);
                    async move {
                        validator
                            .validate(FieldValidationContext {
                                phase,
                                path,
                                value,
                                parent,
                            })
                            .await
                    }
                    .boxed_local()
                }) as DynValidator<Root>
            })
            .collect::<Vec<_>>();
        let current = get(&scope.root.values());
        let initial = get(&scope.root.state.initial_values.read());
        let (value_signal, initial_value_signal, meta_signal) =
            scope.root.with_registry(|registry| {
                let value_signal = registry.ensure_value_signal(&path, current.clone());
                let initial_value_signal =
                    registry.ensure_initial_value_signal(&path, initial.clone());
                let node = registry
                    .nodes
                    .entry(path.clone())
                    .or_insert_with(|| FormNodeState::new(FormNodeKind::Field, path.clone()));
                node.validators.extend(validators);
                node.value_refresher = Some({
                    let get = get.clone();
                    Rc::new(move |root: &Root| {
                        set_signal_if_changed(value_signal, get(root));
                    })
                });
                node.initial_value_refresher = Some({
                    let get = get.clone();
                    Rc::new(move |root: &Root| {
                        set_signal_if_changed(initial_value_signal, get(root));
                    })
                });
                (value_signal, initial_value_signal, node.field_meta_signal)
            });
        let handle = Self {
            root: scope.root.clone(),
            path,
            get,
            set,
            is_empty: spec.is_empty,
            value_signal,
            initial_value_signal,
            meta_signal,
        };
        handle.refresh_node_state();
        handle
    }

    pub fn path(&self) -> FieldPath {
        self.path.clone()
    }

    pub fn value(&self) -> Value {
        self.value_signal.read().clone()
    }

    pub fn value_signal(&self) -> ReadSignal<Value> {
        self.value_signal.into()
    }

    pub fn meta(&self) -> FieldMeta {
        self.meta_signal.read().clone()
    }

    pub fn meta_signal(&self) -> ReadSignal<FieldMeta> {
        self.meta_signal.into()
    }

    pub fn set_value(&self, value: Value) {
        let set = self.set.clone();
        set_signal_if_changed(self.value_signal, value.clone());
        self.root
            .mutate_values_at(self.path.clone(), move |root| set(root, value));
        self.root.with_registry(|registry| {
            if let Some(node) = registry.nodes.get_mut(&self.path) {
                node.touched = true;
            }
        });
        self.refresh_node_state();
    }

    pub fn update_value(&self, f: impl FnOnce(&mut Value) + 'static) {
        let mut value = self.value();
        f(&mut value);
        self.set_value(value);
    }

    pub fn mark_touched(&self) {
        self.root.with_registry(|registry| {
            if let Some(node) = registry.nodes.get_mut(&self.path) {
                node.touched = true;
            }
        });
        self.root.recompute_all_meta();
    }

    pub fn set_focused(&self, focused: bool) {
        self.root.with_registry(|registry| {
            if let Some(node) = registry.nodes.get_mut(&self.path) {
                node.focused = focused;
                if !focused {
                    node.touched = true;
                }
            }
        });
        self.root.recompute_all_meta();
    }

    pub fn reset(&self) {
        let initial = (self.get)(&self.root.state.initial_values.read());
        self.set_value(initial);
        self.root.with_registry(|registry| {
            if let Some(node) = registry.nodes.get_mut(&self.path) {
                node.touched = false;
                node.focused = false;
                node.errors.clear();
                node.submit_errors.clear();
            }
        });
        self.refresh_node_state();
    }

    pub async fn validate(&self) -> std::result::Result<(), Vec<FormError>> {
        self.root
            .validate_path(self.path.clone(), ValidationPhase::Manual)
            .await
    }

    pub fn clear_errors(&self) {
        self.root
            .with_registry(|registry| registry.clear_errors_under(&self.path, false));
        self.root.recompute_all_meta();
    }

    pub fn clear_submit_errors(&self) {
        self.root
            .with_registry(|registry| registry.clear_errors_under(&self.path, true));
        self.root.recompute_all_meta();
    }

    pub(crate) fn refresh_node_state(&self) {
        let current = self.value();
        let initial = self.initial_value_signal.read().clone();
        let empty = (self.is_empty)(&current);
        self.root.with_registry(|registry| {
            let node = registry
                .nodes
                .entry(self.path.clone())
                .or_insert_with(|| FormNodeState::new(FormNodeKind::Field, self.path.clone()));
            node.dirty = current != initial;
            node.empty = empty;
        });
        self.root.recompute_all_meta();
    }
}
