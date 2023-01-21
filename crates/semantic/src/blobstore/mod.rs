pub mod blobfs;
pub mod logfs;
pub mod memory;

use std::{sync::Arc, task::Poll};

use futures::future::BoxFuture;

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
