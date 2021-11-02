use factordb::{AnyError, Id, query::select::Page};
use semantic_core::base::Collection;

use crate::context;

mod collection_select;
pub use collection_select::CollectionSelect;

mod entity_collection_manager;
pub use entity_collection_manager::entity_collection_manager;

async fn search_collections(term: String) -> Result<Page<Collection>, AnyError> {
    context::api()
        .select_entities(Collection::search_collections(term, 10))
        .await
}

async fn load_entity_collections(id: Id) -> Result<Page<Collection>, AnyError> {
    context::api()
        .select_entities(Collection::query_collections_with_entity(id))
        .await
}

async fn collection_add_entity(collection: Id, entity: Id) -> Result<(), AnyError> {
    context::api()
        .mutate(Collection::mutate_add_item(collection, entity))
        .await
}

async fn collection_remove_entity(collection: Id, entity: Id) -> Result<(), AnyError> {
    context::api()
        .mutate(Collection::mutate_remove_item(collection, entity))
        .await
}
