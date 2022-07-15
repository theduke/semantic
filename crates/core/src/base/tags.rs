use serde::{Deserialize, Serialize};

use factordb::{
    prelude::{
        AttrType, Attribute, AttributeDescriptor, Batch, DataMap, Db, Entity, EntityContainer,
        EntityDescriptor, Expr, Id, IdOrIdent, Mutate, Patch, Select, Value,
    },
    query::{
        self,
        mutate::{MutateSelect, MutateSelectAction},
    },
    AnyError,
};

use super::{AttrDescription, AttrTitle};

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

    pub async fn search_by_title(db: &Db, title: &str) -> Result<Option<Tag>, AnyError> {
        let filter = Expr::is_entity::<Tag>()
            .and_with(Expr::eq(AttrTitle::expr(), Expr::literal(title.trim())));
        let page = db.select(Select::new().with_filter(filter)).await?;

        if let Some(item) = page.items.into_iter().next() {
            let tag = Tag::try_from_map(item.data)?;
            Ok(Some(tag))
        } else {
            Ok(None)
        }
    }

    pub async fn search_by_ident(db: &Db, ident: &IdOrIdent) -> Result<Tag, AnyError> {
        match ident {
            IdOrIdent::Id(id) => {
                let map = db.entity(*id).await?;
                let tag = Tag::try_from_map(map)?;
                Ok(tag)
            }
            IdOrIdent::Name(name) => {
                // Find by title.
                let tag = Self::search_by_title(db, &name)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Tag not found: '{}'", name))?;
                Ok(tag)
            }
        }
    }

    pub async fn merge_tags(
        db: &Db,
        source_tag: Tag,
        target_tag: Tag,
    ) -> Result<(), anyhow::Error> {
        let mutate_replace_tag_id = MutateSelect {
            filter: Tag::filter_entity_has_tag(source_tag.id),
            variables: Default::default(),
            action: MutateSelectAction::Patch(
                Patch::new()
                    .remove_with_old(AttrTags::QUALIFIED_NAME, Value::Id(source_tag.id))
                    .add(AttrTags::QUALIFIED_NAME, Value::Id(target_tag.id)),
            ),
        };
        let mutate_delete_tag = query::mutate::Delete { id: source_tag.id };
        let batch = Batch::new()
            .and_select(mutate_replace_tag_id)
            .and_delete(mutate_delete_tag);

        db.batch(batch).await?;

        Ok(())
    }

    /// Build an expression that selects entities with the given tag
    pub fn filter_entity_has_tag(tag_id: Id) -> Expr {
        Expr::eq(Expr::attr::<AttrTags>(), Value::Id(tag_id))
            .or_with(Expr::in_(Value::Id(tag_id), Expr::attr::<AttrTags>()))
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
            Patch::new().add(AttrTags::QUALIFIED_NAME, tag_id),
        )
    }

    /// Build a [`Mutate`] that removes a tag from an entity.
    pub fn mutate_remove_tag(entity_id: Id, tag_id: Id) -> Mutate {
        Mutate::patch(
            entity_id,
            Patch::new().remove_with_old(AttrTags::QUALIFIED_NAME, tag_id),
        )
    }

    /// Build a [`Mutate`] that removes a tag from an entity.
    pub fn mutate_set_tags(entity_id: Id, tag_ids: Vec<Id>) -> Mutate {
        let mut map = DataMap::new();
        map.insert(AttrTags::QUALIFIED_NAME.into(), tag_ids.into());
        Mutate::merge(entity_id, map)
    }
}
