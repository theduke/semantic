use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Render, Tag, View},
    signal::signal_vec::MutableVec,
};
use factordb::{query::select::Page, AnyError};
use semantic_ui_core::{
    components::{
        loader::Loader,
        util::{notification_default, notification_error, ButtonBuilder, Color},
    },
    context, datetime_to_locale_string_js,
};

use crate::habits::{Habit, HabitMode, HabitOccurence};

pub struct HabitView {
    pub habit: Habit,
    pub autoload_occurences: bool,
}

impl Render for HabitView {
    fn render(self) -> View {
        State::build(self)
    }
}

enum Msg {
    Trigger,
    TriggerLoad(Result<HabitOccurence, AnyError>),
    OccurencesLoaded(Result<Page<HabitOccurence>, AnyError>),
}

struct State {
    habit: Habit,
    trigger_loading: Loader<()>,
    list_loader: Loader<()>,
    occurences: MutableVec<HabitOccurence>,
}

impl MsgComponent for State {
    type Properties = HabitView;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: brass::component::Context<Self>) -> Self {
        let list_loader = if props.autoload_occurences {
            let id = props.habit.id;
            Loader::new_loading(_ctx.spawn_map(
                async move {
                    context::api()
                        .select_entities(HabitOccurence::query_for_habit(id))
                        .await
                },
                Msg::OccurencesLoaded,
            ))
        } else {
            Loader::new_idle()
        };
        Self {
            habit: props.habit,
            trigger_loading: Loader::new_idle(),
            list_loader,
            occurences: MutableVec::new(),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: brass::component::Context<Self>) {
        match msg {
            Msg::Trigger => {
                if self.trigger_loading.is_loading() {
                    return;
                }

                let occurence = self.habit.new_ocurrence(semantic_ui_core::now());
                let guard = ctx.spawn(async move {
                    let res = context::api()
                        .entity_create(occurence.clone())
                        .await
                        .map(move |_| occurence);
                    Msg::TriggerLoad(res)
                });
                self.trigger_loading.set_loading(guard);
            }
            Msg::TriggerLoad(res) => match res {
                Ok(oc) => {
                    self.trigger_loading.set_success(());
                    self.occurences.lock_mut().insert_cloned(0, oc);
                }
                Err(err) => {
                    self.trigger_loading.set_err(err);
                }
            },
            Msg::OccurencesLoaded(res) => {
                match res {
                    Ok(page) => {
                        // TODO: append to current page instead of replacing?
                        // (to respect new entries already added on current page)
                        self.list_loader.set_success(());
                        self.occurences.lock_mut().extend(page.items);
                    }
                    Err(err) => {
                        self.list_loader.set_err(err);
                    }
                }
            }
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> brass::dom::TagBuilder {
        let (color, icon) = match self.habit.mode {
            HabitMode::Positive => (Color::Success, "fas fa-thumbs-up"),
            HabitMode::Negative => (Color::Danger, "fas fa-thumbs-down"),
            HabitMode::Neutral => (Color::Primary, "far fa-dot-circle"),
        };

        div()
            .and(div().class("mb-2").and(Tag::B.new().and(&self.habit.title)))
            .and(
                div().and(
                    ButtonBuilder::new()
                        .size_large()
                        .icon(icon)
                        .signal_loading(self.trigger_loading.signal_loading())
                        .color(color)
                        .on(ctx.callback_msg(|| Msg::Trigger))
                        .build(),
                ),
            )
            .signal(self.trigger_loading.get().signal_ref(|s| {
                s.as_error()
                    .map(|err| notification_error().class("mt-4").and(err).into_view())
                    .unwrap_or(View::Empty)
            }))
            .and(div().class("mt-4").class("mb-4").signal_vec_with_fallback(
                self.occurences.signal_vec_cloned(),
                |oc| {
                    let dt = oc.time.to_datetime();
                    div()
                        .class("mb-2")
                        .and(datetime_to_locale_string_js(&dt))
                        .build()
                },
                notification_default().and("No records yet."),
            ))
            .signal(self.list_loader.signal_render_loading())
    }
}
