use brass::{vdom, Callback};

pub struct BackendSetupForm {
    pub on_submit: Callback<semantics_core::api::BackendConfig>,
}

enum Msg {
    DataPath(String),
    Key(String),
    Submit,
}

struct BackendSetupFormComp {
    on_submit: Callback<semantics_core::api::BackendConfig>,

    data_path: String,
    key: String,
}

brass::enable_props!(BackendSetupForm => BackendSetupFormComp);

impl brass::Component for BackendSetupFormComp {
    type Properties = BackendSetupForm;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            on_submit: props.on_submit,
            data_path: String::new(),
            key: String::new(),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::DataPath(path) => {
                self.data_path = path;
            }
            Msg::Key(key) => {
                self.key = key;
            }
            Msg::Submit => {
                if self.key.is_empty() {
                    return;
                }

                let data_path = {
                    let p = self.data_path.trim();
                    if p.is_empty() {
                        None
                    } else {
                        Some(p.to_string())
                    }
                };
                let key = self.key.clone();

                let config = semantics_core::api::BackendConfig::Crypto { data_path, key };
                self.on_submit.send(config);
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        let key = brass_bulma::FieldHorizontal {
            label: "Password".into(),
            help: None,
            control: brass_bulma::Input {
                _type: "password".into(),
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.key.clone(),
                on_input: ctx
                    .on_opt(|ev: web_sys::Event| brass::util::input_event_value(ev).map(Msg::Key)),
            },
        };

        let path = brass_bulma::FieldHorizontal {
            label: "Data Path".into(),
            help: Some(brass_bulma::Help {
                message: vdom::text(
                    "File system data path. Leave empty to use the default location.",
                )
                .into(),
                color: brass_bulma::Color::Default,
            }),
            control: brass_bulma::Input {
                _type: "text".into(),
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.data_path.clone(),
                on_input: ctx.on_opt(|ev: web_sys::Event| {
                    brass::util::input_event_value(ev).map(Msg::DataPath)
                }),
            },
        };

        let submit = brass_bulma::button()
            .and("Submit")
            .on_click(ctx.on_simple(|| Msg::Submit));
        let actions = brass_bulma::buttons().and(submit);

        vdom::div().and((key, path, actions)).build()
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        self.on_submit = props.on_submit;
        true
    }
}
