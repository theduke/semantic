use brass::dom::{builder::div, TagBuilder};
use factordb::{query::select::Item, Id};
use semantic_ui_core::{context, routing::Route, EntityRenderOpts};

use crate::habits::{Habit, HabitMode};

use super::habit_form::habit_form;

pub fn habit_create() -> TagBuilder {
    let habit = Habit {
        id: Id::random(),
        title: String::new(),
        description: None,
        mode: HabitMode::Neutral,
        extra: Default::default(),
    };

    habit_form(&habit.clone(), move |values| {
        let mut habit = habit.clone();
        values.apply(&mut habit);

        Box::pin(async move {
            context::api().entity_create(habit).await?;
            context::router().goto(Route::Plugin(super::HabitRouter::route_dashboard()));
            Ok(())
        })
    })
}

pub fn habit_create_page(_: &Item, _: &EntityRenderOpts) -> TagBuilder {
    div().and(habit_create())
}
