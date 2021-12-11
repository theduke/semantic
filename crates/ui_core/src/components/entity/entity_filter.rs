use std::collections::HashSet;

use brass::dom::{builder::div, TagBuilder};
use factordb::{query::expr::Expr, schema::AttributeDescriptor, Id};
use semantic_core::base::Tag;

use crate::{
    base::tags::load_all_tags,
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
    tags: HashSet<Id>,
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
            e = e.and_with(Tag::filter_entity_has_any_tag(
                self.tags.iter().cloned().collect(),
            ));
        }

        e
    }
}

pub fn entity_filter(on_submit: impl Fn(EntityFilter) + 'static) -> TagBuilder {
    let form = form::Form::new(EntityFilter {
        search: String::new(),
        entity_types: HashSet::new(),
        tags: HashSet::new(),
    })
    .on_submit(move |values| {
        on_submit(values.clone());
    })
    .build();

    let search = form_field_input("Search", form.field(|v| &mut v.search));

    let type_options = context::registry()
        .entities_without_ignored()
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
    let types = form_field_tag_select("Type", type_options, form.field(|v| &mut v.entity_types));

    let form2 = form.clone();
    let tags = brass::dom::ApplyFuture(async move {
        match load_all_tags().await {
            Ok(tags) => {
                let options = tags
                    .into_iter()
                    .map(|t| SelectOption {
                        label: t.name.clone(),
                        value: t.id,
                    })
                    .collect();
                form_field_tag_select("Tags", options, form2.field(|v| &mut v.tags))
            }
            Err(_err) => div(),
        }
    });

    FormRenderer::new(form)
        .and(search)
        .and(types)
        .and(tags)
        .buttons_submit("Apply")
}
