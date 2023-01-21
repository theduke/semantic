use std::future::ready;

use futures::FutureExt;

use super::{BlobFuture, BlobMeta, BlobStore, BlobStream, MpscStream};

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
