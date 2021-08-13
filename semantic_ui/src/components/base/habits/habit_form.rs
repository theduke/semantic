use std::rc::Rc;

use brass::{
    vdom::{self, div, Render},
    Callback, VNode,
};
use semantic_ui_core::components::form::{self, Field, InputField};
use semantics_core::base::{Habit, HabitMode};

pub struct HabitForm {
    pub habit: Habit,
    pub on_submit: Callback<Habit>,
}

enum Msg {
    Title(String),
    Description(String),
    Mode(HabitMode),
    Submit,
}

struct HabitFormComp {
    habit: Habit,
    mode_callback: Callback<Option<HabitMode>>,
    on_submit: Callback<Habit>,
}

brass::enable_props!(HabitForm => HabitFormComp);

impl brass::Component for HabitFormComp {
    type Properties = HabitForm;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            habit: props.habit,
            mode_callback: ctx.callback_map(|mode: Option<HabitMode>| {
                Msg::Mode(mode.unwrap_or(HabitMode::Neutral))
            }),
            on_submit: props.on_submit,
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Title(val) => {
                self.habit.title = val;
            }
            Msg::Description(value) => self.habit.description = Some(value),
            Msg::Mode(mode) => {
                self.habit.mode = mode;
            }
            Msg::Submit => {
                self.habit.title = self.habit.title.trim().to_string();
                if let Some(val) = self.habit.description.as_mut() {
                    *val = val.trim().to_string();
                }

                let is_valid = !self.habit.title.is_empty();

                if is_valid {
                    self.on_submit.send(self.habit.clone());
                }
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        let title = brass_bulma::FieldHorizontal {
            label: "Title".into(),
            help: None,
            control: brass_bulma::Input {
                _type: "text".into(),
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.habit.title.clone(),
                on_input: ctx.on(|ev: web_sys::Event| {
                    let val = brass::util::input_event_value(ev).unwrap();
                    Msg::Title(val)
                }),
            },
        };

        let description = brass_bulma::FieldHorizontal {
            label: "Description".into(),
            help: None,
            control: brass_bulma::Textarea {
                color: brass_bulma::Color::Default,
                placeholder: None,
                value: self.habit.description.clone().unwrap_or_default(),
                on_input: ctx.on(|ev: web_sys::Event| {
                    let val = brass::util::textarea_input_value(ev).unwrap();
                    Msg::Description(val)
                }),
                on_keydown: None,
                style_raw: None,
            },
        };

        let mode = brass_bulma::FieldHorizontal {
            label: "Mode".into(),
            help: None,
            control: brass_bulma::Select {
                value: Some(self.habit.mode),
                options: Rc::new(vec![
                    brass_bulma::SelectOption {
                        label: "Neutral".into(),
                        value: HabitMode::Neutral,
                    },
                    brass_bulma::SelectOption {
                        label: "Negative".into(),
                        value: HabitMode::Negative,
                    },
                    brass_bulma::SelectOption {
                        label: "Positive".into(),
                        value: HabitMode::Positive,
                    },
                ]),
                on_select: self.mode_callback.clone(),
            },
        };

        let submit = brass_bulma::button()
            .and("Submit")
            .on_click(ctx.on_simple(|| Msg::Submit));
        let actions = brass_bulma::buttons().and(submit);

        div().and((title, description, mode, actions)).build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        // FIXME: this will lead to broken components...
        false
    }
}

pub struct HabitForm2 {
    pub habit: Habit,
    pub on_submit: Callback<Habit>,
}

pub fn habit_form(habit: Habit, on_submit: Callback<Habit>) -> VNode {
    form::Form {
        on_submit,
        initial_values: habit,
        render: Rc::new(|mut state| {
            vdom::div()
                .and(state.field(InputField::<Habit> {
                    name: "title".to_string(),
                    get: |h| &h.title,
                    set: |v, h| {
                        h.title = v;
                    },
                    validate: None,
                    label: "Title".to_string(),
                    help: None,
                    placeholder: None,
                }))
                .and(state.field(InputField::<Habit> {
                    name: "description".to_string(),
                    get: |h| &h.title,
                    set: |v, h| {
                        h.title = v;
                    },
                    validate: None,
                    label: "Description".to_string(),
                    help: None,
                    placeholder: None,
                }))
                .build()
        }),
    }
    .render()
}
