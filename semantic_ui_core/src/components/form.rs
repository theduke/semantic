use std::{cell::RefCell, collections::HashMap, rc::Rc};

use brass::{
    vdom::{EventCallback, Render},
    Callback, Component, PropComponent, VNode,
};

pub trait Validator<V> {
    fn validate(&self, value: &V) -> Result<(), Vec<String>>;
}

pub struct FieldState {
    pub name: String,
    pub touched: bool,
    pub errors: Result<(), Vec<String>>,
}

pub struct Field<F: 'static, T: 'static> {
    pub name: String,
    pub get: fn(&F) -> &T,
    pub set: fn(T, &mut F),
    pub render: Rc<dyn Fn(&T, &FieldState, Callback<T>) -> VNode>,
    pub validate: Option<Box<dyn Validator<T>>>,
}

// impl<'a, F, T: 'static> Render for Field<'a, F, T> {
//     fn render(self) -> VNode {
//         let values_borrow = self.form.state.values.borrow();
//         let value = (self.get)(&values_borrow);
//         let set = self.set;

//         let mut fields = self.form.state.fields.borrow_mut();

//         if !fields.contains_key(&self.name) {
//             let validator = self.validate;
//             let errors = validator
//                 .as_ref()
//                 .map(|val| val.validate(value))
//                 .unwrap_or(Ok(()));

//             fields.insert(
//                 self.name.clone(),
//                 FieldData {
//                     state: FieldState {
//                         name: self.name.clone(),
//                         touched: false,
//                         errors,
//                     },
//                     on_change: Rc::new(move |values, dyn_value| {
//                         let real_value: T = *dyn_value.downcast::<T>().unwrap();
//                         let res = validator
//                             .as_ref()
//                             .map(|v| v.validate(&real_value))
//                             .unwrap_or(Ok(()));
//                         (set)(real_value, values);
//                         res
//                     }),
//                 },
//             );
//         }
//         let name = self.name.clone();
//         let callback = self
//             .form
//             .state
//             .callback
//             .clone()
//             .map(move |value: T| Msg::Changed {
//                 name: name.clone(),
//                 value: Box::new(value),
//             });
//         let data = fields.get(&self.name).unwrap();
//         (self.render)(value, &data.state, callback)
//     }
// }

pub struct Form<V: 'static> {
    pub initial_values: V,
    pub render: Rc<dyn Fn(FormRef<V>) -> VNode + 'static>,
    pub on_submit: Callback<V>,
}

impl<V: Clone + 'static> Render for Form<V> {
    fn render(self) -> VNode {
        FormComponent::build(self)
    }
}

struct FieldData<V> {
    state: FieldState,
    on_change: Rc<dyn Fn(&mut V, AnyBox) -> Result<(), Vec<String>>>,
}

struct FormComponent<V> {
    state: RefCell<FormState<V>>,
}

struct FormState<V> {
    values: RefCell<V>,
    fields: RefCell<HashMap<String, FieldData<V>>>,
    callback: Callback<Msg>,
    is_valid: bool,
}

pub struct FormRef<'a, V> {
    state: &'a mut FormState<V>,
}

impl<'a, V: 'static> FormRef<'a, V> {
    pub fn field<T: 'static>(&mut self, field: impl Into<Field<V, T>>) -> VNode {
        let field = field.into();

        let values_borrow = self.state.values.borrow();
        let value = (field.get)(&values_borrow);
        let set = field.set;

        let mut fields = self.state.fields.borrow_mut();

        if !fields.contains_key(&field.name) {
            let validator = field.validate;
            let errors = validator
                .as_ref()
                .map(|val| val.validate(value))
                .unwrap_or(Ok(()));

            if errors.is_err() {
                self.state.is_valid = false;
            }

            fields.insert(
                field.name.clone(),
                FieldData {
                    state: FieldState {
                        name: field.name.clone(),
                        touched: false,
                        errors,
                    },
                    on_change: Rc::new(move |values, dyn_value| {
                        let real_value: T = *dyn_value.downcast::<T>().unwrap();
                        let res = validator
                            .as_ref()
                            .map(|v| v.validate(&real_value))
                            .unwrap_or(Ok(()));
                        (set)(real_value, values);
                        res
                    }),
                },
            );
        }
        let name = field.name.clone();
        let callback = self
            .state
            .callback
            .clone()
            .map(move |value: T| Msg::Changed {
                name: name.clone(),
                value: Box::new(value),
            });
        let data = fields.get(&field.name).unwrap();
        (field.render)(value, &data.state, callback)
    }
}

type AnyBox = Box<dyn std::any::Any>;

enum Msg {
    Changed { name: String, value: AnyBox },
}

impl<V: Clone + 'static> PropComponent for FormComponent<V> {
    type Properties = Form<V>;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            state: RefCell::new(FormState {
                values: RefCell::new(props.initial_values.clone()),
                fields: RefCell::new(HashMap::new()),
                callback: ctx.callback(),
                is_valid: true,
            }),
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Changed { name, value } => {
                let mut state = self.state.borrow_mut();
                let mut fields = state.fields.borrow_mut();

                let is_valid = if let Some(data) = fields.get_mut(&name) {
                    let mut values = state.values.borrow_mut();
                    data.state.errors = (data.on_change)(&mut *values, value);

                    data.state.errors.is_err()
                } else {
                    tracing::error!(?name, "Unknown field in form");
                    true
                };

                drop(fields);
                state.is_valid = is_valid;
            }
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> VNode {
        let mut state = self.state.borrow_mut();
        (props.render)(FormRef { state: &mut state })
    }
}

fn color_from_state(state: &FieldState) -> brass_bulma::Color {
    if state.errors.is_ok() {
        brass_bulma::Color::Default
    } else {
        brass_bulma::Color::Danger
    }
}

pub struct InputField<F> {
    pub name: String,
    pub get: fn(&F) -> &String,
    pub set: fn(String, &mut F),
    pub validate: Option<Box<dyn Validator<String>>>,

    pub label: String,
    pub help: Option<String>,
    pub placeholder: Option<String>,
}

impl<F> Into<Field<F, String>> for InputField<F> {
    fn into(self) -> Field<F, String> {
        let help = self.help;
        let placeholder = self.placeholder;

        Field {
            name: self.name,
            get: self.get,
            set: self.set,
            render: Rc::new(move |value, state, callback| {
                let color = color_from_state(state);

                brass_bulma::FieldHorizontal {
                    label: "Title".into(),
                    help: help
                        .clone()
                        .map(|message| brass_bulma::Help { message, color }),
                    control: brass_bulma::Input {
                        _type: "text".into(),
                        color,
                        placeholder: placeholder.clone(),
                        value: value.clone(),
                        on_input: EventCallback::callback(
                            |ev| brass::util::input_event_value(ev).unwrap_or_default(),
                            callback,
                        ),
                    },
                }
                .render()
            }),
            validate: self.validate,
        }
    }
}
