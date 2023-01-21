use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use super::{BlobFuture, BlobMeta, BlobStore, BlobStream};

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
