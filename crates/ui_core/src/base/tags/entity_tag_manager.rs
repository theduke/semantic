use std::rc::Rc;

use brass::dom::{Render, TagBuilder};
use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_core::base::Tag;

use crate::components::{loader::load, util::notification_error};

pub fn entity_tag_manager(entity: &Item) -> TagBuilder {
    let id = if let Some(id) = entity.data.get_id() {
        id
    } else {
        return notification_error().and("Entity does not have an id");
    };

    load(super::load_entity_tags(id), move |tags: &Vec<Tag>| {
        let entity_id = id;
        super::TagSelect {
            initial_selection: tags.clone(),
            on_submit: None,
            on_change: None,
            on_change_async: None,
            on_add_async: Some(Rc::new(move |tag| {
                Box::pin(async move {
                    super::entity_add_tag(entity_id, tag.id).await?;
                    Ok(tag)
                })
            })),
            on_remove_async: Some(Rc::new(move |tag| {
                Box::pin(async move {
                    super::entity_remove_tag(entity_id, tag.id).await?;
                    Ok(tag)
                })
            })),
        }
        .render()
    })
}
