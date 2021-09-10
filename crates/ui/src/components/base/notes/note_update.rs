use brass::{
    vdom::{self, component},
    Callback,
};
use factordb::{data::value::from_value_map, schema::EntityContainer, AnyError};
use semantic_core::base::Note;

use semantic_ui_core::{loader::LoadState, ContextExt};

use super::note_form::{NoteForm, NoteFormProps};

pub struct NoteUpateProps {
    pub note: Note,
}

pub enum Msg {
    Submit(Note),
    Loaded(Result<(), AnyError>),
}

pub struct NoteUpate {
    note: Note,
    loader: LoadState<()>,
    callback: Callback<Note>,
}

brass::enable_props!(NoteUpateProps => NoteUpate);

impl brass::Component for NoteUpate {
    type Properties = NoteUpateProps;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            note: props.note,
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
                    api.mutate(factordb::query::mutate::Mutate::merge_from_map(data).unwrap())
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
            note: Some(self.note.clone()),
            on_submit: self.callback.clone(),
        });
        let loader = self
            .loader
            .render(|_| brass_bulma::notification_success("Note saved").build());
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

pub fn note_update(
    item: &factordb::query::select::Item,
    _opts: &semantic_ui_core::EntityRenderOpts,
) -> vdom::VNode {
    match from_value_map(item.data.clone()) {
        Ok(note) => component::<NoteUpate>(NoteUpateProps { note }),
        Err(_err) => vdom::div().and("Invalid Note").build(),
    }
}
