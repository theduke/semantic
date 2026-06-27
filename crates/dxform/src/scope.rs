use std::rc::Rc;

use dioxus::prelude::*;
use futures::FutureExt;

use crate::{
    DynValidator, FieldHandle, FieldPath, FieldSpec, FormNodeKind, FormNodeState, FormRoot,
    ListHandle, ListSpec, ScopeMeta, ScopeValidationContext, ScopeValidator, ValidationPhase,
    ValidationStrategy,
};

type Getter<Root, T> = Rc<dyn Fn(&Root) -> T>;
type Setter<Root, T> = Rc<dyn Fn(&mut Root, T)>;
type Empty<T> = Rc<dyn Fn(&T) -> bool>;

pub struct SubformSpec<Parent, Child> {
    pub name: String,
    pub get: Rc<dyn Fn(&Parent) -> Child>,
    pub set: Rc<dyn Fn(&mut Parent, Child)>,
    pub is_empty: Rc<dyn Fn(&Child) -> bool>,
    pub validators: Vec<ScopeValidator<Child>>,
    pub validation: ValidationStrategy,
}

impl<Parent, Child> SubformSpec<Parent, Child>
where
    Child: Default + PartialEq + 'static,
{
    pub fn new(
        name: impl Into<String>,
        get: impl Fn(&Parent) -> Child + 'static,
        set: impl Fn(&mut Parent, Child) + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            get: Rc::new(get),
            set: Rc::new(set),
            is_empty: Rc::new(|value: &Child| value == &Child::default()),
            validators: Vec::new(),
            validation: ValidationStrategy::default(),
        }
    }
}

impl<Parent, Child> SubformSpec<Parent, Child> {
    pub fn validator(mut self, validator: ScopeValidator<Child>) -> Self {
        self.validators.push(validator);
        self
    }

    pub fn is_empty(mut self, f: impl Fn(&Child) -> bool + 'static) -> Self {
        self.is_empty = Rc::new(f);
        self
    }

    pub fn validation(mut self, strategy: ValidationStrategy) -> Self {
        self.validation = strategy;
        self
    }
}

pub struct FormScope<T, Root = T>
where
    Root: Clone + PartialEq + 'static,
    T: Clone + PartialEq + 'static,
{
    pub(crate) root: FormRoot<Root>,
    pub(crate) path: FieldPath,
    pub(crate) get: Getter<Root, T>,
    pub(crate) set: Setter<Root, T>,
    pub(crate) is_empty: Empty<T>,
}

impl<T, Root> Clone for FormScope<T, Root>
where
    Root: Clone + PartialEq + 'static,
    T: Clone + PartialEq + 'static,
{
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            path: self.path.clone(),
            get: self.get.clone(),
            set: self.set.clone(),
            is_empty: self.is_empty.clone(),
        }
    }
}

impl<T, Root> PartialEq for FormScope<T, Root>
where
    Root: Clone + PartialEq + 'static,
    T: Clone + PartialEq + 'static,
{
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.path == other.path
    }
}

impl<T, Root> FormScope<T, Root>
where
    Root: Clone + PartialEq + 'static,
    T: Clone + PartialEq + 'static,
{
    pub(crate) fn new(
        root: FormRoot<Root>,
        path: FieldPath,
        get: Getter<Root, T>,
        set: Setter<Root, T>,
        is_empty: Empty<T>,
    ) -> Self {
        root.with_registry(|registry| {
            registry.ensure_node(FormNodeKind::Scope, path.clone());
        });
        let scope = Self {
            root,
            path,
            get,
            set,
            is_empty,
        };
        scope.refresh_node_state();
        scope
    }

    pub fn path(&self) -> FieldPath {
        self.path.clone()
    }

    pub fn value(&self) -> T {
        (self.get)(&self.root.values())
    }

    pub fn meta(&self) -> ScopeMeta {
        self.root.scope_meta(&self.path)
    }

    pub fn meta_signal(&self) -> Signal<ScopeMeta> {
        let scope = self.clone();
        Signal::new(scope.meta())
    }

    pub fn reset(&self) {
        let initial = (self.get)(&self.root.state.initial_values.read());
        self.set_value(initial);
        self.root.with_registry(|registry| {
            for node in registry
                .nodes
                .values_mut()
                .filter(|node| node.path.starts_with(&self.path))
            {
                node.touched = false;
                node.focused = false;
                node.validating = false;
                node.errors.clear();
                node.submit_errors.clear();
            }
        });
        self.root.recompute_all_meta();
    }

    pub fn touch_all(&self) {
        self.root
            .with_registry(|registry| registry.touch_under(&self.path));
        self.root.recompute_all_meta();
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

    pub async fn validate(&self) -> std::result::Result<(), Vec<crate::FormError>> {
        self.root
            .validate_path(self.path.clone(), ValidationPhase::Manual)
            .await
    }

    pub fn subform<Child>(&self, spec: SubformSpec<T, Child>) -> FormScope<Child, Root>
    where
        Child: Clone + PartialEq + 'static,
    {
        let path = self.path.child(&spec.name);
        let parent_get_for_child = self.get.clone();
        let parent_get_for_set = self.get.clone();
        let parent_set = self.set.clone();
        let child_get = spec.get.clone();
        let child_set = spec.set.clone();
        let get: Getter<Root, Child> = Rc::new(move |root| {
            let parent = parent_get_for_child(root);
            child_get(&parent)
        });
        let set: Setter<Root, Child> = Rc::new(move |root, value| {
            let mut parent = parent_get_for_set(root);
            child_set(&mut parent, value);
            parent_set(root, parent);
        });
        let validators = spec
            .validators
            .into_iter()
            .map(|validator| {
                let get = get.clone();
                let path = path.clone();
                Rc::new(move |phase, root: Root| {
                    let validator = validator.clone();
                    let path = path.clone();
                    let value = get(&root);
                    async move {
                        validator
                            .validate(ScopeValidationContext { phase, path, value })
                            .await
                    }
                    .boxed_local()
                }) as DynValidator<Root>
            })
            .collect::<Vec<_>>();
        self.root.with_registry(|registry| {
            let node = registry
                .nodes
                .entry(path.clone())
                .or_insert_with(|| FormNodeState::new(FormNodeKind::Scope, path.clone()));
            node.validators.extend(validators);
        });
        FormScope::new(self.root.clone(), path, get, set, spec.is_empty)
    }

    pub fn field<Value>(&self, spec: FieldSpec<T, Value>) -> FieldHandle<Value, Root>
    where
        Value: Clone + PartialEq + 'static,
    {
        FieldHandle::new(self, spec)
    }

    pub fn list<Item>(&self, spec: ListSpec<T, Item>) -> ListHandle<Item, Root>
    where
        Item: Clone + PartialEq + 'static,
    {
        ListHandle::new(self, spec)
    }

    pub(crate) fn set_value(&self, value: T) {
        let set = self.set.clone();
        self.root.mutate_values(move |root| set(root, value));
        self.refresh_node_state();
    }

    pub(crate) fn refresh_node_state(&self) {
        let current = self.value();
        let initial = (self.get)(&self.root.state.initial_values.read());
        let empty = (self.is_empty)(&current);
        self.root.with_registry(|registry| {
            let node = registry
                .nodes
                .entry(self.path.clone())
                .or_insert_with(|| FormNodeState::new(FormNodeKind::Scope, self.path.clone()));
            node.dirty = current != initial;
            node.empty = empty;
        });
        self.root.recompute_all_meta();
    }
}

pub fn provide_form_scope<T, Root>(scope: FormScope<T, Root>) -> FormScope<T, Root>
where
    Root: Clone + PartialEq + 'static,
    T: Clone + PartialEq + 'static,
{
    provide_context(scope)
}

pub fn use_form_scope<T>() -> FormScope<T>
where
    T: Clone + PartialEq + 'static,
{
    use_context()
}

pub fn use_form_scope_in<T, Root>() -> FormScope<T, Root>
where
    Root: Clone + PartialEq + 'static,
    T: Clone + PartialEq + 'static,
{
    use_context()
}
