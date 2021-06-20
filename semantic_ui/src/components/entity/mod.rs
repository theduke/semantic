pub mod browse_page;
pub mod entity_page;

use brass::{
    vdom::{self, component, div, div_with, span_with, TagBuilder},
    VNode,
};
use factordb::{
    data::{DataMap, Value},
    query::select::Item,
    schema::{
        builtin::{AttrId, AttrIdent, AttrType},
        AttrMapExt, AttributeDescriptor, AttributeSchema, EntitySchema,
    },
};
use semantic_ui_core::{EntityInfo, EntityRenderOpts, Registry};
use semantics_core::base::AttrTitle;
use vdom::text;

pub fn entity_header(data: &DataMap, entity: Option<&EntityInfo>) -> TagBuilder {
    let title_text = data
        .get_attr::<semantics_core::base::AttrTitle>()
        .or_else(|| data.get_id().map(|x| x.to_string()))
        .unwrap_or_else(|| "<No Title>".to_string());

    let title_content = if let Some(ident) = data.get_ident() {
        component::<super::router::Link>(super::router::LinkProps {
            route: super::router::Route::Entity(ident),
            text: title_text,
            class: None,
        })
    } else {
        vdom::text(title_text)
    };

    let title = span_with(title_content).class("pr-2");

    let type_name = data.get_type().map(|x| {
        entity
            .and_then(|entity| entity.schema.title.clone())
            .unwrap_or_else(|| x.to_string())
    });
    let ty = span_with(type_name);

    div().class("mb-3").and((title, ty))
}

pub fn attr_title(attr: &AttributeSchema) -> &str {
    if let Some(title) = &attr.title {
        title.as_str()
    } else {
        attr.ident.as_str()
    }
}

pub fn render_image(src: &str, title: Option<&str>, is_preview: bool) -> TagBuilder {
    let mut img = vdom::img(src);

    if let Some(title) = title {
        img = img.attr(brass::dom::Attr::Alt, title);
    }

    if is_preview {
        img = img.style_raw("max-height: 200px");
    }

    img
}

pub fn render_video(src: &str, preview_img_url: Option<&str>, is_preview: bool) -> TagBuilder {
    let mut video = vdom::tag(brass::dom::Tag::Video).attr_toggle(brass::dom::Attr::Controls);

    if let Some(thumb) = preview_img_url {
        video = video.attr(brass::dom::Attr::Poster, thumb);
    }

    if is_preview {
        video = video.style_raw("max-height: 200px");
    }

    let source = vdom::tag(brass::dom::Tag::Source).attr(brass::dom::Attr::Src, src);

    video.and(source)
}

pub fn render_value(value: &Value) -> VNode {
    match value {
        Value::Unit => text("<no value>"),
        Value::Bool(flag) => text(if *flag { "Yes" } else { "No" }),
        Value::UInt(v) => text(v.to_string()),
        Value::Int(v) => text(v.to_string()),
        Value::Float(v) => text(v.to_string()),
        Value::String(v) => text(v.as_str()),
        Value::Bytes(_v) => text("Bytes [...]"),
        Value::List(items) => {
            let entries = items.iter().map(render_value);
            vdom::ul().and_iter(entries).build()
        }
        Value::Map(map) => {
            let rows = map.iter().map(|(key, value)| {
                let title = render_value(key);
                let val = render_value(value);
                vdom::tr().and((vdom::td_with(title), vdom::td_with(val)))
            });
            brass_bulma::table().and_iter(rows).build()
        }
        Value::Id(id) => {
            let link = super::router::LinkProps {
                route: super::router::Route::Entity(id.clone().into()),
                text: id.to_string(),
                class: None,
            };
            vdom::component::<super::router::Link>(link)
        }
    }
}

pub fn attr_value_generic(attr: &AttributeSchema, value: &Value) -> VNode {
    match (&attr.value_type, value) {
        _ => render_value(value),
    }
}

pub fn attr_value(
    attr: &AttributeSchema,
    value: &Value,
    data: Option<&DataMap>,
    registry: &Registry,
) -> VNode {
    if let Some(renderer) = registry.attr_renderer(&attr.ident) {
        tracing::trace!("USING CUSTOM RENDERER");
        renderer(value, data)
    } else {
        tracing::trace!(?attr.ident, "no custom renderer, using generic");
        attr_value_generic(attr, value)
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
            tracing::trace!(%key, attr=?registry.attr(&key), "getting attr key");
            let (title, val) = if let Some(field) = info.and_then(|info| info.fields.get(key)) {
                let title = attr_title(&field.attr);
                let value = attr_value(&field.attr, value, Some(entity), registry);
                (title, value)
            } else if let Some(attr) = registry.attr(key) {
                let title = attr_title(&attr);
                let value = attr_value(&attr, value, Some(entity), registry);
                (title, value)
            } else {
                let value = render_value(value);
                (key.as_str(), value)
            };

            vdom::tr().and((vdom::th_with(title), vdom::td_with(val)))
        });

    brass_bulma::table().and_iter(rows)
}

pub fn generic_entity_view(data: &DataMap, registry: &Registry, opts: &EntityRenderOpts) -> VNode {
    let info = data.get_type().and_then(|ty| registry.entity_by_ident(&ty));

    if let Some(renderer) = info.and_then(|info| registry.entity_item_renderer(&info.schema.ident))
    {
        renderer(data, opts)
    } else {
        let header = entity_header(data, info);
        let fields = entity_fields_table(data, info, registry);
        div().and(header).and(fields).build()
    }
}

pub fn entity_item(item: &Item, registry: &Registry, opts: &EntityRenderOpts) -> TagBuilder {
    let data = generic_entity_view(&item.data, registry, opts);

    let joins = if item.joins.is_empty() {
        VNode::Empty
    } else {
        let joins = item.joins.iter().map(|join| {
            let items = join
                .items
                .iter()
                .map(|join_item| entity_item(join_item, registry, opts));

            div()
                .class("mb-2 ml-4")
                .and(div_with(&join.name).class("mb-2"))
                .and_iter(items)
        });

        div().and(vdom::hr()).and_iter(joins).build()
    };

    div().class("box").and(data).and(div().and(joins))
}

pub fn entity_page(
    page: &factordb::query::select::ItemPage,
    registry: &Registry,
    opts: &EntityRenderOpts,
) -> TagBuilder {
    let items = page
        .items
        .iter()
        .map(|item| entity_item(item, registry, opts));
    div().and_iter(items)
}
