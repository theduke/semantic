use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Attr, Render, Style, Tag, TagBuilder, View},
    effect::{set_interval, IntervalGuard},
    signal::signal::Mutable,
};
use factdb::query::select::Page;
use semantic_ui_core::{
    components::{
        loader::Loader,
        util::{notification_default, notification_error, ButtonBuilder, Color},
    },
    context,
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
    TriggerLoaded(Result<HabitOccurence, anyhow::Error>),
    OccurencesLoaded(Result<Vec<HabitOccurence>, anyhow::Error>),
}

struct State {
    habit: Habit,
    trigger_loading: Loader<()>,
    list_loader: Loader<()>,
    occurences: Mutable<Page<HabitOccurence>>,

    timer_guard: Option<IntervalGuard>,
    timer: Mutable<String>,
}

impl State {
    fn start_timer(&mut self, last_ocurrence: HabitOccurence) {
        fn build_timer(oc: &HabitOccurence) -> String {
            let secs = time::OffsetDateTime::now_utc()
                .unix_timestamp()
                .saturating_sub(oc.time.to_datetime().unix_timestamp());
            format!(
                "{:02}:{:02}:{:02}",
                secs / (60 * 60),
                secs / 60 % 60,
                secs % 60
            )
        }

        self.timer.set(build_timer(&last_ocurrence));

        let timer = self.timer.clone();
        self.timer_guard = Some(set_interval(std::time::Duration::from_secs(1), move || {
            timer.set(build_timer(&last_ocurrence));
        }));
    }
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
                        .select_entities(HabitOccurence::query_for_habit(id).with_limit(1000))
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
            occurences: Mutable::new(Page::new()),
            timer_guard: None,
            timer: Mutable::new(String::new()),
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
                    Msg::TriggerLoaded(res)
                });
                self.trigger_loading.set_loading(guard);
            }
            Msg::TriggerLoaded(res) => match res {
                Ok(oc) => {
                    self.trigger_loading.set_success(());
                    self.start_timer(oc.clone());
                    self.occurences.lock_mut().items.push(oc);
                }
                Err(err) => {
                    self.trigger_loading.set_err(err);
                }
            },
            Msg::OccurencesLoaded(res) => {
                match res {
                    Ok(items) => {
                        if let Some(latest) = items.first() {
                            self.start_timer(latest.clone());
                        }

                        // TODO: append to current page instead of replacing?
                        // (to respect new entries already added on current page)
                        self.list_loader.set_success(());
                        self.occurences.lock_mut().items.extend(items);
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

        let title = if self.habit.title.trim().is_empty() {
            format!("Habit {}", self.habit.id)
        } else {
            self.habit.title.clone()
        };

        let habit = self.habit.clone();

        let toggle_button = ButtonBuilder::new()
            .size_large()
            .icon(icon)
            .signal_loading(self.trigger_loading.signal_loading())
            .color(color)
            .on(ctx.callback_msg(|| Msg::Trigger))
            .build();

        div()
            .and(div().class("mb-2").and(Tag::B.new().text(title)))
            .and(
                div()
                    .class("is-flex")
                    .and(div().class("mr-4").and(toggle_button))
                    .signal(self.timer.signal_ref(|time| {
                        div()
                            .style_raw("align-items: center; font-size: 2rem;")
                            .and(time)
                    })),
            )
            .signal(self.trigger_loading.get().signal_ref(|s| {
                s.as_error()
                    .map(|err| notification_error().class("mt-4").and(err).into_view())
                    .unwrap_or(View::Empty)
            }))
            .and(
                div().class("mt-4").class("mb-4").signal(
                    self.occurences
                        .signal_ref(move |page| render_graph(&habit, &page.items)),
                ),
            )
            .signal(self.list_loader.signal_render_loading())
    }
}

fn render_graph(_habit: &Habit, items: &[HabitOccurence]) -> TagBuilder {
    if items.is_empty() {
        return notification_default().and("No records yet.");
    }

    let mut items = items.iter().collect::<Vec<_>>();
    items.sort_by(|a, b| a.time.cmp(&b.time));

    let mut date_counter = Vec::<(time::Date, usize)>::new();

    // NOTE: items are assumed to be sorted by time (asc) already.

    for item in items.iter() {
        let date = item.time.to_datetime().date();

        match date_counter.last_mut() {
            Some((cur_date, counter)) if cur_date == &date => {
                *counter = *counter + 1;
            }
            _ => {
                date_counter.push((date, 1));
            }
        }
    }

    // let min_date = items.first().unwrap().time.to_datetime().date().naive_utc();
    // let max_date =
    //     items.last().unwrap().time.to_datetime().date().naive_utc() + chrono::Duration::days(1);

    let max_count = date_counter
        .iter()
        .map(|(_, x)| *x)
        .max()
        .unwrap_or_default();
    let max_x = max_count + (5 - max_count % 5);

    let mut x_labels = Vec::new();
    for x in (0..=max_x).rev() {
        if x == 0 || x == max_x || ((x < max_x - 5) && x % 5 == 0) {
            x_labels.push(div().and(x.to_string()));
        }
    }

    let x_axis = div()
        .class("is-flex")
        .class("is-justify-content-space-between")
        .class("is-flex-direction-column")
        .class("mr-3")
        .and_iter(x_labels);

    let bars = date_counter.into_iter().map(|(date, count)| {
        let date_formatted = date
            .format(&time::format_description::parse("[year]-[month]-[day]").unwrap())
            .unwrap_or_default();
        div()
            .style(Style::Width, "20px")
            .style(Style::Height, format!("{}%", count * 100 / max_x))
            .style(Style::BackgroundColor, "red")
            .attr(
                Attr::Title,
                format!("Count: {count} - Date: {date_formatted}"),
            )
    });

    let bar_wrap = div()
        .class("is-flex")
        .class("is-flex-grow-1")
        .class("is-align-items-end")
        .and_iter(bars);

    div()
        .style(Style::Height, "400px")
        .class("is-flex")
        .tag(x_axis)
        .tag(bar_wrap)
}
