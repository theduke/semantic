use std::{future::ready, sync::Arc, task::Poll};

use factordb::AnyError;
use futures::{future::BoxFuture, FutureExt};

pub type BlobFuture<T> = BoxFuture<'static, Result<T, AnyError>>;

#[derive(Clone, Debug)]
pub struct BlobMeta {
    pub key: String,
    pub size: u64,
}

pub trait BlobStore {
    fn paths_offset(&self, offset: usize, max: usize) -> BlobFuture<Vec<String>>;

    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>>;
    fn get_meta(&self, path: &str) -> BlobFuture<Option<BlobMeta>>;
    fn get_stream(&self, path: &str, offset: Option<u64>) -> BlobFuture<BlobStream>;
    fn put(&self, path: &str, content: Vec<u8>) -> BlobFuture<()>;
    fn remove(&self, path: &str) -> BlobFuture<()>;

    fn get_std_reader(&self, path: &str) -> BlobFuture<Box<dyn std::io::Read + Send>>;
    fn put_std_writer(&self, path: &str) -> BlobFuture<Box<dyn std::io::Write + Send>>;

    fn size_storage(&self) -> BlobFuture<Option<u64>>;
    fn size_data(&self) -> BlobFuture<Option<u64>>;
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
    fn paths_offset(&self, offset: usize, max: usize) -> BlobFuture<Vec<String>> {
        let res = self.paths_offset(offset, max).map_err(anyhow::Error::from);
        Box::pin(futures::future::ready(res))
    }

    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>> {
        let path = path.to_string();
        run_blocking(self, move |s| s.get(&path).map_err(AnyError::from))
    }

    fn get_meta(&self, path: &str) -> BlobFuture<Option<BlobMeta>> {
        // No need for run_blocking because get_meta is quasi-instant.
        // (metadata is all in memory)
        let res = match self.get_meta(path) {
            Ok(Some(meta)) => Ok(Some(BlobMeta {
                key: path.to_string(),
                size: meta.size,
            })),
            Ok(None) => Ok(None),
            Err(err) => Err(AnyError::from(err)),
        };
        Box::pin(futures::future::ready(res))
    }

    fn get_stream(&self, path: &str, offset: Option<u64>) -> BlobFuture<BlobStream> {
        let path = path.to_string();

        run_blocking(self, move |s| {
            let mut iter = s.get_chunks(path)?;
            if let Some(offset) = offset {
                iter.skip_bytes(offset)?;
            }
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
                        Err(error) => {
                            tracing::trace!(?error, "could not read from logfs stream");
                            if let Err(_err) = tx.blocking_send(Err(AnyError::from(error))) {
                                tracing::warn!(?_err, "Could not send blob stream termination");
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

    fn get_std_reader(&self, path: &str) -> BlobFuture<Box<dyn std::io::Read + Send>> {
        let path = path.to_string();
        Box::pin(run_blocking(self, move |s| {
            s.get_reader(path)
                .map(|x| Box::new(x) as Box<dyn std::io::Read + Send>)
                .map_err(AnyError::from)
        }))
    }

    fn put_std_writer(&self, path: &str) -> BlobFuture<Box<dyn std::io::Write + Send>> {
        let path = path.to_string();
        Box::pin(run_blocking(self, move |s| {
            s.insert_writer(path)
                .map(|x| Box::new(x) as Box<dyn std::io::Write + Send>)
                .map_err(AnyError::from)
        }))
    }

    fn size_storage(&self) -> BlobFuture<Option<u64>> {
        ready(self.size_log().map(Some).map_err(anyhow::Error::from)).boxed()
    }

    fn size_data(&self) -> BlobFuture<Option<u64>> {
        ready(self.size_data().map(Some).map_err(anyhow::Error::from)).boxed()
    }
}
