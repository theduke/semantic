use brass::Callback;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Browse,
    Import,
    Upload,
    Logout,
    Entity(factordb::Ident),
    EntityCreateSelect,
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
            ["entity", id] => Some(Route::Entity(id.to_string().into())),
            ["create"] => Some(Route::EntityCreateSelect),
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
            Route::EntityCreateSelect => "/create".to_string(),
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
            Route::EntityCreateSelect => "Create - Semantic",
            Route::EntityCreate { entity_type: _ } => "Create - Semantic",
            Route::Play => "Play - Semnatic",
        }
    }
}

pub struct Router {
    callback: brass::Callback<Route>,
}

impl Router {
    pub fn new(callback: Callback<Route>) -> Self {
        Self { callback }
    }

    pub fn goto(&self, route: Route) {
        self.callback.send(route);
    }
}
