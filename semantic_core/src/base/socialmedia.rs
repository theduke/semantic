use factordb::{data::DataMap, Attribute, Entity, Id};
use serde::{Deserialize, Serialize};

use super::{AttrIdent, AttrTitle, AttrUrl, AttrUsername};

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    name = "social_media_post_content",
    title = "Content"
)]
pub struct AttrSocialMediaPostContent(Id);

#[derive(Serialize, Deserialize, Entity)]
#[factor(namespace = "semantic", title = "SocialMediaPost")]
pub struct SocialMediaPost {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,
    #[factor(attr = AttrIdent)]
    #[serde(rename = "factor/ident")]
    pub ident: Option<String>,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: Option<String>,
    #[factor(attr = AttrUrl)]
    #[serde(rename = "semantic/url")]
    pub url: Option<url::Url>,
    #[factor(attr = AttrUsername)]
    #[serde(rename = "semantic/title")]
    pub username: Option<String>,

    #[factor(attr = AttrSocialMediaPostContent)]
    #[serde(rename = "semantic/social_media_post_content")]
    pub content_ids: Vec<Id>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}
