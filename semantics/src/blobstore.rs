use std::sync::Arc;

use factordb::AnyError;
use futures::{future::BoxFuture, FutureExt};

pub type BlobFuture<T> = BoxFuture<'static, Result<T, AnyError>>;

pub trait BlobStore {
    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>>;
    fn put(&self, path: &str, content: Vec<u8>) -> BlobFuture<()>;
    fn remove(&self, path: &str) -> BlobFuture<()>;
}

pub type DynBlobStore = Arc<dyn BlobStore + Send + Sync>;

fn run_blocking<T, F>(log: &logfs::LogFs, f: F) -> BlobFuture<T>
where
    T: Send + 'static,
    F: FnOnce(&logfs::LogFs) -> Result<T, AnyError> + Send + 'static,
{
    let s = log.clone();
    (async move {
        let out = tokio::task::spawn_blocking(move || f(&s)).await??;
        Ok(out)
    })
    .boxed()
}

impl BlobStore for logfs::LogFs {
    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>> {
        let path = path.to_string();
        run_blocking(self, move |s| s.get(&path).map_err(AnyError::from))
    }

    fn put(&self, path: &str, content: Vec<u8>) -> BlobFuture<()> {
        let path = path.to_string();
        run_blocking(self, move |s| {
            s.insert(path, content).map_err(AnyError::from)
        })
    }

    fn remove(&self, path: &str) -> BlobFuture<()> {
        let path = path.to_string();
        run_blocking(self, move |s| s.remove(path).map_err(AnyError::from))
    }
}
