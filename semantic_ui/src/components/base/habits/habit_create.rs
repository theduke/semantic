use std::rc::Rc;

use brass::{vdom::Render, VNode};
use factordb::{
    query::{
        mutate::{BatchUpdate, Mutate},
        select::Item,
    },
    schema::EntityContainer,
};
use semantic_core::base::Habit;
use semantic_ui_core::{
    entity::{
        create_page::CreatePage,
        persister::{EntityFormProps, FormValid},
    },
    EntityRenderOpts,
};

use super::habit_form::habit_form;

pub fn habit_create(_item: &Item, _opts: &EntityRenderOpts) -> VNode {
    CreatePage {
        render: Rc::new(|props: &EntityFormProps| {
            let habit = semantic_core::base::Habit {
                id: factordb::Id::nil(),
                title: String::new(),
                description: None,
                mode: semantic_core::base::HabitMode::Neutral,
                extra: Default::default(),
            };

            let on_submit = props.on_submit.clone().map(|habit: Habit| {
                let id = habit.id;
                let item = Item::new(habit.into_map().unwrap());

                FormValid {
                    mutation: BatchUpdate::with_action(Mutate::create(id, item.data.clone())),
                    item,
                }
            });

            habit_form(habit, on_submit)
        }),
    }
    .render()
}
