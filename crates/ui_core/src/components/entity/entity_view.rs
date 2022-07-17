use std::pin::Pin;

use brass::{
    dom::{builder::div, Attr, ClickEvent, Render, Tag, TagBuilder, View},
    signal::signal::Signal,
    DomStr,
};
use factordb::{prelude::DataMap, schema::AttrMapExt};
use semantic_core::base::entity_title;

use crate::{
    components::util::{
        button, buttons, card, card_content, card_header, card_header_title, icon_fa, BtnSize, Cls,
    },
    EntityRenderOpts, Registry,
};

pub struct EntityView<'a> {
    pub link_path: Option<String>,
    pub title: DomStr<'a>,
    pub type_name: Option<DomStr<'a>>,
    pub on_open: Option<Box<dyn Fn()>>,
    pub actions: Vec<EntityActionButton<'a>>,
    pub content: TagBuilder,
}

impl<'a> EntityView<'a> {
    pub fn from_item(item: &DataMap, registry: &Registry, opts: &EntityRenderOpts) -> Self {
        let ty_ident = item.get_type();
        let entity = ty_ident
            .as_ref()
            .and_then(|ty| registry.entity_by_ident(ty))
            .cloned();
        let ty = entity.as_ref().map(|e| &e.schema.ident);
        let content_renderer = ty
            .as_ref()
            .and_then(|ty| registry.entity_content_renderer(ty));
        let type_name = super::entity_type_name(&item, entity.as_ref()).map(DomStr::from);
        let title = DomStr::from(entity_title(&item));

        let content = match content_renderer {
            Some(r) => r(&item, opts),
            None => super::entity_fields_table(&item, entity.as_ref(), registry),
        };

        Self {
            link_path: super::entity_href(item),
            title,
            type_name,
            on_open: None,
            actions: Vec::new(),
            content,
        }
    }
}

impl<'a> Render for EntityView<'a> {
    fn render(self) -> View {
        // Header.

        let title = {
            let t = card_header_title().style_raw("flex-grow: 0; cursor: pointer;");
            let t = if let Some(path) = self.link_path {
                let a = Tag::A
                    .new()
                    .style_raw("color: inherit;")
                    .attr(Attr::Href, path)
                    .and(self.title);
                let a = if self.on_open.is_some() {
                    a.on(|e: ClickEvent| {
                        e.prevent_default();
                    })
                } else {
                    a
                };
                t.and(a)
            } else {
                t.and(self.title)
            };

            if let Some(on) = self.on_open {
                t.on(move |e: ClickEvent| {
                    e.prevent_default();
                    on();
                })
            } else {
                t
            }
        };

        let ty = self.type_name.map(|name| {
            div()
                .and(name)
                .classes([Cls::IsFlex, Cls::IsAlignItemsCenter, Cls::Mr3])
        });

        let actions = buttons().style_raw("margin: 0;").and_iter(self.actions);

        let header = card_header().and((title, ty, actions));

        let card_content = card_content().and(self.content);

        card().class("mb-4").and((header, card_content)).into()
    }
}

pub struct EntityActionButton<'a> {
    pub icon: DomStr<'a>,
    pub label: DomStr<'a>,
    pub is_active: Option<Pin<Box<dyn Signal<Item = bool>>>>,
    pub is_disabled: bool,
    pub on: Box<dyn Fn()>,
}

impl<'a> Render for EntityActionButton<'a> {
    fn render(self) -> View {
        let on = self.on;
        let mut btn = button()
            .class(BtnSize::Small)
            .attr(Attr::Title, self.label)
            .attr_toggle_if(self.is_disabled, Attr::Disabled)
            .style_raw("margin: 0")
            .and(icon_fa(self.icon))
            .on(move |_: ClickEvent| on());
        if let Some(is_active) = self.is_active {
            btn = btn.class_signal_toggle(Cls::IsActive, is_active);
        }
        btn.into()
    }
}
