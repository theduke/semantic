use std::{cell::RefCell, rc::Rc};

use brass::{
    dom::{builder::tag, Apply, ClickEvent, TagBuilder},
    signal::signal::{Mutable, Signal},
};
use factordb::Ident;
use futures::TryFutureExt;
use url::Url;
use wasm_bindgen::{JsCast, JsValue};

use crate::{context::router, SharedRenderer0};

pub trait PluginRouter {
    fn parse_path(&self, path: &[&str]) -> Option<PluginRoute>;
}

pub type DynPluginRouter = Box<dyn PluginRouter>;

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

    Apps,
    Browse,
    Import { url: Option<Url> },
    Upload,
    Logout,
    Entity(factordb::Ident),
    Create,
    EntityCreate { entity_type: String },
    Play,
    Tags,

    PluginManager,
    PluginCreate,
    PluginUpdate { id: String },
    PluginTest,

    Plugin(PluginRoute),
}

impl Route {
    pub fn to_path(&self) -> String {
        match self {
            Route::Logout => "/logout".into(),
            Route::Browse => "/browse".to_string(),
            Route::Import { url: target } => {
                if let Some(target) = target {
                    let query = form_urlencoded::Serializer::new(String::new())
                        .append_pair("url", &target.to_string())
                        .finish();
                    format!("/import?{}", query)
                } else {
                    "/import".to_string()
                }
            }
            Route::Upload => "/upload".to_string(),
            Route::Tags => "/tags".to_string(),
            Route::Play => "/play".to_string(),
            Route::Entity(ident) => format!("/entity/{}", ident.to_string()),
            Route::Create => "/create".to_string(),
            Route::EntityCreate { entity_type } => format!("/create/{}", entity_type),
            Route::PluginManager => "/plugins".to_string(),
            Route::PluginCreate => "/plugins/create".to_string(),
            Route::PluginUpdate { id } => format!("plugins/{}/edit", id),
            Route::PluginTest => "/plugins/test".to_string(),
            Route::Plugin(p) => p.path.clone(),
            Route::Settings => "/settings".to_string(),
            Route::Apps => "/apps".to_string(),
        }
    }

    pub fn title(&self) -> String {
        // TODO: this sucks.
        // Specific components will have to set the path.
        match self {
            Route::Logout => "Logout".to_string(),
            Route::Browse => "Browse".to_string(),
            Route::Import { .. } => "Import".to_string(),
            Route::Upload => "Upload".to_string(),
            Route::Tags => "Tags".to_string(),
            Route::Entity(_) => "Show".to_string(),
            Route::Create => "Create".to_string(),
            Route::EntityCreate { entity_type: _ } => "Create".to_string(),
            Route::Play => "Play - Semnatic".to_string(),
            Route::Plugin(p) => p.title.clone(),
            Route::PluginManager => "Manage Plugins".to_string(),
            Route::PluginCreate => "Create Plugin".to_string(),
            Route::PluginUpdate { .. } => "Update Plugin".to_string(),
            Route::PluginTest => "Test Plugin".to_string(),
            Route::Settings => "Settings".to_string(),
            Route::Apps => "Apps".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct Router {
    // NOTE: the Mutable contains an inner Rc<RefCell<_>> to allow updating the
    // route without triggering a re-render.
    route: Rc<RefCell<Route>>,
    signal: Mutable<usize>,
    routers: Rc<RefCell<Vec<Box<dyn PluginRouter>>>>,

    popstate_callback:
        Rc<RefCell<Option<wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>>>>,
}

impl Router {
    pub fn new() -> Self {
        let route = Rc::new(RefCell::new(Route::Browse));
        Self {
            route: route.clone(),
            signal: Mutable::new(0),
            routers: Rc::new(RefCell::new(Vec::new())),
            popstate_callback: Rc::new(RefCell::new(None)),
        }
    }

    pub fn register_router(&self, router: DynPluginRouter) {
        self.routers.borrow_mut().push(router);
    }

    /// Event handler for location changes triggered by the browser.
    pub fn on_location_changed(&self) {
        let route = brass::web::window()
            .location()
            .href()
            .ok()
            .and_then(|href| url::Url::parse(&href).ok())
            .and_then(|url| self.parse_url(url));
        if let Some(route) = route {
            self.goto(route)
        }
    }

    pub fn subscribe_to_history(&self) {
        let router = self.clone();

        let callback =
            wasm_bindgen::closure::Closure::wrap(Box::new(move |_event: web_sys::Event| {
                router.on_location_changed();
            }) as Box<dyn FnMut(web_sys::Event)>);
        brass::web::window()
            .add_event_listener_with_callback("popstate", callback.as_ref().unchecked_ref())
            .unwrap();
        *self.popstate_callback.borrow_mut() = Some(callback);
    }

    pub fn parse_url(&self, url: Url) -> Option<Route> {
        let parts = url.path().split('/').skip(1).collect::<Vec<_>>();

        tracing::trace!(?parts, "url parts");

        match parts.as_slice() {
            ["apps"] => Some(Route::Apps),
            ["settings"] => Some(Route::Settings),
            ["logout"] => Some(Route::Logout),
            ["browse"] => Some(Route::Browse),
            ["import"] => {
                let target = url
                    .query_pairs()
                    .find(|(name, _)| name == "url")
                    .and_then(|(_name, value)| Url::parse(&value).ok());
                tracing::trace!(?target, "url parse import");

                Some(Route::Import { url: target })
            }
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
            ["plugins", id, "edit"] => Some(Route::PluginUpdate { id: id.to_string() }),
            ["plugins", "test"] => Some(Route::PluginTest),
            _other => self
                .routers
                .borrow()
                .iter()
                .find_map(|router| router.parse_path(&parts))
                .map(Route::Plugin),
        }
    }

    pub fn goto(&self, route: Route) {
        {
            let current = self.route.borrow_mut();
            if &*current == &route {
                // Do nothing if route has not changed.
                return;
            }
        }

        self.set_route_without_navigation(route);
        self.signal.set(0);
    }

    /// Update the current route without triggering a re-render.
    pub fn set_route_without_navigation(&self, route: Route) {
        let win = brass::web::window();
        let loc = win.location();

        let cur_path = loc.pathname().unwrap();
        let cur_query = loc.search().unwrap();
        let cur_path = format!("{cur_path}{cur_query}");

        let new_path = route.to_path();
        if cur_path != new_path {
            brass::web::window()
                .history()
                .unwrap()
                .push_state_with_url(&JsValue::NULL, &route.title(), Some(&new_path))
                .unwrap();
        }
        *self.route.borrow_mut() = route;
    }

    pub fn signal(&self) -> impl Signal<Item = Route> {
        let route = self.route.clone();
        self.signal.signal_ref(move |_| {
            let rr: &Route = &*route.borrow();
            rr.clone()
        })
    }
}

pub fn link(route: Route, label: impl Apply) -> TagBuilder {
    tag(brass::dom::Tag::A)
        .and(label)
        .on(move |_: ClickEvent| router().goto(route.clone()))
}

pub fn link_with_class(route: Route, label: impl Apply, class: &str) -> TagBuilder {
    tag(brass::dom::Tag::A)
        .and(label)
        .classes_raw(class)
        .on(move |_: ClickEvent| router().goto(route.clone()))
}
