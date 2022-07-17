use std::rc::Rc;

use brass::dom::{Render, View};
use factordb::{prelude::DataMap, schema::AttrMapExt};
use semantic_core::base::AttrTags;

use crate::components::util::notification_error;

pub fn entity_tag_manager(entity: &DataMap) -> View {
    let id = if let Some(id) = entity.get_id() {
        id
    } else {
        return notification_error()
            .and("Entity does not have an id")
            .into();
    };

    let item_tag_ids = entity.get_attr_vec::<AttrTags>().unwrap_or_default();

    let entity_id = id;
    super::TagSelect {
        initial_selection: item_tag_ids,
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
}
