use std::rc::Rc;

use dioxus::prelude::*;
use futures::FutureExt;

use crate::{
    DynValidator, FieldPath, FormNodeKind, FormNodeState, FormRoot, FormScope, ListItemKey,
    ListMeta, ListValidationContext, ListValidator, ValidationPhase, ValidationStrategy,
};

pub struct ListSpec<Parent, Item> {
    pub name: String,
    pub get: Rc<dyn Fn(&Parent) -> Vec<Item>>,
    pub set: Rc<dyn Fn(&mut Parent, Vec<Item>)>,
    pub item_empty: Rc<dyn Fn(&Item) -> bool>,
    pub validators: Vec<ListValidator<Item>>,
    pub validation: ValidationStrategy,
}

impl<Parent, Item> ListSpec<Parent, Item>
where
    Item: Default + PartialEq + 'static,
{
    pub fn new(
        name: impl Into<String>,
        get: impl Fn(&Parent) -> Vec<Item> + 'static,
        set: impl Fn(&mut Parent, Vec<Item>) + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            get: Rc::new(get),
            set: Rc::new(set),
            item_empty: Rc::new(|item: &Item| item == &Item::default()),
            validators: Vec::new(),
            validation: ValidationStrategy::default(),
        }
    }
}

impl<Parent, Item> ListSpec<Parent, Item> {
    pub fn validator(mut self, validator: ListValidator<Item>) -> Self {
        self.validators.push(validator);
        self
    }

    pub fn item_empty(mut self, f: impl Fn(&Item) -> bool + 'static) -> Self {
        self.item_empty = Rc::new(f);
        self
    }

    pub fn validation(mut self, strategy: ValidationStrategy) -> Self {
        self.validation = strategy;
        self
    }
}

pub struct ListHandle<Item, Root = Item>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    pub(crate) root: FormRoot<Root>,
    pub(crate) path: FieldPath,
    get: Rc<dyn Fn(&Root) -> Vec<Item>>,
    set: Rc<dyn Fn(&mut Root, Vec<Item>)>,
    item_empty: Rc<dyn Fn(&Item) -> bool>,
}

impl<Item, Root> Clone for ListHandle<Item, Root>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            path: self.path.clone(),
            get: self.get.clone(),
            set: self.set.clone(),
            item_empty: self.item_empty.clone(),
        }
    }
}

impl<Item, Root> PartialEq for ListHandle<Item, Root>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.path == other.path
    }
}

impl<Item, Root> ListHandle<Item, Root>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    pub(crate) fn new<Parent>(scope: &FormScope<Parent, Root>, spec: ListSpec<Parent, Item>) -> Self
    where
        Parent: Clone + PartialEq + 'static,
    {
        let path = scope.path.child(&spec.name);
        let parent_get_for_child = scope.get.clone();
        let parent_get_for_set = scope.get.clone();
        let parent_set = scope.set.clone();
        let list_get = spec.get.clone();
        let list_set = spec.set.clone();
        let get: Rc<dyn Fn(&Root) -> Vec<Item>> = Rc::new(move |root| {
            let parent = parent_get_for_child(root);
            list_get(&parent)
        });
        let set: Rc<dyn Fn(&mut Root, Vec<Item>)> = Rc::new(move |root, value| {
            let mut parent = parent_get_for_set(root);
            list_set(&mut parent, value);
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
                    let items = get(&root);
                    async move {
                        validator
                            .validate(ListValidationContext { phase, path, items })
                            .await
                    }
                    .boxed_local()
                }) as DynValidator<Root>
            })
            .collect::<Vec<_>>();
        scope.root.with_registry(|registry| {
            let node = registry
                .nodes
                .entry(path.clone())
                .or_insert_with(|| FormNodeState::new(FormNodeKind::List, path.clone()));
            node.validators.extend(validators);
            registry.list_keys.entry(path.clone()).or_insert_with(|| {
                let len = get(&scope.root.values()).len();
                (0..len).map(|_| scope.root.allocate_key()).collect()
            });
        });
        let handle = Self {
            root: scope.root.clone(),
            path,
            get,
            set,
            item_empty: spec.item_empty,
        };
        handle.refresh_node_state();
        handle
    }

    pub fn path(&self) -> FieldPath {
        self.path.clone()
    }

    pub fn meta(&self) -> ListMeta {
        self.root
            .state
            .registry
            .read()
            .nodes
            .get(&self.path)
            .map(FormNodeState::list_meta)
            .unwrap_or_else(|| ListMeta::new(self.path.clone()))
    }

    pub fn meta_signal(&self) -> Signal<ListMeta> {
        Signal::new(self.meta())
    }

    pub fn values(&self) -> Vec<Item> {
        (self.get)(&self.root.values())
    }

    pub fn items(&self) -> Vec<ListItemHandle<Item, Root>> {
        self.ensure_key_count();
        let keys = self
            .root
            .state
            .registry
            .read()
            .list_keys
            .get(&self.path)
            .cloned()
            .unwrap_or_default();
        keys.into_iter()
            .enumerate()
            .map(|(index, key)| ListItemHandle {
                list: self.clone(),
                index,
                key,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.values().len()
    }

    pub fn is_empty(&self) -> bool {
        self.values().is_empty()
    }

    pub fn push(&self, value: Item) {
        self.insert(self.len(), value);
    }

    pub fn insert(&self, index: usize, value: Item) {
        let mut values = self.values();
        let index = index.min(values.len());
        values.insert(index, value);
        let set = self.set.clone();
        self.root.mutate_values(move |root| set(root, values));
        let key = self.root.allocate_key();
        self.root.with_registry(|registry| {
            registry
                .list_keys
                .entry(self.path.clone())
                .or_default()
                .insert(index, key);
        });
        self.refresh_node_state();
    }

    pub fn remove(&self, index: usize) -> Option<Item> {
        let mut values = self.values();
        if index >= values.len() {
            return None;
        }
        let removed = values.remove(index);
        let set = self.set.clone();
        self.root.mutate_values(move |root| set(root, values));
        let item_path = self.path.index(index);
        self.root.with_registry(|registry| {
            if let Some(keys) = registry.list_keys.get_mut(&self.path) {
                if index < keys.len() {
                    keys.remove(index);
                }
            }
            registry.remove_descendants(&item_path);
        });
        self.rebuild_item_nodes();
        self.refresh_node_state();
        Some(removed)
    }

    pub fn clear(&self) {
        let set = self.set.clone();
        self.root.mutate_values(move |root| set(root, Vec::new()));
        self.root.with_registry(|registry| {
            registry.list_keys.insert(self.path.clone(), Vec::new());
            let path = self.path.clone();
            registry
                .nodes
                .retain(|node_path, _| !node_path.starts_with(&path) || node_path == &path);
        });
        self.refresh_node_state();
    }

    pub fn swap(&self, a: usize, b: usize) {
        let mut values = self.values();
        if a >= values.len() || b >= values.len() {
            return;
        }
        values.swap(a, b);
        let set = self.set.clone();
        self.root.mutate_values(move |root| set(root, values));
        self.root.with_registry(|registry| {
            if let Some(keys) = registry.list_keys.get_mut(&self.path) {
                if a < keys.len() && b < keys.len() {
                    keys.swap(a, b);
                }
            }
        });
        self.rebuild_item_nodes();
        self.refresh_node_state();
    }

    pub fn move_item(&self, from: usize, to: usize) {
        let mut values = self.values();
        if from >= values.len() || to >= values.len() || from == to {
            return;
        }
        let value = values.remove(from);
        values.insert(to, value);
        let set = self.set.clone();
        self.root.mutate_values(move |root| set(root, values));
        self.root.with_registry(|registry| {
            if let Some(keys) = registry.list_keys.get_mut(&self.path) {
                if from < keys.len() && to < keys.len() {
                    let key = keys.remove(from);
                    keys.insert(to, key);
                }
            }
        });
        self.rebuild_item_nodes();
        self.refresh_node_state();
    }

    pub fn reset(&self) {
        let initial = (self.get)(&self.root.state.initial_values.read());
        let set = self.set.clone();
        self.root.mutate_values(move |root| set(root, initial));
        self.root.with_registry(|registry| {
            registry.list_keys.remove(&self.path);
        });
        self.ensure_key_count();
        self.refresh_node_state();
    }

    pub async fn validate(&self) -> std::result::Result<(), Vec<crate::FormError>> {
        self.root
            .validate_path(self.path.clone(), ValidationPhase::Manual)
            .await
    }

    pub fn add_handler(&self, new_item: Rc<dyn Fn() -> Item>) -> EventHandler<MouseEvent> {
        let list = self.clone();
        EventHandler::new(move |_| list.push(new_item()))
    }

    pub fn clear_handler(&self) -> EventHandler<MouseEvent> {
        let list = self.clone();
        EventHandler::new(move |_| list.clear())
    }

    fn ensure_key_count(&self) {
        let len = self.values().len();
        self.root.with_registry(|registry| {
            let current_len = registry.list_keys.get(&self.path).map_or(0, Vec::len);
            let missing = len.saturating_sub(current_len);
            let new_keys = (0..missing)
                .map(|_| self.root.allocate_key())
                .collect::<Vec<_>>();
            let keys = registry.list_keys.entry(self.path.clone()).or_default();
            keys.truncate(len);
            keys.extend(new_keys);
        });
        self.rebuild_item_nodes();
    }

    fn rebuild_item_nodes(&self) {
        let keys = self
            .root
            .state
            .registry
            .read()
            .list_keys
            .get(&self.path)
            .cloned()
            .unwrap_or_default();
        self.root.with_registry(|registry| {
            for (index, _) in keys.iter().enumerate() {
                let path = self.path.index(index);
                registry.ensure_node(FormNodeKind::ListItem, path);
            }
        });
    }

    fn refresh_node_state(&self) {
        let current = self.values();
        let initial = (self.get)(&self.root.state.initial_values.read());
        self.root.with_registry(|registry| {
            let node = registry
                .nodes
                .entry(self.path.clone())
                .or_insert_with(|| FormNodeState::new(FormNodeKind::List, self.path.clone()));
            node.dirty = current != initial;
            node.empty = current.is_empty() || current.iter().all(|item| (self.item_empty)(item));
        });
        self.root.recompute_all_meta();
    }
}

pub struct ListItemHandle<Item, Root = Item>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    pub(crate) list: ListHandle<Item, Root>,
    index: usize,
    key: ListItemKey,
}

impl<Item, Root> Clone for ListItemHandle<Item, Root>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    fn clone(&self) -> Self {
        Self {
            list: self.list.clone(),
            index: self.index,
            key: self.key,
        }
    }
}

impl<Item, Root> PartialEq for ListItemHandle<Item, Root>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list && self.key == other.key
    }
}

impl<Item, Root> ListItemHandle<Item, Root>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    pub fn key(&self) -> u64 {
        self.key
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn path(&self) -> FieldPath {
        self.list.path.index(self.index)
    }

    pub fn scope(&self) -> FormScope<Item, Root> {
        let list_get_for_item = self.list.get.clone();
        let list_get_for_set = self.list.get.clone();
        let list_set = self.list.set.clone();
        let index = self.index;
        let get = Rc::new(move |root: &Root| list_get_for_item(root)[index].clone());
        let set = Rc::new(move |root: &mut Root, item: Item| {
            let mut items = list_get_for_set(root);
            if index < items.len() {
                items[index] = item;
                list_set(root, items);
            }
        });
        FormScope::new(
            self.list.root.clone(),
            self.path(),
            get,
            set,
            self.list.item_empty.clone(),
        )
    }

    pub fn remove(&self) {
        self.list.remove(self.index);
    }

    pub fn remove_handler(&self) -> EventHandler<MouseEvent> {
        let item = self.clone();
        EventHandler::new(move |_| item.remove())
    }
}

pub fn provide_list_item<Item, Root>(item: ListItemHandle<Item, Root>) -> ListItemHandle<Item, Root>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    provide_context(item)
}

pub fn use_list_item<Item, Root>() -> ListItemHandle<Item, Root>
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    use_context()
}

#[allow(non_snake_case)]
pub fn ListAddButton<Item, Root>(
    list: ListHandle<Item, Root>,
    new_item: Rc<dyn Fn() -> Item>,
    disabled: Option<bool>,
    children: Element,
) -> Element
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    rsx! {
        button {
            r#type: "button",
            disabled: disabled.unwrap_or(false),
            onclick: list.add_handler(new_item),
            {children}
        }
    }
}

#[allow(non_snake_case)]
pub fn ListClearButton<Item, Root>(
    list: ListHandle<Item, Root>,
    disabled: Option<bool>,
    children: Element,
) -> Element
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    rsx! {
        button {
            r#type: "button",
            disabled: disabled.unwrap_or(false),
            onclick: list.clear_handler(),
            {children}
        }
    }
}

#[allow(non_snake_case)]
pub fn ListItemRemoveButton<Item, Root>(
    item: ListItemHandle<Item, Root>,
    disabled: Option<bool>,
    children: Element,
) -> Element
where
    Root: Clone + PartialEq + 'static,
    Item: Clone + PartialEq + 'static,
{
    rsx! {
        button {
            r#type: "button",
            disabled: disabled.unwrap_or(false),
            onclick: item.remove_handler(),
            {children}
        }
    }
}
