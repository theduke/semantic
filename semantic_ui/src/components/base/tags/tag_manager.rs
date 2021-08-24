use std::{collections::HashMap, rc::Rc};

use brass::{vdom, VNode};
use factordb::{AnyError, Id};
use semantic_ui_core::{components::small_title, loader::LoadState};
use semantic_core::base::Tag;

use super::tag_form::ExistingTagValidator;

pub struct TagManager {}

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
        let children = child_map
            .remove(&tag.id)
            .unwrap_or_default()
            .into_iter()
            .map(|child| Self::build(child, child_map))
            .collect();
        Self { tag, children }
    }

    fn render_level(items: &[TagNode], depth: usize) -> VNode {
        vdom::div()
            .and_iter(items.iter().map(|node| node.render(depth)))
            .build()
    }

    fn render(&self, depth: usize) -> VNode {
        let tag = brass_bulma::button().and(&self.tag.name);
        let children = Self::render_level(&self.children, depth + 1);
        vdom::div().class("mb-2").and((tag, children)).build()
    }
}

enum Msg {
    TagsLoaded(Result<(Vec<TagNode>, ExistingTagValidator), AnyError>),
    Created(Tag),
}

struct State {
    loader: LoadState<Vec<TagNode>>,
    existing_tag_validator: Rc<ExistingTagValidator>,
}

brass::enable_props!(TagManager => State);

impl brass::Component for State {
    type Properties = TagManager;
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let guard = ctx.run_map(
            async {
                let page = crate::api()
                    .select(Tag::query_all())
                    .await?
                    .convert_data::<Tag>()?;
                let existing_names = page.items.iter().map(|t| t.name.clone()).collect();
                let existing_validator = ExistingTagValidator {
                    tags: existing_names,
                };

                let nodes = TagNode::build_tree(page.items);
                Ok((nodes, existing_validator))
            },
            Msg::TagsLoaded,
        );

        Self {
            loader: LoadState::Loading(Some(guard)),
            existing_tag_validator: Rc::new(ExistingTagValidator {
                tags: Default::default(),
            }),
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::TagsLoaded(res) => match res {
                Ok((tree, val)) => {
                    self.loader.set_success(tree);
                    self.existing_tag_validator = Rc::new(val);
                }
                Err(err) => {
                    self.loader.set_failed(err);
                }
            },
            Msg::Created(tag) => {
                // TODO: handle create with parent.
                self.loader.as_success_mut().map(|roots| {
                    roots.push(TagNode {
                        tag,
                        children: Vec::new(),
                    })
                });
            }
        }
    }

    fn render(&self, mut ctx: brass::RenderContext<Self>) -> brass::VNode {
        let form = super::tag_create(
            ctx.callback_map(Msg::Created),
            None,
            Some(self.existing_tag_validator.clone()),
        );
        let form_wrap = brass_bulma::box_().and((small_title("New Tag"), form));

        let tree = self.loader.render(|tree| {
            if tree.is_empty() {
                brass_bulma::notification_warning("No tags found.").build()
            } else {
                vdom::div().and(TagNode::render_level(&tree, 0)).build()
            }
        });

        vdom::div()
            .and((brass_bulma::h2_with("Tags"), form_wrap, tree))
            .build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        true
    }
}
