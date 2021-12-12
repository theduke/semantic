use std::{sync::Arc, task::Poll};

use factordb::AnyError;
use futures::{future::BoxFuture, FutureExt};

pub type BlobFuture<T> = BoxFuture<'static, Result<T, AnyError>>;

pub trait BlobStore {
    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>>;
    fn get_stream(&self, path: &str) -> BlobFuture<BlobStream>;
    fn put(&self, path: &str, content: Vec<u8>) -> BlobFuture<()>;
    fn remove(&self, path: &str) -> BlobFuture<()>;
}

pub type DynBlobStore = Arc<dyn BlobStore + Send + Sync>;

pub type BlobStream = futures::stream::BoxStream<'static, Result<Vec<u8>, AnyError>>;

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

struct MpscStream<T>(tokio::sync::mpsc::Receiver<T>);

impl<T> futures::stream::Stream for MpscStream<T> {
    type Item = T;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.get_mut().0.poll_recv(cx)
    }
}

impl BlobStore for logfs::LogFs {
    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>> {
        let path = path.to_string();
        run_blocking(self, move |s| s.get(&path).map_err(AnyError::from))
    }

    fn get_stream(&self, path: &str) -> BlobFuture<BlobStream> {
        let path = path.to_string();

        run_blocking(self, move |s| {
            let iter = s.get_chunks(path)?;
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, AnyError>>(2);
            let stream = MpscStream(rx);

            tokio::task::spawn_blocking(move || {
                for res in iter {
                    match res {
                        Ok(data) => {
                            if let Err(err) = tx.blocking_send(Ok(data)) {
                                tracing::warn!(%err, "Could not finish sending logfs blob data");
                                break;
                            }
                        }
                        Err(err) => {
                            if let Err(_err) = tx.blocking_send(Err(AnyError::from(err))) {
                                tracing::warn!(?_err, "Could not finish sending logfs blob data");
                            }
                            break;
                        }
                    }
                }
            });

            Ok(Box::pin(stream) as BlobStream)
        })
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
