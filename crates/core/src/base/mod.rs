mod file;
use std::collections::HashSet;

mod container;
pub use self::container::*;

use crate::plugin::{Plugin, PluginDescriptor, PluginSchema};

pub use self::file::*;

mod notes;
pub use self::notes::*;

mod socialmedia;
pub use self::socialmedia::*;

mod collection;
pub use self::collection::*;

mod person;
pub use self::person::*;

mod tags;
pub use self::tags::*;

mod bookmark;
pub use self::bookmark::*;

mod code_snippet;
pub use self::code_snippet::*;

mod listing;
pub use self::listing::*;

use factdb::{
    macros::Attribute,
    query::migrate::{
        self, AttributeCreateIndex, EntityAttributeAdd, EntityAttributeChangeCardinality,
        EntityAttributeRemove, SchemaAction,
    },
    AttrId, AttrIdent, AttrMapExt, Attribute, AttributeMeta, Cardinality, Class, ClassAttribute,
    ClassMeta, DataMap, Expr, Id, IdOrIdent, Migration, Timestamp, Value, ValueType,
};

mod common_actions;
pub use self::common_actions::RecordEntityVisit;

// Common default attributes.

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Title")]
pub struct AttrTitle(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Name")]
pub struct AttrName(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Comment")]
pub struct AttrComment(String);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Created at",
    name = "created_at",
    index
)]
pub struct AttrCreatedAt(Timestamp);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Updated at",
    name = "updated_at",
    index
)]
pub struct AttrUpdatedAt(Timestamp);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Parent", name = "parent", index)]
pub struct AttrParent(Id);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Children", name = "children")]
pub struct AttrChildren(Vec<Id>);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Parent Sort Order",
    name = "parent_sort_order"
)]
pub struct AttrParentSortOrder(i64);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Embedded in parent",
    name = "embedded_in_parent"
)]
pub struct AttrEmbeddedInParent(bool);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Visit count",
    name = "visit_count",
    index
)]
pub struct AttrVisitCount(u64);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Last visited",
    name = "last_visit_time",
    index
)]
pub struct AttrLastVisitTime(Timestamp);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Import Url",
    name = "import_url",
    index
)]
pub struct AttrImportUrl(url::Url);

pub fn entity_title(data: &DataMap) -> String {
    data.get_attr::<AttrTitle>()
        .or_else(|| data.get_id().map(|x| x.to_string()))
        .and_then(|x| if x.trim().is_empty() { None } else { Some(x) })
        .unwrap_or_else(|| {
            if let Some(id) = data.get_id() {
                id.to_string()
            } else {
                "<No Title>".to_string()
            }
        })
}

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Description")]
pub struct AttrDescription(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Url", index)]
pub struct AttrUrl(url::Url);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Secondary Url",
    name = "secondary_url"
)]
pub struct AttrSecondaryUrl(url::Url);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Preview")]
pub struct AttrPreviewImageUrl(url::Url);

#[derive(Attribute)]
#[factor(
    namespace = "semantic",
    title = "Preview Image (Blob)",
    name = "preview_image_blob_uri"
)]
pub struct AttrPreviewImageBlobUri(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Datetime", name = "datetime")]
pub struct AttrDateTime(Timestamp);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Username")]
pub struct AttrUsername(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", name = "imported_at", title = "Imported at")]
pub struct AttrImportedAt(Timestamp);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Content", name = "text_content")]
pub struct AttrTextContent(String);

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub enum TextFormat {
    #[serde(rename = "plain")]
    Plain,
    #[serde(rename = "markdown")]
    Markdown,
}

impl TextFormat {
    fn to_str(&self) -> &'static str {
        match self {
            TextFormat::Plain => "plain",
            TextFormat::Markdown => "markdown",
        }
    }
}

impl AttributeMeta for TextFormat {
    const NAMESPACE: &'static str = "semantic";
    const PLAIN_NAME: &'static str = "text_format";
    const QUALIFIED_NAME: &'static str = "semantic/text_format";
    const IDENT: IdOrIdent = IdOrIdent::new_static(Self::QUALIFIED_NAME);
    type Type = TextFormat;

    fn schema() -> factdb::schema::Attribute {
        factdb::schema::Attribute {
            id: Id::nil(),
            ident: Self::QUALIFIED_NAME.to_string(),
            title: Some("Text Format".to_string()),
            description: None,
            value_type: ValueType::Union(vec![
                ValueType::Const(Value::String("plain".to_string())),
                ValueType::Const(Value::String("markdown".to_string())),
            ]),
            unique: false,
            index: false,
            strict: false,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

pub struct SemanticBasePlugin;

impl PluginDescriptor for SemanticBasePlugin {
    const NAME: &'static str = "semantic/base";
    const IDENT: IdOrIdent = IdOrIdent::new_static(Self::NAME);

    fn new() -> crate::plugin::DynPlugin {
        std::sync::Arc::new(Self)
    }
}

impl Plugin for SemanticBasePlugin {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn schema(&self) -> PluginSchema {
        PluginSchema {
            name: Self::NAME.into(),
            description: None,
            import_matchers: Vec::new(),
            db: Some(factdb::schema::DbSchema {
                attributes: vec![
                    AttrTitle::schema(),
                    AttrComment::schema(),
                    AttrDescription::schema(),
                    AttrUrl::schema(),
                    AttrPreviewImageUrl::schema(),
                    AttrPreviewImageBlobUri::schema(),
                    AttrUsername::schema(),
                    TextFormat::schema(),
                    AttrCreatedAt::schema(),
                    AttrUpdatedAt::schema(),
                    AttrVisitCount::schema(),
                    AttrLastVisitTime::schema(),
                    AttrName::schema(),
                    AttrSecondaryUrl::schema(),
                    AttrParent::schema(),
                    AttrEmbeddedInParent::schema(),
                    AttrImportedAt::schema(),
                    // file
                    AttrBlobUri::schema(),
                    AttrBlobUriWeb::schema(),
                    AttrMimeType::schema(),
                    AttrHash::schema(),
                    AttrOriginalHash::schema(),
                    AttrDuration::schema(),
                    AttrFileSize::schema(),
                    AttrDownloadUrl::schema(),
                    AttrFileName::schema(),
                    AttrVideoHasSound::schema(),
                    AttrPixelWidth::schema(),
                    AttrPixelHeight::schema(),
                    AttrVisualHash::schema(),
                    // socialmedia
                    AttrSocialMediaPostContent::schema(),
                    AttrSocialMediaPostUserId::schema(),
                    AttrSocialMediaPlatformName::schema(),
                    AttrSocialMediaPlatformId::schema(),
                    // Notes.
                    notes::AttrNoteBody::schema(),
                    // Collection.
                    collection::AttrCollectionItem::schema(),
                    // Tags.
                    tags::AttrTagName::schema(),
                    tags::AttrTagParent::schema(),
                    tags::AttrTags::schema(),
                    // Person
                    person::AttrGivenName::schema(),
                    person::AttrFamilyName::schema(),
                    person::AttrBirthDate::schema(),
                    person::Gender::schema(),
                    AttrLikeCount::schema(),
                ],
                classes: vec![
                    // File
                    File::schema(),
                    Image::schema(),
                    Video::schema(),
                    Audio::schema(),
                    // socialmedia
                    SocialMediaPost::schema(),
                    SocialMediaAccount::schema(),
                    // Notes
                    notes::Note::schema(),
                    // collection
                    collection::Collection::schema(),
                    // tags
                    tags::Tag::schema(),
                    // person
                    person::Person::schema(),
                    // Bookmarks.
                    Bookmark::schema(),
                ],
                indexes: vec![],
            }),
        }
    }

    fn migrations(&self, _already_applied_migrations: &HashSet<String>) -> Vec<migrate::Migration> {
        let create_title =
            Attribute::new(AttrTitle::QUALIFIED_NAME, ValueType::String).with_title("Title");
        let create_datetime = Attribute::new(AttrDateTime::QUALIFIED_NAME, ValueType::DateTime)
            .with_title("Datetime");
        let create_comment =
            Attribute::new(AttrComment::QUALIFIED_NAME, ValueType::String).with_title("Comment");
        let create_description = Attribute::new(AttrDescription::QUALIFIED_NAME, ValueType::String)
            .with_title("Description");
        let create_url = Attribute::new(AttrUrl::QUALIFIED_NAME, ValueType::Url).with_title("Url");
        let create_preview_image_url =
            Attribute::new(AttrPreviewImageUrl::QUALIFIED_NAME, ValueType::Url)
                .with_title("Preview");
        let create_username =
            Attribute::new(AttrUsername::QUALIFIED_NAME, ValueType::String).with_title("Username");
        let create_blob_uri =
            Attribute::new(AttrBlobUri::QUALIFIED_NAME, ValueType::String).with_title("Blob");
        let create_mime_type =
            Attribute::new(AttrMimeType::QUALIFIED_NAME, ValueType::String).with_title("MIME Type");
        let create_hash =
            Attribute::new(AttrHash::QUALIFIED_NAME, ValueType::String).with_title("Content Hash");
        let create_original_hash =
            Attribute::new(AttrOriginalHash::QUALIFIED_NAME, ValueType::String)
                .with_title("Original Content Hash");
        let create_duration =
            Attribute::new(AttrDuration::QUALIFIED_NAME, ValueType::Int).with_title("Duration");
        let create_file_size =
            Attribute::new(AttrFileSize::QUALIFIED_NAME, ValueType::Int).with_title("Size");
        let create_download_url = Attribute::new(AttrDownloadUrl::QUALIFIED_NAME, ValueType::Url)
            .with_title("Download URL");
        let create_filename =
            Attribute::new(AttrFileName::QUALIFIED_NAME, ValueType::String).with_title("Filename");

        let create_social_media_post_content =
            Attribute::new(AttrSocialMediaPostContent::QUALIFIED_NAME, ValueType::Ref)
                .with_title("Content");
        let create_note_body =
            Attribute::new(AttrNoteBody::QUALIFIED_NAME, ValueType::String).with_title("Note");
        let create_collection_items =
            Attribute::new(AttrCollectionItem::QUALIFIED_NAME, ValueType::Ref).with_title("Items");

        let create_tag_name =
            Attribute::new(AttrTagName::QUALIFIED_NAME, ValueType::String).with_title("Tag Name");
        let create_tag_parent =
            Attribute::new(AttrTagParent::QUALIFIED_NAME, ValueType::Ref).with_title("Tag Parent");
        let create_tags = Attribute::new(
            AttrTags::QUALIFIED_NAME,
            ValueType::new_list(ValueType::Ref),
        )
        .with_title("Tags");

        let create_file = Class {
            id: Id::nil(),
            ident: File::QUALIFIED_NAME.to_string(),
            title: Some("File".to_string()),
            description: None,
            attributes: vec![
                ClassAttribute {
                    attribute: AttrIdent::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrTitle::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrFileName::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrFileSize::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrMimeType::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrHash::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrOriginalHash::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrUrl::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrDownloadUrl::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrPreviewImageUrl::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrBlobUri::QUALIFIED_NAME.to_string(),
                    required: false,
                },
            ],
            extends: vec![],
            strict: false,
        };

        let create_image = Class {
            id: Id::nil(),
            ident: Image::QUALIFIED_NAME.to_string(),
            title: Some("Image".to_string()),
            description: None,
            attributes: vec![],
            extends: vec![File::QUALIFIED_NAME.to_string()],
            strict: false,
        };

        let create_video = Class {
            id: Id::nil(),
            ident: Video::IDENT.to_string(),
            title: Some("Video".to_string()),
            description: None,
            attributes: vec![ClassAttribute {
                attribute: AttrDuration::QUALIFIED_NAME.to_string(),
                required: false,
            }],
            extends: vec![File::QUALIFIED_NAME.into()],
            strict: false,
        };

        let create_social_media_post = Class {
            id: Id::nil(),
            ident: SocialMediaPost::IDENT.to_string(),
            title: Some("SocialMediaPost".to_string()),
            description: None,
            attributes: vec![
                ClassAttribute {
                    attribute: AttrIdent::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrTitle::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrUrl::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrUsername::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrSocialMediaPostContent::QUALIFIED_NAME.to_string(),
                    required: false,
                },
            ],
            extends: vec![],
            strict: false,
        };

        let create_note = Class {
            id: Id::nil(),
            ident: Note::QUALIFIED_NAME.to_string(),
            title: Some("Note".to_string()),
            description: None,
            attributes: vec![
                ClassAttribute {
                    attribute: AttrTitle::QUALIFIED_NAME.to_string(),
                    required: true,
                },
                ClassAttribute {
                    attribute: AttrNoteBody::QUALIFIED_NAME.to_string(),
                    required: true,
                },
            ],
            extends: vec![],
            strict: false,
        };

        let create_collection = Class {
            id: Id::nil(),
            ident: Collection::QUALIFIED_NAME.to_string(),
            title: Some("Collection".to_string()),
            description: None,
            attributes: vec![
                ClassAttribute {
                    attribute: AttrIdent::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrUrl::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrTitle::QUALIFIED_NAME.to_string(),
                    required: true,
                },
                ClassAttribute {
                    attribute: AttrDescription::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrCollectionItem::QUALIFIED_NAME.to_string(),
                    required: true,
                },
            ],
            extends: vec![],
            strict: false,
        };

        let create_tag = Class {
            id: Id::nil(),
            ident: Tag::QUALIFIED_NAME.to_string(),
            title: Some("Tag".to_string()),
            description: None,
            attributes: vec![
                ClassAttribute {
                    attribute: AttrTagName::QUALIFIED_NAME.to_string(),
                    required: true,
                },
                ClassAttribute {
                    attribute: AttrDescription::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrTagParent::QUALIFIED_NAME.to_string(),
                    required: false,
                },
            ],
            extends: vec![],
            strict: false,
        };

        let first = Migration::with_name("semantic/base/v1")
            .attr_create(create_title)
            .attr_create(create_datetime)
            .attr_create(create_description)
            .attr_create(create_url)
            .attr_create(create_preview_image_url)
            .attr_create(create_username)
            .attr_create(create_blob_uri)
            .attr_create(create_mime_type)
            .attr_create(create_hash)
            .attr_create(create_original_hash)
            .attr_create(create_duration)
            .attr_create(create_file_size)
            .attr_create(create_download_url)
            .attr_create(create_filename)
            .attr_create(create_social_media_post_content)
            .attr_create(create_note_body)
            .attr_create(create_collection_items)
            .attr_create(create_tag_name)
            .attr_create(create_tag_parent)
            .attr_create(create_tags)
            .entity_create(create_file)
            .entity_create(create_image)
            .entity_create(create_video)
            .entity_create(create_social_media_post)
            .entity_create(create_note)
            .entity_create(create_collection)
            .entity_create(create_tag);

        let create_comment =
            Migration::with_name("create_comment_attribute").attr_create(create_comment);

        let create_note_body_format =
            Migration::with_name("create_text_format").attr_create(factdb::schema::Attribute {
                id: Id::nil(),
                ident: TextFormat::QUALIFIED_NAME.to_string(),
                title: Some("Text Format".to_string()),
                description: None,
                value_type: ValueType::Union(vec![
                    ValueType::Const(Value::String("plain".to_string())),
                    ValueType::Const(Value::String("markdown".to_string())),
                ]),
                unique: false,
                index: false,
                strict: false,
            });

        let add_text_format_to_note = Migration::with_name("add_text_format_to_note").action(
            migrate::SchemaAction::EntityAttributeAdd(migrate::EntityAttributeAdd {
                entity: Note::IDENT.to_string(),
                attribute: TextFormat::IDENT.to_string(),
                cardinality: Cardinality::Required,
                default_value: Some(TextFormat::Markdown.to_str().into()),
            }),
        );

        let create_file_blob_uri_web = Migration::with_name("create_file_blob_uri_web")
            .attr_create(Attribute {
                id: Id::nil(),
                ident: AttrBlobUriWeb::IDENT.to_string(),
                title: Some("Blob".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: File::IDENT.to_string(),
                    attribute: AttrBlobUriWeb::IDENT.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                },
            ));

        let create_attr_created_updated_at = Migration::with_name("create_attr_created_updated_at")
            .attr_create(Attribute {
                id: Id::nil(),
                ident: AttrCreatedAt::IDENT.to_string(),
                title: Some("Created at".to_string()),
                description: None,
                value_type: ValueType::DateTime,
                unique: false,
                index: true,
                strict: false,
            })
            .attr_create(Attribute {
                id: Id::nil(),
                ident: AttrUpdatedAt::IDENT.to_string(),
                title: Some("Updated at".to_string()),
                description: None,
                value_type: ValueType::DateTime,
                unique: false,
                index: true,
                strict: false,
            });

        let add_created_updated_at_to_file = Migration::with_name("add_created_updated_at_to_file")
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: File::IDENT.to_string(),
                    attribute: AttrCreatedAt::IDENT.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                },
            ))
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: File::IDENT.to_string(),
                    attribute: AttrUpdatedAt::IDENT.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                },
            ));

        let create_preview_blob_uri = Migration::with_name("create_attr_preview_blob_uri")
            .attr_create(Attribute {
                id: Id::nil(),
                ident: "semantic/preview_image_blob_uri".to_string(),
                title: Some("Preview Image (Blob)".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: true,
            });

        let add_preview_blob_uri_to_file = Migration::with_name("add_preview_blob_uri_to_file")
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: File::IDENT.to_string(),
                    attribute: AttrPreviewImageBlobUri::IDENT.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                },
            ));

        let create_attr_video_has_sound = Migration::with_name("create_attr_video_has_sound")
            .attr_create(Attribute {
                id: Id::nil(),
                ident: "semantic/video_has_sound".to_string(),
                title: Some("Sound available".to_string()),
                description: None,
                value_type: ValueType::Bool,
                unique: false,
                index: false,
                strict: true,
            });

        let add_video_has_audio_to_video = Migration::with_name("add_video_has_sound_to_video")
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: Video::IDENT.to_string(),
                    attribute: AttrVideoHasSound::IDENT.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                },
            ));

        let create_parent_id = Migration::with_name("create_parent_id").attr_create(Attribute {
            id: Id::nil(),
            ident: "semantic/parent_id".to_string(),
            title: Some("Parent ID".to_string()),
            description: None,
            value_type: ValueType::Ref,
            unique: false,
            index: false,
            strict: true,
        });

        let create_social_media_post_user_id =
            Migration::with_name("create_social_media_post_user_id").attr_create(Attribute {
                id: Id::nil(),
                ident: "semantic/social_media_post_user_id".to_string(),
                title: Some("Social Media Post User ID".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: false,
                strict: true,
            });

        let add_social_media_post_user_id_to_social_media_post =
            Migration::with_name("add_social_media_post_user_id_to_social_media_post").action(
                migrate::SchemaAction::EntityAttributeAdd(migrate::EntityAttributeAdd {
                    entity: SocialMediaPost::IDENT.to_string(),
                    attribute: AttrSocialMediaPostUserId::IDENT.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                }),
            );

        let create_attr_name = Migration::with_name("create_name_attr").attr_create(Attribute {
            id: Id::nil(),
            ident: "semantic/name".to_string(),
            title: Some("Name".to_string()),
            description: None,
            value_type: ValueType::String,
            unique: false,
            index: false,
            strict: false,
        });

        let create_person_attrs = Migration::with_name("create_person_attrs")
            .attr_create(Attribute {
                id: Id::nil(),
                ident: "semantic/family_name".to_string(),
                title: Some("Family name".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(Attribute {
                id: Id::nil(),
                ident: "semantic/given_name".to_string(),
                title: Some("Given name".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(Attribute {
                id: Id::nil(),
                ident: "semantic/birthdate".to_string(),
                title: Some("Date of birth".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(Attribute {
                id: Id::nil(),
                ident: Gender::QUALIFIED_NAME.to_string(),
                title: Some("Gender".to_string()),
                description: None,
                value_type: ValueType::Union(vec![
                    ValueType::Const("male".into()),
                    ValueType::Const("female".into()),
                ]),
                unique: false,
                index: false,
                strict: false,
            });

        let create_person = Migration::with_name("create_person").entity_create(Class {
            id: Id::nil(),
            ident: "semantic/Person".to_string(),
            title: Some("Person".to_string()),
            description: None,
            attributes: vec![
                ClassAttribute {
                    attribute: AttrIdent::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrDescription::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrUrl::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrName::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrFamilyName::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrGivenName::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: Gender::QUALIFIED_NAME.to_string(),
                    required: false,
                },
            ],
            extends: vec![],
            strict: false,
        });

        let create_social_media_platform_attrs =
            Migration::with_name("create_social_media_platform_attrs")
                .attr_create(Attribute {
                    id: Id::nil(),
                    ident: "semantic/social_media_platform_name".to_string(),
                    title: Some("Social Media Platform".to_string()),
                    description: None,
                    value_type: ValueType::String,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .attr_create(Attribute {
                    id: Id::nil(),
                    ident: "semantic/social_media_platform_id".to_string(),
                    title: Some("Social Media Platform".to_string()),
                    description: None,
                    value_type: ValueType::Ref,
                    unique: false,
                    index: false,
                    strict: false,
                });

        let create_social_media_account = Migration::with_name("create_social_media_account")
            .entity_create(Class {
                id: Id::nil(),
                ident: "semantic/SocialMediaAccount".to_string(),
                title: Some("Social Media Account".to_string()),
                description: None,
                attributes: vec![
                    ClassAttribute {
                        attribute: AttrUsername::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                    ClassAttribute {
                        attribute: AttrSocialMediaPlatformName::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                    ClassAttribute {
                        attribute: AttrSocialMediaPlatformId::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                ],
                extends: vec![Person::QUALIFIED_NAME.to_string()],
                strict: false,
            });

        let create_visit_attrs = Migration::with_name("create_visit_attrs")
            .attr_create(Attribute {
                id: Id::nil(),
                ident: AttrVisitCount::QUALIFIED_NAME.to_string(),
                title: Some("Visit count".to_string()),
                description: None,
                value_type: ValueType::UInt,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(Attribute {
                id: Id::nil(),
                ident: AttrLastVisitTime::QUALIFIED_NAME.to_string(),
                title: Some("Last visit".to_string()),
                description: None,
                value_type: ValueType::DateTime,
                unique: false,
                index: false,
                strict: false,
            });

        let create_attr_secondary_url = Migration::with_name("create_attr_secondary_url")
            .attr_create(Attribute {
                id: Id::nil(),
                ident: AttrSecondaryUrl::QUALIFIED_NAME.to_string(),
                title: Some("Secondary url".to_string()),
                description: None,
                value_type: ValueType::Url,
                unique: false,
                index: true,
                strict: false,
            });

        let change_attr_hash_to_indexed = Migration::with_name("change_attr_hash_to_indexed")
            .action(migrate::SchemaAction::AttributeCreateIndex(
                AttributeCreateIndex {
                    attribute: AttrHash::QUALIFIED_NAME.to_string(),
                    unique: false,
                },
            ));

        let create_parent_attrs = Migration::with_name("create_parent_attrs")
            .attr_create(Attribute {
                id: Id::nil(),
                ident: AttrParent::QUALIFIED_NAME.to_string(),
                title: Some("Parent".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: true,
                strict: false,
            })
            .attr_create(Attribute {
                id: Id::nil(),
                ident: AttrEmbeddedInParent::QUALIFIED_NAME.to_string(),
                title: Some("Embedded in parent".to_string()),
                description: None,
                value_type: ValueType::Bool,
                unique: false,
                index: false,
                strict: false,
            });

        let change_cardinality_many_attrs_to_vec =
            Migration::with_name("change_cardinality_many_attrs_to_vec")
                .attr_change_type(
                    AttrCollectionItem::QUALIFIED_NAME,
                    ValueType::List(Box::new(ValueType::Ref)),
                )
                .attr_change_type(
                    AttrSocialMediaPostContent::QUALIFIED_NAME,
                    ValueType::List(Box::new(ValueType::Ref)),
                );

        let change_cardinality_many_attrs_to_optional =
            Migration::with_name("change_cardinality_many_attrs_to_optional")
                .action(migrate::SchemaAction::EntityAttributeChangeCardinality(
                    EntityAttributeChangeCardinality {
                        entity_type: SocialMediaPost::QUALIFIED_NAME.to_string(),
                        attribute: AttrSocialMediaPostContent::QUALIFIED_NAME.to_string(),
                        new_cardinality: Cardinality::Optional,
                    },
                ))
                // .action(migrate::SchemaAction::EntityAttributeChangeCardinality(
                //     EntityAttributeChangeCardinality {
                //         entity_type: SocialMediaPost::QUALIFIED_NAME.to_string(),
                //         attribute: AttrSocialMediaPostUserId::QUALIFIED_NAME.to_string(),
                //         new_required: false,
                //     },
                // ))
                .action(migrate::SchemaAction::EntityAttributeChangeCardinality(
                    EntityAttributeChangeCardinality {
                        entity_type: Collection::QUALIFIED_NAME.to_string(),
                        attribute: AttrCollectionItem::QUALIFIED_NAME.to_string(),
                        new_cardinality: Cardinality::Required,
                    },
                ));

        let remove_user_id_from_social_media_post = Migration::with_name(
            "remove_user_id_from_social_media_post",
        )
        .action(SchemaAction::EntityAttributeRemove(EntityAttributeRemove {
            entity_type: SocialMediaPost::QUALIFIED_NAME.to_string(),
            attribute: AttrSocialMediaPostUserId::QUALIFIED_NAME.to_string(),
            delete_values: true,
        }));

        let batch1 = vec![
            first,
            create_comment,
            create_note_body_format,
            add_text_format_to_note,
            create_file_blob_uri_web,
            create_attr_created_updated_at,
            add_created_updated_at_to_file,
            create_preview_blob_uri,
            add_preview_blob_uri_to_file,
            // video_has_sound
            create_attr_video_has_sound,
            add_video_has_audio_to_video,
            create_parent_id,
            create_social_media_post_user_id,
            add_social_media_post_user_id_to_social_media_post,
            create_attr_name,
            create_person_attrs,
            create_person,
            create_social_media_platform_attrs,
            create_social_media_account,
            create_visit_attrs,
            create_attr_secondary_url,
            change_attr_hash_to_indexed,
            create_parent_attrs,
            change_cardinality_many_attrs_to_vec,
            change_cardinality_many_attrs_to_optional,
            remove_user_id_from_social_media_post,
        ];

        let mut rollup = factdb::query::migrate::unify_migrations(batch1.clone()).unwrap();
        rollup.name = Some("batch1_rollup".to_string());

        let create_like_count = Migration::with_name("create_like_count").attr_create(
            Attribute::new(AttrLikeCount::QUALIFIED_NAME, ValueType::UInt).with_title("Likes"),
        );

        let create_container_and_parent_sort =
            Migration::with_name("create_container_and_parent_sort")
                .attr_create(Attribute::new(
                    AttrParentSortOrder::QUALIFIED_NAME,
                    ValueType::Int,
                ))
                .entity_create(Class {
                    id: Id::nil(),
                    ident: Container::QUALIFIED_NAME.to_string(),
                    title: Some("Container".to_string()),
                    description: None,
                    attributes: vec![ClassAttribute {
                        attribute: AttrTitle::QUALIFIED_NAME.to_string(),
                        required: false,
                    }],
                    extends: vec![],
                    strict: false,
                });

        let create_bookmark = Migration::with_name("create_bookmark").entity_create(Class {
            id: Id::nil(),
            ident: Bookmark::QUALIFIED_NAME.to_string(),
            title: Some("Bookmark".to_string()),
            description: None,
            attributes: vec![
                ClassAttribute {
                    attribute: AttrUrl::QUALIFIED_NAME.to_string(),
                    required: true,
                },
                ClassAttribute {
                    attribute: AttrTitle::QUALIFIED_NAME.to_string(),
                    required: false,
                },
                ClassAttribute {
                    attribute: AttrCreatedAt::QUALIFIED_NAME.to_string(),
                    required: false,
                },
            ],
            extends: vec![],
            strict: false,
        });

        let add_pixel_width_height = Migration::with_name("add_pixel_width_height")
            .attr_create(Attribute::new(
                AttrPixelWidth::QUALIFIED_NAME,
                ValueType::UInt,
            ))
            .attr_create(Attribute::new(
                AttrPixelHeight::QUALIFIED_NAME,
                ValueType::UInt,
            ))
            .action(
                EntityAttributeAdd {
                    entity: Image::QUALIFIED_NAME.to_string(),
                    attribute: AttrPixelWidth::QUALIFIED_NAME.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                }
                .into(),
            )
            .action(
                EntityAttributeAdd {
                    entity: Image::QUALIFIED_NAME.to_string(),
                    attribute: AttrPixelHeight::QUALIFIED_NAME.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                }
                .into(),
            )
            .action(
                EntityAttributeAdd {
                    entity: Video::QUALIFIED_NAME.to_string(),
                    attribute: AttrPixelWidth::QUALIFIED_NAME.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                }
                .into(),
            )
            .action(
                EntityAttributeAdd {
                    entity: Video::QUALIFIED_NAME.to_string(),
                    attribute: AttrPixelHeight::QUALIFIED_NAME.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                }
                .into(),
            );

        let create_imported_at = Migration::with_name("create_imported_at")
            .attr_create(Attribute::new(
                AttrImportedAt::QUALIFIED_NAME,
                ValueType::DateTime,
            ))
            .action(
                EntityAttributeAdd {
                    entity: File::QUALIFIED_NAME.to_string(),
                    attribute: AttrImportedAt::QUALIFIED_NAME.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                }
                .into(),
            )
            .action(
                EntityAttributeAdd {
                    entity: SocialMediaPost::QUALIFIED_NAME.to_string(),
                    attribute: AttrImportedAt::QUALIFIED_NAME.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                }
                .into(),
            )
            .action(
                EntityAttributeAdd {
                    entity: SocialMediaAccount::QUALIFIED_NAME.to_string(),
                    attribute: AttrImportedAt::QUALIFIED_NAME.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                }
                .into(),
            );

        let make_url_indexed = Migration::with_name("make_url_indexed").action(
            SchemaAction::AttributeCreateIndex(AttributeCreateIndex {
                attribute: AttrUrl::QUALIFIED_NAME.to_string(),
                unique: false,
            }),
        );

        let add_imported_at_and_description_to_bookmark =
            Migration::with_name("add_imported_at_and_description_to_bookmark")
                .action(
                    EntityAttributeAdd {
                        entity: Bookmark::QUALIFIED_NAME.to_string(),
                        attribute: AttrDescription::QUALIFIED_NAME.to_string(),
                        cardinality: Cardinality::Optional,
                        default_value: None,
                    }
                    .into(),
                )
                .action(
                    EntityAttributeAdd {
                        entity: Bookmark::QUALIFIED_NAME.to_string(),
                        attribute: AttrImportedAt::QUALIFIED_NAME.to_string(),
                        cardinality: Cardinality::Optional,
                        default_value: None,
                    }
                    .into(),
                );

        let create_audio = Migration::with_name("create_audio_entity").entity_create(Class {
            id: Id::nil(),
            ident: Audio::QUALIFIED_NAME.to_string(),
            title: Some("Audio".to_string()),
            description: None,
            attributes: vec![ClassAttribute {
                attribute: AttrDuration::QUALIFIED_NAME.to_string(),
                required: false,
            }],
            extends: vec![File::QUALIFIED_NAME.to_string()],
            strict: false,
        });

        let create_attr_text_content = Migration::with_name("create_attr_text_content")
            .attr_create(Attribute::new(
                AttrTextContent::QUALIFIED_NAME,
                ValueType::String,
            ));

        let create_code_snippet = Migration::with_name("create_code_snippet")
            .attr_create(Attribute::new(
                AttrCodeSnippetLanguage::QUALIFIED_NAME,
                ValueType::String,
            ))
            .entity_create(Class {
                id: Id::nil(),
                ident: CodeSnippet::QUALIFIED_NAME.to_string(),
                title: Some("Snippet".to_string()),
                description: None,
                attributes: vec![
                    ClassAttribute {
                        attribute: AttrTextContent::QUALIFIED_NAME.to_string(),
                        required: true,
                    },
                    ClassAttribute {
                        attribute: AttrCodeSnippetLanguage::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                    ClassAttribute {
                        attribute: AttrUrl::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                    ClassAttribute {
                        attribute: AttrTitle::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                    ClassAttribute {
                        attribute: AttrDescription::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                    ClassAttribute {
                        attribute: AttrCreatedAt::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                    ClassAttribute {
                        attribute: AttrUpdatedAt::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                    ClassAttribute {
                        attribute: AttrImportedAt::QUALIFIED_NAME.to_string(),
                        required: false,
                    },
                ],
                extends: vec![],
                strict: false,
            });

        let create_visual_hash = Migration::with_name("create_visual_hash")
            .attr_create(Attribute::new(
                AttrVisualHash::QUALIFIED_NAME,
                ValueType::Bytes,
            ))
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: Image::IDENT.to_string(),
                    attribute: AttrVisualHash::IDENT.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                },
            ));

        let add_description_to_socialmediaaccount =
            Migration::with_name("add_description_to_socialmediaaccount").action(
                SchemaAction::EntityAttributeAdd(migrate::EntityAttributeAdd {
                    entity: SocialMediaAccount::IDENT.to_string(),
                    attribute: AttrDescription::IDENT.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                }),
            );

        vec![
            rollup,
            create_like_count,
            create_container_and_parent_sort,
            create_bookmark,
            add_pixel_width_height,
            create_imported_at,
            make_url_indexed,
            add_imported_at_and_description_to_bookmark,
            create_audio,
            create_attr_text_content,
            create_code_snippet,
            create_visual_hash,
            add_description_to_socialmediaaccount,
        ]
    }
}

pub fn expr_find_by_id_ident_or_title(ident: &str) -> Expr {
    if let Ok(id) = ident.parse::<Id>() {
        Expr::eq(AttrId::expr(), id)
    } else {
        Expr::eq(AttrIdent::expr(), ident).or_with(Expr::contains(AttrTitle::expr(), ident.trim()))
    }
}
