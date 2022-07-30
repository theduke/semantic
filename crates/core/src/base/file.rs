use anyhow::Context;
use factordb::{
    prelude::{
        AttrIdent, AttrMapExt, Attribute, AttributeDescriptor, DataMap, Db, Entity,
        EntityContainer, EntityDescriptor, Expr, Id, IdOrIdent, Select, Timestamp, Value,
        ValueTypeDescriptor,
    },
    AnyError,
};

use serde::{Deserialize, Serialize};

use super::{
    AttrCreatedAt, AttrPreviewImageBlobUri, AttrPreviewImageUrl, AttrTitle, AttrUpdatedAt, AttrUrl,
};

/// A hash, prefixed by the hash type.
/// eg: 'sha1:XXXXXXXXXX'
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
pub struct UniversalHash(String);

impl UniversalHash {
    pub const MD5: &'static str = "md5";
    pub const SHA256: &'static str = "sha256";

    // TODO: should probably use a Result with a custom error type here...
    pub fn try_from_string(hash: impl Into<String>) -> Option<Self> {
        let hash = hash.into();
        if hash.split_once(':').is_none() {
            None
        } else {
            Some(Self(hash))
        }
    }

    pub fn new(kind: &str, hash: &str) -> Self {
        Self(format!("{}:{}", kind, hash))
    }

    /// Get the kind of the hash, eg 'sha1'.
    pub fn kind(&self) -> Option<&str> {
        self.0.split_once(':').map(|x| x.0)
    }

    /// Get the actual hash value, without the kind prefix.
    pub fn hash(&self) -> Option<&str> {
        self.0.split_once(':').map(|x| x.1)
    }

    /// Get a pair of (hash_type, hash).
    pub fn split(&self) -> Option<(&str, &str)> {
        self.0.split_once(':')
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<UniversalHash> for Value {
    fn from(h: UniversalHash) -> Self {
        Value::String(h.0)
    }
}

impl ValueTypeDescriptor for UniversalHash {
    fn value_type() -> factordb::data::ValueType {
        factordb::data::ValueType::String
    }
}

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Blob")]
pub struct AttrBlobUri(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Blob")]
pub struct AttrBlobUriWeb(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "MIME Type")]
pub struct AttrMimeType(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "hash", title = "Content Hash")]
pub struct AttrHash(UniversalHash);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    name = "original_hash",
    title = "Original Content Hash"
)]
pub struct AttrOriginalHash(UniversalHash);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Download URL")]
pub struct AttrDownloadUrl(url::Url);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Size")]
pub struct AttrFileSize(u64);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "filename", title = "Filename")]
pub struct AttrFileName(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Duration")]
pub struct AttrDuration(u64);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "pixel_width", title = "Width")]
pub struct AttrPixelWidth(u64);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "pixel_height", title = "Height")]
pub struct AttrPixelHeight(u64);

impl AttrDuration {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl TryFrom<Value> for AttrDuration {
    type Error = anyhow::Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        value.as_uint().map(Self).context("expected a number")
    }
}

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Sound available",
    name = "video_has_sound"
)]
pub struct AttrVideoHasSound(bool);

impl TryFrom<Value> for AttrVideoHasSound {
    type Error = anyhow::Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        value.as_bool().map(Self).context("expected a boolean")
    }
}

impl AttrVideoHasSound {
    pub fn as_bool(self) -> bool {
        self.0
    }
}

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct File {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrIdent)]
    #[serde(rename = "factor/ident")]
    pub ident: Option<String>,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: Option<String>,

    #[factor(attr = AttrFileName)]
    #[serde(rename = "semantic/filename")]
    pub filename: Option<String>,

    #[factor(attr = AttrFileSize)]
    #[serde(rename = "semantic/file_size")]
    pub size: Option<u64>,

    #[factor(attr = AttrMimeType)]
    #[serde(rename = "semantic/mime_type")]
    pub mime_type: Option<String>,

    #[factor(attr = AttrHash)]
    #[serde(rename = "semantic/hash")]
    pub hash: Option<UniversalHash>,

    #[factor(attr = AttrOriginalHash)]
    #[serde(rename = "semantic/original_hash")]
    pub original_hash: Option<UniversalHash>,

    #[factor(attr = AttrUrl)]
    #[serde(rename = "semantic/url")]
    pub url: Option<url::Url>,

    #[factor(attr = AttrDownloadUrl)]
    #[serde(rename = "semantic/download_url")]
    pub download_url: Option<url::Url>,

    #[factor(attr = AttrPreviewImageUrl)]
    #[serde(rename = "semantic/preview_image_url")]
    pub preview_image_url: Option<url::Url>,

    #[factor(attr = AttrPreviewImageBlobUri)]
    #[serde(rename = "semantic/preview_image_blob_uri")]
    pub preview_image_blob_uri: Option<String>,

    #[factor(attr = AttrBlobUri)]
    #[serde(rename = "semantic/blob_uri")]
    pub blob_uri: Option<String>,

    #[factor(attr = AttrBlobUriWeb)]
    #[serde(rename = "semantic/blob_uri_web")]
    pub blob_uri_web: Option<String>,

    #[factor(attr = AttrCreatedAt)]
    #[serde(rename = "semantic/created_at")]
    pub created_at: Option<Timestamp>,

    #[factor(attr = AttrUpdatedAt)]
    #[serde(rename = "semantic/updated_at")]
    pub updated_at: Option<Timestamp>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}

impl File {
    pub fn query_by_hash(hash: &UniversalHash) -> Select {
        let filter = Expr::eq(AttrHash::expr(), hash.as_str());
        Select::new().with_filter(filter).with_limit(100)
    }

    pub async fn find_by_hash(
        db: &Db,
        hash: &UniversalHash,
    ) -> Result<Option<DataMap>, anyhow::Error> {
        let page = db.select(Self::query_by_hash(hash)).await?;

        let map = page.items.into_iter().find_map(|item| {
            let ty = item.data.get_type()?.as_name().unwrap().to_string();

            if ty == File::QUALIFIED_NAME
                || ty == Image::QUALIFIED_NAME
                || ty == Video::QUALIFIED_NAME
            {
                Some(item.data)
            } else {
                None
            }
        });

        Ok(map)
    }

    pub fn query_by_hash_or_original(
        hash: &UniversalHash,
        original: Option<&UniversalHash>,
    ) -> Select {
        // TODO: use an "extends entity type" query once implemented in factor.
        let is_file = Expr::is_entity::<File>()
            .or_with(Expr::is_entity::<Image>())
            .or_with(Expr::is_entity::<Video>());
        let is_hash = Expr::eq(AttrHash::expr(), hash.as_str());

        let filter = is_file.and_with(is_hash);

        let filter = if let Some(original) = original {
            Expr::or(
                filter,
                Expr::eq(AttrOriginalHash::expr(), original.as_str()),
            )
        } else {
            filter
        };

        Select::new().with_filter(filter).with_limit(1)
    }

    pub async fn find_by_hash_or_original(
        db: &Db,
        hash: &UniversalHash,
        original: Option<&UniversalHash>,
    ) -> Result<Option<DataMap>, AnyError> {
        let page = db
            .select(Self::query_by_hash_or_original(hash, original))
            .await?;
        let data = page.items.first().map(|item| item.data.clone());
        Ok(data)
    }

    pub fn build_blob_uri(file_id: Id, filename: Option<&str>) -> String {
        let mut uri = format!("/blob/file/{file_id}");
        if let Some(name) = filename {
            uri.push('/');
            uri.push_str(name);
        }
        uri
    }

    pub fn blob_uri_from_map(map: &DataMap) -> Option<String> {
        // Ensure that blob path is set.
        if !(map.has_attr::<AttrBlobUri>() || map.has_attr::<AttrBlobUriWeb>()) {
            return None;
        }

        let id = map.get_id()?;
        let filename = map.get_attr::<AttrFileName>();

        let url = Self::build_blob_uri(id, filename.as_ref().map(|x| x.as_str()));

        Some(url)
    }

    pub fn blob_uri(&self) -> String {
        Self::build_blob_uri(self.id, self.filename.as_ref().map(|x| x.as_str()))
    }
}

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct Image {
    #[factor(extend)]
    #[serde(flatten)]
    pub file: File,

    #[factor(attr = AttrPixelWidth)]
    #[serde(rename = "semantic/pixel_width")]
    pub width: Option<u64>,

    #[factor(attr = AttrPixelHeight)]
    #[serde(rename = "semantic/pixel_height")]
    pub height: Option<u64>,
}

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct Video {
    #[factor(extend)]
    #[serde(flatten)]
    pub file: File,

    #[factor(attr = AttrDuration)]
    #[serde(rename = "semantic/duration")]
    pub duration: Option<u64>,

    #[factor(attr = AttrVideoHasSound)]
    #[serde(rename = "semantic/video_has_sound")]
    pub video_has_sound: Option<bool>,

    #[factor(attr = AttrPixelWidth)]
    #[serde(rename = "semantic/pixel_width")]
    pub width: Option<u64>,

    #[factor(attr = AttrPixelHeight)]
    #[serde(rename = "semantic/pixel_height")]
    pub height: Option<u64>,
}

impl Video {
    pub fn build_video_uri(file_id: Id, filename: Option<&str>) -> String {
        let mut uri = format!("/blob/video/{file_id}");
        if let Some(name) = filename {
            uri.push('/');
            uri.push_str(name);
        }
        uri
    }

    pub fn video_uri_from_map(map: &DataMap) -> Option<String> {
        // Ensure that blob path is set.
        if !(map.has_attr::<AttrBlobUri>() || map.has_attr::<AttrBlobUriWeb>()) {
            return None;
        }

        let id = map.get_id()?;
        let filename = map.get_attr::<AttrFileName>();

        let url = Self::build_video_uri(id, filename.as_ref().map(|x| x.as_str()));

        Some(url)
    }

    pub fn video_uri(&self) -> String {
        Self::build_video_uri(
            self.file.id,
            self.file.filename.as_ref().map(|x| x.as_str()),
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TypedFile {
    Video(Video),
    Image(Image),
    File(File),
}

impl TypedFile {
    pub fn from_map(map: DataMap) -> Result<Self, anyhow::Error> {
        match map.get_type_name() {
            Some(Video::QUALIFIED_NAME) => Video::try_from_map(map)
                .map(TypedFile::Video)
                .context("could not deserialize video"),
            Some(Image::QUALIFIED_NAME) => Image::try_from_map(map)
                .map(TypedFile::Image)
                .context("could not deserialize image"),
            Some(other) => Err(anyhow::anyhow!("unsupported file type {other}")),
            None => Err(anyhow::anyhow!("map has no type")),
        }
    }

    pub fn from_file(file: File) -> Self {
        match file.entity_type().as_name().unwrap_or_default() {
            Video::QUALIFIED_NAME => Self::Video(Video {
                duration: file.extra.get_attr::<AttrDuration>(),
                video_has_sound: file.extra.get_attr::<AttrVideoHasSound>(),
                width: file.extra.get_attr::<AttrPixelWidth>(),
                height: file.extra.get_attr::<AttrPixelHeight>(),
                file,
            }),
            Image::QUALIFIED_NAME => Self::Image(Image {
                width: file.extra.get_attr::<AttrPixelWidth>(),
                height: file.extra.get_attr::<AttrPixelHeight>(),
                file,
            }),
            _ => Self::File(file),
        }
    }
}

impl factordb::schema::EntityContainer for TypedFile {
    fn id(&self) -> Id {
        match self {
            TypedFile::Video(e) => e.file.id,
            TypedFile::Image(e) => e.file.id,
            TypedFile::File(e) => e.id,
        }
    }

    fn entity_type(&self) -> IdOrIdent {
        match self {
            TypedFile::Video(_) => Video::IDENT,
            TypedFile::Image(_) => Image::IDENT,
            TypedFile::File(_) => File::IDENT,
        }
    }

    fn into_map(self) -> Result<factordb::data::DataMap, factordb::data::value::ValueSerializeError>
    where
        Self: serde::Serialize + Sized,
    {
        match self {
            TypedFile::Video(e) => e.into_map(),
            TypedFile::Image(e) => e.into_map(),
            TypedFile::File(e) => e.into_map(),
        }
    }
}
