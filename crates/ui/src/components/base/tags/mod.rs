mod tag_form;
pub use tag_form::tag_form;

mod entity_tag_manager;
pub use entity_tag_manager::EntityTagManager;

mod tag_manager;
mod tag_selector;
pub use tag_selector::tag_select;

use std::rc::Rc;

use factordb::{
    data::value::from_value_map,
    query::{mutate::Mutate, select::Item},
    schema::EntityContainer,
    Id,
};

use brass::{
    vdom::{self, Render},
    Callback, VNode,
};

use semantic_core::base::Tag;
use semantic_ui_core::entity::persister::{EntityFormProps, EntityPersister, FormValid};
pub use tag_manager::TagManager;

use self::tag_form::ExistingTagValidator;

pub fn tag_create(
    on_complete: Callback<Tag>,
    on_cancel: Option<Callback<()>>,
    existing_validator: Option<Rc<ExistingTagValidator>>,
) -> VNode {
    EntityPersister {
        item: None,
        renderer: Rc::new(move |props: &EntityFormProps| {
            let tag = Tag {
                id: Id::nil(),
                name: String::new(),
                description: None,
                parent_id: None,
                extra: Default::default(),
            };
            let on_submit = props.on_submit.clone().map(|mut tag: Tag| {
                tag.id = Id::random();
                let map = tag.clone().into_map().unwrap();
                FormValid {
                    item: Item::new(map.clone()),
                    mutation: Mutate::create(tag.id, map).into(),
                }
            });
            let form = tag_form(tag, on_submit, existing_validator.clone());
            // let submit = brass_bulma::button().and_class("is-primary").on_click(EventCallback::closure(|_| (), props.sub))

            let error = props
                .error
                .as_ref()
                .map(|err| brass_bulma::notification_error(err));

            vdom::div().and((form, error)).build()
        }),
        on_complete: on_complete.map(|item: Item| -> Tag { from_value_map(item.data).unwrap() }),
        on_cancel,
    }
    .render()
}


