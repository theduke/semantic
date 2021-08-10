use factordb::{schema::EntityDescriptor, Attribute, Entity, Id};
use serde::{Deserialize, Serialize};

use super::{AttrPreviewImageUrl, AttrTitle, AttrUrl};

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Blob")]
pub struct AttrBlobUri(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "MIME Type")]
pub struct AttrMimeType(String);

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

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct File {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

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

    #[factor(attr = AttrUrl)]
    #[serde(rename = "semantic/url")]
    pub url: Option<url::Url>,

    #[factor(attr = AttrDownloadUrl)]
    #[serde(rename = "semantic/download_url")]
    pub download_url: Option<url::Url>,

    #[factor(attr = AttrPreviewImageUrl)]
    #[serde(rename = "semantic/preview_image_url")]
    pub preview_image_url: Option<url::Url>,

    #[factor(attr = AttrBlobUri)]
    #[serde(rename = "semantic/blob_uri")]
    pub blob_uri: Option<String>,
}

#[derive(Serialize, Deserialize, Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct Image {
    #[factor(extend)]
    #[serde(flatten)]
    pub file: File,
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
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TypedFile {
    Video(Video),
    Image(Image),
    File(File),
}

impl factordb::schema::EntityContainer for TypedFile {
    fn id(&self) -> Id {
        match self {
            TypedFile::Video(e) => e.file.id,
            TypedFile::Image(e) => e.file.id,
            TypedFile::File(e) => e.id,
        }
    }

    fn entity_type(&self) -> factordb::Ident {
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
