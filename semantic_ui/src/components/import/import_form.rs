use brass::{dom::Attr, vdom::div};

pub struct ImportForm {
    loading: bool,
    on_preview: brass::Callback<url::Url>,
    on_import: brass::Callback<url::Url>,
    url: String,
}

pub struct ImportFormProps {
    pub loading: bool,
    pub on_preview: brass::Callback<url::Url>,
    pub on_import: brass::Callback<url::Url>,
}

brass::enable_props!(ImportFormProps => ImportForm);

pub enum Msg {
    Changed(String),
    Preview,
    Import,
}

impl brass::Component for ImportForm {
    type Properties = ImportFormProps;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            loading: props.loading,
            url: String::new(),
            on_preview: props.on_preview,
            on_import: props.on_import,
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Changed(url) => {
                self.url = url;
            }
            Msg::Preview => {
                if !self.loading {
                    let clean = self.url.trim();
                    if let Ok(url) = clean.parse() {
                        self.on_preview.send(url);
                    }
                }
            }
            Msg::Import => {
                let clean = self.url.trim();
                if let Ok(url) = clean.parse() {
                    self.on_import.send(url);
                }
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        let url = brass_bulma::Field {
            label: "Url".into(),
            help: None,
            control: brass_bulma::Input {
                _type: "text".into(),
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.url.clone(),
                on_input: ctx.on(|ev: web_sys::Event| {
                    let value = brass::util::input_event_value(ev).unwrap();
                    Msg::Changed(value)
                }),
            },
        };

        let btn_preview = brass_bulma::button_medium()
            .attr_toggle_if(self.loading, Attr::Disabled)
            .and(if self.loading { "..." } else { "Preview" })
            .on(
                brass::dom::Event::Click,
                ctx.on(|_ev: web_sys::Event| Msg::Preview),
            );

        let btn_import = brass_bulma::button_medium()
            .attr_toggle_if(self.loading, Attr::Disabled)
            .and(if self.loading { "..." } else { "Import" })
            .on(
                brass::dom::Event::Click,
                ctx.on(|_ev: web_sys::Event| Msg::Import),
            );

        let actions = brass_bulma::buttons().and((btn_preview, btn_import));

        div().and((url, actions)).build()
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
