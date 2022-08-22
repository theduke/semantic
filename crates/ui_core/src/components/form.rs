use std::{cell::RefCell, collections::HashSet, hash::Hash, rc::Rc};

use brass::{
    dom::{DomEvent, TagBuilder},
    effect::{spawn_guarded, EffectGuard},
    signal::signal::{Mutable, Signal, SignalExt},
};

use futures::{future::LocalBoxFuture, Future};

use crate::validate::{PassingValidator, Validator};

pub type FormLoadFuture = LocalBoxFuture<'static, Result<(), anyhow::Error>>;

pub struct Form<V: 'static> {
    values: V,
    validator: Option<Box<dyn Validator<V>>>,
    on_valid: Option<Box<dyn Fn(&V)>>,
    on_submit: Option<Rc<dyn Fn(&V)>>,
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
        self.on_submit = Some(Rc::new(f));
        self
    }

    pub fn on_submit_async(mut self, f: impl Fn(&V) -> FormLoadFuture + 'static) -> Self {
        self.on_submit = None;
        self.on_submit_async = Some(Box::new(f));
        self
    }

    pub fn build(self) -> FormHandle<V> {
        FormHandle(Rc::new(RefCell::new(FormState {
            form: self,
            fields: Vec::new(),
            status: Mutable::new(FormStatus {
                is_valid: false,
                is_loading: false,
                errors: Ok(()),
                submit_error: None,
            }),
            load_guard: None,
        })))
    }

    pub fn render(self, f: impl FnOnce(FormHandle<V>) -> TagBuilder) -> TagBuilder {
        f(self.build())
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
    fields: Vec<Rc<FieldData<V>>>,
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
impl<V: Clone> FormHandle<V> {
    pub fn field_validated<F: 'static, VAL: Validator<F> + 'static>(
        &self,
        get: fn(&mut V) -> &mut F,
        validator: VAL,
    ) -> FieldHandle<V, F>
    where
        VAL: Validator<F> + 'static,
    {
        self.make_field(get, validator)
    }

    pub fn field<F: 'static>(&self, get: fn(&mut V) -> &mut F) -> FieldHandle<V, F> {
        self.make_field(get, PassingValidator::<F>::new())
    }

    fn make_field<F>(
        &self,
        get: fn(&mut V) -> &mut F,
        validator: impl Validator<F> + 'static,
    ) -> FieldHandle<V, F>
    where
        F: 'static,
        V: 'static,
    {
        let mut state = self.0.borrow_mut();
        let index = state.fields.len();
        let field_status = Mutable::new(FieldStatus {
            touched: false,
            changed: false,
            errors: Ok(()),
        });

        let get2 = get.clone();
        let data = Rc::new(FieldData {
            index,
            status: field_status.clone(),
            validate: Box::new(move |values| validator.validate(get2(values))),
        });

        state.fields.push(data.clone());

        FieldHandle {
            form: self.clone(),
            data: data.clone(),
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
        self.0.borrow().status.signal_ref(|s| s.is_loading)
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
    ) {
        let mut state = self.0.borrow_mut();

        let is_changed = {
            let value = getter(&mut state.form.values);
            modifier(value)
        };

        if is_changed {
            Self::on_value_change(&mut *state, field_index, is_changed);
        }
    }

    fn set_value_eq<F: PartialEq>(
        &self,
        field_index: usize,
        getter: fn(&mut V) -> &mut F,
        value: F,
    ) {
        let mut state = self.0.borrow_mut();
        let is_changed = { getter(&mut state.form.values) != &value };
        *getter(&mut state.form.values) = value;

        if is_changed {
            Self::on_value_change(&mut *state, field_index, is_changed);
        }
    }

    fn validate(state: &mut FormState<V>, touch_all: bool) -> bool {
        // Update validations.
        let mut status = state.status.lock_mut();

        let mut all_fields_valid = true;
        for field in &state.fields {
            let mut field_status = field.status.lock_mut();

            if !field_status.touched {
                let res = (field.validate)(&mut state.form.values);
                if res.is_err() {
                    all_fields_valid = false;
                }
                field_status.errors = res;
                if touch_all {
                    field_status.touched = true;
                }
            } else {
                if !field_status.errors.is_ok() {
                    all_fields_valid = false;
                }
            }
        }

        if all_fields_valid {
            if let Some(val) = &state.form.validator {
                status.errors = val.validate(&state.form.values);
            }
        }

        status.is_valid = all_fields_valid && status.errors.is_ok();
        status.is_valid
    }

    fn on_value_change(state: &mut FormState<V>, field_index: usize, is_changed: bool) {
        if let Some(field) = state.fields.get(field_index) {
            let errors = (field.validate)(&mut state.form.values);

            field.status.replace_with(move |status| FieldStatus {
                touched: true,
                changed: status.changed || is_changed,
                errors,
            });
        } else {
            panic!("Invalid form field access");
        };

        if Self::validate(state, false) {
            if let Some(callback) = &state.form.on_valid {
                callback(&state.form.values);
            }
        }
    }

    pub fn set_loading(&self) {
        self.0.borrow_mut().status.replace_with(|old| FormStatus {
            is_valid: old.is_valid,
            is_loading: true,
            errors: old.errors.clone(),
            submit_error: None,
        });
    }

    pub fn set_loaded(&self) {
        self.0.borrow_mut().status.replace_with(|old| FormStatus {
            is_valid: old.is_valid,
            is_loading: false,
            errors: old.errors.clone(),
            submit_error: None,
        });
    }

    pub fn submit(&self) {
        let mut state = self.0.borrow_mut();
        Self::validate(&mut state, true);

        let mut status = state.status.lock_mut();
        if status.is_loading {
            return;
        }

        if status.is_valid {
            if let Some(callback) = &state.form.on_submit {
                let values = state.form.values.clone();
                let cb = callback.clone();
                std::mem::drop(status);
                std::mem::drop(state);
                cb(&values)
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
            field.status.replace(FieldStatus {
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

struct FieldData<V> {
    index: usize,
    status: Mutable<FieldStatus>,
    validate: Box<dyn Fn(&mut V) -> Result<(), Vec<String>>>,
}

pub struct FieldHandle<V: Clone + 'static, F> {
    form: FormHandle<V>,
    data: Rc<FieldData<V>>,
    get: fn(&mut V) -> &mut F,
}

impl<V: Clone + 'static, F> Clone for FieldHandle<V, F> {
    fn clone(&self) -> Self {
        Self {
            form: self.form.clone(),
            data: self.data.clone(),
            get: self.get.clone(),
        }
    }
}

impl<V: Clone + 'static, F: 'static> FieldHandle<V, F> {
    pub fn signal_value(&self) -> impl Signal<Item = F> + 'static
    where
        F: Clone,
    {
        let form = self.form.clone();
        let get = self.get;
        self.data.status.signal_ref(move |_| {
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
        self.data
            .status
            .signal_ref(move |status| f(status))
            .for_each(|_| async {})
    }

    pub fn signal_touched(&self) -> impl Signal<Item = bool> + 'static {
        self.data.status.signal_ref(|x| x.touched)
    }

    pub fn signal_changed(&self) -> impl Signal<Item = bool> + 'static {
        self.data.status.signal_ref(|x| x.changed)
    }

    pub fn signal_is_valid(&self) -> impl Signal<Item = bool> + 'static {
        self.data.status.signal_ref(|x| x.errors.is_ok())
    }

    pub fn signal_errors(&self) -> impl Signal<Item = Option<Vec<String>>> + 'static {
        self.data.status.signal_ref(|x| match &x.errors {
            Ok(_) => None,
            Err(errs) => Some(errs.clone()),
        })
    }

    pub fn set(&self, value: F)
    where
        F: PartialEq,
    {
        self.form.set_value_eq(self.data.index, self.get, value);
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

impl<V: Clone + 'static, F: Hash + Eq + 'static> FieldHandle<V, HashSet<F>> {
    pub fn add(&self, value: F) {
        self.form
            .modify_value(self.data.index, self.get, move |values| {
                values.insert(value)
            });
    }

    pub fn remove(&self, value: F) {
        self.form
            .modify_value(self.data.index, self.get.clone(), move |values| {
                values.remove(&value)
            });
    }
}
