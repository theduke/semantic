use std::rc::Rc;

use brass::{
    vdom::{self, s, Render},
    Callback, VNode,
};
use semantic_ui_core::components::form::{self, InputField, SelectField};
use semantic_core::base::{Habit, HabitMode};

pub fn habit_form(habit: Habit, on_submit: Callback<Habit>) -> VNode {
    form::Form {
        on_submit,
        initial_values: habit,
        render: Rc::new(|mut state| {
            vdom::div()
                // Title.
                .and(state.field(InputField::<Habit> {
                    name: s("title"),
                    get: |h| &h.title,
                    set: |v, h| {
                        h.title = v;
                    },
                    validate: None,
                    label: s("Title"),
                    help: None,
                    placeholder: None,
                }))
                // Description
                .and(state.field(InputField::<Habit> {
                    name: s("description"),
                    get: |h| &h.title,
                    set: |v, h| {
                        h.title = v;
                    },
                    validate: None,
                    label: s("Description"),
                    help: None,
                    placeholder: None,
                }))
                // Mode
                .and(state.field(SelectField::<Habit, HabitMode> {
                    name: s("mode"),
                    get: |h| &h.mode,
                    set: |v, h| {
                        h.mode = v;
                    },
                    validate: None,
                    label: s("Description"),
                    help: None,
                    // FIXME: don't clone...
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
                }))
                // Actions
                .and(
                    brass_bulma::buttons().and(
                        brass_bulma::button()
                            .and_class("is-primary")
                            .and("Save")
                            .on_click(state.submit().on(|_| ())),
                    ),
                )
                .build()
        }),
    }
    .render()
}
