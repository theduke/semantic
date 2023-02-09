use factdb::{
    macros::{Attribute, Class},
    DataMap, Id, Timestamp,
};

use super::{AttrCreatedAt, AttrTitle};

#[derive(Attribute)]
#[factor(
    namespace = "semantic.import_url_mapping",
    name = "source_url",
    title = "Source Url"
)]
pub struct AttrImportUrlMappingSourceUrl(String);

#[derive(Attribute)]
#[factor(
    namespace = "semantic.import_url_mapping",
    name = "target_url",
    title = "Target Url"
)]
pub struct AttrImportUrlMappingTargetUrl(String);

#[derive(serde::Serialize, serde::Deserialize, Class, Clone, Debug)]
#[factor(namespace = "semantic")]
pub struct ImportUrlMapping {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: Option<String>,

    #[factor(attr = AttrImportUrlMappingSourceUrl)]
    #[serde(rename = "semantic.import_url_mapping/source_url")]
    pub source_url: String,

    #[factor(attr = AttrImportUrlMappingTargetUrl)]
    #[serde(rename = "semantic.import_url_mapping/target_url")]
    pub target_url: String,

    #[factor(attr = AttrCreatedAt)]
    #[serde(rename = "semantic/created_at")]
    pub created_at: Option<Timestamp>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}
