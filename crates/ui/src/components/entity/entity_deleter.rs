use brass::{vdom, Callback};
use factordb::{query::mutate::Mutate, AnyError};

use semantic_ui_core::{loader::LoadState, ContextExt};

pub struct EntityDeleterProps {
    pub id: factordb::Id,
    pub title: String,
    pub modal: bool,
    pub on_cancel: Option<Callback<()>>,
    pub on_success: Callback<()>,
}

pub enum Msg {
    Submit,
    Cancel,
    Loaded(Result<(), AnyError>),
}

pub struct EntityDeleter {
    props: EntityDeleterProps,
    loader: LoadState<()>,
}

impl brass::Component for EntityDeleter {
    type Properties = EntityDeleterProps;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            props,
            loader: LoadState::Idle,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Submit => {
                let id = self.props.id;
                let api = ctx.api().clone();
                let f = async move { api.mutate(Mutate::delete(id)).await };
                let guard = ctx.run_map(f, Msg::Loaded);
                self.loader.set_loading_guarded(guard);
            }
            Msg::Cancel => {
                if let Some(callback) = &self.props.on_cancel {
                    callback.send(());
                }
            }
            Msg::Loaded(res) => {
                self.props.on_success.send(());
                self.loader.set_result(res);
            }
        }
    }

    fn render(&self, ctx: &mut brass::RenderContext<Self>) -> brass::VNode {
        let title = brass_bulma::h5_with(format!("Delete {}", self.props.title));

        let btn_submit = brass_bulma::button_medium()
            .and("Delete")
            .and_class(brass_bulma::Color::Danger.as_class())
            .on_click(ctx, || Msg::Submit);

        let btn_cancel = if self.props.on_cancel.is_some() {
            Some(
                brass_bulma::button_medium()
                    .and("Cancel")
                    .on_click(ctx, || Msg::Cancel),
            )
        } else {
            None
        };

        let actions = vdom::div()
            .class("buttons")
            .and(btn_submit)
            .and_opt(btn_cancel);

        vdom::div().and(title).and(actions).build()
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        if props.id != self.props.id {
            *self = Self::init(props, ctx);
            true
        } else {
            false
        }
    }
}
