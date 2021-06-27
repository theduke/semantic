use brass::{vdom::div, Callback};
use factordb::Id;
use semantics_core::base::Note;

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
            id: Id::random(),
            title: String::new(),
            body: String::new(),
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

    fn render(&self, _ctx: brass::RenderContext<Self>) -> brass::VNode {
        let title = brass_bulma::Field {
            label: "Title".into(),
            help: None,
            control: brass_bulma::Input {
                _type: "text",
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.note.title.clone(),
                on_input: _ctx.callback(|ev: web_sys::Event| {
                    let value = brass::util::input_event_value(ev).unwrap();
                    Msg::Title(value)
                }),
            },
        };

        let body = brass_bulma::Field {
            label: "Body".into(),
            help: None,
            control: brass_bulma::Textarea {
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.note.body.clone(),
                on_input: _ctx.callback(|ev: web_sys::Event| {
                    let value = brass::util::textarea_input_value(ev).unwrap();
                    Msg::Body(value)
                }),
            },
        };

        let submit_btn = brass_bulma::button().and("Submit").on(
            brass::dom::Event::Click,
            _ctx.callback_ignore_event(|| Msg::Submit),
        );
        let submit = brass_bulma::field().and(brass_bulma::control().and(submit_btn));

        div().and(title).and(body).and(submit).build()
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
