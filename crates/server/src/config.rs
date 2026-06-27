use http::HeaderName;

pub const DEFAULT_INTERFACE: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 8888;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub interface: String,
    pub port: u16,
    pub rpc_path: String,
    pub ws_path: String,
    pub file_api_prefix: String,
    pub scope_header: HeaderName,
    pub file_entity_header: HeaderName,
    pub file_path_header: HeaderName,
    pub file_id_header: HeaderName,
}

impl ServerConfig {
    pub fn from_env() -> std::result::Result<Self, String> {
        let mut config = Self::default();
        if let Some(interface) = std::env::var_os("SEMANTIC_INTERFACE") {
            config.interface = interface.to_string_lossy().into_owned();
        }
        if let Some(port) = std::env::var_os("SEMANTIC_PORT") {
            config.port = port
                .to_string_lossy()
                .parse()
                .map_err(|err| format!("invalid SEMANTIC_PORT: {err}"))?;
        }
        Ok(config)
    }

    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.interface, self.port)
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            interface: DEFAULT_INTERFACE.to_string(),
            port: DEFAULT_PORT,
            rpc_path: "/rpc".to_string(),
            ws_path: "/rpc/ws".to_string(),
            file_api_prefix: "/api/v1/file".to_string(),
            scope_header: HeaderName::from_static("x-semantic-scope"),
            file_entity_header: HeaderName::from_static("x-semantic-file-entity"),
            file_path_header: HeaderName::from_static("x-semantic-file-path"),
            file_id_header: HeaderName::from_static("x-semantic-file-id"),
        }
    }
}
