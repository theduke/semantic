use std::rc::Rc;

use brass::{vdom::Render, VNode};
use factordb::{
    query::{
        mutate::{BatchUpdate, Mutate},
        select::Item,
    },
    schema::EntityContainer,
};
use semantic_ui_core::{
    entity::{
        create_page::CreatePage,
        persister::{EntityFormProps, FormValid},
    },
    EntityRenderOpts,
};
use semantic_core::base::{Collection, CollectionWithItems};

use super::collection_form::CollectionForm;

pub fn collection_create(_item: &Item, _opts: &EntityRenderOpts) -> VNode {
    let item = brass::Shared::new(CollectionWithItems::new());
    CreatePage {
        render: Rc::new(move |props: &EntityFormProps| {
            CollectionForm {
                item: item.clone(),
                is_loading: props.is_loading,
                error: props.error.clone(),
                auto_edit_metadata: true,
                on_submit: props.on_submit.clone().map(|data: Collection| {
                    let id = data.id;
                    let item = Item::new(data.into_map().unwrap());

                    FormValid {
                        mutation: BatchUpdate::with_action(Mutate::create(id, item.data.clone())),
                        item,
                    }
                }),
            }
            .render()
        }),
    }
    .render()
}
