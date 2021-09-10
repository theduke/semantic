use brass::{
    vdom::{div, event::ClickEvent, Render},
    Callback,
};
use factordb::Id;
use semantic_core::base::Note;

pub struct NoteFormProps {
    pub note: Option<Note>,
    pub on_submit: Callback<Note>,
}

pub enum Msg {
    Title(String),
    Body(String),
    Submit,
}

pub struct NoteForm {
    note: Note,
    on_submit: Callback<Note>,
}

impl brass::Component for NoteForm {
    type Properties = NoteFormProps;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        let note = props.note.unwrap_or_else(|| Note {
            id: Id::from_uuid(uuid::Uuid::new_v4()),
            title: String::new(),
            body: String::new(),
            extra: Default::default(),
        });

        Self {
            note,
            on_submit: props.on_submit,
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Title(title) => {
                self.note.title = title;
            }
            Msg::Body(body) => {
                self.note.body = body;
            }
            Msg::Submit => {
                self.on_submit.send(self.note.clone());
            }
        }
    }

    fn render(&self, ctx: &mut brass::RenderContext<Self>) -> brass::VNode {
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
            .on(ctx, |_: ClickEvent| Msg::Submit);
        let submit = brass_bulma::field().and(brass_bulma::control().and(submit_btn));

        div().and((title, body, preview, submit)).build()
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        if let Some(note) = &props.note {
            if note.id != self.note.id {
                *self = Self::init(props, ctx);
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}
