use brass::{
    dom::{builder::tag, ClickEvent, TagBuilder},
    signal::signal::{Mutable, Signal},
};
use factordb::Ident;
use wasm_bindgen::JsValue;

use crate::context::router;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Browse,
    Import,
    Upload,
    Logout,
    Entity(factordb::Ident),
    Create,
    EntityCreate { entity_type: String },
    Play,
    Tags,
}

impl Route {
    pub fn from_path(path: &str) -> Option<Self> {
        // Skip first slash.
        let path = if path.starts_with('/') {
            &path[1..]
        } else {
            path
        };
        let parts = path.split('/').collect::<Vec<_>>();

        match parts.as_slice() {
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
            _other => None,
        }
    }

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
        }
    }

    pub fn title(&self) -> &'static str {
        // TODO: this sucks.
        // Specific components will have to set the path.
        match self {
            Route::Logout => "Logout - Semantic",
            Route::Browse => "Browse - Semantic",
            Route::Import => "Import - Semantic",
            Route::Upload => "Upload - Semantic",
            Route::Tags => "Tags - Semantic",
            Route::Entity(_) => "Show - Semantic",
            Route::Create => "Create - Semantic",
            Route::EntityCreate { entity_type: _ } => "Create - Semantic",
            Route::Play => "Play - Semnatic",
        }
    }
}

#[derive(Clone)]
pub struct Router {
    route: Mutable<Route>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            route: Mutable::new(Route::Browse),
        }
    }

    pub fn goto(&self, route: Route) {
        let path = route.to_path();
        let title = route.title();

        self.route.set(route);
        if let Some(history) = web_sys::window().and_then(|w| w.history().ok()) {
            history
                .push_state_with_url(&JsValue::NULL, title, Some(&path))
                .ok();
        }
    }

    pub fn route(&self) -> &Mutable<Route> {
        &self.route
    }

    pub fn signal(&self) -> impl Signal<Item = Route> {
        self.route.signal_cloned()
    }
}

pub fn link(route: Route, label: &str) -> TagBuilder {
    tag(brass::dom::Tag::A)
        .and(label)
        .on(move |_: ClickEvent| router().goto(route.clone()))
}
