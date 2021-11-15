use std::{collections::HashMap, rc::Rc};

use brass::{
    dom::{builder::div, TagBuilder},
    signal::signal::Mutable,
};
use factordb::Id;
use semantic_core::base::Tag;

use crate::components::{
    loader::load,
    util::{box_, button, notification_warning, subtitle_4, title_2},
};

use super::tag_form::ExistingTagValidator;

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

    fn render_level(items: &[TagNode], depth: usize) -> TagBuilder {
        div().and_iter(items.iter().map(|node| node.render(depth)))
    }

    fn render(&self, depth: usize) -> TagBuilder {
        let tag = button().and(&self.tag.name);
        let children = Self::render_level(&self.children, depth + 1);
        div().class("mb-2").and((tag, children))
    }

    fn sort_children(&mut self) {
        self.children.sort_by(|a, b| a.tag.name.cmp(&b.tag.name));
    }
}

struct State {
    tree: Vec<TagNode>,
    validator: Rc<ExistingTagValidator>,
}

pub fn tag_manager() -> TagBuilder {
    let f = async move {
        let page = crate::context::api()
            .select(Tag::query_all())
            .await?
            .convert_data::<Tag>()?;
        let existing_names = page.items.iter().map(|t| t.name.clone()).collect();
        let validator = Rc::new(ExistingTagValidator {
            tags: existing_names,
        });

        let tree = TagNode::build_tree(page.items);
        Ok(Mutable::new(State { tree, validator }))
    };

    load(f, |state| {
        let state2 = state.clone();

        let content = state.signal_ref(move |data| {
            let state = state2.clone();
            let form = super::tag_create(
                move |tag| {
                    let mut data = state.lock_mut();
                    data.tree.push(TagNode {
                        tag,
                        children: Vec::new(),
                    });
                    data.tree.sort_by(|a, b| a.tag.name.cmp(&b.tag.name));
                    // FIXME: extend validator.
                },
                data.validator.clone(),
            );
            let form_wrap = box_().and((subtitle_4().and("New Tag"), form));

            let tree = if data.tree.is_empty() {
                notification_warning().and("No tags found.")
            } else {
                TagNode::render_level(&data.tree, 0)
            };

            div().and(form_wrap).and(tree)
        });

        div().and(title_2().and("Tags")).child_signal(content)
    })
}
