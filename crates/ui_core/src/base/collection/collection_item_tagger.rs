use std::rc::Rc;

use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Render},
};
use factordb::{
    data::value::patch::{Patch, PatchOp},
    query::{
        mutate::{BatchUpdate, Mutate},
        select::Item,
    },
    schema::{AttrMapExt, AttributeDescriptor},
    AnyError,
};
use semantic_core::base::{AttrTags, Tag};

use crate::{
    base::tags::TagSelect,
    components::loader::{spinner, LoadState, Loader},
};

pub struct CollectionItemTagger {
    pub items: Vec<Item>,
    pub on_complete: Rc<dyn Fn()>,
}

impl Render for CollectionItemTagger {
    fn render(self) -> brass::dom::TagBuilder {
        State::build(self)
    }
}

enum Msg {
    Submit(Vec<Tag>),
    PersistLoaded(Result<(), AnyError>),
}

struct State {
    items: Vec<Item>,
    on_complete: Rc<dyn Fn()>,

    loader: Loader<()>,
}

impl MsgComponent for State {
    type Properties = CollectionItemTagger;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: brass::component::Context<Self>) -> Self {
        Self {
            items: props.items,
            loader: Loader::new_idle(),
            on_complete: props.on_complete,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: brass::component::Context<Self>) {
        match msg {
            Msg::Submit(tags) => {
                if tags.is_empty() {
                    (self.on_complete)();
                    return;
                }

                let patch = Patch(
                    tags.iter()
                        .map(|tag| PatchOp::add(AttrTags::QUALIFIED_NAME, tag.id))
                        .collect(),
                );

                let actions = self
                    .items
                    .iter()
                    .filter_map(|item| {
                        let id = item.data.get_id()?;

                        Some(Mutate::patch(id, patch.clone()))
                    })
                    .collect();
                let batch = BatchUpdate { actions };

                let guard = ctx.spawn_map(
                    async move {
                        crate::context::api().batch(batch).await?;
                        Ok(())
                    },
                    Msg::PersistLoaded,
                );
                self.loader.set_loading(guard);
            }
            Msg::PersistLoaded(res) => match res {
                Ok(_) => {
                    (self.on_complete)();
                }
                Err(err) => {
                    self.loader.set_err(err);
                }
            },
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> brass::dom::TagBuilder {
        // let first_tags = self
        //     .items
        //     .first()
        //     .and_then(|item| item.data.get_attr_vec::<AttrTags>());

        // let current_tag_ids = if let Some(initial_tags) = first_tags {
        //     let mut tags: HashSet<Id> = initial_tags.into_iter().collect();

        //     for item in self.items.iter().skip(1) {
        //         let item_tags = item.data.get_attr_vec::<AttrTags>().unwrap_or_default();
        //         tags.retain(|t| item_tags.contains(t));
        //     }

        //     tags.into_iter().collect::<Vec<_>>()
        // } else {
        //     Vec::new()
        // };
        let handle = ctx.handle();
        let sig = self.loader.signal_render_state(move |state| match state {
            LoadState::Idle => {
                let handle = handle.clone();
                div()
                    .and(brass::dom::Tag::P.new().class("mb-2").and("Tag all items."))
                    .and(TagSelect {
                        initial_selection: Vec::new(),
                        on_submit: Some(Rc::new(move |tags| {
                            handle.send(Msg::Submit(tags.clone()));
                        })),
                        on_change: None,
                        on_add_async: None,
                        on_remove_async: None,
                        on_change_async: None,
                    })
            }
            LoadState::Loading(_) => spinner(),
            LoadState::Success(_) => unimplemented!(),
            LoadState::Failed(_) => unimplemented!(),
        });
        div().child_signal(sig)
    }
}
