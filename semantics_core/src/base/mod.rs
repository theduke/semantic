mod file;
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
#[factor(namespace = "semantic")]
pub struct AttrTitle(String);

#[derive(Attribute)]
#[factor(namespace = "semantic")]
pub struct AttrUrl(url::Url);

#[derive(Attribute)]
#[factor(namespace = "semantic")]
pub struct AttrPreviewImageUrl(url::Url);

#[derive(Attribute)]
#[factor(namespace = "semantic")]
pub struct AttrUsername(String);

pub struct SemanticPlugin;

impl crate::PluginDescriptor for SemanticPlugin {
    const NAME: &'static str = "semantics/base";

    fn schema() -> crate::PluginSchema {
        crate::PluginSchema {
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
