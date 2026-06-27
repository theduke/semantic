use std::rc::Rc;

use futures::future::LocalBoxFuture;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionPlacement {
    Global,
    Collection,
    Entity,
    Attribute,
    Media,
}

#[derive(Clone)]
pub struct UiAction {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub placement: ActionPlacement,
    pub enabled: bool,
    pub run: Rc<dyn Fn() -> LocalBoxFuture<'static, std::result::Result<(), String>>>,
}

#[derive(Clone, Default)]
pub struct MenuSection {
    pub id: String,
    pub label: String,
    pub actions: Vec<UiAction>,
}
