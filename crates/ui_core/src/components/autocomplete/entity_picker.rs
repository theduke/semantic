use std::rc::Rc;

use brass::{
    dom::{builder::div, Attr, InputEvent, Tag, TagBuilder},
    signal::{signal::Mutable, signal_vec::MutableVec},
};
use factordb::{
    query::{
        expr::Expr,
        select::{Item, Select},
    },
    schema::AttributeDescriptor,
};
use semantic_core::base::{entity_title, AttrTitle};

use crate::{
    components::{
        loader::Loader,
        util::{ButtonBuilder, Cls},
    },
    context,
};

pub fn entity_picker(filter: Expr, on_select: impl Fn(Item) + 'static) -> TagBuilder {
    let on_select: Rc<dyn Fn(Item)> = Rc::new(on_select);

    let value = Mutable::new(String::new());
    let loader = Loader::<Vec<Item>>::new_idle();

    let on_change = {
        let loader = loader.clone();
        let value = value.clone();

        move |ev: InputEvent| {
            let val = ev.value().unwrap_or_default();
            value.set(val.clone());

            let trimmed = val.trim();
            if !trimmed.is_empty() {
                let expr = filter
                    .clone()
                    .and_with(Expr::contains(AttrTitle::expr(), trimmed));
                let f = async move {
                    context::api()
                        .select(Select::new().with_limit(10).with_filter(expr))
                        .await
                        .map(|p| p.items)
                };
                loader.spawn(f);
            }
        }
    };

    let input = Tag::Input
        .new()
        .class(Cls::Input)
        .attr(Attr::Placeholder, "Search...")
        .attr_signal(Attr::Value, value.signal_cloned())
        .on(on_change);

    let items = loader.signal_render(move |items| {
        let on_select = on_select.clone();
        let options = items.iter().map(move |item| {
            let name = entity_title(&item.data);

            let on_select = on_select.clone();

            let item = item.clone();
            ButtonBuilder::new()
                .label(name)
                .on(move || {
                    on_select(item.clone());
                })
                .build()
        });
        Tag::Ul.new().and_iter(options)
    });

    div().and(div().and(input)).child_signal(items)
}
