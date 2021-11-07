use serde::{Deserialize, Serialize};

use factordb::{
    data::{value::patch::Patch, DataMap},
    query::{
        expr::Expr,
        mutate::Mutate,
        select::{Item, Select},
    },
    schema::{
        builtin::{self, AttrId, AttrIdent},
        AttrMapExt, AttributeDescriptor, EntityDescriptor,
    },
    Attribute, Entity, Id,
};

use super::{AttrDescription, AttrTitle, AttrUrl};

#[derive(Attribute)]
#[factor(namespace = "semantic", title = "Items", name = "collection_items")]
pub struct AttrCollectionItem(Id);

#[derive(Serialize, Deserialize, Entity, Clone, Debug, PartialEq, Eq)]
#[factor(namespace = "semantic")]
pub struct Collection {
    #[factor(attr = AttrId)]
    #[serde(rename = "factor/id")]
    pub id: Id,

    #[factor(attr = AttrIdent)]
    #[serde(rename = "factor/ident")]
    pub ident: Option<String>,

    #[factor(attr = AttrUrl)]
    #[serde(rename = "semantic/url")]
    pub url: Option<url::Url>,

    #[factor(attr = AttrTitle)]
    #[serde(rename = "semantic/title")]
    pub title: String,

    #[factor(attr = AttrDescription)]
    #[serde(rename = "semantic/description")]
    pub description: Option<String>,

    #[factor(attr = AttrCollectionItem)]
    #[serde(rename = "semantic/collection_items")]
    pub item_ids: Vec<Id>,

    #[factor(ignore)]
    #[serde(flatten)]
    pub extra: DataMap,
}

impl Collection {
    pub const ITEMS_JOIN: &'static str = "items";

    /// Build a select query that returns all collections that contain the given
    /// entity.
    pub fn query_collections_with_entity(id: Id) -> Select {
        let expr = Expr::in_(id, Expr::attr::<AttrCollectionItem>());
        Select::new().with_filter(expr)
    }

    pub fn query_collection_items(col: &Self) -> Select {
        Select::new()
            .with_limit(1000)
            .with_filter(Expr::in_(AttrId::expr(), col.item_ids.clone()))
    }

    pub fn query_all_collections() -> Select {
        let expr = Expr::eq(builtin::AttrType::expr(), Collection::QUALIFIED_NAME);
        Select::new().with_filter(expr).with_limit(1000)
    }

    pub fn search_collections(term: String, limit: u64) -> Select {
        let expr = Expr::eq(builtin::AttrType::expr(), Collection::QUALIFIED_NAME)
            .and_with(Expr::contains(AttrTitle::expr(), term));
        Select::new().with_filter(expr).with_limit(limit)
    }

    /// Build a mutation for adding an item to a colleciton.
    pub fn mutate_add_item(collection: Id, entity: Id) -> Mutate {
        Mutate::patch(
            collection,
            Patch::new().add(AttrCollectionItem::QUALIFIED_NAME, entity),
        )
    }

    pub fn mutate_remove_item(collection: Id, entity: Id) -> Mutate {
        Mutate::patch(
            collection,
            Patch::new().remove_with_old(AttrCollectionItem::QUALIFIED_NAME, entity),
        )
    }
}

/// A [Collection] with it's entities loaded.
#[derive(Clone)]
pub struct CollectionWithItems {
    pub collection: Collection,
    pub items: Vec<Item>,
}

impl CollectionWithItems {
    pub fn new() -> Self {
        Self {
            collection: Collection {
                id: Id::random(),
                ident: None,
                url: None,
                title: String::new(),
                description: None,
                item_ids: Vec::new(),
                extra: Default::default(),
            },
            items: Vec::new(),
        }
    }

    pub fn from_query_result(
        collection: Collection,
        entities: Vec<factordb::query::select::Item>,
    ) -> Self {
        let sorted = collection
            .item_ids
            .iter()
            .filter_map(|id| {
                entities
                    .iter()
                    .find(|item| {
                        item.data
                            .get_id()
                            .map(|item_id| id == &item_id)
                            .unwrap_or_default()
                    })
                    .cloned()
            })
            .collect();

        Self {
            collection,
            items: sorted,
        }
    }

    /// Build a query that retrieves the entities in a collection.
    pub fn build_query(collection: &Collection) -> Select {
        let expr = Expr::in_(
            Expr::attr::<builtin::AttrId>(),
            Expr::literal(collection.item_ids.clone()),
        );
        Select::new().with_filter(expr)
    }
}
