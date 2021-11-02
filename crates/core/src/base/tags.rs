use serde::{Deserialize, Serialize};

use factordb::{Attribute, Entity, Id, data::{DataMap, value::patch::Patch}, query::{expr::Expr, mutate::Mutate, select::Select}, schema::{builtin::AttrType, AttributeDescriptor, EntityDescriptor}};

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

    /// Build an expression that selects entities with the given tag
    pub fn filter_entity_has_tag(tag_id: Id) -> Expr {
        Expr::in_(tag_id, Expr::attr::<AttrTags>())
    }

    /// Build an expression that selects entities with the given tag
    pub fn filter_entity_has_any_tag(tag_ids: Vec<Id>) -> Expr {
        Expr::contains(tag_ids, AttrTags::expr())
    }

    /// Build a select query returning all tags for a given entity.
    pub fn query_entities_with_tag(tag_id: Id) -> Select {
        Select::new().with_filter(Self::filter_entity_has_tag(tag_id))
    }

    /// Build a [`Mutate`] that adds a tag to an entity.
    pub fn mutate_add_tag(entity_id: Id, tag_id: Id) -> Mutate {
        Mutate::patch(
            entity_id,
            Patch::new().add(AttrTags::QUALIFIED_NAME, tag_id)
        )
    }

    /// Build a [`Mutate`] that removes a tag from an entity.
    pub fn mutate_remove_tag(entity_id: Id, tag_id: Id) -> Mutate {
        Mutate::patch(
            entity_id,
            Patch::new().remove_with_old(AttrTags::QUALIFIED_NAME, tag_id)
        )
    }


    /// Build a [`Mutate`] that removes a tag from an entity.
    pub fn mutate_set_tags(entity_id: Id, tag_ids: Vec<Id>) -> Mutate {
        let mut map = DataMap::new();
        map.insert(AttrTags::QUALIFIED_NAME.into(), tag_ids.into());
        Mutate::merge(entity_id, map)
    }
}
