use factordb::prelude::{AttrIdent, DataMap, Id, Timestamp};

use super::{AttrCreatedAt, AttrDescription, AttrTitle, AttrUrl};

#[derive(serde::Serialize, serde::Deserialize, factordb::Entity, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct Bookmark {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrIdent)]
    #[serde(rename = "factor/ident")]
    pub ident: Option<String>,

    #[factor(attr = AttrUrl)]
    #[serde(rename = "semantic/url")]
    pub url: url::Url,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: Option<String>,

    #[factor(attr = AttrDescription)]
    #[serde(rename = "semantic/description")]
    pub description: Option<String>,

    #[factor(attr = AttrCreatedAt)]
    #[serde(rename = "semantic/created_at")]
    pub created_at: Option<Timestamp>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}
