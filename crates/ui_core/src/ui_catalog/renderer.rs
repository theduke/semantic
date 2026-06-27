use std::rc::Rc;

use dioxus::prelude::Element;
use semantic_data::schema::{AttributeType, ClassType, Type};
use semantic_data::value::{Object, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    Inline,
    Preview,
    Detail,
    Edit,
    Create,
}

#[derive(Clone)]
pub struct ValueRenderContext {
    pub value: Value,
    pub type_hint: Option<Type>,
    pub mode: RenderMode,
}

#[derive(Clone)]
pub struct AttributeRenderContext {
    pub object: Object,
    pub attribute: AttributeType,
    pub field_alias: Option<String>,
    pub value: Option<Value>,
    pub mode: RenderMode,
}

#[derive(Clone)]
pub struct ClassRenderContext {
    pub collection: Option<String>,
    pub id: Option<String>,
    pub class: ClassType,
    pub object: Object,
    pub mode: RenderMode,
}

pub type ValueRenderer = Rc<dyn Fn(ValueRenderContext) -> Element>;
pub type AttributeRenderer = Rc<dyn Fn(AttributeRenderContext) -> Element>;
pub type ClassRenderer = Rc<dyn Fn(ClassRenderContext) -> Element>;
