use http::HeaderName;

pub const DEFAULT_INTERFACE: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 8888;
pub const DEFAULT_MAX_FILE_UPLOAD_SIZE: u64 = 100 * 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub interface: String,
    pub port: u16,
    pub rpc_path: String,
    pub ws_path: String,
    pub file_api_prefix: String,
    pub scope_header: HeaderName,
    pub file_entity_header: HeaderName,
    pub file_filename_header: HeaderName,
    pub file_id_header: HeaderName,
    pub max_file_upload_size: u64,
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
        if let Some(size) = std::env::var_os("SEMANTIC_MAX_FILE_UPLOAD_SIZE") {
            config.max_file_upload_size = size
                .to_string_lossy()
                .parse()
                .map_err(|err| format!("invalid SEMANTIC_MAX_FILE_UPLOAD_SIZE: {err}"))?;
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
            rpc_path: "/api/v1/rpc".to_string(),
            ws_path: "/api/v1/rpc/ws".to_string(),
            file_api_prefix: "/api/v1/file".to_string(),
            scope_header: HeaderName::from_static("x-semantic-scope"),
            file_entity_header: HeaderName::from_static("x-semantic-file-entity"),
            file_filename_header: HeaderName::from_static("x-semantic-filename"),
            file_id_header: HeaderName::from_static("x-semantic-file-id"),
            max_file_upload_size: DEFAULT_MAX_FILE_UPLOAD_SIZE,
        }
    }
}
