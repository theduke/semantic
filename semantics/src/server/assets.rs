use std::io::Read;

use factordb::AnyError;
use hyper::{Body, Response};

pub trait AssetSource {
    fn file(&self, path: &str) -> Result<Option<Vec<u8>>, AnyError>;

    fn request(&self, path: &str) -> Response<Body> {
        match self.file(path) {
            Ok(Some(data)) => {
                let mime = mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string();

                Response::builder()
                    .header(hyper::header::CONTENT_TYPE, mime)
                    .body(Body::from(data))
                    .unwrap()
            }
            Ok(None) => super::not_found(),
            Err(err) => {
                tracing::error!(error=%err, %path, "Could not serve asset");
                super::internal_server_error("Could not read asset")
            }
        }
    }
}

pub struct StaticAssetSource {
    dir: include_dir::Dir<'static>,
}

impl StaticAssetSource {
    #[allow(dead_code)]
    pub fn new(dir: include_dir::Dir<'static>) -> Self {
        Self { dir }
    }
}

impl AssetSource for StaticAssetSource {
    fn file(&self, path: &str) -> Result<Option<Vec<u8>>, AnyError> {
        Ok(self.dir.get_file(path).map(|f| f.contents().to_vec()))
    }
}

pub struct FsAssetSource {
    root: std::path::PathBuf,
}

impl FsAssetSource {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl AssetSource for FsAssetSource {
    fn file(&self, path: &str) -> Result<Option<Vec<u8>>, AnyError> {
        // TODO: this is nedless work for a hand full of files.
        // We should probably read all assets into a cache.
        let full_path = self.root.join(path);
        match std::fs::File::open(full_path) {
            Ok(file) => {
                let mut buffer = Vec::new();
                std::io::BufReader::new(file).read_to_end(&mut buffer)?;
                Ok(Some(buffer))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

#[derive(Clone)]
pub struct Assets(std::sync::Arc<dyn AssetSource + Send + Sync>);

impl std::fmt::Debug for Assets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Assets")
    }
}

impl Assets {
    pub fn new(source: impl AssetSource + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(source))
    }
}

impl Assets {
    pub fn request(&self, path: &str) -> Response<Body> {
        let clean_path = path.trim_start_matches('/').trim_start_matches("assets/");
        self.0.request(clean_path)
    }
}
