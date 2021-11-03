mod file;
use crate::plugin::{PluginDescriptor, PluginSchema};

pub use self::file::*;

mod notes;
pub use self::notes::*;

mod socialmedia;
pub use self::socialmedia::*;

mod habit;
pub use self::habit::*;

mod collection;
pub use self::collection::*;

mod tags;
pub use self::tags::*;

mod health;
pub use self::health::*;

use factordb::{
    data::{DataMap, Timestamp},
    query::migrate::Migration,
    schema::{builtin::AttrIdent, AttrMapExt, AttributeDescriptor, EntityDescriptor},
    Attribute,
};

// Common default attributes.

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Title")]
pub struct AttrTitle(String);

pub fn entity_title(data: &DataMap) -> String {
    data.get_attr::<AttrTitle>()
        .or_else(|| data.get_id().map(|x| x.to_string()))
        .unwrap_or_else(|| "<No Title>".to_string())
}

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Description")]
pub struct AttrDescription(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Url")]
pub struct AttrUrl(url::Url);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Preview")]
pub struct AttrPreviewImageUrl(url::Url);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Datetime", name = "datetime")]
pub struct AttrDateTime(Timestamp);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Username")]
pub struct AttrUsername(String);

pub struct SemanticPlugin;

impl PluginDescriptor for SemanticPlugin {
    const NAME: &'static str = "semantic/base";
    const IDENT: factordb::Ident = factordb::Ident::new_static(Self::NAME);

    fn migrations() -> Vec<factordb::query::migrate::Migration> {
        let first = Migration::with_name("semantic/base/v1".to_string())
            .attr_create(AttrTitle::schema())
            .attr_create(AttrDateTime::schema())
            .attr_create(AttrDescription::schema())
            .attr_create(AttrUrl::schema())
            .attr_create(AttrPreviewImageUrl::schema())
            .attr_create(AttrUsername::schema())
            .attr_create(AttrBlobUri::schema())
            .attr_create(AttrMimeType::schema())
            .attr_create(AttrHash::schema())
            .attr_create(AttrDuration::schema())
            .attr_create(AttrFileSize::schema())
            .attr_create(AttrDownloadUrl::schema())
            .attr_create(AttrFileName::schema())
            .attr_create(AttrSocialMediaPostContent::schema())
            .attr_create(notes::AttrNoteBody::schema())
            .attr_create(habit::AttrHabitOccurenceComment::schema())
            .attr_create(habit::AttrHabitOccurenceParentId::schema())
            .attr_create(habit::AttrHabitOccurenceTime::schema())
            .attr_create(habit::HabitMode::schema())
            .attr_create(collection::AttrCollectionItem::schema())
            .attr_create(tags::AttrTagName::schema())
            .attr_create(tags::AttrTagParent::schema())
            .attr_create(tags::AttrTags::schema())
            .entity_create(File::schema())
            .entity_create(Image::schema())
            .entity_create(Video::schema())
            .entity_create(SocialMediaPost::schema())
            .entity_create(notes::Note::schema())
            .entity_create(habit::Habit::schema())
            .entity_create(habit::HabitOccurence::schema())
            .entity_create(collection::Collection::schema())
            .entity_create(tags::Tag::schema());

        let health_create = Migration::with_name("semantic/base/health-create".to_string())
            .attr_create(AttrWeight::schema())
            .entity_create(health::WeightLogEntry::schema());

        vec![first, health_create]
    }

    fn schema() -> PluginSchema {
        PluginSchema {
            name: Self::NAME.into(),
            description: None,
            import_matchers: Vec::new(),
            db: Some(factordb::schema::DbSchema {
                attributes: vec![
                    AttrTitle::schema(),
                    AttrDescription::schema(),
                    AttrUrl::schema(),
                    AttrPreviewImageUrl::schema(),
                    AttrUsername::schema(),
                    // file
                    AttrBlobUri::schema(),
                    AttrMimeType::schema(),
                    AttrHash::schema(),
                    AttrDuration::schema(),
                    AttrFileSize::schema(),
                    AttrDownloadUrl::schema(),
                    AttrFileName::schema(),
                    // socialmedia
                    AttrSocialMediaPostContent::schema(),
                    // Notes.
                    notes::AttrNoteBody::schema(),
                    // Habit.
                    habit::AttrHabitOccurenceComment::schema(),
                    habit::AttrHabitOccurenceParentId::schema(),
                    habit::AttrHabitOccurenceTime::schema(),
                    habit::HabitMode::schema(),
                    // Collection.
                    collection::AttrCollectionItem::schema(),
                    // Tags.
                    tags::AttrTagName::schema(),
                    tags::AttrTagParent::schema(),
                    tags::AttrTags::schema(),
                ],
                entities: vec![
                    // File
                    File::schema(),
                    Image::schema(),
                    Video::schema(),
                    // socialmedia
                    SocialMediaPost::schema(),
                    // Notes
                    notes::Note::schema(),
                    // Habits
                    habit::Habit::schema(),
                    habit::HabitOccurence::schema(),
                    // collection
                    collection::Collection::schema(),
                    // tags
                    tags::Tag::schema(),
                ],
                indexes: vec![],
            }),
        }
    }
}
