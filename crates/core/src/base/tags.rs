use serde::{Deserialize, Serialize};

use factordb::{
    data::DataMap,
    query::{expr::Expr, mutate::Mutate, select::Select},
    schema::{builtin::AttrType, AttributeDescriptor, EntityDescriptor},
    Attribute, Entity, Id,
};

use super::AttrDescription;

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Tag Name", name = "tag_name")]
pub struct AttrTagName(String);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Tag Parent", name = "tag_parent")]
pub struct AttrTagParent(Id);

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Tags", name = "tags")]
pub struct AttrTags(Id);

#[derive(Serialize, Deserialize, Entity, Clone, Debug, PartialEq, Eq)]
#[factor(namespace = "semantic")]
pub struct Tag {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrTagName)]
    #[serde(rename = "semantic/tag_name")]
    pub name: String,

    #[factor(attr = AttrDescription)]
    #[serde(rename = "semantic/description")]
    pub description: Option<String>,

    #[factor(attr = AttrTagParent)]
    #[serde(rename = "semantic/tag_parent")]
    pub parent_id: Option<Id>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}

impl Tag {
    pub fn query_all() -> Select {
        let filter = Expr::eq(Expr::attr::<AttrType>(), Tag::QUALIFIED_NAME);
        Select::new().with_filter(filter).with_limit(10_000)
    }

    /// Build a select query returning all tags for a given entity.
    pub fn query_entities_with_tag(tag_id: Id) -> Select {
        let filter = Expr::in_(tag_id, Expr::attr::<AttrTags>());
        Select::new().with_filter(filter)
    }

    /// Build a [`Mutate`] that adds a tag to an entity.
    pub fn mutate_add_tag(entity_id: Id, tag_id: Id) -> Mutate {
        Mutate::merge(
            entity_id,
            DataMap::new().with_insert(AttrTags::QUALIFIED_NAME, vec![tag_id]),
        )
    }

    /// Build a [`Mutate`] that removes a tag from an entity.
    pub fn mutate_remove_tag(entity_id: Id, current_tags: &[Id], tag_id: Id) -> Mutate {
        // FIXME: use a Patch to remove the specific item id instead of
        // overwriting. Needs Patch support implemented in factordb.

        let mut tag_ids = current_tags.to_vec();
        tag_ids.retain(|item_id| item_id != &tag_id);
        let mut map = DataMap::new();
        map.insert(AttrTags::QUALIFIED_NAME.into(), tag_ids.into());
        Mutate::merge(entity_id, map)
    }
}
