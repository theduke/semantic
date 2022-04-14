mod file;
use crate::plugin::{Plugin, PluginDescriptor, PluginSchema};

pub use self::file::*;

mod notes;
pub use self::notes::*;

mod socialmedia;
pub use self::socialmedia::*;

mod collection;
pub use self::collection::*;

mod tags;
pub use self::tags::*;

use factordb::prelude::IdOrIdent;
use factordb::prelude::{
    AttrIdent, AttrMapExt, Attribute, AttributeDescriptor, DataMap, EntityAttribute,
    EntityDescriptor, EntitySchema, Id, Migration, Timestamp, Value, ValueType,
};
use factordb::query::migrate;

// Common default attributes.

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Title")]
pub struct AttrTitle(String);

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
                    AttrTitle::schema(),
                    AttrComment::schema(),
                    AttrDescription::schema(),
                    AttrUrl::schema(),
                    AttrPreviewImageUrl::schema(),
                    AttrPreviewImageBlobUri::schema(),
                    AttrUsername::schema(),
                    TextFormat::schema(),
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
                    // socialmedia
                    AttrSocialMediaPostContent::schema(),
                    // Notes.
                    notes::AttrNoteBody::schema(),
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
                    // collection
                    collection::Collection::schema(),
                    // tags
                    tags::Tag::schema(),
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
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrTitle::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrFileName::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrFileSize::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrMimeType::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrHash::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrOriginalHash::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrUrl::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrDownloadUrl::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrPreviewImageUrl::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                    EntityAttribute {
                        attribute: AttrBlobUri::IDENT,
                        cardinality: factordb::schema::Cardinality::Optional,
                    },
                ],
                extends: vec![],
                strict: false,
            })
            .entity_create(Image::schema())
            .entity_create(Video::schema())
            .entity_create(SocialMediaPost::schema())
            .entity_create(EntitySchema {
                id: Id::nil(),
                ident: Note::QUALIFIED_NAME.to_string(),
                title: Some("Note".to_string()),
                description: None,
                attributes: vec![
                    EntityAttribute {
                        attribute: AttrTitle::IDENT,
                        cardinality: factordb::schema::Cardinality::Required,
                    },
                    EntityAttribute {
                        attribute: AttrNoteBody::IDENT,
                        cardinality: factordb::schema::Cardinality::Required,
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
                cardinality: factordb::schema::Cardinality::Required,
                default_value: Some(TextFormat::Markdown.to_str().into()),
            }),
        );

        let create_file_blob_uri_web = Migration::with_name("create_file_blob_uri_web")
            .attr_create(AttrBlobUriWeb::schema())
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: File::IDENT.to_string(),
                    attribute: AttrBlobUriWeb::IDENT.to_string(),
                    cardinality: factordb::schema::Cardinality::Optional,
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
                    cardinality: factordb::schema::Cardinality::Optional,
                    default_value: None,
                },
            ))
            .action(migrate::SchemaAction::EntityAttributeAdd(
                migrate::EntityAttributeAdd {
                    entity: File::IDENT.to_string(),
                    attribute: AttrUpdatedAt::IDENT.to_string(),
                    cardinality: factordb::schema::Cardinality::Optional,
                    default_value: None,
                },
            ));

        let create_preview_blob_uri = Migration::with_name("create_attr_preview_blob_uri")
            .attr_create(factordb::prelude::AttributeSchema {
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
                    cardinality: factordb::schema::Cardinality::Optional,
                    default_value: None,
                },
            ));

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
        ]
    }
}
