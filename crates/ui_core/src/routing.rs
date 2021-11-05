use std::{cell::RefCell, rc::Rc};

use brass::{
    dom::{builder::tag, Apply, ClickEvent, TagBuilder},
    signal::signal::{Mutable, Signal},
};
use factordb::Ident;
use wasm_bindgen::JsValue;

use crate::{context::router, SharedRenderer0};

pub trait PluginRouter {
    fn parse_path(&self, path: &[&str]) -> Option<PluginRoute>;
}

#[derive(Clone)]
pub struct PluginRoute {
    pub path: String,
    pub title: String,
    pub render: SharedRenderer0,
}

impl PartialEq for PluginRoute {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.title == other.title
    }
}

impl Eq for PluginRoute {}

impl std::fmt::Debug for PluginRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRoute")
            .field("path", &self.path)
            .field("title", &self.title)
            .field("render", &())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Settings,

    Browse,
    Import,
    Upload,
    Logout,
    Entity(factordb::Ident),
    Create,
    EntityCreate { entity_type: String },
    Play,
    Tags,

    PluginManager,
    PluginCreate,
    PluginTest,

    Plugin(PluginRoute),
}

impl Route {
    pub fn to_path(&self) -> String {
        match self {
            Route::Logout => "/logout".into(),
            Route::Browse => "/browse".to_string(),
            Route::Import => "/import".to_string(),
            Route::Upload => "/upload".to_string(),
            Route::Tags => "/tags".to_string(),
            Route::Play => "/play".to_string(),
            Route::Entity(ident) => format!("/entity/{}", ident.to_string()),
            Route::Create => "/create".to_string(),
            Route::EntityCreate { entity_type } => format!("/create/{}", entity_type),
            Route::PluginManager => "/plugins".to_string(),
            Route::PluginCreate => "/plugins/create".to_string(),
            Route::PluginTest => "/plugins/test".to_string(),
            Route::Plugin(p) => p.path.clone(),
            Route::Settings => "/manage".to_string(),
        }
    }

    pub fn title(&self) -> String {
        // TODO: this sucks.
        // Specific components will have to set the path.
        match self {
            Route::Logout => "Logout".to_string(),
            Route::Browse => "Browse".to_string(),
            Route::Import => "Import".to_string(),
            Route::Upload => "Upload".to_string(),
            Route::Tags => "Tags".to_string(),
            Route::Entity(_) => "Show".to_string(),
            Route::Create => "Create".to_string(),
            Route::EntityCreate { entity_type: _ } => "Create".to_string(),
            Route::Play => "Play - Semnatic".to_string(),
            Route::Plugin(p) => p.title.clone(),
            Route::PluginManager => "Manage Plugins".to_string(),
            Route::PluginCreate => "Create Plugin".to_string(),
            Route::PluginTest => "Test Plugin".to_string(),
            Route::Settings => "Settings".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct Router {
    // NOTE: the Mutable contains an inner Rc<RefCell<_>> to allow updating the
    // route without triggering a re-render.
    route: Rc<RefCell<Route>>,
    signal: Mutable<Rc<RefCell<Route>>>,
    routers: Rc<RefCell<Vec<Box<dyn PluginRouter>>>>,
}

impl Router {
    pub fn new() -> Self {
        let route = Rc::new(RefCell::new(Route::Browse));
        Self {
            route: route.clone(),
            signal: Mutable::new(route),
            routers: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn parse_path(&self, path: &str) -> Option<Route> {
        // Skip first slash.
        let path = if path.starts_with('/') {
            &path[1..]
        } else {
            path
        };
        let parts = path.split('/').collect::<Vec<_>>();

        match parts.as_slice() {
            ["settings"] => Some(Route::Settings),
            ["logout"] => Some(Route::Logout),
            ["browse"] => Some(Route::Browse),
            ["import"] => Some(Route::Import),
            ["upload"] => Some(Route::Upload),
            ["play"] => Some(Route::Play),
            ["entity", id] => Some(Route::Entity(Ident::from_str(id))),
            ["create"] => Some(Route::Create),
            ["create", tail @ ..] => Some(Route::EntityCreate {
                entity_type: tail.join("/"),
            }),
            ["tags"] => Some(Route::Tags),
            ["plugins"] => Some(Route::PluginManager),
            ["plugins", "create"] => Some(Route::PluginCreate),
            ["plugins", "test"] => Some(Route::PluginTest),
            _other => {
                let route = self
                    .routers
                    .borrow()
                    .iter()
                    .find_map(|router| router.parse_path(&parts))?;
                Some(Route::Plugin(route))
            }
        }
    }

    pub fn goto(&self, route: Route) {
        let path = route.to_path();
        let title = route.title();
        {
            let mut current = self.route.borrow_mut();
            if &*current == &route {
                // Do nothing if route has not changed.
                return;
            }
            *current = route;
        }

        self.signal.set(self.route.clone());
        if let Some(history) = web_sys::window().and_then(|w| w.history().ok()) {
            history
                .push_state_with_url(&JsValue::NULL, &title, Some(&path))
                .ok();
        }
    }

    /// Update the current route without triggering a re-render.
    pub fn set_route_without_navigation(&self, route: Route) {
        *self.route.borrow_mut() = route;
    }

    pub fn signal(&self) -> impl Signal<Item = Route> {
        self.signal.signal_ref(|r| {
            let rr: &Route = &*r.borrow();
            rr.clone()
        })
    }
}

pub fn link(route: Route, label: impl Apply) -> TagBuilder {
    tag(brass::dom::Tag::A)
        .and(label)
        .on(move |_: ClickEvent| router().goto(route.clone()))
}
