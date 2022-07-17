use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Render, View},
};
use factordb::{schema::EntityDescriptor, AnyError};
use semantic_ui_core::{
    components::{
        loader::Loader,
        util::{box_, buttons, notification_warning},
    },
    context,
    routing::link_with_class,
};

use super::{super::Habit, habit_view::HabitView};

pub struct HabitDashboard {}

impl Render for HabitDashboard {
    fn render(self) -> View {
        State::build(self)
    }
}

enum Msg {
    HabitsLoaded(Result<Vec<Habit>, AnyError>),
}

struct State {
    page: Loader<Vec<Habit>>,
}

impl MsgComponent for State {
    type Properties = HabitDashboard;
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: brass::component::Context<Self>) -> Self {
        Self {
            page: Loader::new_loading(ctx.spawn_map(
                async {
                    context::api()
                        .select_entities::<Habit>(Habit::query_all())
                        .await
                },
                Msg::HabitsLoaded,
            )),
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: brass::component::Context<Self>) {
        match msg {
            Msg::HabitsLoaded(res) => {
                self.page.set_result(res);
            }
        }
    }

    fn render(&mut self, _ctx: brass::component::Context<Self>) -> brass::dom::TagBuilder {
        div()
            .and(buttons().and(link_with_class(
                semantic_ui_core::routing::Route::EntityCreate {
                    entity_type: Habit::QUALIFIED_NAME.to_string(),
                },
                "New Habit",
                "button is-large",
            )))
            .signal(self.page.signal_render(|items| {
                if items.is_empty() {
                    notification_warning()
                        .text("No habits found. Why don't you create a new one?")
                        .into_view()
                } else {
                    let items = items.iter().map(|habit| {
                        box_()
                            .and(HabitView {
                                habit: habit.clone(),
                                autoload_occurences: true,
                            })
                            .into_view()
                    });
                    div().and_iter(items).into_view()
                }
            }))
    }
}
