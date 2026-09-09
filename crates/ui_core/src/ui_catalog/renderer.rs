use std::rc::Rc;

use dioxus::prelude::Element;
use semantic_data::schema::{ClassType, Type};
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

#[derive(Clone, PartialEq)]
pub struct RenderSettings {
    pub show_media: bool,
    pub enable_label_editor: bool,
    pub file_api_prefix: String,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            show_media: true,
            enable_label_editor: true,
            file_api_prefix: "/api/v1/file".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct RenderCtx {
    pub settings: RenderSettings,
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
pub type AttributeRenderer = Rc<dyn Fn(RenderCtx, &Value, Option<&Object>) -> Element>;
pub type ClassRenderer = Rc<dyn Fn(ClassRenderContext) -> Element>;
