use std::collections::HashSet;

use brass::dom::TagBuilder;
use factordb::{query::expr::Expr, schema::AttributeDescriptor};
use semantic_core::base::Tag;

use crate::{
    components::{
        form,
        util::{form_field_input, form_field_tag_select, FormRenderer, SelectOption},
    },
    context,
};

#[derive(Clone)]
pub struct EntityFilter {
    search: String,
    entity_types: HashSet<String>,
    tags: Vec<Tag>,
}

impl EntityFilter {
    pub fn build_expr(&self) -> Expr {
        let mut e = Expr::Literal(factordb::data::Value::Bool(true));

        let search = self.search.trim();
        if !search.is_empty() {
            let se = Expr::contains(
                Expr::attr::<semantic_core::base::AttrTitle>(),
                search.clone(),
            );
            e = e.and_with(se);
        }

        if !self.entity_types.is_empty() {
            let values = self
                .entity_types
                .iter()
                .map(|val| factordb::Value::from(val.clone()))
                .collect();
            let se = Expr::in_(
                Expr::Attr(factordb::schema::builtin::AttrType::IDENT),
                factordb::Value::List(values),
            );
            e = e.and_with(se);
        }

        if !self.tags.is_empty() {
            let ids = self.tags.iter().map(|t| t.id).collect();
            e = e.and_with(Tag::filter_entity_has_any_tag(ids));
        }

        e
    }
}

pub fn entity_filter(on_submit: impl Fn(EntityFilter) + 'static) -> TagBuilder {
    form::Form::new(EntityFilter {
        search: String::new(),
        entity_types: HashSet::new(),
        tags: Vec::new(),
    })
    .on_submit(move |values| {
        on_submit(values.clone());
    })
    .render(|handle| {
        let search = form_field_input("Search", handle.field(|v| &mut v.search));

        let type_options = context::registry()
            .entities()
            .values()
            .map(|info| SelectOption {
                value: info.schema.ident.clone(),
                label: info
                    .schema
                    .title
                    .clone()
                    .unwrap_or_else(|| info.schema.ident.clone())
                    .into(),
            })
            .collect();
        let types =
            form_field_tag_select("Type", type_options, handle.field(|v| &mut v.entity_types));

        FormRenderer::new(handle.clone())
            .and(search)
            .and(types)
            .buttons_submit("Apply")
    })
}
