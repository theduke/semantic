use http::HeaderName;

#[derive(Clone)]
pub struct ServerConfig {
    pub rpc_path: String,
    pub ws_path: String,
    pub scope_header: HeaderName,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            rpc_path: "/rpc".to_string(),
            ws_path: "/rpc/ws".to_string(),
            scope_header: HeaderName::from_static("x-semantic-scope"),
        }
    }
}
