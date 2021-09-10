use brass::{
    vdom::{self, component},
    Callback,
};
use factordb::{schema::EntityContainer, AnyError};
use semantic_core::base::Note;

use semantic_ui_core::{loader::LoadState, ContextExt};

use super::note_form::{NoteForm, NoteFormProps};

pub struct NoteCreateProps {}

pub enum Msg {
    Submit(Note),
    Loaded(Result<(), AnyError>),
}

pub struct NoteCreate {
    loader: LoadState<()>,
    callback: Callback<Note>,
}

impl brass::Component for NoteCreate {
    type Properties = NoteCreateProps;
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            loader: LoadState::Idle,
            callback: ctx.callback_map(Msg::Submit),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Submit(item) => {
                // TODO: no unwrap, move to helper method.
                let data = item.into_map().unwrap();

                let api = ctx.api().clone();
                let f = async move {
                    api.mutate(factordb::query::mutate::Mutate::create_from_map(data))
                        .await
                };

                let guard = ctx.run_map(f, Msg::Loaded);
                self.loader.set_loading_guarded(guard);
            }
            Msg::Loaded(res) => {
                if res.is_ok() {
                    ctx.router().goto(semantic_ui_core::routing::Route::Browse);
                } else {
                    self.loader.set_result(res);
                }
            }
        }
    }

    fn render(&self, _ctx: &mut brass::RenderContext<Self>) -> brass::VNode {
        let form = vdom::component::<NoteForm>(NoteFormProps {
            note: None,
            on_submit: self.callback.clone(),
        });
        let loader = self
            .loader
            .render(|_| brass_bulma::notification_success("Entity created").build());
        vdom::div().and((loader, form)).build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        false
    }
}

pub fn note_create(
    _data: &factordb::query::select::Item,
    _opts: &semantic_ui_core::EntityRenderOpts,
) -> vdom::VNode {
    component::<NoteCreate>(NoteCreateProps {})
}
