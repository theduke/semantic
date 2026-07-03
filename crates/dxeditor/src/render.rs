use std::{collections::BTreeMap, rc::Rc};

use dioxus::prelude::*;

use crate::{
    document::{BlockNode, InlineNode, Mark},
    state::EditorState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentRenderKind {
    Block,
    Inline,
    Mark,
}

#[derive(Clone)]
pub struct ComponentRendererContext {
    pub kind: ComponentRenderKind,
    pub block: Option<BlockNode>,
    pub inline: Option<InlineNode>,
    pub mark: Option<Mark>,
    pub state: EditorState,
}

#[derive(Clone)]
pub struct ChromeRendererContext {
    pub chrome: String,
    pub state: EditorState,
}

pub type ComponentRenderer = Rc<dyn Fn(ComponentRendererContext) -> Element>;
pub type ChromeRenderer = Rc<dyn Fn(ChromeRendererContext) -> Element>;

#[derive(Clone, Default)]
pub struct EditorRenderRegistry {
    component_renderers: BTreeMap<String, ComponentRenderer>,
    chrome_renderers: BTreeMap<String, ChromeRenderer>,
    fallback_component_renderer: Option<ComponentRenderer>,
}

impl EditorRenderRegistry {
    pub fn register_component_renderer(
        &mut self,
        component: impl Into<String>,
        renderer: ComponentRenderer,
    ) {
        self.component_renderers.insert(component.into(), renderer);
    }

    pub fn register_chrome_renderer(
        &mut self,
        chrome: impl Into<String>,
        renderer: ChromeRenderer,
    ) {
        self.chrome_renderers.insert(chrome.into(), renderer);
    }

    pub fn set_fallback_component_renderer(&mut self, renderer: ComponentRenderer) {
        self.fallback_component_renderer = Some(renderer);
    }

    pub fn component_renderer(&self, component: &str) -> Option<ComponentRenderer> {
        self.component_renderers
            .get(component)
            .cloned()
            .or_else(|| self.fallback_component_renderer.clone())
    }

    pub fn exact_component_renderer(&self, component: &str) -> Option<ComponentRenderer> {
        self.component_renderers.get(component).cloned()
    }

    pub fn chrome_renderer(&self, chrome: &str) -> Option<ChromeRenderer> {
        self.chrome_renderers.get(chrome).cloned()
    }
}
