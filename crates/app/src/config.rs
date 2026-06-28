use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppConfig {
    pub data_dir: Option<PathBuf>,
    pub temp_dir: Option<PathBuf>,
    pub auto_analyze_media: bool,
}

impl AppConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_data_dir(mut self, data_dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(data_dir.into());
        self
    }

    pub fn with_temp_dir(mut self, temp_dir: impl Into<PathBuf>) -> Self {
        self.temp_dir = Some(temp_dir.into());
        self
    }

    pub fn with_auto_analyze_media(mut self, enabled: bool) -> Self {
        self.auto_analyze_media = enabled;
        self
    }

    pub fn from_env() -> Self {
        let data_dir = std::env::var_os("SEMANTIC_DATA_DIR").map(PathBuf::from);
        let temp_dir = std::env::var_os("SEMANTIC_TEMP_DIR").map(PathBuf::from);
        let auto_analyze_media = std::env::var("SEMANTIC_AUTO_ANALYZE_MEDIA")
            .ok()
            .is_some_and(|value| parse_bool(&value).unwrap_or(false));
        Self {
            data_dir,
            temp_dir,
            auto_analyze_media,
        }
    }

    pub fn default_db_path(&self) -> PathBuf {
        self.data_dir
            .as_deref()
            .unwrap_or_else(|| Path::new("."))
            .join("db")
            .join("default")
    }

    pub fn default_blob_path(&self) -> PathBuf {
        self.data_dir
            .as_deref()
            .unwrap_or_else(|| Path::new("."))
            .join("blob")
            .join("default")
    }

    pub fn default_blob_uri(&self) -> std::result::Result<String, String> {
        fs_uri_for_path(self.default_blob_path())
    }

    pub fn ensure_default_db_parent_dir(&self) -> std::io::Result<()> {
        let db_path = self.default_db_path();
        if let Some(parent) = db_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn fs_uri_for_path(path: PathBuf) -> std::result::Result<String, String> {
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|err| format!("failed to resolve current directory: {err}"))?
            .join(path)
    };
    let file_url = url::Url::from_directory_path(&path)
        .map_err(|()| format!("failed to build fs uri for '{}'", path.display()))?;
    let fs_uri = format!("fs://{}", file_url.path());
    url::Url::parse(&fs_uri)
        .map_err(|err| format!("failed to parse fs uri for '{}': {err}", path.display()))?;
    Ok(fs_uri)
}
