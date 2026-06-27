use std::{collections::BTreeMap, rc::Rc};

use futures::future::LocalBoxFuture;

use crate::{FieldMeta, FieldPath, FormError, ListMeta, ScopeMeta, ValidationPhase, Validity};

pub type ListItemKey = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormNodeKind {
    Field,
    Scope,
    List,
    ListItem,
}

pub(crate) type DynValidator<Root> =
    Rc<dyn Fn(ValidationPhase, Root) -> LocalBoxFuture<'static, Vec<FormError>>>;

pub(crate) struct FormNodeState<Root> {
    pub kind: FormNodeKind,
    pub path: FieldPath,
    pub touched: bool,
    pub focused: bool,
    pub dirty: bool,
    pub empty: bool,
    pub validating: bool,
    pub validity: Validity,
    pub errors: Vec<FormError>,
    pub submit_errors: Vec<FormError>,
    pub validation_epoch: u64,
    pub validators: Vec<DynValidator<Root>>,
}

impl<Root> Clone for FormNodeState<Root> {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind,
            path: self.path.clone(),
            touched: self.touched,
            focused: self.focused,
            dirty: self.dirty,
            empty: self.empty,
            validating: self.validating,
            validity: self.validity,
            errors: self.errors.clone(),
            submit_errors: self.submit_errors.clone(),
            validation_epoch: self.validation_epoch,
            validators: self.validators.clone(),
        }
    }
}

impl<Root> FormNodeState<Root> {
    pub fn new(kind: FormNodeKind, path: FieldPath) -> Self {
        Self {
            kind,
            path,
            touched: false,
            focused: false,
            dirty: false,
            empty: true,
            validating: false,
            validity: Validity::NotValidated,
            errors: Vec::new(),
            submit_errors: Vec::new(),
            validation_epoch: 0,
            validators: Vec::new(),
        }
    }

    pub fn field_meta(&self) -> FieldMeta {
        FieldMeta {
            path: self.path.clone(),
            touched: self.touched,
            focused: self.focused,
            dirty: self.dirty,
            empty: self.empty,
            validating: self.validating,
            validity: self.validity,
            errors: self.errors.clone(),
            submit_errors: self.submit_errors.clone(),
        }
    }

    pub fn scope_meta(&self) -> ScopeMeta {
        ScopeMeta {
            path: self.path.clone(),
            touched: self.touched,
            dirty: self.dirty,
            empty: self.empty,
            validating: self.validating,
            validity: self.validity,
            errors: self.errors.clone(),
            submit_errors: self.submit_errors.clone(),
        }
    }

    pub fn list_meta(&self) -> ListMeta {
        ListMeta {
            path: self.path.clone(),
            touched: self.touched,
            dirty: self.dirty,
            empty: self.empty,
            validating: self.validating,
            validity: self.validity,
            errors: self.errors.clone(),
            submit_errors: self.submit_errors.clone(),
        }
    }
}

pub(crate) struct FormRegistry<Root> {
    pub nodes: BTreeMap<FieldPath, FormNodeState<Root>>,
    pub list_keys: BTreeMap<FieldPath, Vec<ListItemKey>>,
}

impl<Root> Default for FormRegistry<Root> {
    fn default() -> Self {
        Self {
            nodes: BTreeMap::new(),
            list_keys: BTreeMap::new(),
        }
    }
}

impl<Root> Clone for FormRegistry<Root> {
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            list_keys: self.list_keys.clone(),
        }
    }
}

impl<Root> FormRegistry<Root> {
    pub fn ensure_node(&mut self, kind: FormNodeKind, path: FieldPath) {
        self.nodes
            .entry(path.clone())
            .or_insert_with(|| FormNodeState::new(kind, path));
    }

    pub fn remove_descendants(&mut self, path: &FieldPath) {
        self.nodes
            .retain(|node_path, _| !node_path.starts_with(path));
        self.list_keys
            .retain(|list_path, _| !list_path.starts_with(path));
    }

    pub fn clear_errors_under(&mut self, path: &FieldPath, submit: bool) {
        for node in self
            .nodes
            .values_mut()
            .filter(|node| node.path.starts_with(path))
        {
            if submit {
                node.submit_errors.clear();
            } else {
                node.errors.clear();
            }
            if node.errors.is_empty() {
                node.validity = Validity::NotValidated;
            }
        }
    }

    pub fn touch_under(&mut self, path: &FieldPath) {
        for node in self
            .nodes
            .values_mut()
            .filter(|node| node.path.starts_with(path))
        {
            node.touched = true;
        }
    }
}
