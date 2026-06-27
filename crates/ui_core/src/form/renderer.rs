use std::rc::Rc;

use dioxus::prelude::Element;
use dxform::{FieldHandle, FieldPath, FormRoot, FormScope};
use semantic_data::schema::{AttributeType, ClassAttribute, ClassType, Type};
use semantic_data::value::Value;

use crate::form::SemanticFormMode;

#[derive(Clone)]
pub struct ValueFormRenderContext {
    pub scope: FormScope<Value, Value>,
    pub value_type: Option<Type>,
    pub mode: SemanticFormMode,
    pub path: FieldPath,
}

#[derive(Clone)]
pub struct AttributeFormRenderContext {
    pub scope: FormScope<Value, Value>,
    pub field: FieldHandle<Value, Value>,
    pub class: ClassType,
    pub field_name: String,
    pub attribute: AttributeType,
    pub class_attribute: ClassAttribute,
    pub mode: SemanticFormMode,
}

#[derive(Clone)]
pub struct ClassFormRenderContext {
    pub form: FormRoot<Value>,
    pub scope: FormScope<Value, Value>,
    pub class: ClassType,
    pub collection: Option<String>,
    pub id: Option<String>,
    pub mode: SemanticFormMode,
}

pub type ValueFormRenderer = Rc<dyn Fn(ValueFormRenderContext) -> Element>;
pub type AttributeFormRenderer = Rc<dyn Fn(AttributeFormRenderContext) -> Element>;
pub type ClassFormRenderer = Rc<dyn Fn(ClassFormRenderContext) -> Element>;
