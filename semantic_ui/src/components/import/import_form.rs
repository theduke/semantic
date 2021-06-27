use brass::{dom::Attr, vdom::div};

pub struct ImportForm {
    loading: bool,
    on_submit: brass::Callback<url::Url>,
    url: String,
}

pub struct ImportFormProps {
    pub loading: bool,
    pub on_submit: brass::Callback<url::Url>,
}

brass::enable_props!(ImportFormProps => ImportForm);

pub enum Msg {
    Changed(String),
    Submit,
}

impl brass::Component for ImportForm {
    type Properties = ImportFormProps;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            loading: props.loading,
            url: String::new(),
            on_submit: props.on_submit,
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Changed(url) => {
                self.url = url;
            }
            Msg::Submit => {
                if !self.loading {
                    let clean = self.url.trim();
                    if let Ok(url) = clean.parse() {
                        self.on_submit.send(url);
                    }
                }
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        div()
            .and(brass_bulma::Field {
                label: "Url".into(),
                help: None,
                control: brass_bulma::Input {
                    _type: "text".into(),
                    color: brass_bulma::Color::Default,
                    placeholder: None,
                    value: self.url.clone(),
                    on_input: ctx.callback(|ev: web_sys::Event| {
                        let value = brass::util::input_event_value(ev).unwrap();
                        Msg::Changed(value)
                    }),
                },
            })
            .and(
                brass_bulma::button_medium()
                    .attr_toggle_if(self.loading, Attr::Disabled)
                    .and(if self.loading { "..." } else { "Load" })
                    .on(
                        brass::dom::Event::Click,
                        ctx.callback(|_ev: web_sys::Event| Msg::Submit),
                    ),
            )
            .build()
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        if props.loading != self.loading {
            self.loading = props.loading;
            true
        } else {
            false
        }
    }
}
