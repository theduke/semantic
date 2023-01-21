use std::future::ready;

use futures::{future::BoxFuture, FutureExt, TryFutureExt};

use super::{BlobFuture, BlobMeta, BlobStore, BlobStream};

#[derive(Clone, Debug)]
pub struct TokioSpawner {}

impl blobfs_async::Spawner for TokioSpawner {
    fn spawn_blocking<F, R>(&self, f: F) -> BoxFuture<'static, R>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        let f = async move {
            let out = tokio::task::spawn_blocking(f).await.unwrap();
            out
        };
        Box::pin(f)
    }
}

impl BlobStore for blobfs_async::AsyncRepo<TokioSpawner> {
    fn paths_offset(&self, _offset: usize, _max: usize) -> BlobFuture<Vec<String>> {
        let err = anyhow::anyhow!("blobfs does not support paths_offset");
        Box::pin(ready(Err(err)))
    }

    fn get(&self, path: &str) -> BlobFuture<Option<Vec<u8>>> {
        let f = self.get(path);
        let f = async move {
            match f.await {
                Ok(data) => Ok(Some(data)),
                Err(err) if err.is_not_found() => Ok(None),
                Err(err) => Err(anyhow::Error::from(err)),
            }
        };
        Box::pin(f)
    }

    fn get_meta(&self, path: &str) -> BlobFuture<Option<BlobMeta>> {
        let f = self.meta(path);
        let key = path.to_string();

        let f = async move {
            match f.await {
                Ok(meta) => Ok(Some(BlobMeta {
                    key,
                    size: meta.size,
                })),
                Err(err) if err.is_not_found() => Ok(None),
                Err(err) => Err(anyhow::Error::from(err)),
            }
        };

        Box::pin(f)
    }

    fn get_stream(
        &self,
        path: &str,
        offset: Option<u64>,
        take: Option<u64>,
    ) -> BlobFuture<BlobStream> {
        let repo = self.sync_repo().clone();
        let path = path.to_string();

        let (mut tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, anyhow::Error>>(100);

        fn try_send(
            tx: &mut tokio::sync::mpsc::Sender<Result<Vec<u8>, anyhow::Error>>,
            repo: blobfs::Repo,
            path: String,
            offset: Option<u64>,
            take: Option<u64>,
        ) -> Result<(), anyhow::Error> {
            let mut iter = repo.get_iter(&path)?;
            let mut to_take = take;

            if let Some(mut offset) = offset {
                while offset > 0 {
                    while let Some(res) = iter.next() {
                        let mut chunk = res?;
                        let chunk_len = chunk.len() as u64;
                        if chunk_len > offset {
                            let rest = chunk.split_off(offset as usize);
                            let rest_len = rest.len() as u64;

                            if let Some(to_take) = to_take.as_mut() {
                                if *to_take < rest_len {
                                    chunk.truncate(*to_take as usize);
                                }
                                (*to_take) -= rest_len;
                            }

                            tx.blocking_send(Ok(rest))?;
                            break;
                        }
                        offset = offset.checked_sub(chunk.len() as u64).unwrap();
                    }
                }
            }

            while let Some(next) = iter.next() {
                if to_take == Some(0) {
                    break;
                }

                let mut chunk = next?;
                let chunk_len = chunk.len() as u64;

                if let Some(to_take) = to_take.as_mut() {
                    if (*to_take) < chunk_len {
                        chunk.truncate(*to_take as usize);
                    }
                    (*to_take) -= chunk_len;
                }

                tx.blocking_send(Ok(chunk))?;
            }

            Ok(())
        }

        tokio::task::spawn_blocking(move || {
            if let Err(err) = try_send(&mut tx, repo, path, offset, take) {
                tx.blocking_send(Err(anyhow::Error::from(err))).unwrap();
            }
        });

        let stream: BlobStream = Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx));

        let out: BlobFuture<BlobStream> = Box::pin(async move { Ok(stream) });

        out
    }

    fn put(&self, path: &str, content: Vec<u8>) -> BlobFuture<()> {
        self.insert(path, content)
            .map_err(anyhow::Error::from)
            .boxed()
    }

    fn remove(&self, path: &str) -> BlobFuture<()> {
        self.remove(path).map_err(anyhow::Error::from).boxed()
    }

    fn get_std_reader(&self, path: &str) -> BlobFuture<Box<dyn std::io::Read + Send>> {
        let repo = self.sync_repo().clone();
        let path = path.to_string();
        let f = async move {
            let reader = tokio::task::spawn_blocking(move || repo.get_reader(&path))
                .await
                .map_err(anyhow::Error::from)??;
            Ok(reader.into_boxed())
        };
        Box::pin(f)
    }

    fn put_std_writer(&self, path: &str) -> BlobFuture<Box<dyn std::io::Write + Send>> {
        let repo = self.sync_repo().clone();
        let path = path.to_string();
        let f = async move {
            let writer = tokio::task::spawn_blocking(move || repo.insert_writer(&path))
                .await
                .map_err(anyhow::Error::from)??;
            Ok(writer.into_boxed_std())
        };
        Box::pin(f)
    }

    fn size_storage(&self) -> BlobFuture<Option<u64>> {
        // TODO: implement
        ready(Ok(None)).boxed()
    }

    fn size_data(&self) -> BlobFuture<Option<u64>> {
        // TODO: implement
        ready(Ok(None)).boxed()
    }
}
