use std::rc::Rc;

use brass::{vdom::Render, VNode};
use factordb::{
    query::{
        mutate::{BatchUpdate, Mutate},
        select::Item,
    },
    schema::EntityContainer,
};
use semantic_ui_core::{
    entity::{
        create_page::CreatePage,
        persister::{EntityFormProps, FormValid},
    },
    EntityRenderOpts,
};
use semantics_core::base::Habit;

use super::habit_form::HabitForm;

pub fn habit_create(_item: &Item, _opts: &EntityRenderOpts) -> VNode {
    CreatePage {
        render: Rc::new(|props: &EntityFormProps| {
            HabitForm {
                habit: semantics_core::base::Habit {
                    id: factordb::Id::nil(),
                    title: String::new(),
                    description: None,
                    mode: semantics_core::base::HabitMode::Neutral,
                },
                on_submit: props.on_valid.clone().map(|habit: Habit| {
                    let id = habit.id;
                    let item = Item::new(habit.into_map().unwrap());

                    FormValid {
                        mutation: BatchUpdate::with_action(Mutate::create(id, item.data.clone())),
                        item,
                    }
                }),
            }
            .render()
        }),
    }
    .render()
}
