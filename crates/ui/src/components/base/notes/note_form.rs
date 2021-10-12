use brass::{
    dom::Attr,
    vdom::{div, event::ClickEvent, Render},
    Callback,
};
use factordb::Id;
use semantic_core::base::Note;

pub struct NoteForm {
    pub note: Option<Note>,
    pub on_submit: Callback<Note>,
    pub on_change: Option<Callback<Note>>,
    pub loading: bool,
}

enum Msg {
    Title(String),
    Body(String),
    Debounced,
    Submit,
}

brass::enable_props!(wrapped NoteForm => State);

struct State {
    note: Note,
    debounce_guard: Option<brass::EffectGuard>,
}

impl State {
    fn on_change(&mut self, props: &NoteForm, ctx: &mut brass::Context<Msg>) {
        if props.on_change.is_some() {
            self.debounce_guard =
                Some(ctx.timeout(Msg::Debounced, std::time::Duration::from_secs(1)));
        }
    }
}

impl brass::PropComponent for State {
    type Properties = NoteForm;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let note = props.note.clone().unwrap_or_else(|| Note {
            id: Id::from_uuid(uuid::Uuid::new_v4()),
            title: String::new(),
            body: String::new(),
            extra: Default::default(),
        });

        Self {
            note,
            debounce_guard: None,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Title(title) => {
                if props.loading {
                    return;
                }
                let has_changed = title.trim() != self.note.title.trim();
                self.note.title = title;
                if has_changed {
                    self.on_change(props, ctx);
                }
            }
            Msg::Body(body) => {
                if props.loading {
                    return;
                }
                let has_changed = body.trim() != self.note.body.trim();
                self.note.body = body;
                if has_changed {
                    self.on_change(props, ctx);
                }
            }
            Msg::Debounced => {
                if let Some(cb) = &props.on_change {
                    cb.send(self.note.clone());
                }
            }
            Msg::Submit => {
                if props.loading {
                    return;
                }
                self.debounce_guard = None;
                props.on_submit.send(self.note.clone());
            }
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: &mut brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        let title = brass_bulma::Field {
            label: "Title".into(),
            help: None,
            control: brass_bulma::Input {
                _type: "text",
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.note.title.clone().into(),
                on_input: ctx.callback_map(Msg::Title),
            },
        };

        let body = brass_bulma::Field {
            label: "Body".into(),
            help: None,
            control: brass_bulma::Textarea {
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.note.body.clone().into(),
                on_input: ctx.callback_map(Msg::Body),
                on_keydown: None,
                style_raw: Some("min-height: 400px;".into()),
            },
        };

        let markdown = semantic_ui_core::components::markdown::Markdown {
            markdown: self.note.body.clone(),
        }
        .render();
        let preview_content = div()
            .and(markdown)
            .style_raw("border: 1px solid black; border-radius: 10px; padding: 0.5rem;");
        let preview = brass_bulma::Field {
            label: "Preview".into(),
            help: None,
            control: preview_content,
        };

        let submit_btn = brass_bulma::button()
            .and("Submit")
            .attr_toggle_if(props.loading, Attr::Disabled)
            .on(ctx, |_: ClickEvent| Msg::Submit);
        let submit = brass_bulma::field().and(brass_bulma::control().and(submit_btn));

        div().and((title, body, preview, submit)).build()
    }
}
