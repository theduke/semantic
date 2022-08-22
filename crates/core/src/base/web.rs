use factdb::{DataMap, Class, Id, Timestamp};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{AttrCreatedAt, AttrDescription, AttrTitle, AttrUpdatedAt, AttrUrl};

#[derive(Serialize, Deserialize, Class, Clone)]
#[factor(namespace = "semantic")]
pub struct Webpage {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrUrl)]
    #[serde(rename = "semantic/url")]
    pub url: Url,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: String,

    #[factor(attr = AttrDescription)]
    #[serde(rename = "semantic/description")]
    pub description: Option<String>,

    #[factor(attr = AttrCreatedAt)]
    #[serde(rename = "semantic/created_at")]
    pub created_at: Timestamp,

    #[factor(attr = AttrUpdatedAt)]
    #[serde(rename = "semantic/updated_at")]
    pub updated_at: Timestamp,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}
