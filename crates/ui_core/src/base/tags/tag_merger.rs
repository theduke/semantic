use std::rc::Rc;

use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Render, View},
    signal::signal::Mutable,
};
use factordb::prelude::{AttrId, AttrMapExt, AttributeDescriptor, DataMap, Expr};
use semantic_core::base::{AttrTagName, Tag};

use crate::{
    components::{
        autocomplete::entity_picker::entity_picker_with_filter,
        loader::Loader,
        util::{bold, box_, buttons, ButtonBuilder},
    },
    context,
};

/// Merge the given tag into a tag to be selected by the user.
pub struct TagMerger {
    pub source_tag: Tag,
    pub on_cancel: Rc<dyn Fn()>,
    pub on_merged: Rc<dyn Fn()>,
}

struct State {
    source_tag: Tag,
    on_cancel: Rc<dyn Fn()>,
    on_merged: Rc<dyn Fn()>,

    target_tag: Mutable<Option<Tag>>,
    loader: Loader<()>,
}

enum Msg {
    TagSelected(DataMap),
    Submit,
    Cancel,
    Complete,
}

impl MsgComponent for State {
    type Properties = TagMerger;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: brass::component::Context<Self>) -> Self {
        Self {
            source_tag: props.source_tag,
            on_cancel: props.on_cancel,
            on_merged: props.on_merged,
            loader: Loader::new_idle(),
            target_tag: Mutable::new(None),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: brass::component::Context<Self>) {
        match msg {
            Msg::TagSelected(item) => {
                if let Ok(tag) = item.try_into_entity() {
                    self.target_tag.set(Some(tag));
                }
            }
            Msg::Cancel => {
                (self.on_cancel)();
            }
            Msg::Submit => {
                if let Some(target_tag) = self.target_tag.get_cloned() {
                    let api = context::api();

                    let handle = ctx.handle();
                    let source_id = self.source_tag.id;
                    let target_id = target_tag.id;
                    self.loader.spawn(async move {
                        api.tag_merge(source_id.into(), target_id.into()).await?;
                        handle.send(Msg::Complete);
                        Ok(())
                    });
                }
            }
            Msg::Complete => {
                (self.on_merged)();
            }
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> brass::dom::TagBuilder {
        let expr = Expr::is_entity::<Tag>()
            .and_with(Expr::not(Expr::eq(AttrId::expr(), self.source_tag.id)));
        let filter = Mutable::new(expr);

        let sig = filter.signal_cloned();

        let handle = ctx.handle();
        let merge_btn = ButtonBuilder::new()
            .label("Merge")
            .signal_disabled(self.target_tag.signal_ref(|x| x.is_none()))
            .signal_loading(self.loader.signal_loading())
            .on(move || handle.send(Msg::Submit))
            .build();

        let handle = ctx.handle();
        let cancel_btn = ButtonBuilder::new()
            .label("Cancel")
            .on(move || handle.send(Msg::Cancel))
            .build();

        let actions = buttons().and(merge_btn).and(cancel_btn);

        let selected = self.target_tag.signal_ref(|opt| {
            if let Some(tag) = opt {
                div()
                    .class("mb-2")
                    .and(bold().and(format!("Merge into tag '{}'", tag.name)))
                    .into_view()
            } else {
                View::Empty
            }
        });

        let handle = ctx.handle();
        box_()
            .bind(filter)
            .and(
                div()
                    .class("mb-2")
                    .and(bold().and(format!("Merge tag {}", self.source_tag.name))),
            )
            .and(div().class("mb-2").and(entity_picker_with_filter(
                sig,
                |term| Expr::contains(AttrTagName::expr(), term.to_string()),
                move |item| handle.send(Msg::TagSelected(item)),
            )))
            .signal(selected)
            .signal(self.loader.signal_render(|_| div().into_view()))
            .and(actions)
    }
}

impl Render for TagMerger {
    fn render(self) -> View {
        State::build(self)
    }
}
