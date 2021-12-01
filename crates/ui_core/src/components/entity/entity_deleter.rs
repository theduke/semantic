use brass::{
    component::msg::MsgComponent,
    dom::{builder::div, ClickEvent, TagBuilder, View},
    signal::signal::Mutable,
};
use factordb::{
    query::{mutate::Mutate, select::Item},
    schema::AttrMapExt,
    AnyError, Id,
};
use semantic_core::base::entity_title;

use crate::{
    components::{
        loader::{spinner, LoadState},
        util::{button, notification_warning, ButtonGroupBuilder, NotificationBuilder},
    },
    context::api,
};

pub struct EntityDeleter {
    pub item: Item,
    pub on_delete: Box<dyn Fn()>,
    pub on_cancel: Box<dyn Fn()>,
}

impl brass::dom::Render for EntityDeleter {
    fn render(self) -> View {
        brass::component::build_component::<State>(self)
    }
}

struct State {
    props: EntityDeleter,
    id: Option<Id>,
    state: Mutable<LoadState<()>>,
}

enum Msg {
    Confirm,
    Cancel,
    Loaded(Result<(), AnyError>),
}

impl MsgComponent for State {
    type Properties = EntityDeleter;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: brass::component::Context<'_, Self>) -> Self {
        Self {
            id: props.item.data.get_id(),
            props,
            state: Mutable::new(LoadState::Idle),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: brass::component::Context<Self>) {
        match msg {
            Msg::Confirm => {
                if let Some(id) = self.id {
                    let guard = ctx.spawn_map(
                        async move { api().mutate(Mutate::delete(id)).await },
                        Msg::Loaded,
                    );
                    self.state.set(LoadState::Loading(Some(guard)));
                }
            }
            Msg::Cancel => {
                (self.props.on_cancel)();
            }
            Msg::Loaded(res) => match res {
                Ok(_) => (self.props.on_delete)(),
                Err(err) => {
                    self.state.set(LoadState::Failed(err.to_string()));
                }
            },
        }
    }

    fn render(&mut self, ctx: brass::component::Context<'_, Self>) -> TagBuilder {
        if self.props.item.data.get_id().is_some() {
            let title = entity_title(&self.props.item.data);

            let handle = ctx.handle();
            let signal = self.state.signal_ref(move |state| match state {
                LoadState::Idle => NotificationBuilder::new()
                    .warning(format!("Really delete '{}'", title))
                    .buttons(
                        ButtonGroupBuilder::new()
                            .button_danger("Delete", handle.callback(|| Msg::Confirm))
                            .button_default("Cancel", handle.callback(|| Msg::Cancel)),
                    )
                    .build(),
                LoadState::Loading(_) => spinner(),
                LoadState::Success(_) => div(),
                LoadState::Failed(err) => NotificationBuilder::new()
                    .error(err.as_str())
                    .buttons(
                        ButtonGroupBuilder::new()
                            .button_default("Cancel", handle.callback(|| Msg::Cancel)),
                    )
                    .build(),
            });

            div().child_signal(signal)
        } else {
            notification_warning().and((
                div().and("Can't delete entity without an ID."),
                button()
                    .and("Cancel")
                    .on(ctx.on(|_: ClickEvent| Msg::Cancel)),
            ))
        }
    }
}
