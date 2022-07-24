mod file;
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

use factordb::{
    prelude::{
        AttrIdent, AttrMapExt, Attribute, AttributeDescriptor, AttributeSchema, Cardinality,
        DataMap, EntityAttribute, EntityDescriptor, EntitySchema, Id, IdOrIdent, Migration,
        Timestamp, Value, ValueType,
    },
    query::{migrate, migrate::AttributeCreateIndex},
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
#[factor(namespace = "semantic", title = "Parent")]
pub struct AttrParentId(Id);

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
                    AttrParentId::schema(),
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
                    // socialmedia
                    AttrSocialMediaPostContent::schema(),
                    AttrSocialMediaPostUserId::schema(),
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
        let first = Migration::with_name("semantic/base/v1")
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/title".to_string(),
                title: Some("Title".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/datetime".to_string(),
                title: Some("Datetime".to_string()),
                description: None,
                value_type: ValueType::DateTime,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/description".to_string(),
                title: Some("Description".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/url".to_string(),
                title: Some("Url".to_string()),
                description: None,
                value_type: ValueType::Url,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/preview_image_url".to_string(),
                title: Some("Preview".to_string()),
                description: None,
                value_type: ValueType::Url,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/username".to_string(),
                title: Some("Username".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/blob_uri".to_string(),
                title: Some("Blob".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/mime_type".to_string(),
                title: Some("MIME Type".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/hash".to_string(),
                title: Some("Content Hash".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            // create original hash schema
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/original_hash".to_string(),
                title: Some("Original Content Hash".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            // create duration schema
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/duration".to_string(),
                title: Some("Duration".to_string()),
                description: None,
                // TODO: this should be UInt!
                value_type: ValueType::Int,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/file_size".to_string(),
                title: Some("Size".to_string()),
                description: None,
                // TODO: this should be UInt!
                value_type: ValueType::Int,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/download_url".to_string(),
                title: Some("Download URL".to_string()),
                description: None,
                value_type: ValueType::Url,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/filename".to_string(),
                title: Some("Filename".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/social_media_post_content".to_string(),
                title: Some("Content".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/note_body".to_string(),
                title: Some("Note".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/collection_items".to_string(),
                title: Some("Items".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/tag_name".to_string(),
                title: Some("Tag Name".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/tag_parent".to_string(),
                title: Some("Tag Parent".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: false,
                strict: false,
            })
            .attr_create(AttributeSchema {
                id: Id::nil(),
                ident: "semantic/tags".to_string(),
                title: Some("Tags".to_string()),
                description: None,
                value_type: ValueType::Ref,
                unique: false,
                index: false,
                strict: false,
            })
            .entity_create(EntitySchema {
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
            })
            .entity_create(EntitySchema {
                id: Id::nil(),
                ident: Image::QUALIFIED_NAME.to_string(),
                title: Some("Image".to_string()),
                description: None,
                attributes: vec![],
                extends: vec![File::IDENT],
                strict: false,
            })
            .entity_create(EntitySchema {
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
            })
            .entity_create(EntitySchema {
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
                        cardinality: Cardinality::Many,
                    },
                ],
                extends: vec![],
                strict: false,
            })
            .entity_create(EntitySchema {
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
            })
            .entity_create(EntitySchema {
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
                        cardinality: Cardinality::Many,
                    },
                ],
                extends: vec![],
                strict: false,
            })
            .entity_create(EntitySchema {
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
            });

        let create_comment =
            Migration::with_name("create_comment_attribute").attr_create(AttributeSchema {
                id: Id::nil(),
                ident: AttrComment::IDENT.to_string(),
                title: Some("Comment".to_string()),
                description: None,
                value_type: ValueType::String,
                unique: false,
                index: false,
                strict: false,
            });

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
                    cardinality: Cardinality::Many,
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

        vec![
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
        ]
    }
}
