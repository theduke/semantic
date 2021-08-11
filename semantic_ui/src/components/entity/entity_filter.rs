use std::collections::HashSet;

use brass::vdom::Render;
use factordb::{query::expr::Expr, schema::AttributeDescriptor};
use semantic_ui_core::ContextExt;

pub struct EntityFilterForm {
    pub on_submit: brass::Callback<Expr>,
}

brass::enable_props!(EntityFilterForm => EntityFilterFormComp);

pub enum Msg {
    SetSearch(String),
    TypeToggled(String),
    Reset,
    Submit,
}

pub struct EntityFilterFormComp {
    on_submit: brass::Callback<Expr>,

    changed: bool,
    search: String,

    entity_type_options: Vec<brass_bulma::SelectOption<String>>,
    entity_types: HashSet<String>,
}

impl EntityFilterFormComp {
    fn build_expr(&self) -> Expr {
        let mut e = Expr::Literal(factordb::data::Value::Bool(true));

        let search = self.search.trim();
        if !search.is_empty() {
            let se = Expr::contains(semantics_core::base::AttrTitle::IDENT, search);
            e = e.and_with(se);
        }

        if !self.entity_types.is_empty() {
            let values = self
                .entity_types
                .iter()
                .map(|val| factordb::Value::from(val.clone()))
                .collect();
            let se = Expr::in_(
                Expr::Ident(factordb::schema::builtin::AttrType::IDENT),
                factordb::Value::List(values),
            );
            e = e.and_with(se);
        }

        e
    }
}

impl brass::Component for EntityFilterFormComp {
    type Properties = EntityFilterForm;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
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
                    .unwrap_or_else(|| info.schema.ident.clone()),
            })
            .collect();

        Self {
            on_submit: props.on_submit,
            changed: false,
            search: String::new(),
            entity_type_options: type_options,
            entity_types: HashSet::new(),
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::SetSearch(value) => {
                if value != self.search {
                    self.search = value;
                    self.changed = true;
                }
            }
            Msg::TypeToggled(ty) => {
                tracing::trace!(?ty, "types toggled");
                if !self.entity_types.remove(&ty) {
                    self.entity_types.insert(ty);
                    self.changed = true;
                }
            }
            Msg::Submit => self.on_submit.send(self.build_expr()),
            Msg::Reset => {
                self.search.clear();
                self.entity_types.clear();
                self.changed = false;
                self.on_submit.send(self.build_expr());
            }
        }
    }

    fn render(&self, mut ctx: brass::RenderContext<Self>) -> brass::VNode {
        let search = brass_bulma::FieldHorizontal {
            label: "Search".to_string(),
            help: None,
            control: brass_bulma::Input {
                _type: "text",
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.search.clone(),
                on_input: ctx.on_opt(|ev| brass::util::input_event_value(ev).map(Msg::SetSearch)),
            },
        };

        let types = brass_bulma::FieldHorizontal {
            label: "Type".into(),
            help: None,
            control: brass_bulma::TagSelect {
                options: &self.entity_type_options,
                selected: &self.entity_types,
                on_select: ctx.callback_map(Msg::TypeToggled),
            }
            .render(),
        };

        let submit = brass_bulma::button()
            .and("Apply")
            .attr_toggle_if(!self.changed, brass::dom::Attr::Disabled)
            .on_click(ctx.on_simple(|| Msg::Submit));

        let clear = brass_bulma::button()
            .and("Clear")
            .attr_toggle_if(!self.changed, brass::dom::Attr::Disabled)
            .on_click(ctx.on_simple(|| Msg::Reset));
        let buttons = brass_bulma::buttons().and((submit, clear));

        brass_bulma::box_().and((search, types, buttons)).build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        false
    }
}
