use factordb::{Attribute, Entity, Id};
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
#[factor(namespace = "semantic", title = "Duration")]
pub struct AttrDuration(u64);

#[derive(Serialize, Deserialize, Entity)]
#[factor(namespace = "semantic")]
pub struct File {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: Option<String>,

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

#[derive(Serialize, Deserialize, Entity)]
#[factor(namespace = "semantic")]
pub struct Image {
    #[factor(extend)]
    #[serde(flatten)]
    pub file: File,
}

#[derive(Serialize, Deserialize, Entity)]
#[factor(namespace = "semantic")]
pub struct Video {
    #[factor(extend)]
    #[serde(flatten)]
    pub file: File,

    #[factor(attr = AttrDuration)]
    #[serde(rename = "semantic/duration")]
    pub duration: Option<u64>,
}
