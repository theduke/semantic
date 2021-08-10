// use std::rc::Rc;

// use brass::{vdom::Renderer, Callback, VNode};

// pub trait Validator {
//     type Value;

//     fn validate(&self, value: &Self::Value) -> Result<(), Vec<String>>;
// }

// pub type AnyBox = Box<dyn std::any::Any>;

// pub struct FieldState {
//     pub name: String,
//     pub touched: bool,
//     pub errors: Vec<String>,
//     pub value: AnyBox,
// }

// pub struct Field {
// }

// pub struct Form {

// }

// struct FormComponent {
//     fields: Vec<FieldState>,
// }
