use std::{collections::HashSet, rc::Rc};

use brass::{
    dom::{builder::div, TagBuilder},
    signal::signal::{Mutable, SignalExt},
};
use factordb::prelude::{AttributeDescriptor, Expr, Id, Select, Value};
use semantic_core::base::Tag;

use crate::{
    base::tags::load_all_tags,
    components::{
        form,
        util::{
            buttons, form_field_input, form_field_tag_select, form_field_textarea, ButtonBuilder,
            FormRenderer, SelectOption,
        },
    },
    context,
    validate::Validator,
};

#[derive(Clone)]
pub struct EntityFilterForm {
    search: String,
    entity_types: HashSet<String>,
    tags: HashSet<Id>,
}

impl EntityFilterForm {
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
                .map(|val| Value::from(val.clone()))
                .collect();
            let se = Expr::in_(
                Expr::Attr(factordb::schema::builtin::AttrType::IDENT),
                Value::List(values),
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

pub fn entity_filter_form(on_submit: impl Fn(EntityFilterForm) + 'static) -> TagBuilder {
    let form = form::Form::new(EntityFilterForm {
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

#[derive(Clone)]
pub struct EntityFilterSql {
    sql: String,
}

struct SqlValidator;

impl Validator<String> for SqlValidator {
    fn validate(&self, value: &String) -> Result<(), Vec<String>> {
        Select::parse_sql(value)
            .map_err(|e| vec![e.to_string()])
            .map(|_| ())
    }
}

pub fn entity_filter_sql(on_submit: impl Fn(EntityFilterSql) + 'static) -> TagBuilder {
    let form = form::Form::new(EntityFilterSql { sql: String::new() })
        .on_submit(move |values| {
            on_submit(values.clone());
        })
        .build();

    let sql = form_field_textarea(
        "SQL",
        form.field_validated(|v| &mut v.sql, SqlValidator),
        5,
        true,
    );

    FormRenderer::new(form).and(sql).buttons_submit("Apply")
}

#[derive(Clone)]
pub enum EntityFilter {
    Form(EntityFilterForm),
    Sql(EntityFilterSql),
}

impl EntityFilter {
    pub fn build_select(&self) -> Select {
        match self {
            EntityFilter::Form(f) => Select::new().with_filter(f.build_expr()),
            // TODO: no unwrap?
            EntityFilter::Sql(f) => Select::parse_sql(&f.sql).unwrap(),
        }
    }
}

pub fn entity_filter(on_submit: impl Fn(EntityFilter) + 'static) -> TagBuilder {
    let on_submit = Rc::new(on_submit);
    let is_sql = Mutable::new(false);
    div()
        .and(
            buttons()
                .and(
                    ButtonBuilder::new()
                        .label("Filter")
                        .on({
                            let is_sql = is_sql.clone();
                            move || {
                                is_sql.set(false);
                            }
                        })
                        .signal_active(is_sql.signal().map(|x| !x))
                        .build(),
                )
                .and(
                    ButtonBuilder::new()
                        .label("SQL")
                        .on({
                            let is_sql = is_sql.clone();
                            move || {
                                is_sql.set(true);
                            }
                        })
                        .signal_active(is_sql.signal())
                        .build(),
                ),
        )
        .signal(is_sql.signal().map(move |is_sql| {
            if is_sql {
                let on_submit = on_submit.clone();
                entity_filter_sql(move |f| on_submit(EntityFilter::Sql(f)))
            } else {
                let on_submit = on_submit.clone();
                entity_filter_form(move |f| on_submit(EntityFilter::Form(f)))
            }
        }))
}
