use std::rc::Rc;

use brass::dom::TagBuilder;
use factordb::{
    query::mutate::{BatchUpdate, Mutate},
    schema::EntityContainer,
    Id,
};
use semantic_core::base::Tag;

use crate::context;

use super::tag_form::ExistingTagValidator;

pub fn tag_create(
    on_created: impl Fn(Tag) + 'static,
    validator: Rc<ExistingTagValidator>,
) -> TagBuilder {
    let tag = Tag {
        id: Id::nil(),
        name: String::new(),
        description: None,
        parent_id: None,
        extra: Default::default(),
    };

    let on_created: Rc<dyn Fn(Tag)> = Rc::new(on_created);
    super::tag_form::tag_form(
        tag,
        move |tag| {
            let on_created = on_created.clone();

            Box::pin(async move {
                let mut tag = tag.clone();
                tag.id = Id::random();
                context::api()
                    .batch(BatchUpdate::with_action(Mutate::create(
                        tag.id,
                        tag.clone().into_map()?,
                    )))
                    .await?;

                on_created(tag);

                Ok(())
            })
        },
        validator,
    )
}
