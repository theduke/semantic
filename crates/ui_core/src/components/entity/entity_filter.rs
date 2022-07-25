use std::{collections::HashSet, rc::Rc};

use brass::{
    dom::{builder::div, TagBuilder},
    signal::signal::{Mutable, SignalExt},
};
use factordb::prelude::{AttrId, AttributeDescriptor, Expr, Id, Order, Select, Value};
use semantic_core::base::{AttrCreatedAt, AttrLastVisitTime, AttrTitle, AttrUpdatedAt, Tag};

use crate::{
    base::tags::load_all_tags,
    components::{
        form,
        util::{
            buttons, form_field_input, form_field_select, form_field_tag_select,
            form_field_textarea, ButtonBuilder, FormRenderer, SelectOption,
        },
    },
    context,
    validate::Validator,
};

use super::build_search_term_expr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterSort {
    Id,
    Title,
    CreatedAt,
    UpdatedAt,
    LastVisisted,
}

#[derive(Clone)]
pub struct EntityFilterForm {
    pub search: String,
    pub entity_types: HashSet<String>,
    pub tags: HashSet<Id>,
    pub sort: FilterSort,
    pub sort_order: Order,
}

impl EntityFilterForm {
    pub fn build_expr(&self) -> Expr {
        let mut e = Expr::Literal(factordb::data::Value::Bool(true));

        if !self.search.trim().is_empty() {
            e = e.and_with(build_search_term_expr(&self.search));
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

    pub fn build_query(&self) -> Select {
        let sort = match self.sort {
            FilterSort::Id => Expr::attr::<AttrId>(),
            FilterSort::CreatedAt => Expr::attr::<AttrCreatedAt>(),
            FilterSort::UpdatedAt => Expr::attr::<AttrUpdatedAt>(),
            FilterSort::LastVisisted => Expr::attr::<AttrLastVisitTime>(),
            FilterSort::Title => Expr::attr::<AttrTitle>(),
        };

        Select::new()
            .with_filter(self.build_expr())
            .with_sort(sort, self.sort_order)
    }
}

pub fn entity_filter_form(on_submit: impl Fn(EntityFilterForm) + 'static) -> TagBuilder {
    let data = EntityFilterForm {
        search: String::new(),
        entity_types: HashSet::new(),
        tags: HashSet::new(),
        sort: FilterSort::Id,
        sort_order: Order::Asc,
    };
    let form = form::Form::new(data)
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

    let sort = form_field_select(
        "Sort by",
        vec![
            SelectOption {
                label: "Title".to_string(),
                value: FilterSort::Title,
            },
            SelectOption {
                label: "Id".to_string(),
                value: FilterSort::Id,
            },
            SelectOption {
                label: "Created at".to_string(),
                value: FilterSort::CreatedAt,
            },
            SelectOption {
                label: "Updated at".to_string(),
                value: FilterSort::UpdatedAt,
            },
            SelectOption {
                label: "Last viewed at".to_string(),
                value: FilterSort::LastVisisted,
            },
        ],
        form.field(|v| &mut v.sort),
    );

    let sort_order = form_field_select(
        "Sort order",
        vec![
            SelectOption {
                label: "Ascending".to_string(),
                value: Order::Asc,
            },
            SelectOption {
                label: "Descending".to_string(),
                value: Order::Desc,
            },
        ],
        form.field(|v| &mut v.sort_order),
    );

    let form2 = form.clone();
    let tags = brass::dom::ApplyFuture(async move {
        match load_all_tags().await {
            Ok(tags) => {
                tracing::info!(count = tags.len(), "entity filter tags loaded");
                let options = tags
                    .into_iter()
                    .map(|t| SelectOption {
                        label: t.name.clone(),
                        value: t.id,
                    })
                    .collect();
                form_field_tag_select("Tags", options, form2.field(|v| &mut v.tags))
            }
            Err(error) => {
                tracing::error!(?error, "could not load tags for entity filter");
                div()
            }
        }
    });

    FormRenderer::new(form)
        .and(search)
        .and(types)
        .and(tags)
        .and(sort)
        .and(sort_order)
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
            EntityFilter::Form(f) => f.build_query(),
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
