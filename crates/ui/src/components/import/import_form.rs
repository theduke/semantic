use brass::{dom::Attr, vdom::div, Str};
use url::Url;

pub struct ImportForm {
    loading: bool,
    can_submit: bool,
    on_preview: brass::Callback<url::Url>,
    on_import: brass::Callback<url::Url>,
    url: Str,
    parsed_url: Option<Url>,
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
            can_submit: false,
            url: Str::new(),
            parsed_url: None,
            on_preview: props.on_preview,
            on_import: props.on_import,
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Changed(url) => {
                if url != self.url {
                    if let Ok(parsed) = url.trim().parse() {
                        self.parsed_url = Some(parsed);
                        self.can_submit = true;
                    }
                    self.url = Str::shared(url);
                }
            }
            Msg::Preview => {
                if let Some(url) = &self.parsed_url {
                    self.on_preview.send(url.clone());
                    self.can_submit = false;
                }
            }
            Msg::Import => {
                if let Some(url) = &self.parsed_url {
                    self.on_import.send(url.clone());
                    self.can_submit = false;
                }
            }
        }
    }

    fn render(&self, ctx: &mut brass::RenderContext<Self>) -> brass::VNode {
        let url = brass_bulma::Field {
            label: "Url".into(),
            help: None,
            control: brass_bulma::Input {
                _type: "text".into(),
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.url.clone(),
                on_input: ctx.callback_map(Msg::Changed),
            },
        };

        let btn_preview = brass_bulma::button_medium()
            .attr_toggle_if(self.loading || !self.can_submit, Attr::Disabled)
            .and(if self.loading { "..." } else { "Preview" })
            .on_click(ctx, || Msg::Preview);

        let btn_import = brass_bulma::button_medium()
            .attr_toggle_if(self.loading || !self.can_submit, Attr::Disabled)
            .and(if self.loading { "..." } else { "Import" })
            .and_class("is-primary")
            .attr(Attr::Title, "Import without previewing first")
            .on_click(ctx, || Msg::Import);

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
