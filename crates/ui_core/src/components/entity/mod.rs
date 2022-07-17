// pub mod create_page;
// pub mod persister;
pub mod entity_box;
pub mod entity_deleter;
pub mod entity_filter;
pub mod entity_view;

use brass::dom::{builder::div, Apply, Attr, Render, Tag, TagBuilder, View};
use factordb::prelude::{
    AttrId, AttrIdent, AttrMapExt, AttrType, AttributeDescriptor, AttributeSchema, DataMap, Id,
    Timestamp, Value, ValueType,
};
use semantic_core::base::AttrTitle;

use crate::{
    context,
    routing::{link, Route},
    EntityInfo, EntityRenderOpts, Registry,
};

use self::{entity_box::EntityBox, entity_view::EntityView};

use super::{loader::Loader, util::table};

fn entity_href(item: &DataMap) -> Option<String> {
    item.get_id().map(|id| Route::Entity(id.into()).to_path())
}

pub fn entity_loader(id: Id, render: impl Fn(&DataMap) -> View + 'static) -> TagBuilder {
    let loader = Loader::new_spawn(async move { context::api().entity(id).await });
    let inner = loader.signal_render(render);
    // FIXME: need to manually retain the loader for now. remove when loader
    // is refactored.
    div().bind(loader).signal(inner)
}

pub fn entity_page(id: Id) -> TagBuilder {
    entity_loader(id, |item| {
        EntityBox {
            item: item.clone(),
            options: EntityRenderOpts {
                editable: true,
                preview: false,
            },
            on_delete: Some(Box::new(|_item| {
                context::router().goto(Route::Browse);
            })),
            show_link: false,
        }
        .render()
    })
}

pub fn entity_type_name(data: &DataMap, entity: Option<&EntityInfo>) -> Option<String> {
    data.get_type().map(|x| {
        entity
            .and_then(|entity| entity.schema.title.clone())
            .unwrap_or_else(|| x.to_string())
    })
}

// pub fn entity_header(data: &DataMap, entity: Option<&EntityInfo>) -> TagBuilder {
//     let title_text = entity_title(data);

//     let title_content = if let Some(ident) = data.get_ident() {
//         link(Route::Entity(ident), &title_text)
//     } else {

//         vdom::text(title_text)
//     };

//     let title = brass_bulma::card_header_title(title_content).style_raw("flex-grow: 0;");

//     let type_name = entity_type_name(data, entity);
//     let ty = span_with(type_name);

//     // div().class("mb-3").and((title, ty))

//     brass_bulma::card_header().and((title, ty))
// }

pub fn attr_title(attr: &AttributeSchema) -> &str {
    if let Some(title) = &attr.title {
        title.as_str()
    } else {
        attr.ident.as_str()
    }
}

pub fn render_image(src: &str, title: Option<&str>, is_preview: bool) -> TagBuilder {
    let mut img = Tag::Image.new().attr(Attr::Src, src);

    if let Some(title) = title {
        img = img.attr(brass::dom::Attr::Alt, title);
    }

    if is_preview {
        img = img.style_raw("max-height: 200px");
    }

    img
}

pub fn render_video(src: &str, preview_img_url: Option<&str>, is_preview: bool) -> TagBuilder {
    let mut video = brass::dom::Tag::Video
        .new()
        .attr_toggle(brass::dom::Attr::Controls);

    if let Some(thumb) = preview_img_url {
        video = video.attr(brass::dom::Attr::Poster, thumb);
    }

    if is_preview {
        video = video.style_raw("max-height: 200px");
    }

    let source = Tag::Source.new().attr(Attr::Src, src);

    video.and(source)
}

pub fn render_value(value: &Value, parent: &mut TagBuilder) {
    match value {
        Value::Unit => "<no value>".apply(parent),
        Value::Bool(flag) => (if *flag { "Yes" } else { "No" }).apply(parent),
        Value::UInt(v) => v.to_string().apply(parent),
        Value::Int(v) => v.to_string().apply(parent),
        Value::Float(v) => v.to_string().apply(parent),
        Value::String(v) => v.as_str().apply(parent),
        Value::Bytes(_v) => "Bytes[...]".apply(parent),
        Value::List(items) => {
            let entries = items.iter().map(|value| {
                let mut li = Tag::Li.new();
                render_value(value, &mut li);
                li
            });

            parent.add_tag(Tag::Ul.new().and_iter(entries))
        }
        Value::Map(map) => {
            let mut table = table();
            for (key, value) in map.iter() {
                let mut td_key = Tag::Td.new();
                render_value(key, &mut td_key);
                let mut td_value = Tag::Td.new();
                render_value(value, &mut td_value);
                let tr = Tag::Tr.new().and((td_key, td_value));
                table.add_tag(tr);
            }
            table.apply(parent);
        }
        Value::Id(id) => {
            let link = link(Route::Entity(id.clone().into()), &id.to_string());
            parent.add_tag(link);
        }
    }
}

pub fn attr_value_generic(attr: &AttributeSchema, value: &Value, parent: &mut TagBuilder) {
    match (&attr.value_type, value) {
        (ValueType::DateTime, Value::UInt(x)) => {
            let stamp = Timestamp::from_millis(*x);
            let dt = stamp.to_datetime();
            let value = dt.to_rfc3339();
            tracing::info!(%value, "datetime value");
        }
        _ => render_value(value, parent),
    }
}

pub fn attr_value(
    attr: &AttributeSchema,
    value: &Value,
    data: Option<&DataMap>,
    registry: &Registry,
    parent: &mut TagBuilder,
) {
    if let Some(renderer) = registry.attr_renderer(&attr.ident) {
        let out = renderer(value, data);
        parent.add_tag(out);
    } else {
        attr_value_generic(attr, value, parent);
    }
}

pub fn entity_fields_table(
    entity: &DataMap,
    info: Option<&EntityInfo>,
    registry: &Registry,
) -> TagBuilder {
    let rows = entity
        .iter()
        .filter(|(key, _value)| {
            let key = key.as_str();
            if key == AttrId::QUALIFIED_NAME
                || key == AttrIdent::QUALIFIED_NAME
                || key == AttrType::QUALIFIED_NAME
                || key == AttrTitle::QUALIFIED_NAME
            {
                false
            } else {
                true
            }
        })
        .map(|(key, value)| {
            let title;
            let mut td_val = Tag::Td.new();

            if let Some(field) = info.and_then(|info| info.fields.get(key)) {
                title = attr_title(&field.attr);
                attr_value(&field.attr, value, Some(entity), registry, &mut td_val);
            } else if let Some(attr) = registry.attr(key) {
                title = attr_title(&attr);
                attr_value(&attr, value, Some(entity), registry, &mut td_val);
            } else {
                title = key.as_str();
            };

            Tag::Tr.new().tag(Tag::Th.new().text(title)).tag(td_val)
        });

    table().and_iter(rows)
}

// pub fn entity_joins(item: &Item, opts: &EntityRenderOpts, registry: &Registry) -> VNode {
//     let joins = item.joins.iter().map(|join| {
//         let items = join
//             .items
//             .iter()
//             .map(|join_item| generic_entity_item(join_item, registry, opts));

//         div()
//             .class("mb-4 ml-5")
//             .and(vdom::hr())
//             .and(div_with(vdom::b().and(&join.name)).class("mb-2"))
//             .and_iter(items)
//     });

//     div().and(vdom::hr()).and_iter(joins).build()
// }

pub fn entity_list(items: &[DataMap], registry: &Registry, opts: &EntityRenderOpts) -> TagBuilder {
    let items = items
        .iter()
        .map(|item| EntityView::from_item(item, registry, opts));
    div().and_iter(items)
}
