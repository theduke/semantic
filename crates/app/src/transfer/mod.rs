//! Streaming logical database transfer formats.
//!
//! JSONL transfers portable entities only; custom schemas/packages must already
//! exist in the destination. Tar transfers additionally carry only blobs that
//! those entities reference by content hash. Imports merge through upserts and
//! do not delete unmentioned destination data.

mod archive;
mod blob_index;
mod jsonl;

use std::path::{Path, PathBuf};

use objstore::DynObjStore;
use semantic_db_core::{DbError, WriteSettings};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite};

pub const DEFAULT_BATCH_SIZE: usize = 1_000;
pub const DEFAULT_MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferFormat {
    #[default]
    Jsonl,
    Tar,
}

impl std::str::FromStr for TransferFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "jsonl" => Ok(Self::Jsonl),
            "tar" => Ok(Self::Tar),
            _ => Err(format!(
                "unknown export format '{value}' (expected jsonl or tar)"
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImportFormat {
    #[default]
    Auto,
    Jsonl,
    Tar,
}

impl std::str::FromStr for ImportFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "jsonl" => Ok(Self::Jsonl),
            "tar" => Ok(Self::Tar),
            _ => Err(format!(
                "unknown import format '{value}' (expected auto, jsonl, or tar)"
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ImportOptions {
    pub format: ImportFormat,
    pub batch_size: usize,
    pub write_settings: WriteSettings,
    pub max_record_bytes: usize,
    pub temp_dir: Option<PathBuf>,
}

impl ImportOptions {
    fn validate(&self) -> Result<(), TransferError> {
        if self.batch_size == 0 {
            return Err(TransferError::InvalidOptions(
                "import batch size must be greater than zero".into(),
            ));
        }
        if self.max_record_bytes == 0 {
            return Err(TransferError::InvalidOptions(
                "maximum record size must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            format: ImportFormat::Auto,
            batch_size: DEFAULT_BATCH_SIZE,
            write_settings: WriteSettings {
                validate_foreign_keys: false,
            },
            max_record_bytes: DEFAULT_MAX_RECORD_BYTES,
            temp_dir: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    pub format: TransferFormat,
    pub temp_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferStats {
    pub entities: u64,
    pub blobs: u64,
    pub blob_bytes: u64,
    pub batches: u64,
}

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(
        "database import failed after {committed_entities} entities in {committed_batches} batches: {source}"
    )]
    ImportDatabase {
        committed_entities: u64,
        committed_batches: u64,
        #[source]
        source: DbError,
    },
    #[error(transparent)]
    ObjectStore(#[from] objstore::ObjStoreError),
    #[error("invalid JSONL at {source_name}:{line}: {message}")]
    Json {
        source_name: String,
        line: u64,
        message: String,
    },
    #[error("invalid entity record at {source_name}:{line}: {message}")]
    InvalidRecord {
        source_name: String,
        line: u64,
        message: String,
    },
    #[error("record at {source_name}:{line} exceeds the {limit}-byte limit")]
    RecordTooLarge {
        source_name: String,
        line: u64,
        limit: usize,
    },
    #[error("invalid archive: {0}")]
    Archive(String),
    #[error("blob transfer failed: {0}")]
    Blob(String),
    #[error("temporary blob index failed: {0}")]
    Index(String),
    #[error("invalid transfer options: {0}")]
    InvalidOptions(String),
    #[error("database protocol error: {0}")]
    DatabaseProtocol(String),
    #[error("full tar transfer requires a blob store")]
    BlobStoreRequired,
}

pub async fn export<W>(
    db: &dyn crate::SemanticDb,
    blob_store: Option<DynObjStore>,
    writer: &mut W,
    options: &ExportOptions,
) -> Result<TransferStats, TransferError>
where
    W: AsyncWrite + Unpin,
{
    match options.format {
        TransferFormat::Jsonl => jsonl::export_jsonl(db, writer, |_| Ok(())).await,
        TransferFormat::Tar => {
            let store = blob_store.ok_or(TransferError::BlobStoreRequired)?;
            archive::export_tar(db, store, writer, options.temp_dir.as_deref()).await
        }
    }
}

pub async fn import<R>(
    db: &dyn crate::SemanticDb,
    blob_store: Option<DynObjStore>,
    reader: &mut R,
    source_name: &str,
    options: &ImportOptions,
) -> Result<TransferStats, TransferError>
where
    R: AsyncRead + Unpin,
{
    options.validate()?;
    let (format, prefix) = match options.format {
        ImportFormat::Auto => detect_format(reader).await?,
        format => (format, Vec::new()),
    };
    let replay = std::io::Cursor::new(prefix).chain(reader);
    let mut reader = tokio::io::BufReader::new(replay);
    match format {
        ImportFormat::Auto => unreachable!("auto import format was resolved"),
        ImportFormat::Jsonl => jsonl::import_jsonl(db, &mut reader, source_name, options).await,
        ImportFormat::Tar => {
            let store = blob_store.ok_or(TransferError::BlobStoreRequired)?;
            archive::import_tar(db, store, &mut reader, source_name, options).await
        }
    }
}

async fn detect_format<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<(ImportFormat, Vec<u8>), TransferError> {
    const TAR_HEADER_BYTES: usize = 512;
    let mut prefix = Vec::<u8>::with_capacity(TAR_HEADER_BYTES);
    while prefix.len() < TAR_HEADER_BYTES {
        if prefix
            .iter()
            .copied()
            .find(|byte| !byte.is_ascii_whitespace())
            == Some(b'{')
        {
            return Ok((ImportFormat::Jsonl, prefix));
        }
        let mut chunk = [0u8; TAR_HEADER_BYTES];
        let remaining = TAR_HEADER_BYTES - prefix.len();
        let read = reader.read(&mut chunk[..remaining]).await?;
        if read == 0 {
            break;
        }
        prefix.extend_from_slice(&chunk[..read]);
    }
    let has_ustar_magic = prefix.get(257..262) == Some(b"ustar");
    let format = if has_ustar_magic || valid_tar_checksum(&prefix) {
        ImportFormat::Tar
    } else {
        ImportFormat::Jsonl
    };
    Ok((format, prefix))
}

fn valid_tar_checksum(prefix: &[u8]) -> bool {
    let Some(header) = prefix.get(..512) else {
        return false;
    };
    let Some(raw) = std::str::from_utf8(&header[148..156]).ok() else {
        return false;
    };
    let Some(expected) = u64::from_str_radix(raw.trim_matches(['\0', ' ']), 8).ok() else {
        return false;
    };
    let actual = header
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                b' ' as u64
            } else {
                *byte as u64
            }
        })
        .sum::<u64>();
    actual == expected
}

fn make_temp_dir(parent: Option<&Path>) -> Result<tempfile::TempDir, TransferError> {
    match parent {
        Some(parent) => {
            std::fs::create_dir_all(parent)?;
            tempfile::Builder::new()
                .prefix("semantic-transfer-")
                .tempdir_in(parent)
                .map_err(TransferError::from)
        }
        None => tempfile::Builder::new()
            .prefix("semantic-transfer-")
            .tempdir()
            .map_err(TransferError::from),
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncReadExt as _, ReadBuf};

    use super::{ImportFormat, detect_format};

    struct ShortReader {
        bytes: std::io::Cursor<Vec<u8>>,
        max_read: usize,
    }

    impl AsyncRead for ShortReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let position = self.bytes.position() as usize;
            let available = self.bytes.get_ref().len().saturating_sub(position);
            let take = available.min(self.max_read).min(buf.remaining());
            if take > 0 {
                let end = position + take;
                buf.put_slice(&self.bytes.get_ref()[position..end]);
                self.bytes.set_position(end as u64);
            }
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn tar_detection_accumulates_and_replays_short_reads() {
        let mut archive = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut archive);
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "entities.jsonl", &b"{}\n"[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let mut reader = ShortReader {
            bytes: std::io::Cursor::new(archive.clone()),
            max_read: 7,
        };
        let (format, prefix) = detect_format(&mut reader).await.unwrap();
        assert_eq!(format, ImportFormat::Tar);
        assert_eq!(prefix, archive[..512]);

        let mut replay = std::io::Cursor::new(prefix).chain(reader);
        let mut restored = Vec::new();
        replay.read_to_end(&mut restored).await.unwrap();
        assert_eq!(restored, archive);
    }
}
