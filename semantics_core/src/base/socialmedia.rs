use factordb::{Attribute, Entity, Id, Ident};
use serde::{Deserialize, Serialize};

use super::{AttrIdent, AttrTitle, AttrUrl, AttrUsername};

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "social_media_post_content")]
pub struct AttrSocialMediaPostContent(Id);

#[derive(Serialize, Deserialize, Entity)]
#[factor(namespace = "semantic")]
pub struct SocialMediaPost {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,
    #[factor(attr = AttrIdent)]
    #[serde(rename = "factor/ident")]
    pub ident: Option<Ident>,
    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantics/title")]
    pub title: Option<String>,
    #[factor(attr = AttrUrl)]
    #[serde(rename = "semantics/url")]
    pub url: Option<url::Url>,
    #[factor(attr = AttrUsername)]
    #[serde(rename = "semantics/title")]
    pub username: Option<String>,

    #[factor(attr = AttrSocialMediaPostContent)]
    #[serde(rename = "semantics/social_media_post.content")]
    pub content_ids: Vec<Id>,
}
