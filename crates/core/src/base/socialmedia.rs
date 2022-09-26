use factdb::{
    macros::{Attribute, Class},
    DataMap, Id, Timestamp,
};
use serde::{Deserialize, Serialize};

use super::{AttrDescription, AttrIdent, AttrImportedAt, AttrTitle, AttrUsername, Person};

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    name = "social_media_post_content",
    title = "Content"
)]
pub struct AttrSocialMediaPostContent(Vec<Id>);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    name = "social_media_post_user_id",
    title = "User ID"
)]
pub struct AttrSocialMediaPostUserId(Id);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    name = "social_media_platform_name",
    title = "Social Media Platform"
)]
pub struct AttrSocialMediaPlatformName(String);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    name = "social_media_platform_id",
    title = "Social Media Platform"
)]
pub struct AttrSocialMediaPlatformId(Id);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "like_count", title = "Likes")]
pub struct AttrLikeCount(u64);

#[derive(Serialize, Deserialize, Class)]
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

    #[factor(attr = AttrUsername)]
    #[serde(rename = "semantic/username")]
    pub username: Option<String>,

    #[factor(attr = AttrSocialMediaPostUserId)]
    #[serde(rename = "semantic/social_media_post_user_id")]
    pub user_id: Option<Id>,

    #[factor(attr = AttrLikeCount)]
    #[serde(rename = "semantic/like_count")]
    pub like_count: Option<u64>,

    #[factor(attr = AttrSocialMediaPostContent)]
    #[serde(rename = "semantic/social_media_post_content")]
    pub content_ids: Vec<Id>,

    #[factor(attr = AttrImportedAt)]
    #[serde(rename = "semantic/imported_at")]
    pub imported_at: Option<Timestamp>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}

#[derive(Serialize, Deserialize, Class)]
#[factor(namespace = "semantic", title = "SocialMediaAccount")]
pub struct SocialMediaAccount {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrSocialMediaPlatformName)]
    #[serde(rename = "semantic/social_media_platform_name")]
    pub platform_name: Option<String>,

    #[factor(attr = AttrSocialMediaPlatformId)]
    #[serde(rename = "semantic/social_media_platform_id")]
    pub platform_id: Option<Id>,

    #[factor(attr = AttrUsername)]
    #[serde(rename = "semantic/username")]
    pub username: String,

    #[factor(extend)]
    #[serde(flatten)]
    pub person: Person,

    #[factor(attr = AttrImportedAt)]
    #[serde(rename = "semantic/imported_at")]
    pub imported_at: Option<Timestamp>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}
