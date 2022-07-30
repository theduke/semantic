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

use factordb::{
    prelude::{
        AttrIdent, AttrMapExt, Attribute, AttributeDescriptor, AttributeSchema, Cardinality,
        DataMap, EntityAttribute, EntityDescriptor, EntitySchema, Id, IdOrIdent, Migration,
        Timestamp, Value, ValueType,
    },
    query::{
        migrate,
        migrate::{
            AttributeCreateIndex, EntityAttributeAdd, EntityAttributeChangeCardinality,
            EntityAttributeRemove, SchemaAction,
        },
    },
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
#[factor(namespace = "semantic", title = "Url")]
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

impl AttributeDescriptor for TextFormat {
    const NAMESPACE: &'static str = "semantic";
    const PLAIN_NAME: &'static str = "text_format";
    const QUALIFIED_NAME: &'static str = "semantic/text_format";
    const IDENT: IdOrIdent = IdOrIdent::new_static(Self::QUALIFIED_NAME);
    type Type = TextFormat;

    fn schema() -> factordb::schema::AttributeSchema {
        factordb::schema::AttributeSchema {
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
            db: Some(factordb::schema::DbSchema {
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
                entities: vec![
                    // File
                    File::schema(),
                    Image::schema(),
                    Video::schema(),
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
                ],
                indexes: vec![],
            }),
        }
    }

    fn migrations(&self, _already_applied_migrations: &HashSet<String>) -> Vec<migrate::Migration> {
        let create_title =
            AttributeSchema::new(AttrTitle::QUALIFIED_NAME, ValueType::String).with_title("Title");
        let create_datetime =
            AttributeSchema::new(AttrDateTime::QUALIFIED_NAME, ValueType::DateTime)
                .with_title("Datetime");
        let create_comment = AttributeSchema::new(AttrComment::QUALIFIED_NAME, ValueType::String)
            .with_title("Comment");
        let create_description =
            AttributeSchema::new(AttrDescription::QUALIFIED_NAME, ValueType::String)
                .with_title("Description");
        let create_url =
            AttributeSchema::new(AttrUrl::QUALIFIED_NAME, ValueType::Url).with_title("Url");
        let create_preview_image_url =
            AttributeSchema::new(AttrPreviewImageUrl::QUALIFIED_NAME, ValueType::Url)
                .with_title("Preview");
        let create_username = AttributeSchema::new(AttrUsername::QUALIFIED_NAME, ValueType::String)
            .with_title("Username");
        let create_blob_uri =
            AttributeSchema::new(AttrBlobUri::QUALIFIED_NAME, ValueType::String).with_title("Blob");
        let create_mime_type =
            AttributeSchema::new(AttrMimeType::QUALIFIED_NAME, ValueType::String)
                .with_title("MIME Type");
        let create_hash = AttributeSchema::new(AttrHash::QUALIFIED_NAME, ValueType::String)
            .with_title("Content Hash");
        let create_original_hash =
            AttributeSchema::new(AttrOriginalHash::QUALIFIED_NAME, ValueType::String)
                .with_title("Original Content Hash");
        let create_duration = AttributeSchema::new(AttrDuration::QUALIFIED_NAME, ValueType::Int)
            .with_title("Duration");
        let create_file_size =
            AttributeSchema::new(AttrFileSize::QUALIFIED_NAME, ValueType::Int).with_title("Size");
        let create_download_url =
            AttributeSchema::new(AttrDownloadUrl::QUALIFIED_NAME, ValueType::Url)
                .with_title("Download URL");
        let create_filename = AttributeSchema::new(AttrFileName::QUALIFIED_NAME, ValueType::String)
            .with_title("Filename");

        let create_social_media_post_content =
            AttributeSchema::new(AttrSocialMediaPostContent::QUALIFIED_NAME, ValueType::Ref)
                .with_title("Content");
        let create_note_body =
            AttributeSchema::new(AttrNoteBody::QUALIFIED_NAME, ValueType::String)
                .with_title("Note");
        let create_collection_items =
            AttributeSchema::new(AttrCollectionItem::QUALIFIED_NAME, ValueType::Ref)
                .with_title("Items");

        let create_tag_name = AttributeSchema::new(AttrTagName::QUALIFIED_NAME, ValueType::String)
            .with_title("Tag Name");
        let create_tag_parent = AttributeSchema::new(AttrTagParent::QUALIFIED_NAME, ValueType::Ref)
            .with_title("Tag Parent");
        let create_tags = AttributeSchema::new(
            AttrTags::QUALIFIED_NAME,
            ValueType::new_list(ValueType::Ref),
        )
        .with_title("Tags");

        let create_file = EntitySchema {
            id: Id::nil(),
            ident: File::QUALIFIED_NAME.to_string(),
            title: Some("File".to_string()),
            description: None,
            attributes: vec![
                EntityAttribute {
                    attribute: AttrIdent::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrTitle::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrFileName::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrFileSize::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrMimeType::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrHash::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrOriginalHash::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrUrl::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrDownloadUrl::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrPreviewImageUrl::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrBlobUri::IDENT,
                    cardinality: Cardinality::Optional,
                },
            ],
            extends: vec![],
            strict: false,
        };

        let create_image = EntitySchema {
            id: Id::nil(),
            ident: Image::QUALIFIED_NAME.to_string(),
            title: Some("Image".to_string()),
            description: None,
            attributes: vec![],
            extends: vec![File::IDENT],
            strict: false,
        };

        let create_video = EntitySchema {
            id: Id::nil(),
            ident: Video::IDENT.to_string(),
            title: Some("Video".to_string()),
            description: None,
            attributes: vec![EntityAttribute {
                attribute: AttrDuration::IDENT,
                cardinality: factordb::prelude::Cardinality::Optional,
            }],
            extends: vec![File::IDENT.into()],
            strict: false,
        };

        let create_social_media_post = EntitySchema {
            id: Id::nil(),
            ident: SocialMediaPost::IDENT.to_string(),
            title: Some("SocialMediaPost".to_string()),
            description: None,
            attributes: vec![
                EntityAttribute {
                    attribute: AttrIdent::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrTitle::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrUrl::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrUsername::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrSocialMediaPostContent::IDENT,
                    cardinality: Cardinality::Optional,
                },
            ],
            extends: vec![],
            strict: false,
        };

        let create_note = EntitySchema {
            id: Id::nil(),
            ident: Note::QUALIFIED_NAME.to_string(),
            title: Some("Note".to_string()),
            description: None,
            attributes: vec![
                EntityAttribute {
                    attribute: AttrTitle::IDENT,
                    cardinality: Cardinality::Required,
                },
                EntityAttribute {
                    attribute: AttrNoteBody::IDENT,
                    cardinality: Cardinality::Required,
                },
            ],
            extends: vec![],
            strict: false,
        };

        let create_collection = EntitySchema {
            id: Id::nil(),
            ident: Collection::QUALIFIED_NAME.to_string(),
            title: Some("Collection".to_string()),
            description: None,
            attributes: vec![
                EntityAttribute {
                    attribute: AttrIdent::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrUrl::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrTitle::IDENT,
                    cardinality: Cardinality::Required,
                },
                EntityAttribute {
                    attribute: AttrDescription::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrCollectionItem::IDENT,
                    cardinality: Cardinality::Required,
                },
            ],
            extends: vec![],
            strict: false,
        };

        let create_tag = EntitySchema {
            id: Id::nil(),
            ident: Tag::QUALIFIED_NAME.to_string(),
            title: Some("Tag".to_string()),
            description: None,
            attributes: vec![
                EntityAttribute {
                    attribute: AttrTagName::IDENT,
                    cardinality: Cardinality::Required,
                },
                EntityAttribute {
                    attribute: AttrDescription::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrTagParent::IDENT,
                    cardinality: Cardinality::Optional,
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

        let create_note_body_format = Migration::with_name("create_text_format").attr_create(
            factordb::schema::AttributeSchema {
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
            },
        );

        let add_text_format_to_note = Migration::with_name("add_text_format_to_note").action(
            migrate::SchemaAction::EntityAttributeAdd(migrate::EntityAttributeAdd {
                entity: Note::IDENT.to_string(),
                attribute: TextFormat::IDENT.to_string(),
                cardinality: Cardinality::Required,
                default_value: Some(TextFormat::Markdown.to_str().into()),
            }),
        );

        let create_file_blob_uri_web = Migration::with_name("create_file_blob_uri_web")
            .attr_create(AttributeSchema {
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
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: AttrCreatedAt::IDENT.to_string(),
                title: Some("Created at".to_string()),
                description: None,
                value_type: ValueType::DateTime,
                unique: false,
                index: true,
                strict: false,
            })
            .attr_create(AttributeSchema {
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
            .attr_create(AttributeSchema {
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
            .attr_create(AttributeSchema {
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

        let create_parent_id =
            Migration::with_name("create_parent_id").attr_create(AttributeSchema {
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
            Migration::with_name("create_social_media_post_user_id").attr_create(AttributeSchema {
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

        let create_attr_name =
            Migration::with_name("create_name_attr").attr_create(AttributeSchema {
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
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/family_name".to_string(),
                title: Some("Family name".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/given_name".to_string(),
                title: Some("Given name".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/birthdate".to_string(),
                title: Some("Date of birth".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
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

        let create_person = Migration::with_name("create_person").entity_create(EntitySchema {
            id: Id::nil(),
            ident: "semantic/Person".to_string(),
            title: Some("Person".to_string()),
            description: None,
            attributes: vec![
                EntityAttribute {
                    attribute: AttrIdent::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrDescription::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrUrl::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrName::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrFamilyName::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrGivenName::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: Gender::IDENT,
                    cardinality: Cardinality::Optional,
                },
            ],
            extends: vec![],
            strict: false,
        });

        let create_social_media_platform_attrs =
            Migration::with_name("create_social_media_platform_attrs")
                .attr_create(AttributeSchema {
                    id: Id::nil(),
                    ident: "semantic/social_media_platform_name".to_string(),
                    title: Some("Social Media Platform".to_string()),
                    description: None,
                    value_type: ValueType::String,
                    unique: false,
                    index: false,
                    strict: false,
                })
                .attr_create(AttributeSchema {
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
            .entity_create(EntitySchema {
                id: Id::nil(),
                ident: "semantic/SocialMediaAccount".to_string(),
                title: Some("Social Media Account".to_string()),
                description: None,
                attributes: vec![
                    EntityAttribute {
                        attribute: AttrUsername::IDENT,
                        cardinality: Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrSocialMediaPlatformName::IDENT,
                        cardinality: Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrSocialMediaPlatformId::IDENT,
                        cardinality: Cardinality::Optional,
                    },
                ],
                extends: vec![Person::IDENT],
                strict: false,
            });

        let create_visit_attrs = Migration::with_name("create_visit_attrs")
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: AttrVisitCount::QUALIFIED_NAME.to_string(),
                title: Some("Visit count".to_string()),
                description: None,
                value_type: ValueType::UInt,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
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
            .attr_create(AttributeSchema {
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
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: AttrParent::QUALIFIED_NAME.to_string(),
                title: Some("Parent".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: true,
                strict: false,
            })
            .attr_create(AttributeSchema {
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
                //         new_cardinality: Cardinality::Optional,
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

        let mut rollup = factordb::query::migrate::unify_migrations(batch1.clone()).unwrap();
        rollup.name = Some("batch1_rollup".to_string());

        let create_like_count = Migration::with_name("create_like_count").attr_create(
            AttributeSchema::new(AttrLikeCount::QUALIFIED_NAME, ValueType::UInt)
                .with_title("Likes"),
        );

        let create_container_and_parent_sort =
            Migration::with_name("create_container_and_parent_sort")
                .attr_create(AttributeSchema::new(
                    AttrParentSortOrder::QUALIFIED_NAME,
                    ValueType::Int,
                ))
                .entity_create(EntitySchema {
                    id: Id::nil(),
                    ident: Container::QUALIFIED_NAME.to_string(),
                    title: Some("Container".to_string()),
                    description: None,
                    attributes: vec![EntityAttribute {
                        attribute: AttrTitle::IDENT,
                        cardinality: Cardinality::Optional,
                    }],
                    extends: vec![],
                    strict: false,
                });

        let create_bookmark = Migration::with_name("create_bookmark").entity_create(EntitySchema {
            id: Id::nil(),
            ident: Bookmark::QUALIFIED_NAME.to_string(),
            title: Some("Bookmark".to_string()),
            description: None,
            attributes: vec![
                EntityAttribute {
                    attribute: AttrUrl::IDENT,
                    cardinality: Cardinality::Required,
                },
                EntityAttribute {
                    attribute: AttrTitle::IDENT,
                    cardinality: Cardinality::Optional,
                },
                EntityAttribute {
                    attribute: AttrCreatedAt::IDENT,
                    cardinality: Cardinality::Optional,
                },
            ],
            extends: vec![],
            strict: false,
        });

        let add_pixel_width_height = Migration::with_name("add_pixel_width_height")
            .attr_create(AttributeSchema::new(
                AttrPixelWidth::QUALIFIED_NAME,
                ValueType::UInt,
            ))
            .attr_create(AttributeSchema::new(
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
            .attr_create(AttributeSchema::new(
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

        vec![
            rollup,
            create_like_count,
            create_container_and_parent_sort,
            create_bookmark,
            add_pixel_width_height,
            create_imported_at,
        ]
    }
}
