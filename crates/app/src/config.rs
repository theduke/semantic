use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppConfig {
    pub data_dir: Option<PathBuf>,
}

impl AppConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_data_dir(mut self, data_dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(data_dir.into());
        self
    }

    pub fn from_env() -> Self {
        let data_dir = std::env::var_os("SEMANTIC_DATA_DIR").map(PathBuf::from);
        Self { data_dir }
    }

    pub fn default_db_path(&self) -> PathBuf {
        self.data_dir
            .as_deref()
            .unwrap_or_else(|| Path::new("."))
            .join("db")
            .join("default")
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
