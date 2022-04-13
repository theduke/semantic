use std::rc::Rc;

use brass::dom::{Render, TagBuilder};
use factordb::prelude::Id;

use crate::components::loader::load;

pub fn entity_collection_manager(id: Id) -> TagBuilder {
    load(super::load_entity_collections(id), move |page| {
        let entity_id = id;
        super::CollectionSelect {
            initial_selection: page.items.clone(),
            on_add_async: Some(Rc::new(move |collection| {
                Box::pin(async move {
                    super::collection_add_entity(collection.id, entity_id).await?;
                    Ok(collection)
                })
            })),
            on_remove_async: Some(Rc::new(move |collection| {
                Box::pin(async move {
                    super::collection_remove_entity(collection.id, entity_id).await?;
                    Ok(collection)
                })
            })),
        }
        .render()
    })
}
