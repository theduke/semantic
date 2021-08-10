use factordb::{query::expr::Expr, schema::AttributeDescriptor};

pub struct EntityFilterForm {
    pub on_submit: brass::Callback<Expr>,
}

brass::enable_props!(EntityFilterForm => EntityFilterFormComp);

pub enum Msg {
    SetSearch(String),
    Reset,
    Submit,
}

pub struct EntityFilterFormComp {
    on_submit: brass::Callback<Expr>,

    changed: bool,
    search: String,
}

impl EntityFilterFormComp {
    fn build_expr(&self) -> Expr {
        let mut e = Expr::Literal(factordb::data::Value::Bool(true));

        let search = self.search.trim();
        if !search.is_empty() {
            let se = Expr::BinaryOp {
                left: Expr::Ident(semantics_core::base::AttrTitle::IDENT).boxed(),
                op: factordb::query::expr::BinaryOp::Contains,
                right: Expr::literal(search).boxed(),
            };

            e = e.and_with(se);
        }

        e
    }
}

impl brass::Component for EntityFilterFormComp {
    type Properties = EntityFilterForm;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            on_submit: props.on_submit,
            changed: false,
            search: String::new(),
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
            Msg::Submit => self.on_submit.send(self.build_expr()),
            Msg::Reset => {
                self.search.clear();
                self.changed = true;
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
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

        let submit = brass_bulma::button()
            .and("Apply")
            .attr_toggle_if(!self.changed, brass::dom::Attr::Disabled)
            .on_click(ctx.on_simple(|| Msg::Submit));

        let clear = brass_bulma::button()
            .and("Clear")
            .attr_toggle_if(!self.changed, brass::dom::Attr::Disabled)
            .on_click(ctx.on_simple(|| Msg::Reset));
        let buttons = brass_bulma::buttons().and((submit, clear));

        brass_bulma::box_().and((search, buttons)).build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        false
    }
}
