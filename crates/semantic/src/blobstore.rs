use std::{
    collections::HashMap,
    future::ready,
    sync::{Arc, RwLock},
    task::Poll,
};

use futures::{future::BoxFuture, FutureExt};

pub type BlobFuture<T> = BoxFuture<'static, Result<T, anyhow::Error>>;

#[derive(Clone, Debug)]
pub struct BlobMeta {
    pub key: String,
    pub size: u64,
}

pub trait BlobStore {
    fn paths_offset(&self, offset: usize, max: usize) -> BlobFuture<Vec<String>>;

    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>>;
    fn get_meta(&self, path: &str) -> BlobFuture<Option<BlobMeta>>;
    fn get_stream(
        &self,
        path: &str,
        offset: Option<u64>,
        take: Option<u64>,
    ) -> BlobFuture<BlobStream>;
    fn put(&self, path: &str, content: Vec<u8>) -> BlobFuture<()>;
    fn remove(&self, path: &str) -> BlobFuture<()>;

    fn get_std_reader(&self, path: &str) -> BlobFuture<Box<dyn std::io::Read + Send>>;
    fn put_std_writer(&self, path: &str) -> BlobFuture<Box<dyn std::io::Write + Send>>;

    fn size_storage(&self) -> BlobFuture<Option<u64>>;
    fn size_data(&self) -> BlobFuture<Option<u64>>;
}

pub type DynBlobStore = Arc<dyn BlobStore + Send + Sync>;

pub type BlobStream = futures::stream::BoxStream<'static, Result<Vec<u8>, anyhow::Error>>;

fn run_blocking<T, F>(log: &logfs::LogFs, f: F) -> BlobFuture<T>
where
    T: Send + 'static,
    F: FnOnce(&logfs::LogFs) -> Result<T, anyhow::Error> + Send + 'static,
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

#[derive(Clone)]
pub struct MemoryBlobStore {
    data: Arc<RwLock<MemoryData>>,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(MemoryData {
                data: HashMap::new(),
            })),
        }
    }
}

struct MemoryData {
    data: HashMap<String, Vec<u8>>,
}

impl BlobStore for MemoryBlobStore {
    fn paths_offset(&self, offset: usize, max: usize) -> BlobFuture<Vec<String>> {
        let keys: Vec<String> = self
            .data
            .read()
            .unwrap()
            .data
            .keys()
            .skip(offset)
            .take(max)
            .cloned()
            .collect();
        Box::pin(std::future::ready(Ok(keys)))
    }

    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>> {
        let data = self.data.read().unwrap().data.get(path).cloned();
        Box::pin(std::future::ready(Ok(data)))
    }

    fn get_meta(&self, path: &str) -> BlobFuture<Option<BlobMeta>> {
        let data = self
            .data
            .read()
            .unwrap()
            .data
            .get(path)
            .map(|data| BlobMeta {
                key: path.to_string(),
                size: data.len() as u64,
            });
        Box::pin(std::future::ready(Ok(data)))
    }

    fn get_stream(
        &self,
        path: &str,
        offset: Option<u64>,
        take: Option<u64>,
    ) -> BlobFuture<BlobStream> {
        let stream = self
            .data
            .read()
            .unwrap()
            .data
            .get(path)
            .map(|data| {
                let slice = data.as_slice();
                let slice = if let Some(offset) = &offset {
                    &slice[*offset as usize..]
                } else {
                    slice
                };
                let slice = if let Some(take) = &take {
                    &slice[..*take as usize]
                } else {
                    slice
                };

                let s: BlobStream = Box::pin(futures::stream::iter(vec![Ok::<_, anyhow::Error>(
                    slice.to_vec(),
                )]));

                s
            })
            .ok_or_else(|| anyhow::anyhow!("Blob not found"));

        Box::pin(std::future::ready(stream))
    }

    fn put(&self, path: &str, content: Vec<u8>) -> BlobFuture<()> {
        self.data
            .write()
            .unwrap()
            .data
            .insert(path.to_string(), content);
        Box::pin(std::future::ready(Ok(())))
    }

    fn remove(&self, path: &str) -> BlobFuture<()> {
        self.data.write().unwrap().data.remove(path);
        Box::pin(std::future::ready(Ok(())))
    }

    fn get_std_reader(&self, path: &str) -> BlobFuture<Box<dyn std::io::Read + Send>> {
        let res = self
            .data
            .read()
            .unwrap()
            .data
            .get(path)
            .map(|data| {
                let reader: Box<dyn std::io::Read + Send> =
                    Box::new(std::io::Cursor::new(data.to_vec()));

                reader
            })
            .ok_or_else(|| anyhow::anyhow!("Not found"));

        Box::pin(std::future::ready(res))
    }

    fn put_std_writer(&self, path: &str) -> BlobFuture<Box<dyn std::io::Write + Send>> {
        let writer: Box<dyn std::io::Write + Send> = Box::new(MemoryWriter {
            path: path.to_string(),
            buffer: Vec::new(),
            store: self.clone(),
        });
        Box::pin(std::future::ready(Ok(writer)))
    }

    fn size_storage(&self) -> BlobFuture<Option<u64>> {
        let size = self
            .data
            .read()
            .unwrap()
            .data
            .values()
            .map(|v| v.len() as u64)
            .sum();
        Box::pin(std::future::ready(Ok(Some(size))))
    }

    fn size_data(&self) -> BlobFuture<Option<u64>> {
        self.size_storage()
    }
}

pub struct MemoryWriter {
    path: String,
    buffer: Vec<u8>,
    store: MemoryBlobStore,
}

impl std::io::Write for MemoryWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for MemoryWriter {
    fn drop(&mut self) {
        self.store
            .data
            .write()
            .unwrap()
            .data
            .insert(self.path.clone(), std::mem::take(&mut self.buffer));
    }
}

impl BlobStore for logfs::LogFs {
    fn paths_offset(&self, offset: usize, max: usize) -> BlobFuture<Vec<String>> {
        let res = self.paths_offset(offset, max).map_err(anyhow::Error::from);
        Box::pin(futures::future::ready(res))
    }

    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>> {
        let path = path.to_string();
        run_blocking(self, move |s| s.get(&path).map_err(anyhow::Error::from))
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
            Err(err) => Err(anyhow::Error::from(err)),
        };
        Box::pin(futures::future::ready(res))
    }

    fn get_stream(
        &self,
        path: &str,
        offset: Option<u64>,
        take: Option<u64>,
    ) -> BlobFuture<BlobStream> {
        let path = path.to_string();

        run_blocking(self, move |s| {
            let mut iter = s.get_chunks(&path)?;
            if let Some(offset) = offset {
                iter.skip_bytes(offset)?;
            }
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, anyhow::Error>>(2);
            let stream = MpscStream(rx);

            tokio::task::spawn_blocking(move || {
                let mut transferred = 0;
                for res in iter {
                    match res {
                        Ok(mut data) => {
                            if let Some(take) = take {
                                let new_transferred = transferred + data.len() as u64;
                                if new_transferred > take {
                                    data.truncate((take - transferred) as usize);

                                    if let Err(err) = tx.blocking_send(Ok(data)) {
                                        tracing::warn!(%err, "Could not finish sending logfs blob data");
                                    }
                                    break;
                                }

                                transferred = new_transferred;
                            }

                            if let Err(err) = tx.blocking_send(Ok(data)) {
                                tracing::warn!(%err, "Could not finish sending logfs blob data");
                                break;
                            }
                        }
                        Err(error) => {
                            tracing::trace!(?error, "could not read from logfs stream");
                            if let Err(_err) = tx.blocking_send(Err(anyhow::Error::from(error))) {
                                tracing::warn!(?_err, "Could not send blob stream termination");
                            }
                            break;
                        }
                    }
                }

                tracing::trace!(%path, "file serving finished");
            });

            Ok(Box::pin(stream) as BlobStream)
        })
    }

    fn put(&self, path: &str, content: Vec<u8>) -> BlobFuture<()> {
        let path = path.to_string();
        run_blocking(self, move |s| {
            s.insert(path, content).map_err(anyhow::Error::from)
        })
    }

    fn remove(&self, path: &str) -> BlobFuture<()> {
        let path = path.to_string();
        run_blocking(self, move |s| s.remove(path).map_err(anyhow::Error::from))
    }

    fn get_std_reader(&self, path: &str) -> BlobFuture<Box<dyn std::io::Read + Send>> {
        let path = path.to_string();
        Box::pin(run_blocking(self, move |s| {
            s.get_reader(path)
                .map(|x| Box::new(x) as Box<dyn std::io::Read + Send>)
                .map_err(anyhow::Error::from)
        }))
    }

    fn put_std_writer(&self, path: &str) -> BlobFuture<Box<dyn std::io::Write + Send>> {
        let path = path.to_string();
        Box::pin(run_blocking(self, move |s| {
            s.insert_writer(path)
                .map(|x| Box::new(x) as Box<dyn std::io::Write + Send>)
                .map_err(anyhow::Error::from)
        }))
    }

    fn size_storage(&self) -> BlobFuture<Option<u64>> {
        ready(self.size_log().map(Some).map_err(anyhow::Error::from)).boxed()
    }

    fn size_data(&self) -> BlobFuture<Option<u64>> {
        ready(self.size_data().map(Some).map_err(anyhow::Error::from)).boxed()
    }
}
