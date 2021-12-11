use brass::dom::TagBuilder;
use semantic_ui_core::{
    components::{
        form::{self, FormLoadFuture},
        util::{
            form_field_input, form_field_select, form_field_textarea, FormRenderer, SelectOption,
        },
    },
    validate::StringRequired,
};

use crate::habits::{Habit, HabitMode};

#[derive(Clone)]
pub struct Values {
    pub title: String,
    pub description: String,
    pub mode: HabitMode,
}

impl Values {
    pub fn apply(self, h: &mut Habit) {
        h.title = self.title;
        h.description = if self.description.trim().is_empty() {
            None
        } else {
            Some(self.description.trim().to_string())
        };
        h.mode = self.mode;
    }
}

pub fn habit_form(
    habit: &Habit,
    on_submit_async: impl Fn(Values) -> FormLoadFuture + 'static,
) -> TagBuilder {
    form::Form::new(Values {
        title: habit.title.clone(),
        description: habit.description.clone().unwrap_or_default(),
        mode: habit.mode,
    })
    .on_submit_async(move |values| on_submit_async(values.clone()))
    .render(move |handle| {
        let title = form_field_input(
            "Title",
            handle.field_validated(|v| &mut v.title, StringRequired),
        );

        let description = form_field_textarea(
            "Description",
            handle.field(|v| &mut v.description),
            3,
            false,
        );

        let mode = form_field_select(
            "Mode",
            vec![
                SelectOption {
                    label: "Positive".to_string(),
                    value: HabitMode::Positive,
                },
                SelectOption {
                    label: "Negative".to_string(),
                    value: HabitMode::Negative,
                },
                SelectOption {
                    label: "Neutral".to_string(),
                    value: HabitMode::Neutral,
                },
            ],
            handle.field(|v| &mut v.mode),
        );

        FormRenderer::new(handle.clone())
            .and(title)
            .and(description)
            .and(mode)
            .buttons_submit("Save")
    })
}
