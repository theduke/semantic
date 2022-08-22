use std::{collections::HashMap, rc::Rc};

use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Render, TagBuilder},
    signal::signal::Mutable,
};
use factdb::{ClassContainer, Id};
use semantic_core::base::Tag;

use crate::components::{
    entity::entity_deleter::EntityDeleter,
    loader::load,
    util::{box_, button, modal::modal, notification_warning, subtitle_4, title_2, ButtonBuilder},
};

use super::{tag_form::ExistingTagValidator, TagMerger};

struct TagNode {
    tag: Tag,
    children: Vec<TagNode>,
}

impl TagNode {
    fn build_tree(items: Vec<Tag>) -> Vec<TagNode> {
        // Mapping from parent tag id to child tags.
        let mut child_map = HashMap::<Id, Vec<Tag>>::new();
        let mut roots = Vec::new();
        for tag in items {
            if let Some(parent_id) = tag.parent_id.clone() {
                child_map.entry(parent_id).or_default().push(tag);
            } else {
                roots.push(tag);
            }
        }

        roots
            .into_iter()
            .map(|t| Self::build(t, &mut child_map))
            .collect()
    }

    fn build(tag: Tag, child_map: &mut HashMap<Id, Vec<Tag>>) -> Self {
        let children: Vec<_> = child_map
            .remove(&tag.id)
            .unwrap_or_default()
            .into_iter()
            .map(|child| Self::build(child, child_map))
            .collect();

        let mut s = Self { tag, children };
        s.sort_children();
        s
    }

    fn render_level(items: &[TagNode], depth: usize, on_delete: Rc<dyn Fn(Id)>) -> TagBuilder {
        div().and_iter(
            items
                .iter()
                .map(|node| node.render(depth, on_delete.clone())),
        )
    }

    fn render(&self, depth: usize, on_delete: Rc<dyn Fn(Id)>) -> TagBuilder {
        let deleting = Mutable::new(false);

        let children = Self::render_level(&self.children, depth + 1, on_delete.clone());

        let tag = self.tag.clone();
        let deleter = {
            let on_delete = on_delete.clone();
            deleting.clone().signal_ref(move |flag| {
                if *flag {
                    let item = tag.clone().into_map().unwrap();

                    let deleting = deleting.clone();
                    let on_delete = on_delete.clone();
                    let id = tag.id;
                    let d = EntityDeleter {
                        item,
                        on_delete: Box::new(move || {
                            on_delete(id);
                        }),
                        on_cancel: Box::new(move || {
                            deleting.set(false);
                        }),
                    }
                    .render();

                    div().class("mt-2").class("mb-2").and(d)
                } else {
                    let deleting = deleting.clone();
                    ButtonBuilder::new()
                        .color(crate::components::util::Color::Danger)
                        .icon("fas fa-trash")
                        .size_small()
                        .on(move || {
                            deleting.set(true);
                        })
                        .build()
                        .class("ml-2")
                }
            })
        };

        // TODO: use unified modal for deleter and merger.

        let merging = Mutable::new(false);
        let tag = self.tag.clone();
        let merger = merging.clone().signal_ref(move |flag| {
            if *flag {
                let merging = merging.clone();
                let id = tag.id;
                let on_delete = on_delete.clone();
                let content = TagMerger {
                    source_tag: tag.clone(),
                    on_cancel: {
                        let merging = merging.clone();
                        Rc::new(move || {
                            merging.set(false);
                        })
                    },
                    on_merged: Rc::new(move || {
                        on_delete(id);
                    }),
                };

                modal(
                    content,
                    move || {
                        merging.set(false);
                    },
                    true,
                )
            } else {
                let merging = merging.clone();
                ButtonBuilder::new()
                    .label("Merge")
                    .size_small()
                    .on(move || {
                        merging.set(true);
                    })
                    .build()
            }
        });

        let tag = button().and(&self.tag.name);
        let row = div().and(tag).signal(deleter).signal(merger);

        div().class("mb-2").and(row).and(children)
    }

    fn sort_children(&mut self) {
        self.children.sort_by(|a, b| a.tag.name.cmp(&b.tag.name));
    }
}

enum Msg {
    TagCreated(Tag),
    TagDeleted(Id),
}

struct State {
    validator: Rc<ExistingTagValidator>,
    tree: Mutable<Vec<TagNode>>,
}

pub struct TagManager {
    items: Vec<Tag>,
}

impl MsgComponent for State {
    type Properties = TagManager;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: brass::component::Context<Self>) -> Self {
        let val = ExistingTagValidator::from_tags(props.items.iter().map(|x| &x.name));
        let mut tree = TagNode::build_tree(props.items);
        tree.sort_by(|a, b| a.tag.name.cmp(&b.tag.name));

        Self {
            tree: Mutable::new(tree),
            validator: Rc::new(val),
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: brass::component::Context<Self>) {
        match msg {
            Msg::TagCreated(tag) => {
                let mut lock = self.tree.lock_mut();
                lock.push(TagNode {
                    tag,
                    children: Vec::new(),
                });
                lock.sort_by(|a, b| a.tag.name.cmp(&b.tag.name));
            }
            Msg::TagDeleted(id) => {
                let mut lock = self.tree.lock_mut();
                lock.retain(|x| x.tag.id != id);
            }
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> TagBuilder {
        let on_created = ctx.on(Msg::TagCreated);
        let form = super::tag_create(on_created, self.validator.clone());
        let form_wrap = box_().and((subtitle_4().and("New Tag"), form));

        let on_delete = Rc::new(ctx.on(Msg::TagDeleted));

        div()
            .and(title_2().and("Tags"))
            .and(form_wrap)
            .signal(self.tree.signal_ref(move |tree| {
                if tree.is_empty() {
                    notification_warning().and("No tags found.")
                } else {
                    TagNode::render_level(tree, 0, on_delete.clone())
                }
            }))
    }
}

pub fn tag_manager() -> TagBuilder {
    let f = async move {
        crate::context::api()
            .select_entities::<Tag>(Tag::query_all())
            .await
    };
    load(f, |items| {
        State::build(TagManager {
            items: items.clone(),
        })
    })
}
