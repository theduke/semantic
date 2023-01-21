use blobfs_async::AsyncRepo;
use factor_engine::backend::{
    self,
    log::{EventId, LogConverter, LogEvent},
};
use futures::{future::ready, FutureExt, StreamExt};

use crate::blobstore::blobfs::TokioSpawner;

// use crate::blobstore::BlobStore;

#[derive(Clone)]
pub struct BlobfsDbStore {
    repo: AsyncRepo<TokioSpawner>,
    prefix: String,
    converter: backend::log::convert_json::JsonConverter,
}

pub const DEFAULT_PREFIX: &str = "_e/";

impl BlobfsDbStore {
    pub fn new(repo: AsyncRepo<TokioSpawner>) -> Self {
        Self {
            repo,
            prefix: DEFAULT_PREFIX.to_string(),
            converter: backend::log::convert_json::JsonConverter,
        }
    }

    pub fn new_with_prefix(repo: AsyncRepo<TokioSpawner>, prefix: String) -> Self {
        Self {
            repo,
            prefix,
            converter: backend::log::convert_json::JsonConverter,
        }
    }

    pub fn repo(&self) -> &AsyncRepo<TokioSpawner> {
        &self.repo
    }

    async fn build_backend(self) -> Result<backend::log::LogDb, anyhow::Error> {
        backend::log::LogDb::open(self).await
    }

    pub async fn build_db(self) -> Result<factdb::Db, anyhow::Error> {
        let be = self.build_backend().await?;
        Ok(factor_engine::Engine::new(be).into_client())
    }

    fn event_path(&self, id: EventId) -> String {
        format!("{}{:0>20}", self.prefix, id)
    }

    async fn iter_events(
        &self,
        _from: EventId,
        _until: EventId,
    ) -> Result<futures::stream::BoxStream<'_, Result<LogEvent, anyhow::Error>>, anyhow::Error>
    {
        // let start = Self::event_path(from);
        // let end = Self::event_path(until);

        let metas = self
            .repo
            // FIXME: currently ignoring start/end because logfs range seems broken - does not
            // iterate properly.
            .list_dir(&self.prefix)
            .await?
            .into_iter()
            .filter_map(|item| match item {
                blobfs::TreeItem::Dir { .. } => None,
                blobfs::TreeItem::Object(meta) => Some(meta),
            });

        let repo = self.repo.clone();

        let stream = futures::stream::iter(metas).then(move |meta| {
            let repo = repo.clone();
            async move {
                let content = repo.get(&meta.path).await?;
                let entry = serde_json::from_slice(&content)?;

                Ok(entry)
            }
        });
        Ok(stream.boxed())
    }

    async fn read_event(&self, id: EventId) -> Result<Option<LogEvent>, anyhow::Error> {
        let res = self.repo.get(&self.event_path(id)).await;

        let data = match res {
            Ok(x) => x,
            Err(err) if err.is_not_found() => {
                return Ok(None);
            }
            Err(err) => {
                return Err(err.into());
            }
        };

        let item = self.converter.deserialize(&data)?;
        Ok(Some(item))
    }

    async fn write_event(&self, event: LogEvent) -> Result<(), anyhow::Error> {
        let data = self.converter.serialize(&event)?;
        self.repo.insert(&self.event_path(event.id()), data).await?;
        Ok(())
    }

    async fn clear(self) -> Result<(), anyhow::Error> {
        self.repo.remove_prefix(&self.prefix).await?;
        Ok(())
    }
}

impl backend::log::LogStore for BlobfsDbStore {
    fn as_any(&self) -> &dyn std::any::Any {
        &*self
    }

    fn iter_events(
        &self,
        from: EventId,
        until: EventId,
    ) -> futures::future::BoxFuture<
        Result<futures::stream::BoxStream<Result<LogEvent, anyhow::Error>>, anyhow::Error>,
    > {
        self.iter_events(from, until).boxed()
    }

    fn read_event(
        &self,
        id: EventId,
    ) -> futures::future::BoxFuture<Result<Option<LogEvent>, anyhow::Error>> {
        let s = self.clone();
        let f = async move { s.read_event(id).await };

        f.boxed()
    }

    fn write_event(
        &mut self,
        event: LogEvent,
    ) -> futures::future::BoxFuture<Result<(), anyhow::Error>> {
        let s = self.clone();
        let f = async move { s.write_event(event).await };
        f.boxed()
    }

    fn clear(&mut self) -> futures::future::BoxFuture<'static, Result<(), anyhow::Error>> {
        self.clone().clear().boxed()
    }

    fn size_log(
        &mut self,
    ) -> futures::future::BoxFuture<'static, Result<Option<u64>, anyhow::Error>> {
        // TODO: implement!
        ready(Ok(None)).boxed()
    }

    fn size_data(
        &mut self,
    ) -> futures::future::BoxFuture<'static, Result<Option<u64>, anyhow::Error>> {
        // TODO: implement!
        ready(Ok(None)).boxed()
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    // FIXME: re-enable!
    // #[tokio::test]
    // async fn test_logdb() {
    //     let path = std::env::temp_dir().join("semantics_tests/logdb/full.log");
    //     if path.exists() {
    //         std::fs::remove_file(&path).unwrap();
    //     }
    //     let log = logfs::ConfigBuilder::new(&path).open().unwrap();
    //     let db = LogDbStore::new(log).build_backend().await.unwrap();
    //     factor_engine::tests::test_backend(db, |f| {
    //         futures::executor::block_on(f);
    //     });
    // }
}
