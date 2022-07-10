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
    query::migrate,
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
                    AttrSecondaryUrl::schema(),
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

    fn migrations(&self) -> Vec<migrate::Migration> {
        let first = Migration::with_name("semantic/base/v1")
            .attr_create(AttrTitle::schema())
            .attr_create(AttrDateTime::schema())
            .attr_create(AttrDescription::schema())
            .attr_create(AttrUrl::schema())
            .attr_create(AttrPreviewImageUrl::schema())
            .attr_create(AttrUsername::schema())
            .attr_create(AttrBlobUri::schema())
            .attr_create(AttrMimeType::schema())
            .attr_create(AttrHash::schema())
            .attr_create(AttrOriginalHash::schema())
            .attr_create(AttrDuration::schema())
            .attr_create(AttrFileSize::schema())
            .attr_create(AttrDownloadUrl::schema())
            .attr_create(AttrFileName::schema())
            .attr_create(AttrSocialMediaPostContent::schema())
            .attr_create(notes::AttrNoteBody::schema())
            .attr_create(collection::AttrCollectionItem::schema())
            .attr_create(tags::AttrTagName::schema())
            .attr_create(tags::AttrTagParent::schema())
            .attr_create(tags::AttrTags::schema())
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
            .entity_create(Image::schema())
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
            .entity_create(collection::Collection::schema())
            .entity_create(tags::Tag::schema());

        let create_comment =
            Migration::with_name("create_comment_attribute").attr_create(AttrComment::schema());

        let create_note_body_format =
            Migration::with_name("create_text_format").attr_create(TextFormat::schema());

        let add_text_format_to_note = Migration::with_name("add_text_format_to_note").action(
            migrate::SchemaAction::EntityAttributeAdd(migrate::EntityAttributeAdd {
                entity: Note::IDENT.to_string(),
                attribute: TextFormat::IDENT.to_string(),
                cardinality: Cardinality::Required,
                default_value: Some(TextFormat::Markdown.to_str().into()),
            }),
        );

        let create_file_blob_uri_web = Migration::with_name("create_file_blob_uri_web")
            .attr_create(AttrBlobUriWeb::schema())
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: File::IDENT.to_string(),
                    attribute: AttrBlobUriWeb::IDENT.to_string(),
                    cardinality: Cardinality::Optional,
                    default_value: None,
                },
            ));

        let create_attr_created_updated_at = Migration::with_name("create_attr_created_updated_at")
            .attr_create(AttrCreatedAt::schema())
            .attr_create(AttrUpdatedAt::schema());

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
            Migration::with_name("create_name_attr").attr_create(AttrName::schema());

        let create_person_attrs = Migration::with_name("create_person_attrs")
            .attr_create(AttrFamilyName::schema())
            .attr_create(AttrGivenName::schema())
            .attr_create(AttrBirthDate::schema())
            .attr_create(Gender::schema());

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
                .attr_create(AttrSocialMediaPlatformName::schema())
                .attr_create(AttrSocialMediaPlatformId::schema());

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
        ]
    }
}
