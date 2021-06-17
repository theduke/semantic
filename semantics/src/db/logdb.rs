use factordb::{backend::log::EventId, AnyError};
use futures::{future::ready, FutureExt, StreamExt};
use logfs::LogFs;

// use crate::blobstore::BlobStore;

#[derive(Clone)]
pub struct LogDbStore {
    log: LogFs,
}

impl LogDbStore {
    pub fn new(log: LogFs) -> Self {
        Self { log }
    }

    async fn build_backend(self) -> Result<factordb::backend::log::LogDb, AnyError> {
        factordb::backend::log::LogDb::open(
            self,
            factordb::backend::log::convert_json::JsonConverter,
        )
        .await
    }

    pub async fn build_db(self) -> Result<factordb::Db, AnyError> {
        let be = self.build_backend().await?;
        Ok(factordb::Db::new(be))
    }

    fn event_path(id: EventId) -> Vec<u8> {
        format!("_e/{}", id).into_bytes()
    }

    async fn iter_events(
        &self,
        _from: EventId,
        _until: EventId,
    ) -> Result<futures::stream::BoxStream<'_, Result<Vec<u8>, AnyError>>, AnyError> {
        // let start = Self::event_path(from);
        // let end = Self::event_path(until);

        let s = self.log.clone();
        let blobs = self
            .log
            // FIXME: currently ignoring start/end because logfs range seems broken - does not
            // iterate properly.
            .paths_prefix(b"_e/")?
            .into_iter()
            .map(move |path| {
                s.get(path)?.ok_or_else(|| {
                    anyhow::anyhow!("Error while iterating events: event key not found")
                })
            });
        let stream = futures::stream::iter(blobs).boxed();
        Ok(stream)
    }

    async fn clear(self) -> Result<(), AnyError> {
        self.log.remove_prefix("_e/")?;
        Ok(())
    }
}

impl factordb::backend::log::LogStore for LogDbStore {
    fn iter_events(
        &self,
        from: EventId,
        until: EventId,
    ) -> futures::future::BoxFuture<
        Result<futures::stream::BoxStream<Result<Vec<u8>, AnyError>>, AnyError>,
    > {
        self.iter_events(from, until).boxed()
    }

    fn read_event(
        &self,
        id: EventId,
    ) -> futures::future::BoxFuture<Result<Option<Vec<u8>>, AnyError>> {
        let res = self.log.get(Self::event_path(id)).map_err(AnyError::from);
        ready(res).boxed()
    }

    fn write_event(
        &mut self,
        id: EventId,
        event: Vec<u8>,
    ) -> futures::future::BoxFuture<Result<EventId, AnyError>> {
        let res = self
            .log
            .insert(Self::event_path(id), event)
            .map(|_| id)
            .map_err(AnyError::from);
        ready(res).boxed()
    }

    fn clear(&mut self) -> futures::future::BoxFuture<'static, Result<(), AnyError>> {
        self.clone().clear().boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_logdb() {
        let path = std::env::temp_dir().join("semantics_tests/logdb/full.log");
        if path.exists() {
            std::fs::remove_file(&path).unwrap();
        }
        let log = LogFs::open(path, "test".into()).unwrap();
        let db = LogDbStore::new(log).build_backend().await.unwrap();
        factordb::tests::test_backend(db, |f| {
            futures::executor::block_on(f);
        });
    }
}
