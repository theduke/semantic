mod file;
use crate::plugin::{PluginDescriptor, PluginSchema};

pub use self::file::*;

mod notes;
pub use self::notes::*;

mod socialmedia;
pub use self::socialmedia::*;

use factordb::{
    schema::{builtin::AttrIdent, AttributeDescriptor, EntityDescriptor},
    Attribute, Id,
};

// Common default attributes.

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Title")]
pub struct AttrTitle(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Url")]
pub struct AttrUrl(url::Url);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Preview")]
pub struct AttrPreviewImageUrl(url::Url);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Username")]
pub struct AttrUsername(String);

pub struct SemanticPlugin;

impl PluginDescriptor for SemanticPlugin {
    const NAME: &'static str = "semantic/base";

    fn schema() -> PluginSchema {
        PluginSchema {
            id: Id::nil(),
            name: Self::NAME.into(),
            description: None,
            db: factordb::schema::DbSchema {
                attributes: vec![
                    AttrTitle::schema(),
                    AttrUrl::schema(),
                    AttrPreviewImageUrl::schema(),
                    AttrUsername::schema(),
                    // file
                    AttrBlobUri::schema(),
                    AttrMimeType::schema(),
                    AttrDuration::schema(),
                    AttrFileSize::schema(),
                    AttrDownloadUrl::schema(),
                    AttrFileName::schema(),
                    // socialmedia
                    AttrSocialMediaPostContent::schema(),
                    // Notes.
                    notes::AttrNoteBody::schema(),
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
                ],
            },
        }
    }
}
