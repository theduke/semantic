use std::{cell::RefCell, collections::HashSet, hash::Hash, rc::Rc};

use brass::{
    dom::{DomEvent, TagBuilder},
    effect::{spawn_guarded, EffectGuard},
    signal::signal::{Mutable, Signal, SignalExt},
};
use factordb::AnyError;
use futures::{future::LocalBoxFuture, Future};

use crate::validate::Validator;

pub type FormLoadFuture = LocalBoxFuture<'static, Result<(), AnyError>>;

pub struct Form<V: 'static> {
    values: V,
    validator: Option<Box<dyn Validator<V>>>,
    on_valid: Option<Box<dyn Fn(&V)>>,
    on_submit: Option<Box<dyn Fn(&V)>>,
    on_submit_async: Option<Box<dyn Fn(&V) -> FormLoadFuture>>,
}

impl<V: 'static> Form<V> {
    pub fn new(values: V) -> Self {
        Self {
            values,
            validator: None,
            on_valid: None,
            on_submit: None,
            on_submit_async: None,
        }
    }

    pub fn on_submit(mut self, f: impl Fn(&V) + 'static) -> Self {
        self.on_submit_async = None;
        self.on_submit = Some(Box::new(f));
        self
    }

    pub fn on_submit_async(mut self, f: impl Fn(&V) -> FormLoadFuture + 'static) -> Self {
        self.on_submit = None;
        self.on_submit_async = Some(Box::new(f));
        self
    }

    pub fn render(self, f: impl FnOnce(FormHandle<V>) -> TagBuilder) -> TagBuilder {
        let h = FormHandle(Rc::new(RefCell::new(FormState {
            form: self,
            fields: Vec::new(),
            status: Mutable::new(FormStatus {
                is_valid: false,
                is_loading: false,
                errors: Ok(()),
                submit_error: None,
            }),
            load_guard: None,
        })));

        f(h)
    }
}

#[derive(Clone)]
pub struct FormStatus {
    pub is_valid: bool,
    pub is_loading: bool,
    pub errors: Result<(), Vec<String>>,

    pub submit_error: Option<String>,
}

struct FormState<V: 'static> {
    form: Form<V>,
    fields: Vec<Mutable<FieldStatus>>,
    status: Mutable<FormStatus>,
    load_guard: Option<EffectGuard>,
}

pub struct FormHandle<V: 'static>(Rc<RefCell<FormState<V>>>);

impl<V> Clone for FormHandle<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

// TODO: the error handling and validatoin logic needs work.
// Need to determine when to re-run global validation and how to update the
// status accordingly.
impl<V> FormHandle<V> {
    pub fn field_validated<F, VAL>(
        &self,
        get: fn(&mut V) -> &mut F,
        validator: VAL,
    ) -> FieldHandle<V, F>
    where
        VAL: Validator<F> + 'static,
    {
        self.make_field(get, Some(Rc::new(validator)))
    }

    pub fn field<F>(&self, get: fn(&mut V) -> &mut F) -> FieldHandle<V, F> {
        self.make_field(get, None)
    }

    fn make_field<F>(
        &self,
        get: fn(&mut V) -> &mut F,
        validator: Option<Rc<dyn Validator<F>>>,
    ) -> FieldHandle<V, F> {
        let mut state = self.0.borrow_mut();
        let index = state.fields.len();
        let field_state = FieldStatus {
            touched: false,
            changed: false,
            errors: Ok(()),
        };

        let mutable = Mutable::new(field_state.clone());
        state.fields.push(mutable.clone());

        FieldHandle {
            index,
            form: self.clone(),
            mutable,
            validator,
            get,
        }
    }

    pub fn signal_status(&self) -> impl Signal<Item = FormStatus> + 'static {
        self.0.borrow().status.signal_cloned()
    }

    pub fn signal_valid(&self) -> impl Signal<Item = bool> + 'static {
        self.0.borrow().status.signal_ref(|s| s.is_valid)
    }

    pub fn signal_loading(&self) -> impl Signal<Item = bool> + 'static {
        self.0.borrow().status.signal_ref(|s| s.is_loading)
    }

    pub fn signal_not_submittable(&self) -> impl Signal<Item = bool> + 'static {
        self.0
            .borrow()
            .status
            .signal_ref(|s| s.is_loading || !s.is_valid)
    }

    pub fn signal_errors(&self) -> impl Signal<Item = Option<Vec<String>>> + 'static {
        self.0.borrow().status.signal_ref(|s| {
            if let Err(err) = &s.errors {
                Some(err.clone())
            } else {
                None
            }
        })
    }

    fn modify_value<F: PartialEq>(
        &self,
        field_index: usize,
        getter: fn(&mut V) -> &mut F,
        modifier: impl FnOnce(&mut F) -> bool,
        validator: Option<&dyn Validator<F>>,
    ) {
        let mut state = self.0.borrow_mut();

        let (is_changed, errors) = {
            let value = getter(&mut state.form.values);
            let is_changed = modifier(value);

            let errors = validator
                .map(|val| val.validate(getter(&mut state.form.values)))
                .unwrap_or(Ok(()));
            (is_changed, errors)
        };

        if is_changed {
            Self::on_value_change(&mut *state, field_index, is_changed, errors);
        }
    }

    fn set_value_eq<F: PartialEq>(
        &self,
        field_index: usize,
        getter: fn(&mut V) -> &mut F,
        value: F,
        validator: Option<&dyn Validator<F>>,
    ) {
        let errors = validator.map(|val| val.validate(&value)).unwrap_or(Ok(()));

        let mut state = self.0.borrow_mut();
        let is_changed = { getter(&mut state.form.values) != &value };
        *getter(&mut state.form.values) = value;

        if is_changed {
            Self::on_value_change(&mut *state, field_index, is_changed, errors);
        }
    }

    fn on_value_change(
        state: &mut FormState<V>,
        field_index: usize,
        is_changed: bool,
        errors: Result<(), Vec<String>>,
    ) {
        let has_errors = errors.is_err();
        if let Some(field) = state.fields.get_mut(field_index) {
            field.replace_with(move |status| FieldStatus {
                touched: true,
                changed: status.changed || is_changed,
                errors,
            });
        } else {
            panic!("Invalid form field access");
        }

        // Update validations.
        let mut status = state.status.lock_mut();
        if has_errors {
            status.is_valid = false;
        } else if !status.is_valid {
            // Check that all fields are valid.
            let all_fields_valid = state.fields.iter().enumerate().all(|(index, field)| {
                if index == field_index {
                    !has_errors
                } else {
                    field.lock_ref().errors.is_ok()
                }
            });

            if !all_fields_valid {
                status.is_valid = false;
            } else {
                // Check global validators.

                if let Some(val) = &state.form.validator {
                    status.errors = val.validate(&state.form.values);
                }

                status.is_valid = status.errors.is_ok();
            }
        }

        if status.is_valid {
            if let Some(callback) = &state.form.on_valid {
                callback(&state.form.values);
            }
        }
    }

    pub fn submit(&self) {
        let mut state = self.0.borrow_mut();

        let mut status = state.status.lock_mut();
        if status.is_valid {
            if let Some(callback) = &state.form.on_submit {
                callback(&state.form.values)
            } else if let Some(callback) = &state.form.on_submit_async {
                if status.is_loading {
                    return;
                }
                status.is_loading = true;
                // NOTE: Need to manually drop for borrow checker.
                std::mem::drop(status);

                let handle = self.clone();

                let f = callback(&state.form.values);
                let f = async move {
                    match f.await {
                        Ok(_) => {
                            handle
                                .0
                                .borrow()
                                .status
                                .replace_with(move |old| FormStatus {
                                    is_valid: true,
                                    is_loading: false,
                                    errors: old.errors.clone(),
                                    submit_error: None,
                                });
                        }
                        Err(err) => {
                            handle
                                .0
                                .borrow()
                                .status
                                .replace_with(move |old| FormStatus {
                                    is_valid: false,
                                    is_loading: false,
                                    errors: old.errors.clone(),
                                    submit_error: Some(err.to_string()),
                                });
                        }
                    }
                };
                state.load_guard = Some(spawn_guarded(f));
            }
        }
    }

    pub fn reset(&self, values: V) {
        let mut state = self.0.borrow_mut();

        {
            let mut status = state.status.lock_mut();
            if status.is_loading {
                return;
            }

            *status = FormStatus {
                is_valid: false,
                is_loading: false,
                errors: Ok(()),
                submit_error: None,
            };
        }

        state.form.values = values;

        for field in &state.fields {
            field.replace(FieldStatus {
                touched: false,
                changed: false,
                errors: Ok(()),
            });
        }
    }

    pub fn reset_default(&self)
    where
        V: Default,
    {
        self.reset(V::default());
    }
}

#[derive(Clone, Debug)]
pub struct FieldStatus {
    pub touched: bool,
    pub changed: bool,
    pub errors: Result<(), Vec<String>>,
}

pub struct FieldHandle<V: 'static, F> {
    index: usize,
    form: FormHandle<V>,
    mutable: Mutable<FieldStatus>,
    validator: Option<Rc<dyn Validator<F>>>,
    get: fn(&mut V) -> &mut F,
}

impl<V: 'static, F> Clone for FieldHandle<V, F> {
    fn clone(&self) -> Self {
        Self {
            index: self.index,
            form: self.form.clone(),
            mutable: self.mutable.clone(),
            validator: self.validator.clone(),
            get: self.get.clone(),
        }
    }
}

impl<V: 'static, F: 'static> FieldHandle<V, F> {
    pub fn signal_value(&self) -> impl Signal<Item = F> + 'static
    where
        F: Clone,
    {
        let form = self.form.clone();
        let get = self.get;
        self.mutable.signal_ref(move |_| {
            let mut state = form.0.borrow_mut();
            get(&mut state.form.values).clone()
        })
    }

    pub fn get_value(&self) -> F
    where
        F: Clone,
    {
        (self.get)(&mut self.form.0.borrow_mut().form.values).clone()
    }

    pub fn for_each(&self, mut f: impl FnMut(&FieldStatus)) -> impl Future<Output = ()> {
        self.mutable
            .signal_ref(move |status| f(status))
            .for_each(|_| async {})
    }

    pub fn signal_touched(&self) -> impl Signal<Item = bool> + 'static {
        self.mutable.signal_ref(|x| x.touched)
    }

    pub fn signal_changed(&self) -> impl Signal<Item = bool> + 'static {
        self.mutable.signal_ref(|x| x.changed)
    }

    pub fn signal_is_valid(&self) -> impl Signal<Item = bool> + 'static {
        self.mutable.signal_ref(|x| x.errors.is_ok())
    }

    pub fn signal_errors(&self) -> impl Signal<Item = Option<Vec<String>>> + 'static {
        self.mutable.signal_ref(|x| match &x.errors {
            Ok(_) => None,
            Err(errs) => Some(errs.clone()),
        })
    }

    pub fn set(&self, value: F)
    where
        F: PartialEq,
    {
        self.form.set_value_eq(
            self.index,
            self.get,
            value,
            self.validator.as_ref().map(|x| &**x),
        );
    }

    pub fn on<E: DomEvent>(self, mut handler: impl FnMut(E) -> Option<F>) -> impl FnMut(E)
    where
        F: PartialEq,
    {
        move |e: E| {
            if let Some(value) = handler(e) {
                self.set(value);
            }
        }
    }
}

impl<V: 'static, F: Hash + Eq + 'static> FieldHandle<V, HashSet<F>> {
    pub fn add(&self, value: F) {
        self.form.modify_value(
            self.index,
            self.get,
            move |values| values.insert(value),
            self.validator.as_ref().map(|x| &**x),
        );
    }

    pub fn remove(&self, value: F) {
        self.form.modify_value(
            self.index,
            self.get.clone(),
            move |values| values.remove(&value),
            self.validator.as_ref().map(|x| &**x),
        );
    }
}
