use serde::{Deserialize, Serialize};

use factordb::{
    query::{
        expr::Expr,
        select::{Item, Select},
    },
    schema::{builtin::AttrIdent, AttrMapExt},
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
            Expr::attr::<factordb::schema::builtin::AttrId>(),
            Expr::literal(collection.item_ids.clone()),
        );
        Select::new().with_filter(expr)
    }
}
