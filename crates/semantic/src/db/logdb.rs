use factor_engine::backend::{
    self,
    log::{EventId, LogConverter, LogEvent},
};
use factordb::AnyError;
use futures::{future::ready, FutureExt, StreamExt};
use logfs::LogFs;

// use crate::blobstore::BlobStore;

#[derive(Clone)]
pub struct LogDbStore {
    log: LogFs,
    prefix: String,
    converter: backend::log::convert_json::JsonConverter,
}

pub const DEFAULT_PREFIX: &str = "_e/";

impl LogDbStore {
    pub fn new(log: LogFs) -> Self {
        Self {
            log,
            prefix: DEFAULT_PREFIX.to_string(),
            converter: backend::log::convert_json::JsonConverter,
        }
    }

    pub fn new_with_prefix(log: LogFs, prefix: String) -> Self {
        Self {
            log,
            prefix,
            converter: backend::log::convert_json::JsonConverter,
        }
    }

    pub fn log(&self) -> &LogFs {
        &self.log
    }

    async fn build_backend(self) -> Result<backend::log::LogDb, AnyError> {
        backend::log::LogDb::open(self).await
    }

    pub async fn build_db(self) -> Result<factordb::db::Db, AnyError> {
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
    ) -> Result<futures::stream::BoxStream<'_, Result<LogEvent, AnyError>>, AnyError> {
        // let start = Self::event_path(from);
        // let end = Self::event_path(until);

        let s = self.log.clone();
        let blobs = self
            .log
            // FIXME: currently ignoring start/end because logfs range seems broken - does not
            // iterate properly.
            .paths_prefix(&self.prefix)?
            .into_iter()
            .map(move |path| {
                let data = s.get(path)?.ok_or_else(|| {
                    anyhow::anyhow!("Error while iterating events: event key not found")
                })?;

                let event = self.converter.deserialize(&data)?;
                Ok(event)
            });
        let stream = futures::stream::iter(blobs).boxed();
        Ok(stream)
    }

    async fn clear(self) -> Result<(), AnyError> {
        self.log.remove_prefix(&self.prefix)?;
        Ok(())
    }
}

impl backend::log::LogStore for LogDbStore {
    fn as_any(&self) -> &dyn std::any::Any {
        &*self
    }

    fn iter_events(
        &self,
        from: EventId,
        until: EventId,
    ) -> futures::future::BoxFuture<
        Result<futures::stream::BoxStream<Result<LogEvent, AnyError>>, AnyError>,
    > {
        self.iter_events(from, until).boxed()
    }

    fn read_event(
        &self,
        id: EventId,
    ) -> futures::future::BoxFuture<Result<Option<LogEvent>, AnyError>> {
        let converter = self.converter.clone();
        let res = self
            .log
            .get(self.event_path(id))
            .map_err(AnyError::from)
            .and_then(move |data| {
                data.map(|data| converter.deserialize(&data).map_err(Into::into))
                    .transpose()
            });
        ready(res).boxed()
    }

    fn write_event(&mut self, event: LogEvent) -> futures::future::BoxFuture<Result<(), AnyError>> {
        let res = self.converter.serialize(&event).and_then(|data| {
            self.log
                .insert(self.event_path(event.id()), data)
                .map_err(AnyError::from)
        });
        ready(res).boxed()
    }

    fn clear(&mut self) -> futures::future::BoxFuture<'static, Result<(), AnyError>> {
        self.clone().clear().boxed()
    }

    fn size_log(&mut self) -> futures::future::BoxFuture<'static, Result<Option<u64>, AnyError>> {
        ready(self.log.size_log().map(Some).map_err(AnyError::from)).boxed()
    }

    fn size_data(&mut self) -> futures::future::BoxFuture<'static, Result<Option<u64>, AnyError>> {
        ready(self.log.size_data().map(Some).map_err(AnyError::from)).boxed()
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
