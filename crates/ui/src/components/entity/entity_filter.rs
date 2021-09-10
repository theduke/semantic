use std::collections::HashSet;

use brass::{
    vdom::{self, s},
    PropWrapper, Shared,
};
use factordb::{query::expr::Expr, schema::AttributeDescriptor};
use semantic_core::base::Tag;
use semantic_ui_core::ContextExt;

#[derive(Default)]
pub struct EntityFilter {
    pub search_term: Option<String>,
    pub entity_types: Option<HashSet<String>>,
    pub tags: Vec<Tag>,
}

impl EntityFilter {
    pub fn build_expr(&self) -> Expr {
        let mut e = Expr::Literal(factordb::data::Value::Bool(true));

        if let Some(term) = &self.search_term {
            let se = Expr::contains(Expr::attr::<semantic_core::base::AttrTitle>(), term.clone());
            e = e.and_with(se);
        }

        if let Some(types) = &self.entity_types {
            if !types.is_empty() {
                let values = types
                    .iter()
                    .map(|val| factordb::Value::from(val.clone()))
                    .collect();
                let se = Expr::in_(
                    Expr::Attr(factordb::schema::builtin::AttrType::IDENT),
                    factordb::Value::List(values),
                );
                e = e.and_with(se);
            }
        }

        if !self.tags.is_empty() {
            let ids = self.tags.iter().map(|t| t.id).collect();
            e = e.and_with(Tag::filter_entity_has_any_tag(ids));
        }

        e
    }
}

pub struct EntityFilterForm {
    pub on_submit: brass::Callback<EntityFilter>,
}

brass::enable_props!(wrapped EntityFilterForm => State);

pub enum Msg {
    SetSearch(String),
    TypeToggled(String),
    TagsChanged(Vec<Tag>),
    Reset,
    Submit,
}

struct State {
    changed: bool,

    search: String,

    entity_type_options: Vec<brass_bulma::SelectOption<String>>,
    entity_types: HashSet<String>,
    tags: Shared<Vec<Tag>>,
}

impl State {
    fn build(&self) -> EntityFilter {
        let search_term = {
            let v = self.search.trim();
            if !v.is_empty() {
                Some(v.to_string())
            } else {
                None
            }
        };

        let entity_types = if !self.entity_types.is_empty() {
            Some(self.entity_types.clone())
        } else {
            None
        };

        EntityFilter {
            search_term,
            entity_types,
            tags: self.tags.as_ref().clone(),
        }
    }
}

impl brass::PropComponent for State {
    type Properties = EntityFilterForm;
    type Msg = Msg;

    fn init(_props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let registry = ctx.registry();
        let type_options = registry
            .entities()
            .values()
            .map(|info| brass_bulma::SelectOption {
                value: info.schema.ident.clone(),
                label: info
                    .schema
                    .title
                    .clone()
                    .unwrap_or_else(|| info.schema.ident.clone())
                    .into(),
            })
            .collect();

        Self {
            changed: false,
            search: String::new(),
            entity_type_options: type_options,
            entity_types: HashSet::new(),
            tags: Vec::new().into(),
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::SetSearch(value) => {
                if value.trim() != self.search.trim() {
                    self.search = value;
                    self.changed = true;
                }
            }
            Msg::TypeToggled(ty) => {
                if !self.entity_types.remove(&ty) {
                    self.entity_types.insert(ty);
                    self.changed = true;
                }
            }
            Msg::TagsChanged(new_tags) => {
                if &new_tags != self.tags.as_ref() {
                    self.changed = true;
                }
                self.tags = new_tags.into();
            }
            Msg::Submit => props.on_submit.send(self.build()),
            Msg::Reset => {
                self.search.clear();
                self.entity_types.clear();
                self.changed = false;
                props.on_submit.send(self.build());
            }
        }
    }

    fn render(
        &self,
        _props: &Self::Properties,
        mut ctx: &mut brass::RenderContext<PropWrapper<Self>>,
    ) -> brass::VNode {
        let title = vdom::div().class("subtitle is-5").and("Filter");

        let search = brass_bulma::FieldHorizontal {
            label: s("Search"),
            help: None,
            control: brass_bulma::Input {
                _type: "text",
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.search.clone().into(),
                on_input: ctx.on_opt(|ev| brass::util::input_event_value(ev).map(Msg::SetSearch)),
            },
        };

        let types = brass_bulma::FieldHorizontal {
            label: s("Type"),
            help: None,
            control: brass_bulma::TagSelect {
                options: &self.entity_type_options,
                selected: &self.entity_types,
                on_select: ctx.callback_map(Msg::TypeToggled),
            },
        };

        let tag_select = crate::components::base::tags::tag_select(
            self.tags.clone(),
            ctx.callback_map(Msg::TagsChanged),
        );
        let tags = brass_bulma::FieldHorizontal {
            label: s("Tags"),
            help: None,
            control: tag_select,
        };

        let submit = brass_bulma::button()
            .and(s("Apply"))
            .attr_toggle_if(!self.changed, brass::dom::Attr::Disabled)
            .on_click(ctx.on_simple(|| Msg::Submit));

        let clear = brass_bulma::button()
            .and(s("Clear"))
            .attr_toggle_if(!self.changed, brass::dom::Attr::Disabled)
            .on_click(ctx.on_simple(|| Msg::Reset));
        let buttons = brass_bulma::buttons().and((submit, clear));

        vdom::div()
            .and((title, search, types, tags, buttons))
            .build()
    }

    fn on_property_change(
        &mut self,
        _old_props: &Self::Properties,
        _new_props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        true
    }
}
